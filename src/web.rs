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
use crate::bus::{BusCommand, SubView};
use crate::can::frame::{CanFrame, Direction};
use crate::observe::SigKey;
use crate::sim::{SrcKind, ValueSrc};

const PORT: u16 = 2010;
const SERIES_POINTS: usize = 120;
/// Rows the web frontend keeps. This matches the source ring's 50k limit
/// and CANoe's ~100k default order of magnitude.
const TRACE_LOG_CAP: usize = 50_000;
/// Samples kept per plotted signal. At the 5 ms stress stride this is a
/// one-minute window; the renderer decimates to pixels.
const SIGNAL_CAP: usize = 12_000;
/// Demo generator period; `ROXY_WEB_CYCLE_US` can shorten it for stress runs.
const DEMO_CYCLE_US: u64 = 50_000;

/// Demo curves: real DBC signals, driven by the generator's own value
/// sources. `color` indexes the frontend palette; ranges stay inside the
/// DBC declarations so the encoder never clamps the shape.
struct SignalSpec {
    ch: u8,
    id: u32,
    name: &'static str,
    color: usize,
    kind: SrcKind,
    lo: f64,
    hi: f64,
    period_us: u64,
    phase_us: u64,
    redraw_us: u64,
}

const SIGNALS: [SignalSpec; 8] = [
    SignalSpec {
        ch: 0,
        id: 0x100,
        name: "EngineSpeed",
        color: 0,
        kind: SrcKind::Sine,
        lo: 800.0,
        hi: 7000.0,
        period_us: 5_000_000,
        phase_us: 0,
        redraw_us: 0,
    },
    SignalSpec {
        ch: 0,
        id: 0x100,
        name: "ThrottlePos",
        color: 1,
        kind: SrcKind::Triangle,
        lo: 0.0,
        hi: 100.0,
        period_us: 3_000_000,
        phase_us: 400_000,
        redraw_us: 0,
    },
    SignalSpec {
        ch: 0,
        id: 0x200,
        name: "VehicleSpeed",
        color: 2,
        kind: SrcKind::Triangle,
        lo: 0.0,
        hi: 220.0,
        period_us: 12_000_000,
        phase_us: 0,
        redraw_us: 0,
    },
    SignalSpec {
        ch: 0,
        id: 0x320,
        name: "BrakePressure",
        color: 3,
        kind: SrcKind::Sine,
        lo: 0.0,
        hi: 90.0,
        period_us: 7_000_000,
        phase_us: 500_000,
        redraw_us: 0,
    },
    SignalSpec {
        ch: 1,
        id: 0x64,
        name: "EngSpeed",
        color: 4,
        kind: SrcKind::Ramp,
        lo: 0.0,
        hi: 6000.0,
        period_us: 8_000_000,
        phase_us: 0,
        redraw_us: 0,
    },
    SignalSpec {
        ch: 1,
        id: 0xC9,
        name: "CarSpeed",
        color: 5,
        kind: SrcKind::Sine,
        lo: 0.0,
        hi: 260.0,
        period_us: 10_000_000,
        phase_us: 0,
        redraw_us: 0,
    },
    SignalSpec {
        ch: 1,
        id: 0xC8,
        name: "WheelSpeedFL",
        color: 6,
        kind: SrcKind::Random,
        lo: 0.0,
        hi: 1200.0,
        period_us: 1_000_000,
        phase_us: 0,
        redraw_us: 50_000,
    },
    SignalSpec {
        ch: 1,
        id: 0xC8,
        name: "WheelSpeedRR",
        color: 7,
        kind: SrcKind::Random,
        lo: 0.0,
        hi: 1200.0,
        period_us: 1_000_000,
        phase_us: 0,
        redraw_us: 50_000,
    },
];
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
    let project = flag_value(args, "--project");
    let replay = flag_value(args, "--replay");
    if !std::path::Path::new("web/dist/index.html").exists() {
        eprintln!("web/dist is missing; run `npm install && npm run build` in web/ first");
    }
    if let Some(path) = &replay {
        println!("web session replay: {path}");
    } else if let Some(path) = &project {
        println!("web session project: {path}");
    }

    let (tx, rx) = std::sync::mpsc::channel();
    let shared = Arc::new(Mutex::new(WebState::default()));
    let trace = Arc::new(Mutex::new(TraceStore::default()));
    let signals = Arc::new(Mutex::new(SignalStore::default()));
    {
        let shared = shared.clone();
        let trace = trace.clone();
        let signals = signals.clone();
        std::thread::Builder::new()
            .name("roxy-web-core".to_string())
            .spawn(move || app_loop(rx, shared, trace, signals, project, replay))
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
                signals,
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

fn flag_value(args: &[String], flag: &str) -> Option<String> {
    args.iter()
        .position(|arg| arg == flag)
        .and_then(|i| args.get(i + 1))
        .cloned()
}

/// Bring up the requested session: a saved project, a replay log, or the
/// generator demo. A project without a log opens stopped; a log starts
/// playback; no flags keeps the old demo behavior.
fn prepare_session(app: &mut App, project: Option<&str>, replay: Option<&str>) {
    if let Some(path) = project {
        app.open_project_path(std::path::Path::new(path));
    }
    if let Some(path) = replay {
        app.load_log(path);
        if !app.log_path.is_empty() {
            app.send(BusCommand::SetRunMode(Mode::Replay));
            app.start_selected();
        }
        return;
    }
    if project.is_none() {
        seed_demo(app);
    }
}

#[derive(Clone)]
struct ApiState {
    shared: Arc<Mutex<WebState>>,
    trace: Arc<Mutex<TraceStore>>,
    signals: Arc<Mutex<SignalStore>>,
    commands: Sender<WebCommand>,
}

#[derive(Default, Deserialize)]
struct TraceQuery {
    epoch: Option<u64>,
    seq: Option<u64>,
    signals_epoch: Option<u64>,
    signals_since: Option<u64>,
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
    signals_epoch: u64,
    signals_max_t: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    signals_seed: Option<Vec<SignalSeries>>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    signals_delta: Vec<SignalSeries>,
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
    let (signals_epoch, signals_max_t, signals_seed, signals_delta) = {
        let store = api.signals.lock().unwrap_or_else(|p| p.into_inner());
        let epoch = store.epoch;
        let max_t = store.max_t();
        let first = store.earliest_t();
        let synced = query.signals_epoch == Some(epoch)
            && query
                .signals_since
                .is_some_and(|client| client >= first && client <= max_t);
        if synced {
            (
                epoch,
                max_t,
                None,
                store.after(query.signals_since.unwrap_or(max_t)),
            )
        } else {
            (epoch, max_t, Some(store.seed()), Vec::new())
        }
    };
    Json(WebResponse {
        state: snapshot,
        trace_epoch: epoch,
        trace_seq: seq,
        trace_seed: seed,
        trace_delta: delta,
        signals_epoch,
        signals_max_t,
        signals_seed,
        signals_delta,
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
    signals: Arc<Mutex<SignalStore>>,
    project: Option<String>,
    replay: Option<String>,
) {
    let mut app = App::headless();
    prepare_session(&mut app, project.as_deref(), replay.as_deref());
    let mut series: VecDeque<WebSeriesPoint> = VecDeque::with_capacity(SERIES_POINTS);
    let mut rate_window: VecDeque<(Instant, u64)> = VecDeque::new();
    let mut last_publish = Instant::now() - PUBLISH_EVERY;

    loop {
        while let Ok(cmd) = inbox.try_recv() {
            match cmd {
                WebCommand::Start => {
                    if !app.snap.measuring {
                        if app.snap.run_mode == Mode::Replay && app.log_path.is_empty() {
                            app.status =
                                "replay needs a log; start with --replay <path>".to_string();
                        } else {
                            app.start_selected();
                            rate_window.clear();
                        }
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
            if let Ok(mut store) = signals.lock() {
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
    let cycle_us = std::env::var("ROXY_WEB_CYCLE_US")
        .ok()
        .and_then(|value| value.parse::<u64>().ok())
        .filter(|value| *value >= 1_000)
        .unwrap_or(DEMO_CYCLE_US);
    let entries: Vec<(u8, u32)> = app
        .snap
        .tx
        .iter()
        .map(|entry| (entry.channel, entry.id))
        .collect();
    for (ch, id) in entries {
        app.send(BusCommand::SetEntryActive { ch, id, on: true });
        app.send(BusCommand::SetEntryCycle { ch, id, cycle_us });
    }
    // A 1 s Graphics window asks the core for the finest sampling stride
    // (5 ms); the 50 ms default would only give each curve 20 points/s.
    if let Some(window) = app.graphics.first_mut() {
        window.time_window_s = 1.0;
        window.opened = true;
    }
    for spec in &SIGNALS {
        let mut src = ValueSrc::new(spec.name, spec.kind, spec.lo, spec.hi);
        src.period_us = spec.period_us;
        src.phase_us = spec.phase_us;
        src.redraw_us = spec.redraw_us;
        // Random wheels would otherwise share one sequence and overlap
        // exactly on the plot.
        src.seed ^= (spec.color as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15);
        app.send(BusCommand::SetEntrySource {
            ch: spec.ch,
            id: spec.id,
            src,
        });
        app.subscribe(SigKey::can(spec.ch, spec.id, false, spec.name));
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
    /// Total frames the source ring has ever held this run (`len + lost`).
    /// Using the total, not `len`, lets a full ring trim its head without
    /// looking like a reset on every publish.
    last_total: u64,
    primed: bool,
}

impl TraceStore {
    fn sync(&mut self, app: &App) {
        let len = app.snap.trace.len();
        let loss = app.snap.trace.head_loss().0;
        let total = len as u64 + loss;
        let new_count = total.saturating_sub(self.last_total) as usize;
        let reset = !self.primed || total < self.last_total || new_count > TRACE_LOG_CAP;
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
        } else if new_count > 0 {
            let mut rows: Vec<WebFrame> = app
                .snap
                .trace
                .iter()
                .rev()
                .take(new_count)
                .map(|frame| frame_dto(app, frame))
                .collect();
            rows.reverse();
            for row in rows {
                self.push(row);
            }
        }
        self.last_total = total;
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
struct SignalSeries {
    key: String,
    name: String,
    bus: String,
    unit: String,
    color: usize,
    /// `[seconds, value]`, oldest first.
    points: Vec<[f64; 2]>,
}

struct SignalBuffer {
    meta: SignalSeries,
    points: VecDeque<(u64, f64)>,
}

/// Incremental cache of the subscribed signal histories. The frontend asks
/// once for a seed and then for everything newer than its last timestamp;
/// the server never resends the two-hour sample ring.
#[derive(Default)]
struct SignalStore {
    epoch: u64,
    primed: bool,
    buffers: Vec<SignalBuffer>,
    last_t: Vec<u64>,
}

impl SignalStore {
    fn sync(&mut self, app: &App) {
        let mut reset = !self.primed;
        if self.primed {
            for (i, spec) in SIGNALS.iter().enumerate() {
                let Some(sub) = signal_sub(app, spec) else {
                    continue;
                };
                let last = sub.history.iter().last().map(|(t, _)| *t).unwrap_or(0);
                if self.last_t.get(i).copied().unwrap_or(0) > last {
                    reset = true;
                }
            }
        }
        if reset {
            self.primed = true;
            self.epoch = self.epoch.wrapping_add(1).max(1);
            self.buffers.clear();
            self.last_t = vec![0; SIGNALS.len()];
            for (i, spec) in SIGNALS.iter().enumerate() {
                let sub = signal_sub(app, spec);
                let meta = signal_meta(app, spec, sub);
                let mut points = VecDeque::new();
                if let Some(sub) = sub {
                    let all: Vec<(u64, f64)> = sub.history.iter().copied().collect();
                    let start = all.len().saturating_sub(SIGNAL_CAP);
                    points.extend(all[start..].iter().copied());
                    if let Some((t, _)) = points.back() {
                        self.last_t[i] = *t;
                    }
                }
                self.buffers.push(SignalBuffer { meta, points });
            }
            return;
        }

        for (i, spec) in SIGNALS.iter().enumerate() {
            let Some(sub) = signal_sub(app, spec) else {
                continue;
            };
            let Some(buffer) = self.buffers.get_mut(i) else {
                continue;
            };
            buffer.meta = signal_meta(app, spec, Some(sub));
            let last = self.last_t.get(i).copied().unwrap_or(0);
            for &(t, value) in sub.history.iter() {
                if t > last {
                    buffer.points.push_back((t, value));
                }
            }
            while buffer.points.len() > SIGNAL_CAP {
                buffer.points.pop_front();
            }
            if let Some((t, _)) = buffer.points.back() {
                self.last_t[i] = *t;
            }
        }
    }

    fn max_t(&self) -> u64 {
        self.last_t.iter().copied().max().unwrap_or(0)
    }

    fn earliest_t(&self) -> u64 {
        self.buffers
            .iter()
            .filter_map(|buffer| buffer.points.front().map(|(t, _)| *t))
            .min()
            .unwrap_or(self.max_t())
    }

    fn seed(&self) -> Vec<SignalSeries> {
        self.buffers
            .iter()
            .filter(|buffer| !buffer.points.is_empty())
            .map(|buffer| Self::dto(buffer, 0))
            .collect()
    }

    fn after(&self, since: u64) -> Vec<SignalSeries> {
        self.buffers
            .iter()
            .filter(|buffer| buffer.points.back().is_some_and(|(t, _)| *t > since))
            .map(|buffer| Self::dto(buffer, since))
            .collect()
    }

    fn dto(buffer: &SignalBuffer, since: u64) -> SignalSeries {
        let mut series = buffer.meta.clone();
        series.points = buffer
            .points
            .iter()
            .filter(|(t, _)| *t > since)
            .map(|(t, value)| [*t as f64 / 1e6, *value])
            .collect();
        series
    }
}

fn signal_sub<'a>(app: &'a App, spec: &SignalSpec) -> Option<&'a SubView> {
    app.snap
        .subs
        .iter()
        .find(|sub| sub.key == SigKey::can(spec.ch, spec.id, false, spec.name))
}

fn signal_meta(app: &App, spec: &SignalSpec, sub: Option<&SubView>) -> SignalSeries {
    SignalSeries {
        key: format!("{}:{}:{}", spec.ch, spec.id, spec.name),
        name: spec.name.to_string(),
        bus: app
            .snap
            .channels
            .get(spec.ch as usize)
            .map(|channel| channel.name.clone())
            .unwrap_or_else(|| format!("CAN{}", spec.ch + 1)),
        unit: sub.map(|sub| sub.unit.clone()).unwrap_or_default(),
        color: spec.color,
        points: Vec::new(),
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
    project: String,
    log: String,
    log_info: String,
    run_mode: String,
    replay_pos_us: Option<u64>,
    replay_total_us: Option<u64>,
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
        project: app
            .project_path
            .as_ref()
            .and_then(|path| path.file_name())
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_default(),
        log: std::path::Path::new(&app.log_path)
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_default(),
        log_info: app.log_info.clone().unwrap_or_default(),
        run_mode: match snap.run_mode {
            Mode::Virtual => "Simulation".to_string(),
            Mode::Replay => "Replay".to_string(),
        },
        replay_pos_us: snap.replay.map(|(pos, _)| pos),
        replay_total_us: snap.replay.map(|(_, total)| total),
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

    /// A replay path must open the log and start playback in Replay mode,
    /// without falling back to the interactive file picker.
    #[test]
    fn a_replay_path_starts_the_web_session() {
        let path = std::env::temp_dir().join("roxy_web_session_replay.asc");
        let text = "date Sat Jan 01 00:00:00.000 2022\n\
                    base hex  timestamps absolute\n\
                    Begin Triggerblock Sat Jan 01 00:00:00.000 2022\n\
                    1.0 1 416 Rx d 8 17 34 51 68 85 102 119 136\n\
                    2.0 1 416 Rx d 8 17 34 51 68 85 102 119 136\n\
                    End TriggerBlock\n";
        std::fs::write(&path, text).expect("write replay fixture");
        let mut app = App::headless();
        prepare_session(&mut app, None, Some(path.to_str().unwrap()));
        assert_eq!(app.snap.run_mode, Mode::Replay);
        assert!(!app.log_path.is_empty(), "the log was accepted");
        assert!(app.snap.measuring, "playback started");
        assert!(app.snap.replay.is_some(), "the playhead is visible");
        std::fs::remove_file(&path).ok();
    }

    /// The demo must actually drive and sample the plotted signals; an empty
    /// SignalStore would draw eight empty lanes forever.
    #[test]
    fn the_demo_samples_the_plotted_signals() {
        let mut app = App::headless();
        seed_demo(&mut app);
        for _ in 0..40 {
            std::thread::sleep(Duration::from_millis(5));
            app.update();
        }
        let mut signals = SignalStore::default();
        signals.sync(&app);
        let seeded = signals.seed();
        assert_eq!(seeded.len(), SIGNALS.len());
        assert!(
            seeded.iter().any(|series| !series.points.is_empty()),
            "the subscriptions received samples"
        );
        assert!(
            seeded
                .iter()
                .flat_map(|series| series.points.iter())
                .any(|[_, value]| *value != 0.0),
            "the generator sources produced non-zero signal values"
        );
    }

    /// At the cap the store must trim its oldest row without discarding the
    /// sequence range the client already holds.
    #[test]
    fn the_trace_store_keeps_the_newest_rows_at_the_cap() {
        let mut store = TraceStore::default();
        for i in 0..TRACE_LOG_CAP + 10 {
            store.push(WebFrame {
                t_us: i as u64,
                ..Default::default()
            });
        }
        assert_eq!(store.log.len(), TRACE_LOG_CAP);
        assert_eq!(store.first_seq(), 10);
        let tail = store.after(store.next_seq - 6);
        assert_eq!(tail.len(), 5);
        assert_eq!(tail[0].t_us, (TRACE_LOG_CAP + 9) as u64);
    }
}
