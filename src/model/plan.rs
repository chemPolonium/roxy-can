//! Flattened decode plans.
//!
//! A plan is built once after import and then used per frame: the hot path
//! never walks the model tree. Container and multiplexed PDUs are expanded
//! here too, so their versioned cache is the only thing Phase 6 has to touch.

use std::collections::BTreeMap;
use std::sync::Arc;

use super::*;

/// One immutable plan set for a communication model.
#[derive(Clone, Debug, Default)]
pub struct PlanCache {
    pub version: u32,
    plans: BTreeMap<FrameId, Arc<DecodePlan>>,
}

impl PlanCache {
    pub fn build(comm: &Comm, version: u32) -> (Self, Vec<Issue>) {
        let mut plans = BTreeMap::new();
        let mut issues = Vec::new();
        for frame in comm.frames.values() {
            let (plan, mut frame_issues) = build_decode_plan(comm, frame, version);
            issues.append(&mut frame_issues);
            plans.insert(frame.id, Arc::new(plan));
        }
        (Self { version, plans }, issues)
    }

    pub fn get(&self, frame: FrameId) -> Option<&Arc<DecodePlan>> {
        self.plans.get(&frame)
    }

    pub fn len(&self) -> usize {
        self.plans.len()
    }
}

pub fn build_decode_plan(comm: &Comm, frame: &Frame, version: u32) -> (DecodePlan, Vec<Issue>) {
    let mut reads = Vec::new();
    let mut issues = Vec::new();
    // A logged FlexRay payload begins at the frame's first PDU, while AUTOSAR
    // START-POSITIONs are measured from the frame head. Rebase once here so
    // the plan's offsets are payload offsets, matching the FIBEX decoder.
    let frame_base = if matches!(frame.address, FrameAddress::FlexRay { .. }) {
        frame
            .pdu_mappings
            .iter()
            .filter_map(|id| comm.frame_pdu_mappings.get(id))
            .map(|mapping| mapping.start_bit)
            .min()
            .unwrap_or(0)
    } else {
        0
    };
    for mapping_id in &frame.pdu_mappings {
        let Some(mapping) = comm.frame_pdu_mappings.get(mapping_id) else {
            issues.push(Issue::error(
                IssueKind::DanglingRef,
                format!("frame {} refers to a missing PDU mapping", frame.name),
            ));
            continue;
        };
        let Some(pdu) = comm.pdus.get(&mapping.pdu) else {
            issues.push(Issue::error(
                IssueKind::DanglingRef,
                format!("frame {} refers to a missing PDU", frame.name),
            ));
            continue;
        };
        let base = mapping.start_bit.saturating_sub(frame_base);
        match pdu.kind {
            PduKind::Signal => {
                expand_pdu(comm, pdu.id, base, None, None, &mut issues, &mut reads);
            }
            PduKind::Container => {
                for contained in comm
                    .contained_pdu_mappings
                    .values()
                    .filter(|m| m.container == pdu.id)
                {
                    // The header names which contained PDU travels in this
                    // frame; an unassigned id keeps the old "read wherever it
                    // sits" behaviour rather than guessing.
                    let gate = (pdu.header_type != ContainerHeaderType::None
                        && contained.header_id != 0)
                        .then_some(ContainerGate {
                            header_start_bit: base,
                            header_type: pdu.header_type,
                            header_id: contained.header_id,
                        });
                    let offset = base + contained.offset.saturating_mul(8);
                    expand_pdu(
                        comm,
                        contained.pdu,
                        offset,
                        None,
                        gate,
                        &mut issues,
                        &mut reads,
                    );
                }
            }
            PduKind::Multiplexed => {
                // The selector (and any static signal on the mux PDU) is
                // always decoded; dynamic parts are gated below.
                expand_pdu(comm, pdu.id, base, None, None, &mut issues, &mut reads);
                let selector = pdu.selector.as_ref().map(|field| {
                    if let Some(signal) = field.signal {
                        MuxSelector::Signal { signal }
                    } else {
                        MuxSelector::Field {
                            start_bit: base.saturating_add(field.start_bit),
                            length: field.length,
                            byte_order: field.byte_order,
                        }
                    }
                });
                for part in comm.multiplexed_parts.values().filter(|p| p.mux == pdu.id) {
                    let Some(code) = part.selector_code else {
                        // Static part: always present.
                        expand_pdu(comm, part.part, base, None, None, &mut issues, &mut reads);
                        continue;
                    };
                    let Some(selector) = selector.clone() else {
                        issues.push(Issue::warning(
                            IssueKind::Unsupported,
                            format!(
                                "multiplexed PDU {} has no selector field; dynamic part {code} is not expanded",
                                pdu.name
                            ),
                        ));
                        continue;
                    };
                    expand_pdu(
                        comm,
                        part.part,
                        base,
                        Some(MuxCondition {
                            selector,
                            ranges: vec![(code, code)],
                        }),
                        None,
                        &mut issues,
                        &mut reads,
                    );
                }
            }
            PduKind::Secured => {
                let Some(secured) = &pdu.secured else {
                    continue;
                };
                let Some(payload) = secured.payload else {
                    issues.push(Issue::warning(
                        IssueKind::Unsupported,
                        format!(
                            "secured PDU {} has no payload reference; its signals are not decoded",
                            pdu.name
                        ),
                    ));
                    continue;
                };
                let header_bits = secured.header.header_size() * 8;
                let pdu_bits = u64::from(pdu.length) * 8;
                let payload_bits = comm
                    .pdus
                    .get(&payload)
                    .map(|pdu| u64::from(pdu.length) * 8)
                    .unwrap_or(0);
                if u64::from(header_bits) + payload_bits > pdu_bits {
                    issues.push(Issue::warning(
                        IssueKind::LengthConvention,
                        format!(
                            "secured PDU {}: header plus payload is longer than the PDU",
                            pdu.name
                        ),
                    ));
                }
                if let Some((offset, length)) = secured.secured_area
                    && u64::from(offset) + u64::from(length) > pdu_bits
                {
                    issues.push(Issue::warning(
                        IssueKind::LengthConvention,
                        format!(
                            "secured PDU {}: the secured area runs past the PDU",
                            pdu.name
                        ),
                    ));
                }
                if let Some(bytes) = secured.auth_info_length
                    && u64::from(header_bits) + u64::from(bytes) * 8 > pdu_bits
                {
                    issues.push(Issue::warning(
                        IssueKind::LengthConvention,
                        format!(
                            "secured PDU {}: the authenticator does not fit the PDU",
                            pdu.name
                        ),
                    ));
                }
                expand_pdu(
                    comm,
                    payload,
                    base + header_bits,
                    None,
                    None,
                    &mut issues,
                    &mut reads,
                );
            }
        }
    }
    let writes = reads
        .iter()
        .map(|read| SignalWrite {
            signal: read.signal,
            bit_offset: read.bit_offset,
            bit_len: read.bit_len,
            byte_order: read.byte_order,
            signed: read.signed,
            float: read.float,
            factor: read.factor,
            offset: read.offset,
            container: read.container,
            transform: read.transform,
        })
        .collect();
    (
        DecodePlan {
            frame: Some(frame.id),
            version,
            reads,
            writes,
        },
        issues,
    )
}

