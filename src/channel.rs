//! One CAN bus: user identity, its DBC, and the bitrate declarations the
//! load view divides wire bits by.

use crate::dbc::SymbolTable;

/// One CAN bus: user-defined name, a DBC database, the path it came from, and
/// the DBC nodes this tool transmits as.
pub struct Channel {
    pub name: String,
    pub dbc: Option<SymbolTable>,
    pub dbc_path: String,
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
    pub fn channel_dbc(&self, ch: u8) -> Option<&SymbolTable> {
        self.channels.get(ch as usize).and_then(|c| c.dbc.as_ref())
    }

    pub fn message_name(&self, ch: u8, id: u32) -> Option<&str> {
        self.channel_dbc(ch).and_then(|db| db.message_name(id))
    }

    pub fn channel_name(&self, ch: u8) -> String {
        self.channels
            .get(ch as usize)
            .map(|c| c.name.clone())
            .unwrap_or_else(|| format!("CAN{}", ch + 1))
    }

    /// Adds a new bus, loads its default DBC, and pre-populates the generator.
    pub fn add_channel(&mut self) {
        self.bus_counter += 1;
        self.core.channels.push(Channel {
            name: format!("CAN{}", self.bus_counter),
            dbc: None,
            dbc_path: "assets/sample.dbc".to_string(),
            sim_nodes: Vec::new(),
            bitrate_kbps: Channel::DEFAULT_BITRATE_KBPS,
            fd_data_kbps: Channel::DEFAULT_FD_DATA_KBPS,
        });
        self.bus_loads.push(crate::load::BusLoad::new());
        let ch = self.core.channels.len() - 1;
        self.load_channel(ch);
        let ids: Vec<u32> = self.core.channels[ch]
            .dbc
            .as_ref()
            .map(|db| db.order.clone())
            .unwrap_or_default();
        for id in ids {
            self.add_tx(ch as u8, id);
        }
    }

    /// Removes a bus and remaps every channel-indexed reference one step down.
    pub fn remove_channel(&mut self, ch: usize) {
        if self.channels.len() <= 1 {
            self.status = "at least one bus is required".to_string();
            return;
        }
        if ch >= self.channels.len() {
            return;
        }
        let name = self.channels[ch].name.clone();
        self.channels.remove(ch);
        self.bus_loads.remove(ch);
        let remap = |c: u8| -> Option<u8> {
            if (c as usize) < ch {
                Some(c)
            } else if (c as usize) == ch {
                None
            } else {
                Some(c - 1)
            }
        };
        self.aggs = self
            .aggs
            .drain()
            .filter_map(|((c, id), mut a)| {
                remap(c).map(|nc| {
                    a.channel = nc;
                    ((nc, id), a)
                })
            })
            .collect();
        self.subs = self
            .subs
            .drain()
            .filter_map(|((c, id, sig), s)| remap(c).map(|nc| ((nc, id, sig), s)))
            .collect();
        self.spec.drop_channel(ch as u8);
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
        self.tx_list.retain(|t| t.channel as usize != ch);
        for t in &mut self.tx_list {
            if t.channel as usize > ch {
                t.channel -= 1;
            }
        }
        self.trace.retain(|f| f.channel as usize != ch);
        for f in self.trace.iter_mut() {
            if f.channel as usize > ch {
                f.channel -= 1;
            }
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
        self.status = format!("bus {name} removed");
    }

    pub fn load_channel(&mut self, ch: usize) -> bool {
        let Some(channel) = self.core.channels.get_mut(ch) else {
            return false;
        };
        let name = channel.name.clone();
        match std::fs::read_to_string(channel.dbc_path.trim()) {
            Ok(content) => match crate::dbc::load_dbc_str(&content) {
                Ok(table) => {
                    self.status = format!("{name} DBC loaded: {} messages", table.order.len());
                    channel.dbc = Some(table);
                    true
                }
                Err(e) => {
                    self.status = format!("{name} DBC error: {e}");
                    false
                }
            },
            Err(e) => {
                self.status = format!("{name} DBC read failed: {e}");
                false
            }
        }
    }

    pub fn load_dbcs(&mut self) {
        for ch in 0..self.channels.len() {
            self.load_channel(ch);
        }
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

    /// Sets a bus's DBC path and loads it; successful parses are recorded
    /// in the recent list.
    pub fn open_dbc_for(&mut self, ch: usize, path: String) {
        if let Some(channel) = self.channels.get_mut(ch) {
            channel.dbc_path = path.clone();
        }
        if self.load_channel(ch) {
            self.push_recent_dbc(path);
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
                self.open_dbc_for(0, path.to_string_lossy().into_owned());
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

    pub fn bus_counter(&self) -> usize {
        self.bus_counter
    }

    pub fn set_bus_counter(&mut self, n: usize) {
        self.bus_counter = n;
    }
}
