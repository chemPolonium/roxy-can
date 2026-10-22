use crate::app::{App, MessageAgg};
use crate::can::frame::{CanFrame, Direction};
use crate::log::AscWriter;

fn wall_stamp(us: u64) -> String {
    let s = us / 1_000_000;
    let ms = (us % 1_000_000) / 1_000;
    format!(
        "[{:02}:{:02}:{:02}.{:03}]",
        s / 3_600,
        (s % 3_600) / 60,
        s % 60,
        ms
    )
}

impl App {
    /// Exports the frames and FlexRay rows that pass the given Trace window's
    /// filter as ASC; an Ethernet capture goes to PCAP instead, chosen by the
    /// file extension. Frames the ring archived to disk (the ones the live view
    /// already trimmed) come first, in trim order -- the export covers
    /// the whole run, not just the hot ring's window. FlexRay and Ethernet rows
    /// have no archive, so the export covers the ring for them.
    pub fn export_trace(&mut self, win: usize, path: &str) {
        let Some(w) = self.trace_windows.get(win) else {
            return;
        };
        let flt = w.filter_lens();
        if path.to_ascii_lowercase().ends_with(".pcap") {
            self.export_trace_pcap(path, &flt);
            return;
        }
        if self.snap.trace.is_empty() && self.snap.fr_trace.is_empty() {
            self.status = "export: trace is empty".to_string();
            return;
        }
        // The archive leads: its frames are strictly older than anything
        // in the hot ring.
        let (archived, archived_total): (Vec<CanFrame>, u64) = match self.snap.trace.archive() {
            Some((archive_path, total)) => (
                crate::trace::SpillFile::read_all(archive_path)
                    .unwrap_or_default()
                    .into_iter()
                    .filter(|f| self.trace_match_lens(&flt, f))
                    .collect(),
                total,
            ),
            None => (Vec::new(), 0),
        };
        let frames: Vec<CanFrame> = archived
            .into_iter()
            .chain(
                self.snap
                    .trace
                    .iter()
                    .copied()
                    .filter(|f| self.trace_match_lens(&flt, f)),
            )
            .collect();
        let rows: Vec<crate::trace::FrRow> = self
            .snap
            .fr_trace
            .iter()
            .filter(|r| self.trace_fr_match(&flt, r))
            .cloned()
            .collect();
        if frames.is_empty() && rows.is_empty() {
            self.status = "export: no frames pass the trace filter".to_string();
            return;
        }
        match AscWriter::new(path) {
            Ok(mut w) => {
                // One time-ordered file: an ASC whose lines go backwards is a
                // malformed log, and both lists are ascending on their own.
                let mut can = frames.iter().peekable();
                let mut fr = rows.iter().peekable();
                loop {
                    match (can.peek().map(|f| f.t_us), fr.peek().map(|r| r.t_us)) {
                        (None, None) => break,
                        (Some(t), Some(f)) if t <= f => {
                            w.write(can.next().expect("peeked")).ok();
                        }
                        (Some(_), Some(_)) => {
                            w.write_fr(fr.next().expect("peeked")).ok();
                        }
                        (Some(_), None) => {
                            w.write(can.next().expect("peeked")).ok();
                        }
                        (None, Some(_)) => {
                            w.write_fr(fr.next().expect("peeked")).ok();
                        }
                    }
                }
                w.finish().ok();
                self.status = format!(
                    "exported {} frames + {} FlexRay rows of {} + {} to {path}",
                    frames.len(),
                    rows.len(),
                    self.snap.trace.len() + archived_total as usize,
                    self.snap.fr_trace.len(),
                );
            }
            Err(e) => self.fail(format!("export failed: {e}")),
        }
    }

