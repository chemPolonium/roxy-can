//! Offline reference runs: a log decoded with the current CAN, FlexRay and
//! Ethernet descriptions and kept beside the live workspace. Only
//! chart-selected signals are sampled; the browser draws them behind the live
//! curves.
use std::path::Path;

use serde::Serialize;

use crate::app::App;
use crate::can::frame::CanFrame;
use crate::eth::EthFrame;
use crate::log::open_stream;
use crate::observe::SigKey;
use crate::trace::FrRow;

/// Sampling stride of a reference curve. The live chart is drawn at 5 ms;
/// a reference is a comparison baseline, so 50 ms keeps long logs small.
const REFERENCE_STRIDE_US: u64 = 50_000;

#[derive(Clone, Debug, Default, Serialize, PartialEq)]
pub struct ReferenceSignal {
    pub key: String,
    pub name: String,
    pub bus: String,
    pub unit: String,
    /// `[seconds, value]`, oldest first.
    pub points: Vec<[f64; 2]>,
}

#[derive(Clone, Debug, Default, Serialize, PartialEq)]
pub struct ReferenceRun {
    pub name: String,
    pub signals: Vec<ReferenceSignal>,
}

enum TargetKind {
    Can { ch: u8, id: u32, ext: bool },
    Fr { bus: u8, slot: u16 },
    Eth { bus: u8, frame: String },
}

struct Target {
    key: SigKey,
    name: String,
    bus: String,
    unit: String,
    kind: TargetKind,
    points: Vec<[f64; 2]>,
    last_us: Option<u64>,
}

/// Resolves every selected key against its description. A key whose bus or
/// signal is no longer described is dropped, exactly like the observer would
/// stop updating it.
fn build_targets(app: &App, keys: &[SigKey]) -> Vec<Target> {
    let mut targets = Vec::new();
    for key in keys {
        match key {
            SigKey::Can { id, ext, name, .. } => {
                let Some(ch) = app.key_bus(key) else {
                    continue;
                };
                let Some(channel) = app.snap.channels.get(ch as usize) else {
                    continue;
                };
                let Some(db) = channel.dbc.as_ref() else {
                    continue;
                };
                let unit = db
                    .messages
                    .get(&(*id, *ext))
                    .and_then(|message| message.signals.iter().find(|signal| signal.name == *name))
                    .map(|signal| signal.unit.clone())
                    .unwrap_or_default();
                targets.push(Target {
                    key: key.clone(),
                    name: name.clone(),
                    bus: channel.name.clone(),
                    unit,
                    kind: TargetKind::Can {
                        ch,
                        id: *id,
                        ext: *ext,
                    },
                    points: Vec::new(),
                    last_us: None,
                });
            }
            SigKey::Fr { slot, name, .. } => {
                let Some(bus) = app.key_bus(key) else {
                    continue;
                };
                let Some(cfg) = app.fr_buses.get(&bus) else {
                    continue;
                };
                let Some(signal) = cfg.slot_signal(*slot, name) else {
                    continue;
                };
                targets.push(Target {
                    key: key.clone(),
                    name: name.clone(),
                    bus: app.fr_bus_name(bus),
                    unit: signal.unit.clone(),
                    kind: TargetKind::Fr { bus, slot: *slot },
                    points: Vec::new(),
                    last_us: None,
                });
            }
            SigKey::Eth { frame, name, .. } => {
                let Some(bus) = app.key_bus(key) else {
                    continue;
                };
                let Some(db) = app.eth_db(bus) else {
                    continue;
                };
                let Some(def) = db.frames.iter().find(|def| def.name == *frame) else {
                    continue;
                };
                let unit = def
                    .pdus
                    .iter()
                    .flat_map(|pdu| pdu.signals.iter())
                    .find(|signal| signal.name == *name)
                    .map(|signal| signal.unit.clone())
                    .unwrap_or_default();
                targets.push(Target {
                    key: key.clone(),
                    name: name.clone(),
                    bus: app.eth_bus_label(bus),
                    unit,
                    kind: TargetKind::Eth {
                        bus,
                        frame: frame.clone(),
                    },
                    points: Vec::new(),
                    last_us: None,
                });
            }
            // A derived channel has no counterpart in an offline log: its
            // expression is evaluated from the live run, not decoded.
            SigKey::Derived { .. } => continue,
        }
    }
    targets
}

fn accept(target: &mut Target, name: &str, value: f64, t_us: u64) {
    if target.name != name {
        return;
    }
    if let Some(last) = target.last_us
        && t_us < last + REFERENCE_STRIDE_US
    {
        return;
    }
    target.last_us = Some(t_us);
    target.points.push([t_us as f64 / 1e6, value]);
}

