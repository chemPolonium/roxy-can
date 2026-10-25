//! One CAN bus: user identity, its DBC, and the bitrate declarations the
//! load view divides wire bits by.

use std::collections::BTreeMap;
use std::sync::Arc;

use crate::dbc::SymbolTable;

/// Who plays a DBC node on this bus -- and there are exactly two
/// answers, like CANoe's simulated bus: the tool transmits as the node
/// (its generator entries and bound scripts drive), or the node is off
/// the simulated bus entirely (nothing is transmitted for it; its real
/// traffic arrives through attached hardware in Real bus mode, or from
/// replay blocks). The declaration can only name nodes the DBC already
/// declares; anything else is a mismatch the UI never offers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NodeRole {
    /// This tool transmits as the node: its DBC-scheduled TX frames come
    /// from the generator, and bound scripts may drive.
    Simulated,
    /// The node is not simulated: nothing is transmitted for it. This is
    /// also the default for every node without an entry, so restbus needs
    /// no configuration -- off is what you get.
    Absent,
}

impl NodeRole {
    /// The two roles in display order (increasing presence).
    pub const ALL: [NodeRole; 2] = [NodeRole::Absent, NodeRole::Simulated];

    /// The project-file spelling (the Rust variant name, like the other
    /// persisted enums).
    pub fn tag(self) -> &'static str {
        match self {
            NodeRole::Absent => "Absent",
            NodeRole::Simulated => "Simulated",
        }
    }

    /// Parses the project-file spelling; `None` for anything else, which
    /// the loader drops rather than guessing.
    pub fn parse(s: &str) -> Option<NodeRole> {
        NodeRole::ALL.iter().copied().find(|r| r.tag() == s)
    }
}

/// One CAN bus: user-defined name, a DBC database, the path it came from, and
/// the DBC nodes this tool transmits as. The parsed database is shared as an
/// `Arc`: it is immutable once loaded, and snapshots hand the frontend the
/// same allocation instead of a copy.
pub struct Channel {
    pub name: String,
    /// The **merged** database of every path in `dbc_paths`: decoded
    /// lookups never need to know how many files back it.
    pub dbc: Option<Arc<SymbolTable>>,
    /// The primary path (UI-editable) plus any extra databases attached
    /// to this bus. First entry is the primary; on duplicate message ids
    /// the earlier database wins.
    pub dbc_paths: Vec<String>,
    /// Content checksums of `dbc_paths`, kept in sync so external edits
    /// to a DBC file can be detected and the database reloaded.
    pub dbc_sums: Vec<u64>,
    /// Per-node roles declared on this bus. Only deviations from the
    /// default are stored: a missing entry *is* the `Absent` role, so
    /// restbus is the default behaviour rather than a configuration.
    /// `BTreeMap` keeps the project file's key order stable.
    pub node_roles: BTreeMap<String, NodeRole>,
    /// Arbitration bitrate in kbit/s, as the load view divides wire bits by
    /// it. There is no hardware behind the simulation, so the value is a
    /// declaration about the bus being analysed, not a device setting.
    pub bitrate_kbps: u32,
    /// CAN FD data-phase bitrate in kbit/s, applied to BRS frames only.
    pub fd_data_kbps: u32,
}

impl Channel {
    pub const DEFAULT_BITRATE_KBPS: u32 = 500;
    pub const DEFAULT_FD_DATA_KBPS: u32 = 2000;

    /// The role declared for `node`; every node without an entry is
    /// `Absent` -- that is the restbus default, not a stored state.
    pub fn role_of(&self, node: &str) -> NodeRole {
        self.node_roles
            .get(node)
            .copied()
            .unwrap_or(NodeRole::Absent)
    }

    /// Records a role. `Absent` removes the entry: the map holds only
    /// deviations from the default, and a node's role outliving its
    /// database is meaningless anyway.
    pub fn set_node_role(&mut self, node: &str, role: NodeRole) {
        match role {
            NodeRole::Absent => {
                self.node_roles.remove(node);
            }
            r => {
                self.node_roles.insert(node.to_string(), r);
            }
        }
    }
}

use std::collections::HashSet;

use crate::app::App;
use crate::observe::{GfxSignal, SigKey};
use crate::workspace::SigScope;

/// The row button that closes a bus's hardware without touching the bus
/// itself, spelled in one place because both tables use it -- a CAN adapter and
/// a FlexRay watch detach the same way. The refusals in this file tell the user
/// to press it by name, so a message must not drift away from the label on
/// screen.
pub const DETACH_LABEL: &str = "Detach";

impl App {
    /// The bus's database, from this frame's snapshot. This inherent method
    /// shadows the live-table lookup on `BusCore` for every `App` receiver,
    /// so the frontend's DBC reads never touch bus state directly.
    pub fn channel_dbc(&self, ch: u8) -> Option<&SymbolTable> {
        self.snap
            .channels
            .get(ch as usize)
            .and_then(|c| c.dbc.as_deref())
    }

