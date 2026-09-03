use crate::app::{App, MessageAgg};
use crate::can::frame::{CanFrame, Direction};
use crate::log::AscWriter;
impl App {
    /// Exports the frames that pass the given Trace window's filter as ASC.
    pub fn export_trace(&mut self, win: usize, path: &str) {
        if self.trace.is_empty() {
            self.status = "export: trace is empty".to_string();
            return;
        }
        let Some(w) = self.trace_windows.get(win) else {
            return;
        };
        let frames: Vec<CanFrame> = self
            .trace
            .iter()
            .copied()
            .filter(|f| self.trace_match(w, f))
            .collect();
        if frames.is_empty() {
            self.status = "export: no frames pass the trace filter".to_string();
            return;
        }
        match AscWriter::new(path) {
            Ok(mut w) => {
                for f in &frames {
                    w.write(f).ok();
                }
                w.finish().ok();
                self.status = format!(
                    "exported {} of {} frames to {path}",
                    frames.len(),
                    self.trace.len()
                );
            }
            Err(e) => self.status = format!("export failed: {e}"),
        }
    }

    pub fn export_trace_dialog(&mut self, win: usize) {
        if let Some(p) = rfd::FileDialog::new()
            .set_title("Export Trace as ASC")
            .set_file_name("trace_export.asc")
            .add_filter("ASC files", &["asc"])
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
            Err(e) => self.status = format!("export failed: {e}"),
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
        let keys: Vec<(u8, u32, String)> = g
            .signals
            .iter()
            .filter(|s| s.visible)
            .map(|s| s.key.clone())
            .collect();
        let mut s = String::from("time_us,bus,signal,value\n");
        let mut n = 0usize;
        for key in &keys {
            let Some(sub) = self.subs.get(key) else {
                continue;
            };
            let bus = self.channel_name(key.0);
            for (t, v) in sub.history.iter() {
                s.push_str(&format!("{t},{bus},{},{v}\n", key.2));
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

    /// Snapshot of the latest signal values shown in a Data window. Each row
    /// carries the raw number and, where the database names this value, its
    /// enum label -- both, so a spreadsheet can sort on the number while a
    /// report keeps the text.
    pub fn export_data_csv(&mut self, i: usize, path: &str) {
        let Some(d) = self.data_windows.get(i) else {
            return;
        };
        let keys: Vec<(u8, u32, String)> = d
            .signals
            .iter()
            .filter(|s| s.visible)
            .map(|s| s.key.clone())
            .collect();
        let mut s = String::from("bus,signal,value,unit,type,label\n");
        for key in &keys {
            let Some(sub) = self.subs.get(key) else {
                continue;
            };
            let bus = self.channel_name(key.0);
            let label = sub.label.as_deref().unwrap_or("");
            s.push_str(&format!(
                "{bus},{},{},{},{},{label}\n",
                key.2, sub.latest, sub.unit, sub.type_tag
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
        for c in &self.channels {
            let dbc = if c.dbc_path.trim().is_empty() {
                "(none)".to_string()
            } else {
                c.dbc_path.clone()
            };
            s.push_str(&format!("# database,{},{dbc}\n", c.name));
        }
        s.push_str(&format!("# tolerance,+/-{}%\n", self.spec_tol_pct));
        s.push_str(&format!("# grace,{}x declared period\n", self.spec_grace));
        let periodic: usize = self
            .channels
            .iter()
            .filter_map(|c| c.dbc.as_ref())
            .map(|db| {
                db.messages
                    .values()
                    .filter(|m| m.cycle_us.is_some_and(|d| d > 0))
                    .count()
            })
            .sum();
        s.push_str(&format!("# periodic messages declared,{periodic}\n"));
        s.push_str("bus,id,name,rule,declared,measured,count,first_s,last_s\n");
        for ((ch, id, kind), l) in &self.spec.rows {
            let name = self.message_name(*ch, *id).unwrap_or("not in database");
            s.push_str(&format!(
                "{},{:X},{},{},{},{},{},{:.3},{:.3}\n",
                self.channel_name(*ch),
                id,
                name,
                kind.label(),
                crate::spec::qty(*kind, l.declared),
                crate::spec::qty(*kind, l.measured),
                l.count,
                l.first_t_us as f64 / 1e6,
                l.last_t_us as f64 / 1e6,
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
