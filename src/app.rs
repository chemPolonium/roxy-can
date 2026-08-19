use std::collections::{HashMap, VecDeque};
use std::time::Instant;

use crate::can::frame::{CanFrame, Direction};
use crate::dbc::SymbolTable;
use crate::log::asc::{AscWriter, parse_asc};
use crate::source::replay::ReplaySource;
use crate::source::virtual_source::VirtualSource;
use crate::source::FrameSource;

pub const TRACE_LIMIT: usize = 50_000;
pub const TOOLBAR_H: f32 = 68.0;
pub const STATUSBAR_H: f32 = 26.0;
const HISTORY_LIMIT: usize = 4_000;
const SAMPLE_INTERVAL_US: u64 = 50_000;

pub const PALETTE: [[f32; 4]; 8] = [
    [0.30, 0.80, 1.00, 1.0],
    [1.00, 0.65, 0.20, 1.0],
    [0.45, 0.95, 0.45, 1.0],
    [1.00, 0.40, 0.40, 1.0],
    [0.75, 0.55, 1.00, 1.0],
    [1.00, 0.85, 0.30, 1.0],
    [0.35, 0.95, 0.85, 1.0],
    [0.95, 0.55, 0.85, 1.0],
];

pub struct Subscription {
    pub latest: f64,
    pub unit: String,
    pub min: f64,
    pub max: f64,
    pub last_update_us: u64,
    pub last_sample_us: u64,
    pub history: VecDeque<(u64, f64)>,
    pub color: usize,
}

pub struct GfxSignal {
    pub key: (u32, String),
    pub visible: bool,
}

pub struct GraphicsWindow {
    pub name: String,
    pub signals: Vec<GfxSignal>,
    pub time_window_s: f64,
    pub stacked: bool,
    pub opened: bool,
}

pub struct DataWindow {
    pub name: String,
    pub signals: Vec<GfxSignal>,
    pub opened: bool,
}

#[derive(Clone, Copy)]
pub struct MessageAgg {
    pub id: u32,
    pub extended: bool,
    pub channel: u8,
    pub dir: Direction,
    pub count: u64,
    pub last_t_us: u64,
    pub cycle_us: f64,
    pub dlc: u8,
    pub data: [u8; 8],
}

pub enum Mode {
    Virtual,
    Replay,
}

pub struct App {
    pub measuring: bool,
    pub recording: bool,
    pub mode: Mode,
    pub t0: Instant,
    pub frame_counter: u64,
    pub trace: VecDeque<CanFrame>,
    pub trace_paused: bool,
    pub dbc: Option<SymbolTable>,
    pub dbc_path: String,
    pub status: String,
    pub asc_path: String,
    pub record_path: String,
    pub last_record: String,
    pub subs: HashMap<(u32, String), Subscription>,
    pub show_trace: bool,
    pub show_messages: bool,
    pub aggs: HashMap<u32, MessageAgg>,
    pub msg_filter: String,
    pub dbc_only: bool,
    pub last_tick_us: u64,
    pub graphics: Vec<GraphicsWindow>,
    pub data_windows: Vec<DataWindow>,
    graphics_counter: usize,
    data_counter: usize,
    color_counter: usize,
    source: Box<dyn FrameSource>,
    writer: Option<AscWriter>,
    buf: Vec<CanFrame>,
}

impl App {
    pub fn new() -> Self {
        let mut app = App {
            measuring: false,
            recording: false,
            mode: Mode::Virtual,
            t0: Instant::now(),
            frame_counter: 0,
            trace: VecDeque::new(),
            trace_paused: false,
            dbc: None,
            dbc_path: "assets/sample.dbc".to_string(),
            status: "stopped".to_string(),
            asc_path: String::new(),
            record_path: String::new(),
            last_record: String::new(),
            subs: HashMap::new(),
            show_trace: true,
            show_messages: true,
            aggs: HashMap::new(),
            msg_filter: String::new(),
            dbc_only: false,
            last_tick_us: 0,
            graphics: Vec::new(),
            data_windows: Vec::new(),
            graphics_counter: 0,
            data_counter: 0,
            color_counter: 0,
            source: Box::new(VirtualSource::new()),
            writer: None,
            buf: Vec::new(),
        };
        app.load_dbc();
        app.new_graphics_window();
        app.new_data_window();
        let keys: Vec<_> = app.all_signal_keys().into_iter().take(2).collect();
        for key in &keys {
            app.subscribe(key.clone());
        }
        if let Some(g) = app.graphics.first_mut() {
            g.signals.extend(keys.clone().into_iter().map(|key| GfxSignal {
                key,
                visible: true,
            }));
        }
        if let Some(d) = app.data_windows.first_mut() {
            d.signals.extend(keys.into_iter().map(|key| GfxSignal {
                key,
                visible: true,
            }));
        }
        app
    }