    /// What the database declares for this message: `Some(0)` for an
    /// event-triggered one, `None` when it says nothing at all. Snapshot
    /// read (shadows the `BusCore` lookup).
    #[cfg(test)]
    pub fn dbc_cycle_us(&self, ch: u8, id: u32) -> Option<u64> {
        self.channel_dbc(ch)
            .and_then(|db| db.message_of(id))
            .and_then(|m| m.cycle_us)
    }

    pub fn message_name(&self, ch: u8, id: u32) -> Option<&str> {
        self.channel_dbc(ch).and_then(|db| db.message_name(id))
    }

    pub fn channel_name(&self, ch: u8) -> String {
        self.snap
            .channels
            .get(ch as usize)
            .map(|c| c.name.clone())
            .unwrap_or_else(|| format!("CAN{}", ch + 1))
    }

    /// Like [`Self::fr_bus_name`], but keeps the index visible when the two
    /// differ. Use it wherever the user has to *type* that number -- a script's
    /// `fr_sig(0, 13, "Speed")`, `on fr 0 slot 13`, the record filter's
    /// `FR0:13` -- because a renamed bus whose number is nowhere on screen
    /// leaves them guessing which cluster they mean.
    pub fn fr_bus_label(&self, bus: u8) -> String {
        let name = self.fr_bus_name(bus);
        let index = format!("FR{bus}");
        if name == index {
            name
        } else {
            format!("{name} ({index})")
        }
    }

    /// Names one FlexRay 路. Blank clears the name back to the `FR{n}` default
    /// rather than storing an empty string, so the project file carries only
    /// the names the user actually chose.
    pub fn set_flexray_name(&mut self, bus: u8, name: &str) {
        let name = name.trim();
        if name.is_empty() || name == format!("FR{bus}") {
            self.fr_names.remove(&bus);
        } else {
            self.fr_names.insert(bus, name.to_string());
        }
    }

    /// What a FlexRay 路 is called: the user's name if they gave one, else the
    /// index spelled the way every table has always spelled it. The index stays
    /// available in the Buses row (`#n`) because the places where a *number* is
    /// typed -- the record filter's `FR0:13`, a script's `fr_sig(0, ..)` -- are
    /// numbered by cluster index, not by name.
    pub fn fr_bus_name(&self, bus: u8) -> String {
        self.fr_names
            .get(&bus)
            .filter(|n| !n.trim().is_empty())
            .cloned()
            .unwrap_or_else(|| format!("FR{bus}"))
    }

    /// The bus a signal key lives on, spelled the way the tables and legends
    /// name it. A FlexRay key's index is not a CAN channel, so it must not be
    /// looked up in the channel list.
    pub fn sig_bus_label(&self, key: &SigKey) -> String {
        match key {
            SigKey::Can { ch, .. } => self.channel_name(*ch),
            SigKey::Fr { bus, .. } => self.fr_bus_name(*bus),
            SigKey::Eth { bus, .. } => self.eth_bus_label(*bus),
            SigKey::Derived { .. } => "Derived".to_string(),
        }
    }

    /// Adds a new bus, loads its default DBC, and pre-populates the
    /// generator. All on the bus via the command.
    pub fn add_channel(&mut self) {
        self.send(crate::bus::BusCommand::AddChannel);
    }