    /// Exports Ethernet frames that pass the same Trace filter as a classic
    /// PCAP file (link type 1, Ethernet). This is the format that can carry
    /// MACs, VLAN tags, EtherType and payload -- ASC has no Ethernet records.
    ///
    /// Timestamps stay relative to the run but are anchored so the newest
    /// frame is "now": a capture opened in Wireshark then reads as a recent
    /// log instead of 1970. The capture has no direction field; Rx and Tx both
    /// become ordinary wire frames, as a physical capture would show them.
    fn export_trace_pcap(&mut self, path: &str, flt: &crate::workspace::TraceFilter) {
        let frames: Vec<crate::eth::EthFrame> = self
            .snap
            .eth_trace
            .iter()
            .filter(|f| self.trace_eth_match(flt, f))
            .cloned()
            .collect();
        if frames.is_empty() {
            self.status = "export: no Ethernet frames pass the trace filter".to_string();
            return;
        }
        let newest_us = frames.last().map(|f| f.t_us).unwrap_or(0);
        let now_us = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_micros())
            .unwrap_or(0);
        let base_us = u64::try_from(now_us)
            .unwrap_or(u64::MAX)
            .saturating_sub(newest_us);
        let file = match std::fs::File::create(path) {
            Ok(f) => f,
            Err(e) => {
                self.fail(format!("export failed: {e}"));
                return;
            }
        };
        let mut w = std::io::BufWriter::new(file);
        let result = (|| -> std::io::Result<()> {
            use std::io::Write;
            // Classic PCAP, little-endian, microsecond resolution:
            // magic, version 2.4, thiszone 0, sigfigs 0, snaplen, LINKTYPE_ETHERNET.
            w.write_all(&0xa1b2_c3d4u32.to_le_bytes())?;
            w.write_all(&2u16.to_le_bytes())?;
            w.write_all(&4u16.to_le_bytes())?;
            w.write_all(&0i32.to_le_bytes())?;
            w.write_all(&0u32.to_le_bytes())?;
            w.write_all(&262_144u32.to_le_bytes())?;
            w.write_all(&1u32.to_le_bytes())?;
            for f in &frames {
                let l2 = eth_l2_bytes(f);
                let t_us = base_us.saturating_add(f.t_us);
                let len = l2.len() as u32;
                w.write_all(&((t_us / 1_000_000) as u32).to_le_bytes())?;
                w.write_all(&((t_us % 1_000_000) as u32).to_le_bytes())?;
                w.write_all(&len.to_le_bytes())?;
                w.write_all(&len.to_le_bytes())?;
                w.write_all(&l2)?;
            }
            w.flush()
        })();
        match result {
            Ok(()) => self.status = format!("exported {} Ethernet frames to {path}", frames.len()),
            Err(e) => self.fail(format!("export failed: {e}")),
        }
    }

    pub fn export_trace_dialog(&mut self, win: usize) {
        if let Some(p) = rfd::FileDialog::new()
            .set_title("Export Trace")
            .set_file_name("trace_export.asc")
            .add_filter("ASC (CAN / FlexRay)", &["asc"])
            .add_filter("PCAP (Ethernet)", &["pcap"])
            .save_file()
        {
            let path = p.to_string_lossy().to_string();
            self.export_trace(win, &path);
        }
    }

    fn csv_save_dialog(title: &str, name: &str) -> Option<std::path::PathBuf> {
        rfd::FileDialog::new()
            .set_title(title)
            .set_file_name(name)
            .add_filter("CSV files", &["csv"])
            .save_file()
    }

    fn write_export(&mut self, path: &str, content: String) {
        match std::fs::write(path, content) {
            Ok(()) => self.status = format!("exported to {path}"),
            Err(e) => self.fail(format!("export failed: {e}")),
        }
    }

    pub fn export_stats_dialog(&mut self, win: usize) {
        if let Some(p) = Self::csv_save_dialog("Export Statistics as CSV", "statistics.csv") {
            self.export_stats_csv(win, &p.to_string_lossy());
        }
    }

    pub fn export_stats_csv(&mut self, win: usize, path: &str) {
        let Some(w) = self.stats_windows.get(win) else {
            return;
        };
        let scope = w.scope;
        let manual = &w.manual;
        let mut rows: Vec<&MessageAgg> = self
            .snap
            .aggs
            .iter()
            .filter(|a| Self::scope_match(scope, manual, a.channel, a.id))
            .collect();
        rows.sort_by_key(|a| (a.channel, a.id));
        let mut s =
            String::from("bus,id,name,count,cycle_min_ms,cycle_avg_ms,cycle_max_ms,len,flags\n");
        for a in &rows {
            let name = self.message_name(a.channel, a.id).unwrap_or("-");
            let bus = self.channel_name(a.channel);
            let (cmin, cavg, cmax) = if a.count > 1 {
                (a.min_us / 1000.0, a.cycle_us / 1000.0, a.max_us / 1000.0)
            } else {
                (0.0, 0.0, 0.0)
            };
            s.push_str(&format!(
                "{bus},{},{},{},{cmin:.3},{cavg:.3},{cmax:.3},{},{}\n",
                a.id,
                name,
                a.count,
                a.len,
                a.flags.tag()
            ));
        }
        // FlexRay slots join the same table, after the CAN rows and under the
        // same scope rule the windows apply: a CAN scope is CAN-only, a
        // FlexRay cluster scope is that cluster only. `id` carries the slot
        // number, `flags` has no FlexRay meaning and reads as blank.
        for a in self
            .snap
            .fr_aggs
            .iter()
            .filter(|a| Self::scope_match_fr(scope, manual, a.bus, a.slot))
        {
            let name = Self::fr_frame_name(a);
            let (cmin, cavg, cmax) = if a.count > 1 {
                (a.min_us / 1000.0, a.cycle_us / 1000.0, a.max_us / 1000.0)
            } else {
                (0.0, 0.0, 0.0)
            };
            s.push_str(&format!(
                "{},{},{name},{},{cmin:.3},{cavg:.3},{cmax:.3},{},{}\n",
                self.fr_bus_name(a.bus),
                a.slot,
                a.count,
                a.payload.len(),
                crate::can::frame::FrameFlags::NONE.tag(),
            ));
        }
        // Ethernet flows follow the same scope rule as the other two kinds:
        // All buses includes them, a CAN or cluster scope does not, and an
        // Ethernet scope keeps its own. There is no address column in a flow,
        // so `id` carries the EtherType and the VLAN lives in the name.
        for a in self
            .snap
            .eth_aggs
            .iter()
            .filter(|a| Self::scope_match_eth(scope, manual, &a.key))
        {
            let (cmin, cavg, cmax) = if a.count > 1 {
                (a.min_us / 1000.0, a.cycle_us / 1000.0, a.max_us / 1000.0)
            } else {
                (0.0, 0.0, 0.0)
            };
            let name = format!(
                "VLAN {}  {}",
                a.key
                    .vlan
                    .map(|v| v.to_string())
                    .unwrap_or_else(|| "-".to_string()),
                self.eth_flow_name(&a.key)
            );
            s.push_str(&format!(
                "{},0x{:04X},{name},{},{cmin:.3},{cavg:.3},{cmax:.3},{},-\n",
                self.eth_bus_label(a.key.bus),
                a.key.ethertype,
                a.count,
                a.key.len,
            ));
        }
        self.write_export(path, s);
    }

    pub fn export_messages_dialog(&mut self, win: usize) {
        if let Some(p) = Self::csv_save_dialog("Export Messages as CSV", "messages.csv") {
            self.export_messages_csv(win, &p.to_string_lossy());
        }
    }

    /// Exports the aggregation rows currently visible in the given Messages window.
    pub fn export_messages_csv(&mut self, win: usize, path: &str) {
        let Some(w) = self.msg_windows.get(win) else {
            return;
        };
        let scope = w.scope;
        let filter = w.filter.trim().to_lowercase();
        let dbc_only = w.dbc_only;
        let manual = &w.manual;
        let mut rows: Vec<MessageAgg> = self
            .snap
            .aggs
            .iter()
            .copied()
            .filter(|a| {
                if !Self::scope_match(scope, manual, a.channel, a.id) {
                    return false;
                }
                let name = self.message_name(a.channel, a.id).unwrap_or("-");
                if dbc_only && name == "-" {
                    return false;
                }
                if filter.is_empty() {
                    return true;
                }
                let id_str = format!("{:x}", a.id);
                name.to_lowercase().contains(&filter) || id_str.contains(&filter)
            })
            .collect();
        rows.sort_by_key(|a| (a.channel, a.id));
        let mut s = String::from("bus,id,name,dir,count,cycle_ms,len,flags,data\n");
        for a in &rows {
            let name = self.message_name(a.channel, a.id).unwrap_or("-");
            let bus = self.channel_name(a.channel);
            let dir = match a.dir {
                Direction::Rx => "Rx",
                Direction::Tx => "Tx",
            };
            let cycle = if a.count > 1 {
                a.cycle_us / 1000.0
            } else {
                0.0
            };
            let data: String = a
                .payload()
                .iter()
                .map(|b| format!("{b:02X}"))
                .collect::<Vec<_>>()
                .join(" ");
            s.push_str(&format!(
                "{bus},{},{},{},{},{cycle:.3},{},{},{}\n",
                a.id,
                name,
                dir,
                a.count,
                a.len,
                a.flags.tag(),
                data
            ));
        }
        // FlexRay slots ride the same CSV, after the CAN rows, matching
        // the on-screen table (window text filter matches names/slots, and
        // "仅 DBC" is the same per-row question -- see
        // [`App::fr_described_frame`]).
        {
            for agg in &self.snap.fr_aggs {
                if !Self::scope_match_fr(scope, manual, agg.bus, agg.slot) {
                    continue;
                }
                if dbc_only && self.fr_described_frame(agg).is_none() {
                    continue;
                }
                let name = Self::fr_frame_name(agg);
                if !filter.is_empty()
                    && !format!("slot {}", agg.slot).contains(&filter)
                    && !agg.slot.to_string().contains(&filter)
                    && !name.to_lowercase().contains(&filter)
                {
                    continue;
                }
                let cycle = if agg.count > 1 {
                    agg.cycle_us / 1000.0
                } else {
                    0.0
                };
                let data: String = agg
                    .payload
                    .iter()
                    .map(|b| format!("{b:02X}"))
                    .collect::<Vec<_>>()
                    .join(" ");
                s.push_str(&format!(
                    "{},{},{},{},{},{cycle:.3},{},{},{}\n",
                    self.fr_bus_name(agg.bus),
                    agg.slot,
                    name,
                    "Rx",
                    agg.count,
                    agg.payload.len(),
                    "-",
                    data
                ));
            }
        }
        // Ethernet flows join the same table after CAN and FlexRay. A flow
        // has no id, so the EtherType fills that column and the VLAN rides
        // in the name -- the same facts the window's text filter matches.
        for agg in &self.snap.eth_aggs {
            if !Self::scope_match_eth(scope, manual, &agg.key) {
                continue;
            }
            if dbc_only && self.eth_flow_def(agg.key.bus, &agg.key).is_none() {
                continue;
            }
            let macs = format!(
                "{} > {}",
                crate::eth::db::mac_text(&agg.key.src),
                crate::eth::db::mac_text(&agg.key.dst)
            );
            let name = format!(
                "VLAN {}  {}",
                agg.key
                    .vlan
                    .map(|v| v.to_string())
                    .unwrap_or_else(|| "-".to_string()),
                self.eth_flow_name(&agg.key)
            );
            if !filter.is_empty() {
                let hay = format!(
                    "eth{} {} {} vlan {:?} 0x{:04X}",
                    agg.key.bus, macs, name, agg.key.vlan, agg.key.ethertype
                )
                .to_lowercase();
                if !hay.contains(&filter) {
                    continue;
                }
            }
            let cycle = if agg.count > 1 {
                agg.cycle_us / 1000.0
            } else {
                0.0
            };
            let data: String = agg
                .payload
                .iter()
                .map(|b| format!("{b:02X}"))
                .collect::<Vec<_>>()
                .join(" ");
            s.push_str(&format!(
                "{},0x{:04X},{},{},{},{cycle:.3},{},-,{}\n",
                self.eth_bus_label(agg.key.bus),
                agg.key.ethertype,
                name,
                "-",
                agg.count,
                agg.key.len,
                data
            ));
        }
        self.write_export(path, s);
    }

    pub fn export_graphics_dialog(&mut self, i: usize) {
        let name = self
            .graphics
            .get(i)
            .map(|g| g.name.to_lowercase().replace(' ', "_"))
            .unwrap_or_else(|| "graphics".to_string());
        if let Some(p) =
            Self::csv_save_dialog("Export Graphics history as CSV", &format!("{name}.csv"))
        {
            self.export_graphics_csv(i, &p.to_string_lossy());
        }
    }

    /// Snapshot of the plotted signal history, long format: one row per sample.
    pub fn export_graphics_csv(&mut self, i: usize, path: &str) {
        let Some(g) = self.graphics.get(i) else {
            return;
        };
        let keys: Vec<crate::observe::SigKey> = g
            .signals
            .iter()
            .filter(|s| s.visible)
            .map(|s| s.key.clone())
            .collect();
        let mut s = String::from("time_us,bus,signal,value\n");
        let mut n = 0usize;
        for key in &keys {
            let Some(sub) = self.sub_view(key) else {
                continue;
            };
            let bus = self.sig_bus_label(key);
            for (t, v) in sub.history.iter() {
                s.push_str(&format!("{t},{bus},{},{v}\n", key.name()));
                n += 1;
            }
        }
        if n == 0 {
            self.status = "export: no signal history yet".to_string();
            return;
        }
        self.write_export(path, s);
    }

    pub fn export_data_dialog(&mut self, i: usize) {
        let name = self
            .data_windows
            .get(i)
            .map(|d| d.name.to_lowercase().replace(' ', "_"))
            .unwrap_or_else(|| "data".to_string());
        if let Some(p) = Self::csv_save_dialog("Export Data values as CSV", &format!("{name}.csv"))
        {
            self.export_data_csv(i, &p.to_string_lossy());
        }
    }

    pub fn export_state_dialog(&mut self, i: usize) {
        let name = self
            .state_trackers
            .get(i)
            .map(|w| w.name.to_lowercase().replace(' ', "_"))
            .unwrap_or_else(|| "state".to_string());
        if let Some(p) = Self::csv_save_dialog("Export state bands as CSV", &format!("{name}.csv"))
        {
            self.export_state_csv(i, &p.to_string_lossy());
        }
    }

    /// The Write window's export: the whole system log as plain text,
    /// same lines the window shows (the current per-kind filter applies,
    /// so "what I see is what I save").
    pub fn export_write_dialog(&mut self) {
        if let Some(p) = rfd::FileDialog::new()
            .set_title("Export Write log as text")
            .set_file_name("write_log.txt")
            .add_filter("Text files", &["txt", "log"])
            .save_file()
        {
            self.export_write_txt(&p.to_string_lossy());
        }
    }

    pub fn export_write_txt(&mut self, path: &str) {
        let want = |k: &crate::bus::WriteKind, filter: &[bool; 4]| -> bool {
            let i = match k {
                crate::bus::WriteKind::Script => 0,
                crate::bus::WriteKind::Info => 1,
                crate::bus::WriteKind::Warning => 2,
                crate::bus::WriteKind::Error => 3,
            };
            filter[i]
        };
        let filter = self.write_filter;
        let lines: Vec<String> = self
            .snap
            .write
            .iter()
            .filter(|l| want(&l.kind, &filter))
            .map(|l| format!("[{}] {}", wall_stamp(l.wall_us), l.text))
            .collect();
        if lines.is_empty() {
            self.status = "export: the Write log is empty".to_string();
            return;
        }
        let mut s = lines.join("\n");
        s.push('\n');
        self.write_export(path, s);
    }

    /// The State Tracker's band table: one row per state segment per
    /// visible signal over the window's live span -- what the bands draw,
    /// as spreadsheet rows.
    pub fn export_state_csv(&mut self, i: usize, path: &str) {
        let Some(w) = self.state_trackers.get(i) else {
            return;
        };
        let tw = w.time_window_s;
        let t_right = self.plot_now_s();
        let t_left = (t_right - tw).max(0.0);
        let lo_us = (t_left * 1e6) as u64;
        let hi_us = (t_right * 1e6) as u64;
        let mut s = String::from("bus,signal,start_s,end_s,duration_s,label\n");
        let mut n = 0usize;
        for s_key in &w.signals {
            if !s_key.visible {
                continue;
            }
            let key = s_key.key.clone();
            let Some(sub) = self.sub_view(&key) else {
                continue;
            };
            let held = sub.history.at(lo_us).map(crate::state_seg::quantize);
            let pts: Vec<(u64, f64)> = sub.history.range(lo_us, hi_us).copied().collect();
            let segs = crate::state_seg::compute_segs(self, i, &key, held, &pts, lo_us, hi_us);
            let bus = self.sig_bus_label(&key);
            for seg in &segs {
                let start = (seg.t0_us as f64 / 1e6).max(t_left);
                let end = (seg.t1_us as f64 / 1e6).min(t_right);
                if end <= start {
                    continue;
                }
                s.push_str(&format!(
                    "{bus},{},{start:.3},{end:.3},{:.3},{}\n",
                    key.name(),
                    end - start,
                    seg.label
                ));
                n += 1;
            }
        }
        if n == 0 {
            self.status = "export: no state segments yet".to_string();
            return;
        }
        self.write_export(path, s);
    }

    /// Snapshot of the latest signal values shown in a Data window. Each row
    /// carries the raw number and, where the database names this value, its
    /// enum label -- both, so a spreadsheet can sort on the number while a
    /// report keeps the text.
    pub fn export_data_csv(&mut self, i: usize, path: &str) {
        let Some(d) = self.data_windows.get(i) else {
            return;
        };
        let keys: Vec<crate::observe::SigKey> = d
            .signals
            .iter()
            .filter(|s| s.visible)
            .map(|s| s.key.clone())
            .collect();
        let mut s = String::from("bus,signal,value,unit,type,label\n");
        for key in &keys {
            let Some(sub) = self.sub_view(key) else {
                continue;
            };
            let bus = self.sig_bus_label(key);
            let label = sub.label.as_deref().unwrap_or("");
            s.push_str(&format!(
                "{bus},{},{},{},{},{label}\n",
                key.name(),
                sub.latest,
                sub.unit,
                sub.type_tag
            ));
        }
        self.write_export(path, s);
    }

    /// The whole violation report, every latched row regardless of the
    /// window's session-only filter checkboxes, prefixed with the premises it
    /// was judged under: which database each bus carried, the tolerance and
    /// grace in effect, and how many messages had a period to break at all.
    /// Without those a report cannot be re-checked -- the same table means
    /// something else at ±5% and grace 2.
    pub fn export_spec_csv(&mut self, path: &str) {
        let mut s = String::new();
        for c in &self.snap.channels {
            let dbc = if c.dbc_paths.is_empty() {
                "(none)".to_string()
            } else {
                c.dbc_paths.join(";")
            };
            s.push_str(&format!("# database,{},{dbc}\n", c.name));
        }
        // Which description each FlexRay verdict was judged against: the
        // schedule is the promise, so a report without it cannot be re-checked.
        for (bus, cfg) in &self.fr_buses {
            s.push_str(&format!(
                "# flexray,{},{}\n",
                self.fr_bus_name(*bus),
                crate::cli::csv_field(&cfg.path)
            ));
        }
        s.push_str(&format!("# tolerance,+/-{}%\n", self.spec_tol_pct));
        s.push_str(&format!("# grace,{}x declared period\n", self.spec_grace));
        let periodic: usize = self
            .snap
            .channels
            .iter()
            .filter_map(|c| c.dbc.as_deref())
            .map(|db| {
                db.messages
                    .values()
                    .filter(|m| m.cycle_us.is_some_and(|d| d > 0))
                    .count()
            })
            .sum();
        s.push_str(&format!("# periodic messages declared,{periodic}\n"));
        s.push_str("bus,id,name,rule,declared,measured,count,first_s,last_s\n");
        for r in self.spec_rows() {
            s.push_str(&format!(
                "{},{},{},{},{},{},{},{:.3},{:.3}\n",
                crate::cli::csv_field(&r.bus),
                crate::cli::csv_field(&r.addr),
                crate::cli::csv_field(&r.name),
                r.kind.label(),
                crate::spec::qty(r.kind, r.declared),
                crate::spec::qty(r.kind, r.measured),
                r.count,
                r.first_t_us as f64 / 1e6,
                r.last_t_us as f64 / 1e6,
            ));
        }
        self.write_export(path, s);
    }

    pub fn export_spec_dialog(&mut self) {
        if let Some(p) =
            App::csv_save_dialog("Export Specification report as CSV", "spec_report.csv")
        {
            self.export_spec_csv(&p.to_string_lossy());
        }
    }
}

/// One captured frame as Ethernet II bytes for a PCAP record: MACs, an
/// 802.1Q tag when the frame carried a VLAN (the capture model keeps the vid,
/// not the original TPID), EtherType and payload. The FCS is not stored in the
/// ring and is not invented here.
fn eth_l2_bytes(f: &crate::eth::EthFrame) -> Vec<u8> {
    let mut out = Vec::with_capacity(14 + usize::from(f.vlan.is_some()) * 4 + f.payload.len());
    out.extend_from_slice(&f.dst);
    out.extend_from_slice(&f.src);
    if let Some(vlan) = f.vlan {
        out.extend_from_slice(&f.vlan_tpid.unwrap_or(0x8100).to_be_bytes());
        out.extend_from_slice(&vlan.to_be_bytes());
    }
    out.extend_from_slice(&f.ethertype.to_be_bytes());
    out.extend_from_slice(&f.payload);
    out
}
