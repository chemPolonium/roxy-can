//! Experimental web frontend host.
//!
//! The bus core is unchanged: this module starts an `App` on a manual drive
//! in its own thread, publishes a small JSON snapshot, and serves the Vite
//! build from `web/dist`. It is a spike, not a second supported frontend --
//! the desktop ImGui path stays the product.

use std::collections::VecDeque;
use std::sync::mpsc::{Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use axum::extract::{Query, State};
use axum::http::StatusCode;
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::{Deserialize, Serialize};
use tower_http::services::{ServeDir, ServeFile};

use crate::app::{App, Mode, NodeRole};
use crate::bus::BusCommand;
use crate::can::frame::{CanFrame, Direction};

const PORT: u16 = 2010;
const SERIES_POINTS: usize = 120;
/// Rows the web frontend keeps. The source ring holds 50k; 5k fills in under
/// half a minute of demo traffic and is enough to exercise virtualization.
const TRACE_LOG_CAP: usize = 5_000;
const PUBLISH_EVERY: Duration = Duration::from_millis(100);
const TICK_EVERY: Duration = Duration::from_millis(5);

/// Serve the web UI until the process is killed. `--port <n>` overrides the
/// default, `--web` itself is already consumed by the caller.
pub fn run(args: &[String]) {
    let port = args
        .iter()
        .position(|arg| arg == "--port")
        .and_then(|i| args.get(i + 1))
        .and_then(|p| p.parse::<u16>().ok())
        .unwrap_or(PORT);
    if !std::path::Path::new("web/dist/index.html").exists() {
        eprintln!("web/dist is missing; run `npm install && npm run build` in web/ first");
    }

    let (tx, rx) = std::sync::mpsc::channel();
    let shared = Arc::new(Mutex::new(WebState::default()));
    let trace = Arc::new(Mutex::new(TraceStore::default()));
    {
        let shared = shared.clone();
        let trace = trace.clone();
        std::thread::Builder::new()
            .name("roxy-web-core".to_string())
            .spawn(move || app_loop(rx, shared, trace))
            .expect("spawn web core thread");
    }

    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("build tokio runtime");
    runtime.block_on(async move {
        let api = Router::new()
            .route("/state", get(state))
            .route("/command", post(command))
            .with_state(ApiState {
                shared,
                trace,
                commands: tx,
            });
        let app = Router::new().nest("/api", api).fallback_service(
            ServeDir::new("web/dist").not_found_service(ServeFile::new("web/dist/index.html")),
        );
        let listener = tokio::net::TcpListener::bind(("127.0.0.1", port))
            .await
            .unwrap_or_else(|e| panic!("bind 127.0.0.1:{port}: {e}"));
        println!("roxy-can web ui: http://127.0.0.1:{port}");
        axum::serve(listener, app).await.expect("web server");
    });
}

#[derive(Clone)]
struct ApiState {
    shared: Arc<Mutex<WebState>>,
    trace: Arc<Mutex<TraceStore>>,
    commands: Sender<WebCommand>,
}

#[derive(Default, Deserialize)]
struct TraceQuery {
    epoch: Option<u64>,
    seq: Option<u64>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct WebResponse {
    #[serde(flatten)]
    state: WebState,
    trace_epoch: u64,
    trace_seq: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    trace_seed: Option<Vec<WebFrame>>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    trace_delta: Vec<WebFrame>,
}

async fn state(State(api): State<ApiState>, Query(query): Query<TraceQuery>) -> Json<WebResponse> {
    let snapshot = api.shared.lock().map(|s| s.clone()).unwrap_or_default();
    let (epoch, seq, seed, delta) = {
        let store = api.trace.lock().unwrap_or_else(|p| p.into_inner());
        let epoch = store.epoch;
        let seq = store.next_seq;
        let first = store.first_seq();
        let synced = query.epoch == Some(epoch)
            && query
                .seq
                .is_some_and(|client| client >= first && client <= seq);
        if synced {
            (epoch, seq, None, store.after(query.seq.unwrap_or(seq)))
        } else {
            (epoch, seq, Some(store.seed()), Vec::new())
        }
    };
    Json(WebResponse {
        state: snapshot,
        trace_epoch: epoch,
        trace_seq: seq,
        trace_seed: seed,
        trace_delta: delta,
    })
}

#[derive(Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum WebCommand {
    Start,
    Stop,
    Pause { on: bool },
    ClearTrace,
}

async fn command(State(api): State<ApiState>, Json(cmd): Json<WebCommand>) -> StatusCode {
    match api.commands.send(cmd) {
        Ok(()) => StatusCode::ACCEPTED,
        Err(_) => StatusCode::SERVICE_UNAVAILABLE,
    }
}

/// Owns the App, applies UI commands, and publishes the web snapshot. The
/// manual drive is what the headless CLI already uses: no window, no
/// mailbox race, commands take effect on the next lap.
fn app_loop(
    inbox: Receiver<WebCommand>,
    shared: Arc<Mutex<WebState>>,
    trace: Arc<Mutex<TraceStore>>,
) {
    let mut app = App::headless();
    seed_demo(&mut app);
    app.start_virtual();
    let mut series: VecDeque<WebSeriesPoint> = VecDeque::with_capacity(SERIES_POINTS);
    let mut rate_window: VecDeque<(Instant, u64)> = VecDeque::new();
    let mut last_publish = Instant::now() - PUBLISH_EVERY;

    loop {
        while let Ok(cmd) = inbox.try_recv() {
            match cmd {
                WebCommand::Start => {
                    if !app.snap.measuring {
                        app.start_virtual();
                        rate_window.clear();
                    }
                }
                WebCommand::Stop => {
                    if app.snap.measuring {
                        app.stop();
                        rate_window.clear();
                    }
                }
                WebCommand::Pause { on } => {
                    app.send(BusCommand::SetTracePaused(on));
                }
                WebCommand::ClearTrace => {
                    app.send(BusCommand::ClearTrace);
                }
            }
        }
        app.update();
        if last_publish.elapsed() >= PUBLISH_EVERY {
            let now = Instant::now();
            rate_window.push_back((now, app.snap.frame_counter));
            while rate_window.len() > 2
                && now.duration_since(rate_window[0].0) > Duration::from_secs(1)
            {
                rate_window.pop_front();
            }
            let (started, frames_then) = rate_window[0];
            let elapsed = now.duration_since(started).as_secs_f64();
            // A one-second window reads as the bus rate; the per-lap EMA
            // swings with the generator's batch arrivals and makes the
            // chart look like noise.
            // A one-second window needs a moment to fill; feeding the
            // frontend the per-lap EMA before that put a 500 f/s spike at
            // the left edge and squashed the whole chart.
            let mut fps = if elapsed >= 0.5 {
                app.snap.frame_counter.saturating_sub(frames_then) as f64 / elapsed
            } else {
                0.0
            };
            let bus_count = app.snap.channels.len();
            let (mut bus_rates, mut bus_loads) = if app.snap.measuring {
                (
                    app.snap
                        .bus_loads
                        .iter()
                        .map(|load| load.frame_rate())
                        .collect(),
                    app.snap
                        .bus_loads
                        .iter()
                        .map(|load| load.load() * 100.0)
                        .collect(),
                )
            } else {
                fps = 0.0;
                (vec![0.0; bus_count], vec![0.0; bus_count])
            };
            if bus_rates.len() < bus_count {
                bus_rates.resize(bus_count, 0.0);
            }
            if bus_loads.len() < bus_count {
                bus_loads.resize(bus_count, 0.0);
            }
            series.push_back(WebSeriesPoint {
                total: fps,
                bus_rates,
                bus_loads,
            });
            if series.len() > SERIES_POINTS {
                series.pop_front();
            }
            if let Ok(mut store) = trace.lock() {
                store.sync(&app);
            }
            let next = build_state(&app, &series, fps);
            if let Ok(mut slot) = shared.lock() {
                *slot = next;
            }
            last_publish = now;
        }
        std::thread::sleep(TICK_EVERY);
    }
}

/// A demo simulation without a project file: open every DBC node's gate,
/// switch every generator entry on, and run it at 20 Hz so the tables move
/// on first paint.
fn seed_demo(app: &mut App) {
    let bus_count = app.snap.channel_count;
    for ch in 0..bus_count {
        let nodes: Vec<String> = app
            .snap
            .channels
            .get(ch)
            .and_then(|c| c.dbc.as_ref())
            .map(|db| db.nodes.clone())
            .unwrap_or_default();
        for node in nodes {
            app.set_node_role(ch as u8, &node, NodeRole::Simulated);
        }
    }
    let entries: Vec<(u8, u32)> = app
        .snap
        .tx
        .iter()
        .map(|entry| (entry.channel, entry.id))
        .collect();
    for (ch, id) in entries {
        app.send(BusCommand::SetEntryActive { ch, id, on: true });
        app.send(BusCommand::SetEntryCycle {
            ch,
            id,
            cycle_us: 50_000,
        });
    }
    app.start_virtual();
}

/// Chronological trace buffer owned by the app thread. The API sends a seed
/// on first contact and only the frames appended since the client's last
/// sequence afterwards; the frontend keeps the 5k window and virtualizes it,
/// so a full table never costs a full JSON snapshot per poll.
#[derive(Default)]
struct TraceStore {
    epoch: u64,
    next_seq: u64,
    log: VecDeque<(u64, WebFrame)>,
    last_len: usize,
    last_loss: u64,
    primed: bool,
}

impl TraceStore {
    fn sync(&mut self, app: &App) {
        let len = app.snap.trace.len();
        let loss = app.snap.trace.head_loss().0;
        let reset = !self.primed || len < self.last_len || loss != self.last_loss;
        if reset {
            self.primed = true;
            self.epoch = self.epoch.wrapping_add(1).max(1);
            self.next_seq = 0;
            self.log.clear();
            let take = len.min(TRACE_LOG_CAP);
            let mut rows: Vec<WebFrame> = app
                .snap
                .trace
                .iter()
                .rev()
                .take(take)
                .map(|frame| frame_dto(app, frame))
                .collect();
            rows.reverse();
            for row in rows {
                self.push(row);
            }
        } else if len > self.last_len {
            let take = (len - self.last_len).min(TRACE_LOG_CAP);
            let mut rows: Vec<WebFrame> = app
                .snap
                .trace
                .iter()
                .rev()
                .take(take)
                .map(|frame| frame_dto(app, frame))
                .collect();
            rows.reverse();
            for row in rows {
                self.push(row);
            }
        }
        self.last_len = len;
        self.last_loss = loss;
    }

    fn push(&mut self, frame: WebFrame) {
        self.log.push_back((self.next_seq, frame));
        self.next_seq += 1;
        while self.log.len() > TRACE_LOG_CAP {
            self.log.pop_front();
        }
    }

    fn first_seq(&self) -> u64 {
        self.log
            .front()
            .map(|(seq, _)| *seq)
            .unwrap_or(self.next_seq)
    }

    /// Newest first, ready to prepend on the client.
    fn seed(&self) -> Vec<WebFrame> {
        self.log
            .iter()
            .rev()
            .map(|(_, frame)| frame.clone())
            .collect()
    }

    fn after(&self, seq: u64) -> Vec<WebFrame> {
        self.log
            .iter()
            .rev()
            .filter(|(s, _)| *s > seq)
            .map(|(_, frame)| frame.clone())
            .collect()
    }
}

fn frame_dto(app: &App, frame: &CanFrame) -> WebFrame {
    let channel = app.snap.channels.get(frame.channel as usize);
    WebFrame {
        t_us: frame.t_us,
        bus: frame.channel,
        bus_name: channel
            .map(|c| c.name.clone())
            .unwrap_or_else(|| format!("CAN{}", frame.channel + 1)),
        id: frame.id,
        extended: frame.extended,
        name: channel
            .and_then(|c| c.dbc.as_ref())
            .and_then(|db| db.message_name(frame.id))
            .unwrap_or("")
            .to_string(),
        dir: match frame.dir {
            Direction::Rx => "Rx",
            Direction::Tx => "Tx",
        },
        len: frame.len,
        data: hex(frame.payload()),
        fd: frame.is_fd(),
    }
}

#[derive(Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
struct WebState {
    measuring: bool,
    trace_paused: bool,
    mode: String,
    frames: u64,
    fps: f64,
    trace_len: usize,
    sim_time_us: u64,
    status: String,
    channels: Vec<WebChannel>,
    messages: Vec<WebMessage>,
    write: Vec<WebLine>,
    series: Vec<WebSeriesPoint>,
}

/// One chart sample: total plus every bus. Rates are frames/s, loads are
/// percent. The frontend draws solid rate lines and dashed load lines.
#[derive(Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
struct WebSeriesPoint {
    total: f64,
    bus_rates: Vec<f64>,
    bus_loads: Vec<f64>,
}

#[derive(Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
struct WebChannel {
    name: String,
    bitrate_kbps: u32,
    fd_data_kbps: u32,
    database: bool,
}

#[derive(Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
struct WebFrame {
    t_us: u64,
    bus: u8,
    bus_name: String,
    id: u32,
    extended: bool,
    name: String,
    dir: &'static str,
    len: u8,
    data: String,
    fd: bool,
}

#[derive(Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
struct WebMessage {
    bus: u8,
    bus_name: String,
    id: u32,
    extended: bool,
    name: String,
    count: u64,
    cycle_ms: f64,
    len: u8,
    data: String,
}

#[derive(Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
struct WebLine {
    wall_us: u64,
    kind: &'static str,
    text: String,
}

fn build_state(app: &App, series: &VecDeque<WebSeriesPoint>, fps: f64) -> WebState {
    let snap = &app.snap;
    let channels: Vec<WebChannel> = snap
        .channels
        .iter()
        .map(|ch| WebChannel {
            name: ch.name.clone(),
            bitrate_kbps: ch.bitrate_kbps,
            fd_data_kbps: ch.fd_data_kbps,
            database: ch.dbc.is_some(),
        })
        .collect();
    let bus_name = |bus: u8| {
        channels
            .get(bus as usize)
            .map(|c| c.name.clone())
            .unwrap_or_else(|| format!("CAN{}", bus + 1))
    };
    let db = |bus: u8| snap.channels.get(bus as usize).and_then(|c| c.dbc.as_ref());

    let mut messages: Vec<WebMessage> = snap
        .aggs
        .iter()
        .map(|m| WebMessage {
            bus: m.channel,
            bus_name: bus_name(m.channel),
            id: m.id,
            extended: m.extended,
            name: db(m.channel)
                .and_then(|d| d.message_name(m.id))
                .unwrap_or("")
                .to_string(),
            count: m.count,
            cycle_ms: m.cycle_us / 1000.0,
            len: m.len,
            data: hex(m.payload()),
        })
        .collect();
    messages.sort_by_key(|m| (m.bus, m.id));

    let write: Vec<WebLine> = snap
        .write
        .iter()
        .rev()
        .take(6)
        .rev()
        .map(|line| WebLine {
            wall_us: line.wall_us,
            kind: match line.kind {
                crate::bus::WriteKind::Script => "script",
                crate::bus::WriteKind::Info => "info",
                crate::bus::WriteKind::Warning => "warn",
                crate::bus::WriteKind::Error => "error",
            },
            text: line.text.clone(),
        })
        .collect();

    WebState {
        measuring: snap.measuring,
        trace_paused: snap.trace_paused,
        mode: match snap.mode {
            Mode::Virtual => "Simulation".to_string(),
            Mode::Replay => "Replay".to_string(),
        },
        frames: snap.frame_counter,
        fps,
        trace_len: snap.trace_len,
        sim_time_us: snap.sim_t_us,
        status: app.status.clone(),
        channels,
        messages,
        write,
        series: series.iter().cloned().collect(),
    }
}

fn hex(bytes: &[u8]) -> String {
    use std::fmt::Write as _;
    let mut out = String::with_capacity(bytes.len() * 3);
    for (i, byte) in bytes.iter().enumerate() {
        if i > 0 {
            out.push(' ');
        }
        let _ = write!(out, "{byte:02X}");
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The demo path must produce real frames through the headless App; a
    /// broken seed would otherwise leave the web UI looking connected but
    /// permanently empty.
    #[test]
    fn the_demo_seeds_and_publishes_real_traffic() {
        let mut app = App::headless();
        seed_demo(&mut app);
        for _ in 0..40 {
            std::thread::sleep(Duration::from_millis(5));
            app.update();
        }
        let series = VecDeque::new();
        let state = build_state(&app, &series, app.frame_rate);
        assert_eq!(state.channels.len(), 2);
        assert!(state.frames > 0, "the generator ran");
        assert!(!state.messages.is_empty(), "the aggregates saw traffic");

        let mut trace = TraceStore::default();
        trace.sync(&app);
        let seed = trace.seed();
        assert!(!seed.is_empty(), "the trace log has rows");
        assert!(
            seed.iter().any(|frame| !frame.name.is_empty()),
            "at least one row decoded through its DBC"
        );
        let before = trace.next_seq;
        for _ in 0..20 {
            std::thread::sleep(Duration::from_millis(5));
            app.update();
        }
        trace.sync(&app);
        assert!(trace.next_seq > before, "later laps append rows");
        assert!(
            !trace.after(before).is_empty(),
            "the client only needs the delta"
        );
    }
}