    /// Removes a bus and remaps every channel-indexed reference one step
    /// down. The bus remaps its own state (command `RemoveChannel`); this
    /// wrapper afterwards remaps the frontend's window state, so both
    /// sides agree on the new indexing.
    pub fn remove_channel(&mut self, ch: usize) {
        // Mirror the command's refusal so the frontend never remaps when
        // the bus did not.
        if self.snap.channel_count <= 1 {
            self.status = "at least one bus is required".to_string();
            return;
        }
        if ch >= self.snap.channel_count {
            return;
        }
        self.send(crate::bus::BusCommand::RemoveChannel { ch });
        let remap = |c: u8| -> Option<u8> {
            if (c as usize) < ch {
                Some(c)
            } else if (c as usize) == ch {
                None
            } else {
                Some(c - 1)
            }
        };
        // The picker's remembered port travels with the bus it belongs to:
        // dropping the entry would send a surviving row back to showing the
        // list's first channel, which is the thing this map exists to avoid.
        self.hw_pick = self
            .hw_pick
            .iter()
            .filter_map(|(&b, &v)| remap(b).map(|nb| (nb, v)))
            .collect();
        // An ARXML-sourced CAN bus follows the same renumbering as its
        // channel: the source map is keyed by the channel index.
        self.can_arxml = std::mem::take(&mut self.can_arxml)
            .into_iter()
            .filter_map(|(bus, src)| remap(bus).map(|nb| (nb, src)))
            .collect();
        // Only a CAN pick carries a CAN channel number. A FlexRay pick names a
        // cluster, which adding or removing a CAN channel does not renumber --
        // it is kept as it is, exactly like a FlexRay curve's key.
        let remap_set = |set: &mut HashSet<crate::workspace::Pick>| {
            *set = set
                .drain()
                .filter_map(|p| match p {
                    crate::workspace::Pick::Can { ch, id } => {
                        remap(ch).map(|nc| crate::workspace::Pick::Can { ch: nc, id })
                    }
                    fr @ crate::workspace::Pick::Fr { .. } => Some(fr),
                    eth @ crate::workspace::Pick::Eth(_) => Some(eth),
                })
                .collect();
        };
        for w in &mut self.trace_windows {
            remap_set(&mut w.manual);
        }
        for w in &mut self.msg_windows {
            remap_set(&mut w.manual);
        }
        for w in &mut self.stats_windows {
            remap_set(&mut w.manual);
        }
        let remap_keys = |signals: &mut Vec<GfxSignal>| {
            // Only CAN keys carry a CAN channel number. A FlexRay key's bus
            // index names a cluster, which this CAN-channel removal does not
            // manage, so such a curve is neither dropped nor shifted.
            signals.retain(|s| match &s.key {
                SigKey::Can { ch, .. } => remap(*ch).is_some(),
                SigKey::Fr { .. } | SigKey::Eth { .. } | SigKey::Derived { .. } => true,
            });
            for s in signals.iter_mut() {
                if let SigKey::Can { ch, .. } = &mut s.key {
                    *ch = remap(*ch).unwrap();
                }
            }
        };
        for g in &mut self.graphics {
            remap_keys(&mut g.signals);
        }
        for d in &mut self.data_windows {
            remap_keys(&mut d.signals);
        }
        // State Tracker rows carry signal keys too -- plus per-key state
        // maps (color memory, rules, overrides) that shift with them.
        fn remap_key_map<V>(
            m: std::collections::HashMap<crate::observe::SigKey, V>,
            remap: &impl Fn(u8) -> Option<u8>,
        ) -> std::collections::HashMap<crate::observe::SigKey, V> {
            m.into_iter()
                .filter_map(|(mut k, v)| {
                    // The map's FlexRay entries are keyed by the FR bus, not
                    // by a CAN channel, so they neither shift nor drop.
                    if let SigKey::Can { ch, .. } = &mut k {
                        let nc = remap(*ch)?;
                        *ch = nc;
                    }
                    Some((k, v))
                })
                .collect()
        }
        for w in &mut self.state_trackers {
            remap_keys(&mut w.signals);
            w.color_slots = remap_key_map(std::mem::take(&mut w.color_slots), &remap);
            w.rules = remap_key_map(std::mem::take(&mut w.rules), &remap);
            w.overrides = remap_key_map(std::mem::take(&mut w.overrides), &remap);
        }
        // Monitor rows carry signal keys too.
        self.monitor_view.rows.retain(|r| match &r.key {
            // Same rule as the curve lists: the removed CAN channel takes its
            // CAN rows with it and leaves every FlexRay, Ethernet and derived
            // row alone.
            SigKey::Can { ch, .. } => remap(*ch).is_some(),
            SigKey::Fr { .. } | SigKey::Eth { .. } | SigKey::Derived { .. } => true,
        });
        for r in &mut self.monitor_view.rows {
            if let SigKey::Can { ch, .. } = &mut r.key
                && let Some(nc) = remap(*ch)
            {
                *ch = nc;
            }
        }
        let fix_scope = |s: &mut SigScope| {
            if let SigScope::Bus(b) = *s {
                *s = match (b as usize).cmp(&ch) {
                    std::cmp::Ordering::Equal => SigScope::All,
                    std::cmp::Ordering::Greater => SigScope::Bus(b - 1),
                    std::cmp::Ordering::Less => *s,
                };
            }
        };
        for w in &mut self.trace_windows {
            fix_scope(&mut w.scope);
        }
        for w in &mut self.msg_windows {
            fix_scope(&mut w.scope);
        }
        for w in &mut self.stats_windows {
            fix_scope(&mut w.scope);
        }
        // The status line ("bus CANx removed") came from the command.
    }