    pub fn now_us(&self) -> u64 {
        self.t0.elapsed().as_micros() as u64
    }

    pub fn start_virtual(&mut self) {
        self.close_writer();
        self.source = Box::new(VirtualSource::new());
        self.mode = Mode::Virtual;
        self.reset_time();
        self.measuring = true;
        self.status = "measuring (virtual)".to_string();
        if self.recording {
            self.open_writer();
        }
    }

    pub fn stop(&mut self) {
        self.measuring = false;
        self.close_writer();
        self.status = "stopped".to_string();
    }

    fn close_writer(&mut self) {
        if let Some(w) = self.writer.take() {
            w.finish().ok();
        }
    }

    fn open_writer(&mut self) -> bool {
        let b = self.record_path.trim();
        let b = b
            .strip_suffix(".asc")
            .or_else(|| b.strip_suffix(".ASC"))
            .unwrap_or(b);
        let base = if b.is_empty() { "record" } else { b };
        let path = format!(
            "{}_{}.asc",
            base,
            chrono::Local::now().format("%Y%m%d_%H%M%S")
        );
        match AscWriter::new(&path) {
            Ok(w) => {
                self.writer = Some(w);
                self.last_record = path.clone();
                self.status = format!("recording to {path}");
                true
            }
            Err(e) => {
                self.status = format!("record failed: {e}");
                false
            }
        }
    }

    fn reset_time(&mut self) {
        self.t0 = Instant::now();
        self.frame_counter = 0;
        self.trace.clear();
        self.aggs.clear();
        for sub in self.subs.values_mut() {
            sub.history.clear();
        }
    }

    pub fn toggle_record(&mut self) {
        if self.recording {
            self.close_writer();
            self.recording = false;
        } else {
            self.recording = self.open_writer();
        }
    }

    pub fn load_dbc(&mut self) {
        match std::fs::read_to_string(self.dbc_path.trim()) {
            Ok(content) => match crate::dbc::load_dbc_str(&content) {
                Ok(table) => {
                    self.status = format!("DBC loaded: {} messages", table.order.len());
                    self.dbc = Some(table);
                }
                Err(e) => self.status = format!("DBC error: {e}"),
            },
            Err(e) => self.status = format!("DBC read failed: {e}"),
        }
    }

    pub fn pick_dbc(&mut self) {
        if let Some(p) = rfd::FileDialog::new()
            .set_title("Open DBC")
            .add_filter("DBC files", &["dbc"])
            .pick_file()
        {
            self.dbc_path = p.to_string_lossy().to_string();
            self.load_dbc();
        }
    }

    pub fn pick_asc(&mut self) {
        if let Some(p) = rfd::FileDialog::new()
            .set_title("Open ASC")
            .add_filter("ASC files", &["asc"])
            .pick_file()
        {
            self.asc_path = p.to_string_lossy().to_string();
            self.replay();
        }
    }

    pub fn replay(&mut self) {
        let path = {
            let p = self.asc_path.trim();
            if p.is_empty() {
                self.last_record.clone()
            } else {
                p.to_string()
            }
        };
        match std::fs::read_to_string(&path) {
            Ok(content) => {
                let frames = parse_asc(&content);
                if frames.is_empty() {
                    self.status = "ASC: no frames parsed".to_string();
                    return;
                }
                self.close_writer();
                self.source = Box::new(ReplaySource::new(frames.clone()));
                self.mode = Mode::Replay;
                self.reset_time();
                self.measuring = true;
                self.status = format!("replaying {} frames", frames.len());
                if self.recording {
                    self.open_writer();
                }
            }
            Err(e) => self.status = format!("ASC read failed [{path}]: {e}"),
        }
    }

    pub fn update(&mut self) {
        if !self.measuring {
            return;
        }
        if self.trace_paused {
            return;
        }
        let now = self.now_us();
        self.last_tick_us = now;
        self.buf.clear();
        self.source.poll(now, &mut self.buf);

        let replay_done = matches!(self.mode, Mode::Replay) && self.buf.is_empty() && self.source.is_done();

        for &f in &self.buf {
            if let Some(w) = &mut self.writer {
                w.write(&f).ok();
            }
            if self.trace.len() >= TRACE_LIMIT {
                self.trace.pop_front();
            }
            self.trace.push_back(f);
            self.frame_counter += 1;
            let agg = self.aggs.entry(f.id).or_insert(MessageAgg {
                id: f.id,
                extended: f.extended,
                channel: f.channel,
                dir: f.dir,
                count: 0,
                last_t_us: 0,
                cycle_us: 0.0,
                dlc: f.dlc,
                data: f.data,
            });
            if agg.count > 0 {
                let dt = f.t_us.saturating_sub(agg.last_t_us) as f64;
                agg.cycle_us = if agg.count == 1 { dt } else { agg.cycle_us * 0.9 + dt * 0.1 };
            }
            agg.count += 1;
            agg.last_t_us = f.t_us;
            agg.channel = f.channel;
            agg.dir = f.dir;
            agg.dlc = f.dlc;
            agg.data = f.data;
            if let Some(db) = &self.dbc {
                for (name, phys, unit) in db.decode_signals(&f) {
                    let key = (f.id, name.clone());
                    let Some(entry) = self.subs.get_mut(&key) else {
                        continue;
                    };
                    entry.latest = phys;
                    entry.unit = unit;
                    entry.last_update_us = f.t_us;
                    if f.t_us >= entry.last_sample_us + SAMPLE_INTERVAL_US
                        || entry.history.is_empty()
                    {
                        entry.history.push_back((f.t_us, phys));
                        entry.last_sample_us = f.t_us;
                        if entry.history.len() > HISTORY_LIMIT {
                            entry.history.pop_front();
                        }
                        if phys < entry.min {
                            entry.min = phys;
                        }
                        if phys > entry.max {
                            entry.max = phys;
                        }
                    }
                }
            }
        }

        if replay_done {
            self.measuring = false;
            self.close_writer();
            self.status = "replay finished".to_string();
        }
    }

