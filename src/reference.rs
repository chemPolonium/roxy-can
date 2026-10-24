//! Offline reference runs: a log decoded with the current CAN databases and
//! kept beside the live workspace. Only chart-selected CAN signals are
//! sampled; the browser draws them behind the live curves.
use std::path::Path;

use serde::Serialize;

use crate::bus::ChannelView;
use crate::log::open_stream;
use crate::observe::SigKey;

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

struct Target {
    name: String,
    ch: u8,
    id: u32,
    ext: bool,
    bus: String,
    unit: String,
    points: Vec<[f64; 2]>,
    last_us: Option<u64>,
}

/// Decodes `path` with the DBC on each bus, keeping only `keys` (CAN). A
/// FlexRay or Ethernet chart lane is skipped: its key has no CAN triple.
pub fn load(path: &str, keys: &[SigKey], channels: &[ChannelView]) -> Result<ReferenceRun, String> {
    let mut targets: Vec<Target> = Vec::new();
    for key in keys {
        let SigKey::Can { ch, id, ext, name } = key else {
            continue;
        };
        let Some(channel) = channels.get(*ch as usize) else {
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
            name: name.clone(),
            ch: *ch,
            id: *id,
            ext: *ext,
            bus: channel.name.clone(),
            unit,
            points: Vec::new(),
            last_us: None,
        });
    }
    if targets.is_empty() {
        return Err("reference: no CAN chart signals selected".to_string());
    }
    let mut stream =
        open_stream(Path::new(path)).map_err(|e| format!("reference open failed: {e}"))?;
    while let Some(frame) = stream.next_frame() {
        let Some(channel) = channels.get(frame.channel as usize) else {
            continue;
        };
        let Some(db) = channel.dbc.as_ref() else {
            continue;
        };
        let matching: Vec<usize> = targets
            .iter()
            .enumerate()
            .filter(|(_, target)| {
                target.ch == frame.channel && target.id == frame.id && target.ext == frame.extended
            })
            .map(|(index, _)| index)
            .collect();
        if matching.is_empty() {
            continue;
        }
        for decoded in db.decode_signals(&frame) {
            for &index in &matching {
                let target = &mut targets[index];
                if target.name != decoded.name {
                    continue;
                }
                if let Some(last) = target.last_us
                    && frame.t_us < last + REFERENCE_STRIDE_US
                {
                    continue;
                }
                target.last_us = Some(frame.t_us);
                target.points.push([frame.t_us as f64 / 1e6, decoded.phys]);
            }
        }
    }
    let signals: Vec<ReferenceSignal> = targets
        .into_iter()
        .filter(|target| !target.points.is_empty())
        .map(|target| ReferenceSignal {
            key: crate::web::series_key(&SigKey::can(
                target.ch,
                target.id,
                target.ext,
                &target.name,
            )),
            name: target.name,
            bus: target.bus,
            unit: target.unit,
            points: target.points,
        })
        .collect();
    if signals.is_empty() {
        return Err("reference: no matching CAN samples in that log".to_string());
    }
    let name = Path::new(path)
        .file_name()
        .map(|name| name.to_string_lossy().to_string())
        .unwrap_or_else(|| path.to_string());
    Ok(ReferenceRun { name, signals })
}