    /// Opens either a DBC or a CAN ARXML on one existing bus. An ARXML with
    /// exactly one CAN cluster loads that cluster; a multi-cluster matrix is
    /// refused here with a pointer to the unified importer, which creates one
    /// bus per cluster instead of silently choosing one.
    #[cfg(test)]
    pub fn open_can_database_for(&mut self, ch: usize, path: String) {
        let ext = std::path::Path::new(&path)
            .extension()
            .and_then(|e| e.to_str())
            .map(|e| e.to_ascii_lowercase());
        if !matches!(ext.as_deref(), Some("arxml" | "xml")) {
            self.open_dbc_for(ch, path);
            return;
        }
        let text = match std::fs::read(&path) {
            Ok(bytes) => crate::dbc::text_from_bytes(bytes),
            Err(e) => {
                self.fail(format!("ARXML read failed: {path}: {e}"));
                return;
            }
        };
        let clusters = match crate::can_arxml::scan(&text) {
            Ok(clusters) => clusters,
            Err(e) => {
                self.fail(format!("ARXML parse failed: {e}"));
                return;
            }
        };
        let cans: Vec<_> = clusters
            .into_iter()
            .filter(|c| c.kind == crate::can_arxml::ClusterKind::Can)
            .collect();
        match cans.as_slice() {
            [] => self.fail(format!("No CAN cluster found in {path}")),
            [one] => {
                if let Err(e) = self.load_can_arxml_on_bus(&path, &one.name, ch as u8) {
                    self.fail(format!("CAN ARXML load failed: {e}"));
                } else {
                    self.status = format!(
                        "Loaded CAN ARXML cluster `{}` on {}",
                        one.name,
                        self.channel_name(ch as u8)
                    );
                }
            }
            many => {
                self.status = format!(
                    "This ARXML declares {} CAN clusters; use Import ARXML... to add one bus per cluster",
                    many.len()
                );
            }
        }
    }

    /// Loads a cluster description **without** attaching a watch: a replayed
    /// recording can hold two clusters (BLF states which one each frame came
    /// from), and the second one needs its own description to be named or
    /// decoded at all -- which has nothing to do with opening a port for it.
    /// `bus` is the FlexRay 路 the user says this file describes; the log
    /// itself never names its clusters, so guessing (first free bus, in file
    /// picking order) silently reads one network's slots against the other's
    /// schedule -- plausible wrong names, the worst possible failure here.
    /// Loads one Ethernet AUTOSAR matrix. A `None` bus picks the first free
    /// Ethernet index; cancelling the picker adds nothing.
    pub fn load_eth_description(&mut self, path: &str, bus: Option<u8>) -> Option<u8> {
        self.load_eth_description_inner(path, bus, None)
    }

    /// Loads one named Ethernet cluster out of a multi-cluster ARXML matrix.
    pub fn load_eth_description_cluster(
        &mut self,
        path: &str,
        bus: Option<u8>,
        cluster: &str,
    ) -> Option<u8> {
        self.load_eth_description_inner(path, bus, Some(cluster))
    }

    fn load_eth_description_inner(
        &mut self,
        path: &str,
        bus: Option<u8>,
        cluster: Option<&str>,
    ) -> Option<u8> {
        let Ok(bytes) = std::fs::read(path) else {
            self.fail(format!("ARXML read failed: {path}"));
            return None;
        };
        let text = crate::dbc::text_from_bytes(bytes);
        let parsed = match cluster {
            Some(name) => crate::eth::db::EthDb::parse_cluster(&text, name),
            None => crate::eth::db::EthDb::parse(&text),
        };
        let db = match parsed {
            Ok(db) => db,
            Err(e) => {
                self.fail(format!("Ethernet ARXML parse failed: {e}"));
                return None;
            }
        };
        let bus = bus.unwrap_or_else(|| {
            (0..=u8::MAX)
                .find(|b| !self.eth_buses.contains_key(b))
                .unwrap_or(u8::MAX)
        });
        let cluster_label = db
            .clusters
            .first()
            .map(|c| c.name.clone())
            .unwrap_or_else(|| "-".to_string());
        let frames = db.frames.len();
        let vlans: usize = db.clusters.iter().map(|c| c.vlans.len()).sum();
        let persist_cluster = cluster
            .map(str::to_string)
            .or_else(|| (db.clusters.len() == 1).then(|| db.clusters[0].name.clone()));
        self.eth_buses.insert(
            bus,
            crate::app::EthBusCfg {
                path: path.to_string(),
                db: std::sync::Arc::new(db),
            },
        );
        match persist_cluster {
            Some(name) => {
                self.eth_clusters.insert(bus, name);
            }
            None => {
                self.eth_clusters.remove(&bus);
            }
        }
        self.push_eth_db_to_core();
        self.status = format!(
            "Loaded Ethernet matrix ETH{bus} ({cluster_label}, {vlans} VLANs, {frames} frames): {path}"
        );
        Some(bus)
    }

    /// The next free Ethernet bus index, the same "first gap" rule FlexRay uses.
    pub fn next_eth_bus(&self) -> Option<u8> {
        (0..=u8::MAX).find(|b| !self.eth_buses.contains_key(b))
    }