fn handle_can(targets: &mut [Target], frame: &CanFrame, app: &App) {
    let matching: Vec<usize> = targets
        .iter()
        .enumerate()
        .filter(|(_, target)| {
            matches!(
                &target.kind,
                TargetKind::Can { ch, id, ext }
                    if *ch == frame.channel && *id == frame.id && *ext == frame.extended
            )
        })
        .map(|(index, _)| index)
        .collect();
    if matching.is_empty() {
        return;
    }
    let Some(decoded) = app.model_can_all_values(frame) else {
        return;
    };
    for decoded in decoded {
        for &index in &matching {
            accept(&mut targets[index], &decoded.name, decoded.phys, frame.t_us);
        }
    }
}

fn handle_fr(targets: &mut [Target], row: &FrRow, app: &App) {
    let matching: Vec<usize> = targets
        .iter()
        .enumerate()
        .filter(|(_, target)| {
            matches!(
                &target.kind,
                TargetKind::Fr { bus, slot } if *bus == row.bus && *slot == row.slot
            )
        })
        .map(|(index, _)| index)
        .collect();
    if matching.is_empty() {
        return;
    }
    let Some(db) = app.fr_db(row.bus) else {
        return;
    };
    let Some(frame_ix) = db.frame_ix_at(row.slot, row.cycle, row.ab) else {
        return;
    };
    let Some(decoded) = app.model_fr_all_values(row, frame_ix) else {
        return;
    };
    for decoded in decoded {
        for &index in &matching {
            accept(&mut targets[index], &decoded.name, decoded.phys, row.t_us);
        }
    }
}

fn handle_eth(targets: &mut [Target], frame: &EthFrame, app: &App) {
    let matching: Vec<usize> = targets
        .iter()
        .enumerate()
        .filter(
            |(_, target)| matches!(&target.kind, TargetKind::Eth { bus, .. } if *bus == frame.bus),
        )
        .map(|(index, _)| index)
        .collect();
    if matching.is_empty() {
        return;
    }
    let Some(db) = app.eth_db(frame.bus) else {
        return;
    };
    let flow = crate::eth::db::EthFlow {
        vlan: frame.vlan,
        dst: frame.dst,
        src: frame.src,
        ethertype: frame.ethertype,
        payload_len: frame.len(),
    };
    let frame_name = match db.match_flow(&flow, app.eth_length_includes_header) {
        crate::eth::db::EthMatch::Unique(index) => db.frames[index].name.clone(),
        _ => return,
    };
    let Some(decoded) = app.model_eth_all_values(frame.bus, &frame_name, &frame.payload) else {
        return;
    };
    for &index in &matching {
        let wanted = matches!(
            &targets[index].kind,
            TargetKind::Eth { frame: wanted, .. } if wanted == &frame_name
        );
        if !wanted {
            continue;
        }
        for signal in &decoded {
            accept(&mut targets[index], &signal.name, signal.phys, frame.t_us);
        }
    }
}

