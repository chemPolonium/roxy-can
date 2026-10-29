//! Stable element and cluster ids for the AUTOSAR model.
//!
//! Ids are assigned from a canonical source key so a re-import produces the
//! same identity without persisting the imported model. A rename is bridged
//! with `rebind`, which keeps the old source key pointing at the same id.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

use super::{ClusterId, FrameId, MappingId, PduId, SignalId};

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ElementKind {
    Frame,
    Pdu,
    Signal,
    Mapping,
    Ecu,
    Connector,
    SystemSignal,
    SignalGroup,
    Transformation,
    E2e,
    BaseType,
    CompuMethod,
}

impl ElementKind {
    pub const ALL: [ElementKind; 12] = [
        ElementKind::Frame,
        ElementKind::Pdu,
        ElementKind::Signal,
        ElementKind::Mapping,
        ElementKind::Ecu,
        ElementKind::Connector,
        ElementKind::SystemSignal,
        ElementKind::SignalGroup,
        ElementKind::Transformation,
        ElementKind::E2e,
        ElementKind::BaseType,
        ElementKind::CompuMethod,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            ElementKind::Frame => "frame",
            ElementKind::Pdu => "pdu",
            ElementKind::Signal => "signal",
            ElementKind::Mapping => "mapping",
            ElementKind::Ecu => "ecu",
            ElementKind::Connector => "connector",
            ElementKind::SystemSignal => "systemSignal",
            ElementKind::SignalGroup => "signalGroup",
            ElementKind::Transformation => "transformation",
            ElementKind::E2e => "e2e",
            ElementKind::BaseType => "baseType",
            ElementKind::CompuMethod => "compuMethod",
        }
    }

    pub fn from_str(text: &str) -> Option<Self> {
        Self::ALL
            .iter()
            .copied()
            .find(|kind| kind.as_str().eq_ignore_ascii_case(text))
    }
}

/// FNV-1a over the canonical source key. Stable across platforms and Rust
/// releases, unlike `DefaultHasher`.
pub fn source_hash(text: &str) -> u64 {
    let mut hash = 0xcbf2_9ce4_8422_2325u64;
    for byte in text.as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    hash
}

/// Allocates frame/pdu/signal ids from canonical source keys.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ElementIds {
    by_source: BTreeMap<(ElementKind, String), u64>,
    by_id: BTreeMap<(ElementKind, u64), String>,
}

impl ElementIds {
    pub fn new() -> Self {
        Self::default()
    }

    /// Rebuilds the allocator from persisted aliases.
    pub fn from_aliases(aliases: impl IntoIterator<Item = (ElementKind, String, u64)>) -> Self {
        let mut ids = Self::new();
        for (kind, source, id) in aliases {
            ids.claim(kind, &source, id);
        }
        ids
    }

    /// The id for `source`, allocating and remembering one if needed.
    pub fn assign(&mut self, kind: ElementKind, source: &str) -> u64 {
        if let Some(id) = self.by_source.get(&(kind, source.to_string())) {
            return *id;
        }
        let mask = 0x7fff_ffff_ffff_ffff;
        let mut candidate = source_hash(source) & mask;
        if candidate == 0 {
            candidate = 1;
        }
        while self.by_id.contains_key(&(kind, candidate)) {
            candidate = candidate.wrapping_add(1) & mask;
            if candidate == 0 {
                candidate = 1;
            }
        }
        self.claim(kind, source, candidate);
        candidate
    }

    /// Keeps `id` for a renamed source key. The old key stays resolvable so
    /// v1 references keep pointing at the same element after a rename.
    pub fn rebind(&mut self, kind: ElementKind, old_source: &str, new_source: &str) -> Option<u64> {
        let id = *self
            .by_source
            .get(&(kind, old_source.to_string()))
            .or_else(|| self.by_source.get(&(kind, new_source.to_string())))?;
        self.by_source.insert((kind, new_source.to_string()), id);
        self.by_id.insert((kind, id), new_source.to_string());
        Some(id)
    }

    pub fn id_for(&self, kind: ElementKind, source: &str) -> Option<u64> {
        self.by_source.get(&(kind, source.to_string())).copied()
    }

    pub fn source_for(&self, kind: ElementKind, id: u64) -> Option<&str> {
        self.by_id.get(&(kind, id)).map(String::as_str)
    }

    /// Persisted aliases. `by_source` also holds the original keys after a
    /// rebind; a project only needs to store the keys that are not derivable
    /// from the current source text, but writing all of them is harmless.
    pub fn aliases(&self) -> impl Iterator<Item = (ElementKind, &str, u64)> {
        self.by_source
            .iter()
            .map(|((kind, source), id)| (*kind, source.as_str(), *id))
    }

    fn claim(&mut self, kind: ElementKind, source: &str, id: u64) {
        self.by_source.insert((kind, source.to_string()), id);
        self.by_id.insert((kind, id), source.to_string());
    }
}

/// Monotonic cluster ids. Unlike element ids these are not derived from a
/// source: a cluster is a project object, and deleting one must not let the
/// next cluster inherit its identity.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClusterIds {
    next: u32,
}

impl Default for ClusterIds {
    fn default() -> Self {
        Self { next: 1 }
    }
}

impl ClusterIds {
    pub fn new(next: u32) -> Self {
        Self { next: next.max(1) }
    }

    pub fn allocate(&mut self) -> ClusterId {
        let id = ClusterId(self.next.max(1));
        self.next = id.0.saturating_add(1);
        id
    }

    /// Reserves an id read from a project file; later allocations start above
    /// the highest reserved id.
    pub fn reserve(&mut self, id: ClusterId) {
        self.next = self.next.max(id.0.saturating_add(1));
    }

    pub fn next(&self) -> u32 {
        self.next
    }
}

impl ElementIds {
    pub fn frame_id(&self, source: &str) -> Option<FrameId> {
        self.id_for(ElementKind::Frame, source).map(FrameId)
    }

    pub fn pdu_id(&self, source: &str) -> Option<PduId> {
        self.id_for(ElementKind::Pdu, source).map(PduId)
    }

    pub fn signal_id(&self, source: &str) -> Option<SignalId> {
        self.id_for(ElementKind::Signal, source).map(SignalId)
    }

    pub fn mapping_id(&self, source: &str) -> Option<MappingId> {
        self.id_for(ElementKind::Mapping, source).map(MappingId)
    }
}