    /// Loads one named CAN cluster into an existing bus. The caller has
    /// already decided the bus exists; the parsed table replaces its database.
    pub fn load_can_arxml_on_bus(
        &mut self,
        path: &str,
        cluster: &str,
        bus: u8,
    ) -> Result<(), String> {
        let bytes = std::fs::read(path).map_err(|e| format!("{path}: {e}"))?;
        let text = crate::dbc::text_from_bytes(bytes);
        let parsed = crate::can_arxml::parse_cluster(&text, cluster)?;
        self.send(crate::bus::BusCommand::SetChannelDbc {
            ch: bus,
            dbc: std::sync::Arc::new(parsed.db),
        });
        self.can_arxml
            .insert(bus, (path.to_string(), cluster.to_string()));
        Ok(())
    }

    pub fn import_arxml_path(&mut self, path: &str) {
        self.settle();
        let bytes = match std::fs::read(path) {
            Ok(b) => b,
            Err(e) => {
                self.fail(format!("ARXML read failed: {path}: {e}"));
                return;
            }
        };
        let text = crate::dbc::text_from_bytes(bytes);
        let clusters = match crate::can_arxml::scan(&text) {
            Ok(c) => c,
            Err(e) => {
                self.fail(format!("ARXML parse failed: {e}"));
                return;
            }
        };
        if clusters.is_empty() {
            self.fail("ARXML import: the file declares no CAN, FlexRay or Ethernet cluster");
            return;
        }

        let mut added = 0usize;
        let mut updated = 0usize;
        let mut errors: Vec<String> = Vec::new();
        let mut next_can = self.snap.channel_count as u32;
        for info in clusters {
            let existing: Option<u8> = match info.kind {
                crate::can_arxml::ClusterKind::Can => self
                    .can_arxml
                    .iter()
                    .find(|(_, (p, c))| p == path && c == &info.name)
                    .map(|(bus, _)| *bus),
                crate::can_arxml::ClusterKind::FlexRay => self
                    .fr_buses
                    .iter()
                    .find(|(bus, cfg)| {
                        cfg.path == path
                            && self.fr_clusters.get(bus).is_some_and(|c| c == &info.name)
                    })
                    .map(|(bus, _)| *bus),
                crate::can_arxml::ClusterKind::Ethernet => self
                    .eth_buses
                    .iter()
                    .find(|(bus, cfg)| {
                        cfg.path == path
                            && self.eth_clusters.get(bus).is_some_and(|c| c == &info.name)
                    })
                    .map(|(bus, _)| *bus),
            };
            match info.kind {
                crate::can_arxml::ClusterKind::Can => {
                    if existing.is_none() && next_can > u8::MAX as u32 {
                        errors.push(format!(
                            "{} {}: CAN bus index exhausted",
                            info.kind.label(),
                            info.name
                        ));
                        continue;
                    }
                    match crate::can_arxml::parse_cluster(&text, &info.name) {
                        Ok(parsed) => {
                            let name = if parsed.cluster.is_empty() {
                                info.name.clone()
                            } else {
                                parsed.cluster.clone()
                            };
                            let bus = match existing {
                                Some(bus) => {
                                    self.send(crate::bus::BusCommand::SetChannelDbc {
                                        ch: bus,
                                        dbc: std::sync::Arc::new(parsed.db),
                                    });
                                    updated += 1;
                                    bus
                                }
                                None => {
                                    let bus = next_can as u8;
                                    let bitrate = parsed
                                        .bitrate_kbps
                                        .unwrap_or(crate::app::Channel::DEFAULT_BITRATE_KBPS);
                                    self.send(crate::bus::BusCommand::AddChannelWithDbc {
                                        name: name.clone(),
                                        dbc: std::sync::Arc::new(parsed.db),
                                        bitrate_kbps: bitrate,
                                    });
                                    next_can += 1;
                                    added += 1;
                                    bus
                                }
                            };
                            self.can_arxml.insert(bus, (path.to_string(), name));
                        }
                        Err(e) => errors.push(format!("{} {}: {e}", info.kind.label(), info.name)),
                    }
                }
                crate::can_arxml::ClusterKind::FlexRay => {
                    let bus = match existing {
                        Some(bus) => Some(bus),
                        None => self.next_flexray_bus(),
                    };
                    let Some(bus) = bus else {
                        errors.push(format!(
                            "{} {}: FlexRay bus index exhausted",
                            info.kind.label(),
                            info.name
                        ));
                        continue;
                    };
                    if self
                        .load_cluster_description_cluster(path, Some(bus), &info.name)
                        .is_some()
                    {
                        if existing.is_some() {
                            updated += 1;
                        } else {
                            added += 1;
                        }
                    } else {
                        errors.push(format!(
                            "{} {}: cluster could not be loaded",
                            info.kind.label(),
                            info.name
                        ));
                    }
                }
                crate::can_arxml::ClusterKind::Ethernet => {
                    let bus = match existing {
                        Some(bus) => Some(bus),
                        None => self.next_eth_bus(),
                    };
                    let Some(bus) = bus else {
                        errors.push(format!(
                            "{} {}: Ethernet bus index exhausted",
                            info.kind.label(),
                            info.name
                        ));
                        continue;
                    };
                    if self
                        .load_eth_description_cluster(path, Some(bus), &info.name)
                        .is_some()
                    {
                        if existing.is_some() {
                            updated += 1;
                        } else {
                            added += 1;
                        }
                    } else {
                        errors.push(format!(
                            "{} {}: cluster could not be loaded",
                            info.kind.label(),
                            info.name
                        ));
                    }
                }
            }
        }
        self.settle();
        for e in &errors {
            self.log_status(crate::bus::WriteKind::Error, format!("ARXML import: {e}"));
        }
        let mut summary = format!("ARXML import: {added} cluster(s) added");
        if updated > 0 {
            summary.push_str(&format!(", {updated} updated"));
        }
        if !errors.is_empty() {
            summary.push_str(&format!(", {} failed (see Write)", errors.len()));
        }
        self.status = summary;
    }