/// Decodes `path` with the descriptions loaded on the app, keeping only the
/// `keys` the chart is watching.
pub fn load(path: &str, keys: &[SigKey], app: &App) -> Result<ReferenceRun, String> {
    let mut targets = build_targets(app, keys);
    if targets.is_empty() {
        return Err("reference: no described chart signals selected".to_string());
    }
    let mut stream =
        open_stream(Path::new(path)).map_err(|e| format!("reference open failed: {e}"))?;
    // CAN, FlexRay and Ethernet objects share one log timeline. Whichever the
    // stream says is next is consumed first, so the 50 ms stride sees the same
    // order a replay would.
    let mut fr_rows: Vec<FrRow> = Vec::new();
    let mut eth_frames: Vec<EthFrame> = Vec::new();
    loop {
        let can_t = stream.peek_t();
        let fr_t = stream.peek_fr_t();
        let eth_t = stream.peek_eth_t();
        let Some(t) = [can_t, fr_t, eth_t].into_iter().flatten().min() else {
            break;
        };
        if can_t == Some(t) {
            let Some(frame) = stream.next_frame() else {
                break;
            };
            handle_can(&mut targets, &frame, app);
        } else if fr_t == Some(t) {
            fr_rows.clear();
            stream.poll_fr_rows(t, &mut fr_rows);
            if fr_rows.is_empty() {
                break;
            }
            for row in &fr_rows {
                handle_fr(&mut targets, row, app);
            }
        } else {
            eth_frames.clear();
            stream.poll_eth_frames(t, &mut eth_frames);
            if eth_frames.is_empty() {
                break;
            }
            for frame in &eth_frames {
                handle_eth(&mut targets, frame, app);
            }
        }
    }
    let signals: Vec<ReferenceSignal> = targets
        .into_iter()
        .filter(|target| !target.points.is_empty())
        .map(|target| ReferenceSignal {
            key: crate::web::series_key(&target.key),
            name: target.name,
            bus: target.bus,
            unit: target.unit,
            points: target.points,
        })
        .collect();
    if signals.is_empty() {
        return Err("reference: no matching samples in that log".to_string());
    }
    let name = Path::new(path)
        .file_name()
        .map(|name| name.to_string_lossy().to_string())
        .unwrap_or_else(|| path.to_string());
    Ok(ReferenceRun { name, signals })
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;
    use crate::can::frame::Direction;
    use crate::log::AscWriter;

    #[test]
    fn a_flexray_reference_log_decodes_the_selected_slot_signal() {
        let mut app = App::headless();
        let path = crate::test_assets::path("PowerTrain.arxml");
        let text = crate::dbc::text_from_bytes(std::fs::read(path).unwrap());
        let db = crate::fr_db::FrDb::parse(&text).expect("PowerTrain.arxml parses");
        // A frame the schedule resolves at its own base phase, with signals.
        let (frame_ix, slot, cycle, signal) = (0..db.frames.len())
            .find_map(|ix| {
                let frame = db.frame_index(ix)?;
                let slot = u16::try_from(frame.triggering.slot_id).ok()?;
                let cycle = u8::try_from(frame.triggering.base_cycle).ok()?;
                let resolved = db.frame_ix_at(slot, cycle, 0)?;
                // A one-bit lamp cannot hold 40..45; pick a signal with room
                // to carry the probe values and a real scale.
                let signal = db
                    .edit_signals(resolved)
                    .into_iter()
                    .find(|signal| signal.size >= 8 && signal.factor != 0.0)?;
                Some((resolved, slot, cycle, signal))
            })
            .expect("a scheduled frame declares a signal");
        let length = db.frames[frame_ix].length as usize;
        let bytes = (signal.bit + signal.size).div_ceil(8) as usize;
        app.fr_buses.insert(0, Arc::new(db));
        app.push_fr_db_to_core();
        let db = app.fr_db(0).expect("bus 0 has a description");
        let log =
            std::env::temp_dir().join(format!("roxy_reference_fr_{}.asc", std::process::id()));
        let mut writer = AscWriter::new(&log.to_string_lossy()).unwrap();
        for i in 0..6u64 {
            let mut payload = vec![0u8; bytes.max(length)];
            assert!(db.encode_signal(frame_ix, &signal.name, 40.0 + i as f64, &mut payload));
            writer
                .write_fr(&FrRow {
                    bus: 0,
                    t_us: i * 100_000,
                    ab: 0,
                    slot,
                    cycle,
                    payload,
                    header_crc: 0,
                    flags: 0,
                    name: None,
                })
                .unwrap();
        }
        writer.finish().ok();

        let key = app.fr_key(0, slot, signal.name.clone());
        let run = load(&log.to_string_lossy(), std::slice::from_ref(&key), &app)
            .expect("reference decodes FlexRay");
        assert_eq!(run.signals.len(), 1);
        assert_eq!(run.signals[0].bus, "FR0");
        assert!(run.signals[0].points.len() >= 4, "stride keeps samples");
        assert!(
            run.signals[0]
                .points
                .iter()
                .any(|point| (point[1] - 44.0).abs() < 1e-3),
            "the driven physical value is decoded"
        );
        std::fs::remove_file(&log).ok();
    }

    #[test]
    fn an_ethernet_reference_target_decodes_the_matched_frame() {
        let mut app = App::headless();
        let path = crate::test_assets::path("EthSample.arxml");
        let text = std::fs::read_to_string(path).unwrap();
        let db = crate::eth::db::EthDb::parse(&text).expect("EthSample.arxml parses");
        app.eth_buses.insert(0, Arc::new(db));
        app.push_eth_db_to_core();
        let key = app.eth_key(0, "EthFrame".to_string(), "Sig".to_string());
        let mut targets = build_targets(&app, std::slice::from_ref(&key));
        assert_eq!(targets.len(), 1, "the selected frame and signal resolve");
        let mut payload = vec![0u8; 22];
        payload[1] = 0xD2;
        payload[2] = 0x04; // raw 1234 · factor 0.1
        let frame = EthFrame::new(
            1_000_000,
            0,
            Direction::Rx,
            [0; 6],
            [0; 6],
            Some(100),
            0x0800,
            payload,
        );
        handle_eth(&mut targets, &frame, &app);
        assert_eq!(targets[0].points.len(), 1);
        assert!((targets[0].points[0][1] - 123.4).abs() < 1e-9);
        assert_eq!(targets[0].unit, "");
    }
}
