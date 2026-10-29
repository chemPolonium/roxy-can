//! Stable signal references.
//!
//! Projects and the web API carry `sig:{cluster}:{frame}:{signal}`; people
//! type `[Cluster:Frame:Signal]`. Both resolve to [`SignalKey`], whose ids
//! survive renames. The v1 texts are parsed read-only so existing scripts
//! keep compiling after a project migration.

use std::fmt;

use super::{ClusterId, ClusterKind, Comm, Frame, FrameAddress, FrameId, SignalId};

/// Legacy bus indexes from a v1 project: array position is the old `ch` or
/// `bus` number.
#[derive(Clone, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct LegacyBusMap {
    pub can: Vec<ClusterId>,
    pub flexray: Vec<ClusterId>,
    pub ethernet: Vec<ClusterId>,
}

#[derive(
    Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, serde::Serialize, serde::Deserialize,
)]
pub struct SignalKey {
    pub cluster: ClusterId,
    pub frame: FrameId,
    pub signal: SignalId,
}

impl SignalKey {
    pub fn text(self) -> String {
        format!("sig:{}:{}:{}", self.cluster.0, self.frame.0, self.signal.0)
    }

    pub fn display(self, comm: &Comm) -> String {
        let Some(cluster) = comm.clusters.get(&self.cluster) else {
            return self.text();
        };
        let Some(frame) = comm.frames.get(&self.frame) else {
            return self.text();
        };
        let Some(signal) = comm.signals.get(&self.signal) else {
            return self.text();
        };
        format!("[{}:{}:{}]", cluster.name, frame.name, signal.name)
    }

    fn canonical(text: &str, comm: &Comm) -> Result<Self, KeyError> {
        let mut parts = text.split(':');
        let cluster = parts
            .next()
            .and_then(|p| p.parse::<u32>().ok())
            .map(ClusterId)
            .ok_or_else(|| KeyError::Malformed(text.to_string()))?;
        let frame = parts
            .next()
            .and_then(|p| p.parse::<u64>().ok())
            .map(FrameId)
            .ok_or_else(|| KeyError::Malformed(text.to_string()))?;
        let signal = parts
            .next()
            .and_then(|p| p.parse::<u64>().ok())
            .map(SignalId)
            .ok_or_else(|| KeyError::Malformed(text.to_string()))?;
        if parts.next().is_some() {
            return Err(KeyError::Malformed(text.to_string()));
        }
        let key = Self {
            cluster,
            frame,
            signal,
        };
        key.validate(comm)?;
        Ok(key)
    }