    pub fn subscribe(&mut self, key: (u32, String)) {
        if !self.subs.contains_key(&key) {
            let color = self.color_counter;
            self.color_counter += 1;
            self.subs.insert(key, Subscription {
                latest: 0.0,
                unit: String::new(),
                min: f64::INFINITY,
                max: f64::NEG_INFINITY,
                last_update_us: 0,
                last_sample_us: 0,
                history: VecDeque::new(),
                color,
            });
        }
    }

    /// Drops the subscription if no Data/Graphics window references the signal anymore.
    pub fn prune_signal(&mut self, key: &(u32, String)) {
        let in_use = self
            .graphics
            .iter()
            .any(|g| g.signals.iter().any(|s| &s.key == key))
            || self
                .data_windows
                .iter()
                .any(|d| d.signals.iter().any(|s| &s.key == key));
        if !in_use {
            self.subs.remove(key);
        }
    }

    pub fn new_graphics_window(&mut self) {
        self.graphics_counter += 1;
        self.graphics.push(GraphicsWindow {
            name: format!("Graphics {}", self.graphics_counter),
            signals: Vec::new(),
            time_window_s: 10.0,
            stacked: false,
            opened: true,
        });
    }

    pub fn new_data_window(&mut self) {
        self.data_counter += 1;
        self.data_windows.push(DataWindow {
            name: format!("Data {}", self.data_counter),
            signals: Vec::new(),
            opened: true,
        });
    }

    pub fn all_signal_keys(&self) -> Vec<(u32, String)> {
        let Some(db) = &self.dbc else {
            return Vec::new();
        };
        let mut keys = Vec::new();
        for &id in &db.order {
            if let Some(m) = db.messages.get(&id) {
                for s in &m.signals {
                    keys.push((id, s.name.clone()));
                }
            }
        }
        keys
    }
}

impl Default for App {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn record_survives_start_and_writes_frames() {
        let mut app = App::new();
        let path = std::env::temp_dir().join("roxy_can_record_test.asc");
        app.record_path = path.to_string_lossy().to_string();
        app.toggle_record();
        assert!(app.recording);
        app.start_virtual();
        assert!(app.recording, "Start must not clear the Record checkbox");
        std::thread::sleep(std::time::Duration::from_millis(120));
        app.update();
        app.stop();
        let actual = app.last_record.clone();
        assert!(actual.ends_with(".asc"), "no record file: {actual}");
        assert!(
            actual.contains("roxy_can_record_test_"),
            "generated name should keep the user base: {actual}"
        );
        let content = std::fs::read_to_string(&actual).unwrap();
        let frames = crate::log::asc::parse_asc(&content);
        assert!(frames.len() >= 10, "expected frames, got {}", frames.len());
        if let Some(dir) = std::path::Path::new(&actual).parent() {
            if let Ok(rd) = std::fs::read_dir(dir) {
                for e in rd.flatten() {
                    let n = e.file_name().to_string_lossy().to_string();
                    if n.starts_with("roxy_can_record_test") {
                        std::fs::remove_file(e.path()).ok();
                    }
                }
            }
        }
    }

    #[test]
    fn aggregates_frames_per_message_id() {
        let mut app = App::new();
        app.start_virtual();
        std::thread::sleep(std::time::Duration::from_millis(120));
        app.update();
        let agg = app.aggs.get(&0x100).expect("EngineStatus aggregated");
        assert!(agg.count >= 5, "expected several frames, got {}", agg.count);
        assert!(
            (agg.cycle_us / 1000.0 - 10.0).abs() < 5.0,
            "cycle should be ~10ms, got {}ms",
            agg.cycle_us / 1000.0
        );
        app.stop();
    }
}