    pub fn remove_eth_bus(&mut self, bus: u8) {
        if self.eth_buses.remove(&bus).is_some() {
            self.eth_clusters.remove(&bus);
            self.push_eth_db_to_core();
            self.status = format!("Removed Ethernet bus ETH{bus}");
        }
    }

    /// Copies the frontend's parsed Ethernet matrices into the bus core,
    /// which decodes each arriving frame against the one for its own bus.
    pub(crate) fn push_eth_db_to_core(&mut self) {
        let dbs: std::collections::BTreeMap<u8, _> = self
            .eth_buses
            .iter()
            .map(|(b, c)| (*b, std::sync::Arc::clone(&c.db)))
            .collect();
        self.send(crate::bus::BusCommand::SetEthDbs(dbs));
    }

    /// Sets the project's `FRAME-LENGTH` convention and tells the core, which
    /// matches arriving flows against it.
    pub fn set_eth_length_includes_header(&mut self, on: bool) {
        if self.eth_length_includes_header != on {
            self.eth_length_includes_header = on;
            self.send(crate::bus::BusCommand::SetEthLengthIncludesHeader(on));
        }
    }

    pub fn eth_bus_label(&self, bus: u8) -> String {
        format!("ETH{bus}")
    }

    pub fn eth_db(&self, bus: u8) -> Option<&crate::eth::db::EthDb> {
        self.eth_buses.get(&bus).map(|cfg| cfg.db.as_ref())
    }

    /// The index a new FlexRay 路 takes: the first one with no description on
    /// it.
    pub fn next_flexray_bus(&self) -> Option<u8> {
        (0..=u8::MAX).find(|b| !self.fr_buses.contains_key(b))
    }

    /// The rows the Buses table lists: every 路 that exists as state --
    /// described, watched, or carrying traffic in the log being replayed. No
    /// speculative row: an index with nothing on it is not a bus, and a
    /// placeholder made the table look like it had buses the user never added.
    /// A cluster that only the log names is different -- it is real traffic that
    /// cannot be named yet, and giving it a description is what its row's
    /// `加载描述…` is for.
    #[cfg(test)]
    pub fn fr_bus_rows(&self) -> Vec<u8> {
        let mut buses: Vec<u8> = self
            .fr_buses
            .keys()
            .copied()
            .chain(self.snap.fr_watches.iter().map(|w| w.bus))
            .chain(self.snap.fr_loads.keys().copied())
            .collect();
        buses.sort_unstable();
        buses.dedup();
        buses
    }

    /// Parses `path` as a cluster description and puts it on a FlexRay 路, then
    /// hands the whole set to the core. No hardware is touched, so this is also
    /// what a replay-only session uses. `None` picks the first 路 without a
    /// description; `Some(bus)` replaces that 路's description, which is how a
    /// file put on the wrong 路 gets moved. A watched 路 refuses: its port was
    /// configured from the file already there. Returns the bus index, having
    /// set a failure status if it did not.
    pub fn load_cluster_description(&mut self, path: &str, bus: Option<u8>) -> Option<u8> {
        self.load_cluster_description_inner(path, bus, None)
    }

    /// Loads one named FlexRay cluster out of a multi-cluster ARXML matrix.
    /// The cluster becomes the bus identity stored with the project.
    pub fn load_cluster_description_cluster(
        &mut self,
        path: &str,
        bus: Option<u8>,
        cluster: &str,
    ) -> Option<u8> {
        self.load_cluster_description_inner(path, bus, Some(cluster))
    }

