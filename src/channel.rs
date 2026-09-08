//! One CAN bus: user identity, its DBC, and the bitrate declarations the
//! load view divides wire bits by.

use std::sync::Arc;

use crate::dbc::SymbolTable;

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
    /// Names of the DBC nodes marked as simulated on this bus. Kept on the
    /// channel itself so deleting or renumbering a bus takes its nodes along
    /// without a second remap pass.
    pub sim_nodes: Vec<String>,
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
}

use std::collections::HashSet;

use crate::app::App;
use crate::observe::GfxSignal;
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
            signals.retain(|s| remap(s.key.0).is_some());
            for s in signals.iter_mut() {
                s.key.0 = remap(s.key.0).unwrap();
            }
        };
        for g in &mut self.graphics {
            remap_keys(&mut g.signals);
        }
        for d in &mut self.data_windows {
            remap_keys(&mut d.signals);
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
