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
        let base = mapping.start_bit;
        match pdu.kind {
            PduKind::Signal => {
                expand_pdu(comm, pdu.id, base, None, &mut reads);
            }
            PduKind::Container => {
                for contained in comm
                    .contained_pdu_mappings
                    .values()
                    .filter(|m| m.container == pdu.id)
                {
                    let offset = base + contained.offset.saturating_mul(8);
                    expand_pdu(comm, contained.pdu, offset, None, &mut reads);
                }
            }
            PduKind::Multiplexed => {
                // The selector (and any static signal on the mux PDU) is
                // always decoded; dynamic parts are gated below.
                expand_pdu(comm, pdu.id, base, None, &mut reads);
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
                        expand_pdu(comm, part.part, base, None, &mut reads);
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
                expand_pdu(comm, payload, base + header_bits, None, &mut reads);
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
    reads: &mut Vec<SignalRead>,
) {
    for signal in comm.signals_in_pdu(pdu) {
        let mut mux = signal.mux.clone();
        if let Some(part) = mux_part.clone() {
            mux.push(part);
        }
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
        });
    }
}