    fn load_cluster_description_inner(
        &mut self,
        path: &str,
        bus: Option<u8>,
        cluster: Option<&str>,
    ) -> Option<u8> {
        let Ok(bytes) = std::fs::read(path) else {
            self.fail(format!("FIBEX read failed: {path}"));
            return None;
        };
        let text = crate::dbc::text_from_bytes(bytes);
        let parsed = match cluster {
            Some(name) => crate::fr_db::FrDb::parse_cluster(&text, name),
            None => crate::fr_db::FrDb::parse(&text),
        };
        let db = match parsed {
            Ok(db) => db,
            Err(e) => {
                self.fail(format!("FlexRay description parse failed: {e}"));
                return None;
            }
        };
        let bus = match bus {
            Some(want) => {
                if self.snap.fr_watches.iter().any(|w| w.bus == want) {
                    self.status = format!(
                        "{} is attached: click {DETACH_LABEL} on its row before loading a new description",
                        self.fr_bus_label(want)
                    );
                    return None;
                }
                want
            }
            None => match (0..=u8::MAX).find(|b| !self.fr_buses.contains_key(b)) {
                Some(free) => free,
                None => {
                    self.status = "FlexRay bus index exhausted (256 buses)".to_string();
                    return None;
                }
            },
        };
        let cluster_name = cluster
            .map(str::to_string)
            .or_else(|| (!db.params.name.is_empty()).then(|| db.params.name.clone()));
        self.fr_buses.insert(
            bus,
            crate::app::FrBusCfg {
                path: path.to_string(),
                db: std::sync::Arc::new(db),
            },
        );
        match cluster_name {
            Some(name) => {
                self.fr_clusters.insert(bus, name);
            }
            None => {
                self.fr_clusters.remove(&bus);
            }
        }
        self.push_fr_db_to_core();
        Some(bus)
    }

    /// Copies the frontend's parsed FlexRay databases into the bus core, which
    /// decodes arriving FlexRay frames against the one for their own bus.
    pub(crate) fn push_fr_db_to_core(&mut self) {
        let dbs: std::collections::BTreeMap<u8, _> = self
            .fr_buses
            .iter()
            .map(|(b, c)| (*b, std::sync::Arc::clone(&c.db)))
            .collect();
        self.send(crate::bus::BusCommand::SetFrDbs(dbs));
    }

    /// Closes one FlexRay 路's receive port and leaves the 路 alone: its
    /// description, its name, its send entries and every curve pointed at it
    /// stay where they are. The FR twin of a CAN row's `断开` -- and the step to
    /// take before loading a different description onto a watched 路, because
    /// its port was configured from the file already there.
    pub fn detach_flexray_watch(&mut self, bus: u8) {
        self.set_fr_watch(bus, None, "");
    }

    /// Deletes one FlexRay 路: its port, its cluster description, and every
    /// reference to it on either side of the core. The other 路 keep their
    /// indexes -- a cluster number is an identity, not a list position (see
    /// [`crate::bus::BusCore::remove_fr_bus`]) -- so this is a pure drop, with
    /// no renumbering to get wrong anywhere.
    ///
    /// Two things deliberately survive, because both are *typed* references to
    /// an index rather than a table keyed by it, and silently editing what the
    /// user typed is the worse answer: a script's `on fr 1 slot 5` /
    /// `fr_sig(1, ..)` source text, and the record filter's `FR1:5`. Each names
    /// a cluster that now has no description and no port, so it matches nothing
    /// -- and if a new 路 later takes the freed index, they apply to it, which
    /// is what the (visible) text says.
    pub fn remove_fr_bus(&mut self, bus: u8) {
        // Resolved before the maps go, because the status line names the 路 the
        // way the user did.
        let label = self.fr_bus_label(bus);
        self.fr_buses.remove(&bus);
        self.fr_clusters.remove(&bus);
        self.fr_names.remove(&bus);
        // The picker's remembered port goes with the row: it echoes a choice
        // about a 路 that no longer exists, and a later 路 on the freed index
        // should not open showing this one's port.
        self.fr_pick.remove(&bus);
        // Descriptions move as one map, so the core forgets this 路 here; the
        // command below carries everything keyed by its index that the core
        // holds. Its status line is the bar's final word -- a message written
        // here instead would be overwritten by it a frame later once the core
        // runs on its own thread.
        self.push_fr_db_to_core();
        self.reset_fr_scope(bus);
        let on_bus = |k: &SigKey| matches!(k, SigKey::Fr { bus: b, .. } if *b == bus);
        let on_bus_pick = |p: &crate::workspace::Pick| matches!(p, crate::workspace::Pick::Fr { bus: b, .. } if *b == bus);
        for w in &mut self.trace_windows {
            w.manual.retain(|p| !on_bus_pick(p));
        }
        for w in &mut self.msg_windows {
            w.manual.retain(|p| !on_bus_pick(p));
        }
        for w in &mut self.stats_windows {
            w.manual.retain(|p| !on_bus_pick(p));
        }
        for g in &mut self.graphics {
            g.signals.retain(|s| !on_bus(&s.key));
        }
        for d in &mut self.data_windows {
            d.signals.retain(|s| !on_bus(&s.key));
        }
        // State Tracker rows carry the key plus per-key memory that means
        // nothing once the signal it describes is gone.
        for w in &mut self.state_trackers {
            w.signals.retain(|s| !on_bus(&s.key));
            w.color_slots.retain(|k, _| !on_bus(k));
            w.rules.retain(|k, _| !on_bus(k));
            w.overrides.retain(|k, _| !on_bus(k));
        }
        self.monitor_view.rows.retain(|r| !on_bus(&r.key));
        self.send(crate::bus::BusCommand::RemoveFrBus { bus, label });
    }

