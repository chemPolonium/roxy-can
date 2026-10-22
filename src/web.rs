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

use axum::extract::State;
use axum::http::StatusCode;
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::{Deserialize, Serialize};
use tower_http::services::{ServeDir, ServeFile};

use crate::app::{App, Mode, NodeRole};
use crate::bus::BusCommand;
use crate::can::frame::Direction;

const PORT: u16 = 3000;
const SERIES_POINTS: usize = 60;
const TRACE_ROWS: usize = 240;
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
    {
        let shared = shared.clone();
        std::thread::Builder::new()
            .name("roxy-web-core".to_string())
            .spawn(move || app_loop(rx, shared))
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
    commands: Sender<WebCommand>,
}

async fn state(State(api): State<ApiState>) -> Json<WebState> {
    let snapshot = api.shared.lock().map(|s| s.clone()).unwrap_or_default();
    Json(snapshot)
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
fn app_loop(inbox: Receiver<WebCommand>, shared: Arc<Mutex<WebState>>) {
    let mut app = App::headless();
    seed_demo(&mut app);
    app.start_virtual();
    let mut series: VecDeque<f64> = VecDeque::with_capacity(SERIES_POINTS);
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
            let fps = if elapsed >= 0.2 {
                app.snap.frame_counter.saturating_sub(frames_then) as f64 / elapsed
            } else {
                app.frame_rate
            };
            series.push_back(fps);
            if series.len() > SERIES_POINTS {
                series.pop_front();
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
    trace: Vec<WebFrame>,
    messages: Vec<WebMessage>,
    write: Vec<WebLine>,
    series: Vec<f64>,
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

fn build_state(app: &App, series: &VecDeque<f64>, fps: f64) -> WebState {
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

    let trace: Vec<WebFrame> = snap
        .trace
        .iter()
        .rev()
        .take(TRACE_ROWS)
        .map(|f| WebFrame {
            t_us: f.t_us,
            bus: f.channel,
            bus_name: bus_name(f.channel),
            id: f.id,
            extended: f.extended,
            name: db(f.channel)
                .and_then(|d| d.message_name(f.id))
                .unwrap_or("")
                .to_string(),
            dir: match f.dir {
                Direction::Rx => "Rx",
                Direction::Tx => "Tx",
            },
            len: f.len,
            data: hex(f.payload()),
            fd: f.is_fd(),
        })
        .collect();

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
        trace,
        messages,
        write,
        series: series.iter().copied().collect(),
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
        let series = VecDeque::from([app.frame_rate]);
        let state = build_state(&app, &series, app.frame_rate);
        assert_eq!(state.channels.len(), 2);
        assert!(state.frames > 0, "the generator ran");
        assert!(!state.trace.is_empty(), "the trace ring has rows");
        assert!(!state.messages.is_empty(), "the aggregates saw traffic");
        assert!(
            state.trace.iter().any(|frame| !frame.name.is_empty()),
            "at least one row decoded through its DBC"
        );
    }
}