fn expand_pdu(
    comm: &Comm,
    pdu: PduId,
    base: u32,
    mux_part: Option<MuxCondition>,
    container: Option<ContainerGate>,
    issues: &mut Vec<Issue>,
    reads: &mut Vec<SignalRead>,
) {
    for signal in comm.signals_in_pdu(pdu) {
        let mut mux = signal.mux.clone();
        if let Some(part) = mux_part.clone() {
            mux.push(part);
        }
        let transform = match signal.transformation {
            None => None,
            Some(id) => match comm.transformations.get(&id) {
                Some(Transformation {
                    kind: TransformationKind::Identity,
                    ..
                }) => None,
                Some(Transformation {
                    kind: TransformationKind::Linear { scale, offset },
                    ..
                }) => Some(LinearTransform {
                    scale: *scale,
                    offset: *offset,
                }),
                Some(transformation) => {
                    issues.push(Issue::warning(
                        IssueKind::UnsupportedTransformation,
                        format!(
                            "signal {} uses transformation {} of a kind that is shown raw, not converted",
                            signal.name, transformation.name
                        ),
                    ));
                    None
                }
                None => {
                    issues.push(Issue::error(
                        IssueKind::DanglingRef,
                        format!("signal {} refers to a missing transformation", signal.name),
                    ));
                    None
                }
            },
        };
        reads.push(SignalRead {
            signal: signal.id,
            bit_offset: base.saturating_add(signal.start_bit),
            bit_len: signal.length,
            byte_order: signal.byte_order,
            signed: signal.signed,
            float: signal.float,
            factor: signal.factor,
            offset: signal.offset,
            mux,
            container,
            transform,
        });
    }
}