    /// A window left pointing at a cluster that no longer exists would show an
    /// unexplained empty table, so its scope goes back to All buses -- what
    /// `remove_bus` does for a CAN channel. Only called when the whole 路 goes
    /// away: [`Self::detach_flexray_watch`] deliberately leaves scopes alone,
    /// because the 路 and its description are still there to be looked at.
    fn reset_fr_scope(&mut self, bus: u8) {
        let drop_bus = |s: &mut SigScope| {
            if matches!(*s, SigScope::FrBus(b) if b == bus) {
                *s = SigScope::All;
            }
        };
        for w in &mut self.trace_windows {
            drop_bus(&mut w.scope);
        }
        for w in &mut self.msg_windows {
            drop_bus(&mut w.scope);
        }
        for w in &mut self.stats_windows {
            drop_bus(&mut w.scope);
        }
    }

    /// Opens a watch for a bus whose description is already loaded, taking the
    /// cluster configuration from that bus's own file -- the route that needs
    /// no second file picker.
    pub fn attach_fr_watch_on(&mut self, bus: u8, channel_index: i32) {
        let Some(path) = self.fr_buses.get(&bus).map(|c| c.path.clone()) else {
            self.status = format!(
                "{} has no cluster description; cannot attach",
                self.fr_bus_label(bus)
            );
            return;
        };
        self.set_fr_watch(bus, Some(channel_index), &path);
    }

    /// Replaces a bus's primary database.
    pub fn open_dbc_for(&mut self, ch: usize, path: String) {
        self.can_arxml.remove(&(ch as u8));
        let mut paths = self
            .snap
            .channels
            .get(ch)
            .map(|c| c.dbc_paths.clone())
            .unwrap_or_default();
        let new_path = path.clone();
        if paths.is_empty() {
            paths.push(path);
        } else {
            paths[0] = path;
        }
        self.send(crate::bus::BusCommand::LoadDbc {
            ch: ch as u8,
            paths,
        });
        if self.snap.channels.get(ch).is_some_and(|c| c.dbc.is_some()) {
            self.push_recent_dbc(new_path);
        }
    }

    /// Opens a file dropped onto the window: a DBC goes into the first
    /// bus, an ASC becomes the replay log.
    #[cfg(test)]
    pub fn open_dropped(&mut self, path: &std::path::Path) {
        let ext = path
            .extension()
            .and_then(|e| e.to_str())
            .map(|e| e.to_ascii_lowercase());
        match ext.as_deref() {
            Some("dbc") => {
                // Dropping a DBC onto the window **attaches** it to the
                // first bus as an extra database (the primary stays).
                // Replacing the primary goes through "Open..." instead.
                let mut paths = self
                    .snap
                    .channels
                    .first()
                    .map(|c| c.dbc_paths.clone())
                    .unwrap_or_default();
                let p = path.to_string_lossy().into_owned();
                if !paths.contains(&p) {
                    paths.push(p.clone());
                }
                self.send(crate::bus::BusCommand::LoadDbc { ch: 0, paths });
                if self.snap.channels.first().is_some_and(|c| c.dbc.is_some()) {
                    self.push_recent_dbc(p);
                }
            }
            Some("asc") | Some("blf") | Some("mf4") => {
                self.load_log(&path.to_string_lossy());
            }
            Some("arxml") => {
                self.import_arxml_path(&path.to_string_lossy());
            }
            _ => {
                let name = path
                    .file_name()
                    .map(|s| s.to_string_lossy().into_owned())
                    .unwrap_or_default();
                self.status = format!("unsupported file: {name}");
            }
        }
    }

    pub fn set_bus_counter(&mut self, n: usize) {
        self.send(crate::bus::BusCommand::SetBusCounter(n));
    }
}
