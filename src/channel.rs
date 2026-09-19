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

    /// The UI label.
    pub fn label(self) -> &'static str {
        match self {
            NodeRole::Absent => "离线",
            NodeRole::Simulated => "模拟",
        }
    }

    /// The project-file spelling (the Rust variant name, like the other
    /// persisted enums).
    pub fn tag(self) -> &'static str {
        match self {
            NodeRole::Absent => "Absent",
            NodeRole::Simulated => "Simulated",
        }
    }

    /// One-line explanation shown under every role selector.
    pub fn hint(self) -> &'static str {
        match self {
            NodeRole::Simulated => "正常发送报文",
            NodeRole::Absent => "报文需来自回放块或真实总线",
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
    /// database is meaningless anyway. Old projects' `Monitor` entries
    /// go the same way: the loader drops unknown role words, and the
    /// two-state model has no monitor state to remember.
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

    /// The bus a signal key lives on, spelled the way the tables and legends
    /// name it. A FlexRay key's index is not a CAN channel, so it must not be
    /// looked up in the channel list.
    pub fn sig_bus_label(&self, key: &SigKey) -> String {
        match key {
            SigKey::Can { ch, .. } => self.channel_name(*ch),
            SigKey::Fr { bus, .. } => format!("FR{bus}"),
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
        let remap_set = |set: &mut HashSet<(u8, u32)>| {
            *set = set
                .drain()
                .filter_map(|(c, id)| remap(c).map(|nc| (nc, id)))
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
                SigKey::Fr { .. } => true,
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
        self.monitor_rows.retain(|r| match &r.key {
            // Same rule as the curve lists: the removed CAN channel takes its
            // CAN rows with it and leaves every FlexRay row alone.
            SigKey::Can { ch, .. } => remap(*ch).is_some(),
            SigKey::Fr { .. } => true,
        });
        for r in &mut self.monitor_rows {
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
        self.net_selected = 0;
        // The status line ("bus CANx removed") came from the command.
    }

    pub fn pick_dbc(&mut self) {
        let ch = 0;
        let name = self.channel_name(ch as u8);
        if let Some(p) = rfd::FileDialog::new()
            .set_title(format!("Open DBC for {name}"))
            .add_filter("DBC files", &["dbc"])
            .pick_file()
        {
            self.open_dbc_for(ch, p.to_string_lossy().into_owned());
        }
    }

    /// Open a DBC directly for a given channel (used by the Buses window).
    pub fn pick_dbc_for(&mut self, ch: usize) {
        let name = self.channel_name(ch as u8);
        if let Some(p) = rfd::FileDialog::new()
            .set_title(format!("Open DBC for {name}"))
            .add_filter("DBC files", &["dbc"])
            .pick_file()
        {
            self.open_dbc_for(ch, p.to_string_lossy().into_owned());
        }
    }

    /// Opens a FIBEX/ARXML cluster-description picker; a pick parses the
    /// database, keeps it against the first FlexRay bus without one, and
    /// attaches the FlexRay RX-only watch to the Vector channel.
    pub fn pick_fibex_for(&mut self, channel_index: i32) {
        let Some(path) = Self::pick_cluster_file() else {
            return;
        };
        if let Some(bus) = self.load_cluster_description(&path) {
            self.set_fr_watch(bus, Some(channel_index), &path);
        }
    }

    /// Loads a cluster description **without** attaching a watch: a replayed
    /// recording can hold two clusters (BLF states which one each frame came
    /// from), and the second one needs its own description to be named or
    /// decoded at all -- which has nothing to do with opening a port for it.
    pub fn pick_cluster_description(&mut self) {
        let Some(path) = Self::pick_cluster_file() else {
            return;
        };
        if let Some(bus) = self.load_cluster_description(&path) {
            self.status = format!("已加载 FR{bus} 集群描述（未挂监听，供回放解码）: {path}");
        }
    }

    /// The shared file picker of the two entry points above.
    fn pick_cluster_file() -> Option<String> {
        let p = rfd::FileDialog::new()
            .set_title("Open FlexRay cluster description")
            .add_filter("Cluster descriptions", &["xml", "arxml", "fibex"])
            .pick_file()?;
        Some(p.to_string_lossy().into_owned())
    }

    /// Parses `path` as a cluster description and puts it on the first FlexRay
    /// bus that has none, then hands the whole set to the core. No hardware is
    /// touched, so this is also what a replay-only session uses. Returns the
    /// bus index, having set a failure status if it did not.
    pub fn load_cluster_description(&mut self, path: &str) -> Option<u8> {
        let Ok(bytes) = std::fs::read(path) else {
            self.status = format!("FIBEX 读取失败: {path}");
            return None;
        };
        let text = crate::dbc::text_from_bytes(bytes);
        let db = match crate::fr_db::FrDb::parse(&text) {
            Ok(db) => db,
            Err(e) => {
                self.status = format!("FlexRay 描述解析失败: {e}");
                return None;
            }
        };
        let Some(bus) = (0..=u8::MAX)
            .find(|b| !self.fr_buses.contains_key(b))
        else {
            self.status = "FlexRay 总线索引已用尽（256 路）".to_string();
            return None;
        };
        self.fr_buses.insert(
            bus,
            crate::app::FrBusCfg {
                path: path.to_string(),
                db: std::sync::Arc::new(db),
            },
        );
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

    /// Detaches one FlexRay watch and forgets its bus, description included.
    pub fn detach_fr_watch(&mut self, bus: u8) {
        self.fr_buses.remove(&bus);
        self.push_fr_db_to_core();
        self.set_fr_watch(bus, None, "");
    }

    /// Attaches one more DBC file to the bus as an extra database (the
    /// primary stays); re-attaching the same path is a no-op.
    pub fn attach_dbc_to(&mut self, ch: usize, path: String) {
        let mut paths = self
            .snap
            .channels
            .get(ch)
            .map(|c| c.dbc_paths.clone())
            .unwrap_or_default();
        if paths.contains(&path) {
            return;
        }
        paths.push(path);
        self.send(crate::bus::BusCommand::LoadDbc {
            ch: ch as u8,
            paths,
        });
    }

    /// Sets a bus's primary DBC path (extra attached databases stay) and
    /// loads it; successful parses are recorded in the recent list.
    /// "Table present after the load" is the success signal -- a failed
    /// load leaves no table behind.
    pub fn open_dbc_for(&mut self, ch: usize, path: String) {
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

    /// Detaches the extra database at `extra_index` (0 = first extra; the
    /// primary is detached with `open_dbc_for`'s replacement or by emptying
    /// the path). Reloads the bus from the remaining list.
    pub fn detach_dbc_extra(&mut self, ch: usize, extra_index: usize) {
        let mut paths = self
            .snap
            .channels
            .get(ch)
            .map(|c| c.dbc_paths.clone())
            .unwrap_or_default();
        if extra_index + 1 < paths.len() {
            paths.remove(extra_index + 1);
            self.send(crate::bus::BusCommand::LoadDbc {
                ch: ch as u8,
                paths,
            });
        }
    }

    /// Opens a file-picker and attaches the picked DBC to the bus as an
    /// extra database.
    pub fn attach_dbc_dialog(&mut self, ch: usize) {
        let name = self.channel_name(ch as u8);
        if let Some(p) = rfd::FileDialog::new()
            .set_title(format!("Attach extra DBC to {name}"))
            .add_filter("DBC files", &["dbc"])
            .pick_file()
        {
            self.attach_dbc_to(ch, p.to_string_lossy().into_owned());
        }
    }

    /// Opens a file dropped onto the window: a DBC goes into the first
    /// bus, an ASC becomes the replay log.
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