    fn validate(self, comm: &Comm) -> Result<(), KeyError> {
        if !comm.clusters.contains_key(&self.cluster) {
            return Err(KeyError::UnknownCluster(self.cluster.text()));
        }
        let Some(frame) = comm.frames.get(&self.frame) else {
            return Err(KeyError::UnknownFrame(self.frame.text()));
        };
        if frame.cluster != self.cluster {
            return Err(KeyError::UnknownFrame(self.frame.text()));
        }
        let Some(signal) = comm.signals.get(&self.signal) else {
            return Err(KeyError::UnknownSignal(self.signal.text()));
        };
        let carried = comm
            .pdus_in_frame(frame)
            .into_iter()
            .any(|pdu| pdu.id == signal.pdu);
        if !carried {
            return Err(KeyError::UnknownSignal(self.signal.text()));
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum SignalRef {
    Signal(SignalKey),
    Derived(String),
}

impl SignalRef {
    pub fn text(&self) -> String {
        match self {
            SignalRef::Signal(key) => key.text(),
            SignalRef::Derived(id) => format!("derived:{id}"),
        }
    }

    pub fn display(&self, comm: &Comm) -> String {
        match self {
            SignalRef::Signal(key) => key.display(comm),
            SignalRef::Derived(id) => format!("[derived:{id}]"),
        }
    }

    pub fn parse(text: &str, comm: &Comm, legacy: Option<&LegacyBusMap>) -> Result<Self, KeyError> {
        let text = text.trim();
        if text.is_empty() {
            return Err(KeyError::Empty);
        }
        if let Some(id) = text.strip_prefix("derived:") {
            let id = id.trim();
            return (!id.is_empty())
                .then(|| SignalRef::Derived(id.to_string()))
                .ok_or_else(|| KeyError::Malformed(text.to_string()));
        }
        if let Some(rest) = text.strip_prefix("sig:") {
            return Ok(SignalRef::Signal(SignalKey::canonical(rest, comm)?));
        }
        if text.starts_with('[') {
            return Ok(SignalRef::Signal(parse_alias(text, comm)?));
        }
        if let Some(map) = legacy {
            return Ok(SignalRef::Signal(parse_legacy(text, comm, map)?));
        }
        Err(KeyError::Malformed(text.to_string()))
    }
}

fn parse_alias(text: &str, comm: &Comm) -> Result<SignalKey, KeyError> {
    let inner = text
        .strip_prefix('[')
        .and_then(|t| t.strip_suffix(']'))
        .ok_or_else(|| KeyError::Malformed(text.to_string()))?;
    let parts: Vec<&str> = inner.splitn(3, ':').collect();
    if parts.len() != 3 || parts.iter().any(|p| p.trim().is_empty()) {
        return Err(KeyError::Malformed(text.to_string()));
    }
    let cluster_token = parts[0].trim();
    let frame_token = parts[1].trim();
    let signal_token = parts[2].trim();

    let clusters: Vec<_> = comm
        .clusters
        .values()
        .filter(|c| c.matches_name(cluster_token))
        .collect();
    let cluster = match clusters.len() {
        0 => return Err(KeyError::UnknownCluster(cluster_token.to_string())),
        1 => clusters[0],
        n => {
            return Err(KeyError::Ambiguous(cluster_token.to_string(), n));
        }
    };

    let frames: Vec<&Frame> = comm
        .frames_in_cluster(cluster.id)
        .filter(|f| frame_matches(f, cluster.kind, frame_token))
        .collect();
    let frame = match frames.len() {
        0 => return Err(KeyError::UnknownFrame(frame_token.to_string())),
        1 => frames[0],
        n => return Err(KeyError::Ambiguous(frame_token.to_string(), n)),
    };

    let signals = comm.signal_by_name(frame, signal_token);
    match signals.len() {
        0 => Err(KeyError::UnknownSignal(signal_token.to_string())),
        1 => Ok(SignalKey {
            cluster: cluster.id,
            frame: frame.id,
            signal: signals[0].id,
        }),
        n => Err(KeyError::Ambiguous(signal_token.to_string(), n)),
    }
}

fn frame_matches(frame: &Frame, kind: ClusterKind, token: &str) -> bool {
    if frame.name.eq_ignore_ascii_case(token) {
        return true;
    }
    match (kind, frame.address) {
        (ClusterKind::Can, FrameAddress::Can { id, .. }) => parse_address_number(token) == Some(id),
        (ClusterKind::FlexRay, FrameAddress::FlexRay { slot, .. }) => {
            parse_address_number(token).is_some_and(|n| n == u32::from(slot))
        }
        _ => false,
    }
}

fn parse_address_number(token: &str) -> Option<u32> {
    let token = token.trim();
    if let Some(hex) = token
        .strip_prefix("0x")
        .or_else(|| token.strip_prefix("0X"))
    {
        u32::from_str_radix(hex, 16).ok()
    } else {
        token.parse().ok()
    }
}

fn parse_legacy(text: &str, comm: &Comm, map: &LegacyBusMap) -> Result<SignalKey, KeyError> {
    if let Some(rest) = text.strip_prefix("fr:") {
        let mut parts = rest.splitn(3, ':');
        let bus: usize = parts
            .next()
            .and_then(|p| p.parse().ok())
            .ok_or_else(|| KeyError::Malformed(text.to_string()))?;
        let slot: u16 = parts
            .next()
            .and_then(|p| p.parse().ok())
            .ok_or_else(|| KeyError::Malformed(text.to_string()))?;
        let name = parts
            .next()
            .ok_or_else(|| KeyError::Malformed(text.to_string()))?;
        let cluster = *map
            .flexray
            .get(bus)
            .ok_or_else(|| KeyError::UnknownCluster(format!("fr:{bus}")))?;
        let frames = comm.frame_by_slot(cluster, slot);
        return legacy_frame_key(comm, text, frames, name);
    }
    if let Some(rest) = text.strip_prefix("eth:") {
        let mut parts = rest.splitn(3, ':');
        let bus: usize = parts
            .next()
            .and_then(|p| p.parse().ok())
            .ok_or_else(|| KeyError::Malformed(text.to_string()))?;
        let frame_name = parts
            .next()
            .ok_or_else(|| KeyError::Malformed(text.to_string()))?;
        let name = parts
            .next()
            .ok_or_else(|| KeyError::Malformed(text.to_string()))?;
        let cluster = *map
            .ethernet
            .get(bus)
            .ok_or_else(|| KeyError::UnknownCluster(format!("eth:{bus}")))?;
        let frame = comm
            .frame_by_name(cluster, frame_name)
            .ok_or_else(|| KeyError::UnknownFrame(frame_name.to_string()))?;
        return legacy_signal_key(comm, cluster, frame, name);
    }

    let mut parts = text.splitn(3, ':');
    let bus: usize = parts
        .next()
        .and_then(|p| p.parse().ok())
        .ok_or_else(|| KeyError::Malformed(text.to_string()))?;
    let id: u32 = parts
        .next()
        .and_then(parse_address_number)
        .ok_or_else(|| KeyError::Malformed(text.to_string()))?;
    let name = parts
        .next()
        .ok_or_else(|| KeyError::Malformed(text.to_string()))?;
    let cluster = *map
        .can
        .get(bus)
        .ok_or_else(|| KeyError::UnknownCluster(format!("can:{bus}")))?;
    let mut frames = comm.frame_by_can_id(cluster, id);
    // v1 keys had no extended flag and always meant the standard frame.
    if frames.len() > 1 {
        let standard: Vec<_> = frames
            .iter()
            .copied()
            .filter(|f| {
                matches!(
                    f.address,
                    FrameAddress::Can {
                        extended: false,
                        ..
                    }
                )
            })
            .collect();
        if standard.len() == 1 {
            frames = standard;
        }
    }
    legacy_frame_key(comm, text, frames, name)
}

fn legacy_frame_key(
    comm: &Comm,
    text: &str,
    frames: Vec<&Frame>,
    name: &str,
) -> Result<SignalKey, KeyError> {
    let frame = match frames.len() {
        0 => return Err(KeyError::UnknownFrame(text.to_string())),
        1 => frames[0],
        n => return Err(KeyError::Ambiguous(text.to_string(), n)),
    };
    legacy_signal_key(comm, frame.cluster, frame, name)
}

fn legacy_signal_key(
    comm: &Comm,
    cluster: ClusterId,
    frame: &Frame,
    name: &str,
) -> Result<SignalKey, KeyError> {
    let signals = comm.signal_by_name(frame, name);
    match signals.len() {
        0 => Err(KeyError::UnknownSignal(name.to_string())),
        1 => Ok(SignalKey {
            cluster,
            frame: frame.id,
            signal: signals[0].id,
        }),
        n => Err(KeyError::Ambiguous(name.to_string(), n)),
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum KeyError {
    Empty,
    Malformed(String),
    UnknownCluster(String),
    UnknownFrame(String),
    UnknownSignal(String),
    Ambiguous(String, usize),
}

impl fmt::Display for KeyError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            KeyError::Empty => write!(f, "signal reference is empty"),
            KeyError::Malformed(text) => write!(
                f,
                "'{text}' is not a signal reference; use [Cluster:Frame:Signal] or sig:id:id:id"
            ),
            KeyError::UnknownCluster(name) => write!(f, "unknown cluster '{name}'"),
            KeyError::UnknownFrame(name) => write!(f, "unknown frame '{name}'"),
            KeyError::UnknownSignal(name) => write!(f, "unknown signal '{name}'"),
            KeyError::Ambiguous(name, count) => {
                write!(f, "'{name}' matches {count} elements; use the full path")
            }
        }
    }
}

impl std::error::Error for KeyError {}
