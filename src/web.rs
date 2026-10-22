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
use crate::hw::HwDriver;
use crate::observe::SigKey;
use crate::sim::{SrcKind, ValueSrc};
use crate::trigger::{TriggerAction, TriggerCmd, TriggerCond, TriggerKind};

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
    let catalog = Arc::new(Mutex::new(WebCatalog::default()));
    let hardware = Arc::new(Mutex::new(HardwareCache::default()));
    {
        let shared = shared.clone();
        let trace = trace.clone();
        let signals = signals.clone();
        let catalog = catalog.clone();
        std::thread::Builder::new()
            .name("roxy-web-core".to_string())
            .spawn(move || app_loop(rx, shared, trace, signals, catalog, project, replay))
            .expect("spawn web core thread");
    }

    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("build tokio runtime");
    runtime.block_on(async move {
        let api = Router::new()
            .route("/state", get(state))
            .route("/catalog", get(catalog_handler))
            .route("/hardware", get(hardware_handler))
            .route("/command", post(command))
            .with_state(ApiState {
                shared,
                trace,
                signals,
                catalog,
                hardware,
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

fn reset_visual_stores(
    trace: &Arc<Mutex<TraceStore>>,
    series: &mut VecDeque<WebSeriesPoint>,
    rate_window: &mut VecDeque<(Instant, u64)>,
) {
    if let Ok(mut store) = trace.lock() {
        store.reset();
    }
    series.clear();
    rate_window.clear();
}

fn read_project_layout(path: &std::path::Path) -> String {
    std::fs::read_to_string(path)
        .ok()
        .and_then(|text| serde_json::from_str::<serde_json::Value>(&text).ok())
        .and_then(|value| {
            value
                .get("layout")
                .and_then(|layout| layout.as_str())
                .map(str::to_string)
        })
        .unwrap_or_default()
}

/// Validates a system-variable definition coming from the web form. The
/// desktop editor applies the same rules; the host is the boundary, so the
/// web path checks them again instead of trusting the page.
fn checked_sysvar(
    namespace: String,
    name: String,
    init: f64,
    min: Option<f64>,
    max: Option<f64>,
    unit: String,
    comment: String,
) -> Result<crate::bus::SysVarDef, String> {
    let namespace = namespace.trim().to_string();
    let name = name.trim().to_string();
    if namespace.is_empty() || name.is_empty() {
        return Err("namespace and name must not be empty".into());
    }
    if namespace.contains("::") || name.contains("::") {
        return Err("namespace and name must not contain \"::\"".into());
    }
    if let (Some(lo), Some(hi)) = (min, max)
        && lo > hi
    {
        return Err(format!("lower bound {lo} is greater than upper bound {hi}"));
    }
    Ok(crate::bus::SysVarDef {
        namespace,
        name,
        init,
        min,
        max,
        unit: unit.trim().to_string(),
        comment: comment.trim().to_string(),
    })
}

/// Applies the editable parts of a Monitor row (not its signal). The row is
/// UI/project state, so the web edit lands in place; only adding/removing a
/// signal touches the bus.
fn edit_monitor_row(
    app: &mut App,
    key: &SigKey,
    digits: Option<usize>,
    rule_on: Option<bool>,
    rising: Option<bool>,
    threshold: Option<f64>,
    color: Option<usize>,
) {
    let Some(row) = app.monitor_view.rows.iter_mut().find(|row| &row.key == key) else {
        return;
    };
    if let Some(digits) = digits {
        row.digits = digits.min(3);
    }
    if let Some(on) = rule_on {
        row.rule_on = on;
    }
    if let Some(rising) = rising {
        row.rising = rising;
    }
    if let Some(threshold) = threshold {
        row.threshold = threshold;
    }
    if let Some(color) = color {
        row.color = color;
    }
}

/// Six-significant-digit state identity, the same quantization the desktop
/// State Tracker uses: tiny numeric jitter does not split a band.
fn quantize_state(v: f64) -> f64 {
    if v == 0.0 || !v.is_finite() {
        return v;
    }
    let mag = v.abs().log10().floor() as i32;
    let digits = (5 - mag).clamp(0, 9);
    let p = 10f64.powi(digits);
    (v * p).round() / p
}

fn state_label(v: f64) -> String {
    format!("{v}")
}

/// Turns visible samples into at most one band per pixel column (360). Each
/// column keeps the last value seen in it, and empty columns carry the last
/// state forward, so a signal that updates once a second still paints its
/// band across the whole window.
fn state_bands_from(
    held: Option<f64>,
    samples: &[(u64, f64)],
    lo: u64,
    hi: u64,
) -> (String, Vec<WebStateBand>) {
    const COLS: usize = 360;
    let span = hi.saturating_sub(lo).max(1);
    let mut values: Vec<Option<f64>> = vec![None; COLS];
    for &(t, v) in samples {
        let idx =
            (((t.saturating_sub(lo)) as u128 * COLS as u128 / span as u128) as usize).min(COLS - 1);
        values[idx] = Some(quantize_state(v));
    }
    let mut last = held.map(quantize_state);
    for slot in &mut values {
        if let Some(v) = *slot {
            last = Some(v);
        } else {
            *slot = last;
        }
    }
    let mut bands = Vec::new();
    let mut open: Option<(f64, usize)> = None;
    for (i, slot) in values.iter().enumerate() {
        let value = *slot;
        match open {
            Some((open_value, _)) if Some(open_value) == value => {}
            Some((open_value, start)) => {
                bands.push(WebStateBand {
                    start: start as f64 / COLS as f64,
                    end: i as f64 / COLS as f64,
                    label: state_label(open_value),
                    color: 0,
                });
                open = value.map(|v| (v, i));
            }
            None => open = value.map(|v| (v, i)),
        }
    }
    if let Some((value, start)) = open {
        bands.push(WebStateBand {
            start: start as f64 / COLS as f64,
            end: 1.0,
            label: state_label(value),
            color: 0,
        });
    }
    // Few states get distinct palette slots in first-seen order; an analog
    // sweep (more states than the palette) is drawn as one neutral band by
    // the frontend instead of a rainbow of one-pixel stripes.
    let mut seen: Vec<&str> = Vec::new();
    for band in &mut bands {
        let slot = seen
            .iter()
            .position(|label| *label == band.label)
            .unwrap_or_else(|| {
                seen.push(&band.label);
                seen.len() - 1
            });
        band.color = slot % 8;
    }
    let current = last.map(state_label).unwrap_or_else(|| "-".to_string());
    (current, bands)
}

fn trigger_kind(cond: &TriggerCond) -> &'static str {
    match cond {
        TriggerCond::SignalCross { .. } => "signal",
        TriggerCond::IdPresent { .. } => "id",
        TriggerCond::ErrorFrame { .. } => "error",
        TriggerCond::CycleTimeout { .. } => "timeout",
        TriggerCond::FrFramePresent { .. } => "fr_frame",
        TriggerCond::FrSignalCross { .. } => "fr_signal",
        TriggerCond::SysVar { .. } => "sysvar",
    }
}

fn parse_trigger_kind(value: &str) -> Option<TriggerKind> {
    match value {
        "signal" => Some(TriggerKind::Signal),
        "id" => Some(TriggerKind::Id),
        "error" => Some(TriggerKind::ErrorFrame),
        "fr_frame" => Some(TriggerKind::FrFrame),
        "fr_signal" => Some(TriggerKind::FrSignal),
        "timeout" => Some(TriggerKind::Timeout),
        "sysvar" => Some(TriggerKind::SysVar),
        _ => None,
    }
}

fn parse_driver(value: &str) -> Option<HwDriver> {
    match value {
        "Vector" => Some(HwDriver::Vector),
        "Kvaser" => Some(HwDriver::Kvaser),
        _ => None,
    }
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
        let layout = read_project_layout(std::path::Path::new(path));
        app.open_project_path(std::path::Path::new(path));
        // The web has no desktop layout of its own; keep the project's
        // existing dock layout so a web save does not wipe it.
        if !layout.is_empty() {
            app.layout_cache = layout;
        }
    }
    if let Some(path) = replay {
        app.load_log(path);
        if !app.log_path.is_empty() {
            app.send(BusCommand::SetRunMode(Mode::Replay));
            app.start_selected();
        }
        if project.is_some() {
            app.mark_clean();
        }
        return;
    }
    if project.is_none() {
        seed_demo(app);
    } else {
        // Applying the project's live layout to the active desktop is part
        // of loading it (the desktop UI does the same on its first frame),
        // so the freshly opened project has nothing unsaved.
        app.mark_clean();
    }
}

#[derive(Clone)]
struct ApiState {
    shared: Arc<Mutex<WebState>>,
    trace: Arc<Mutex<TraceStore>>,
    signals: Arc<Mutex<SignalStore>>,
    catalog: Arc<Mutex<WebCatalog>>,
    hardware: Arc<Mutex<HardwareCache>>,
    commands: Sender<WebCommand>,
}

async fn catalog_handler(State(api): State<ApiState>) -> Json<WebCatalog> {
    Json(
        api.catalog
            .lock()
            .map(|catalog| catalog.clone())
            .unwrap_or_default(),
    )
}

async fn hardware_handler(State(api): State<ApiState>) -> Json<WebHardware> {
    let generation = api.shared.lock().map(|state| state.hw_refresh).unwrap_or(0);
    if let Ok(cache) = api.hardware.lock()
        && cache.generation == generation
        && let Some(data) = &cache.data
    {
        return Json(data.clone());
    }
    let probed = tokio::task::spawn_blocking(crate::hw::enumerate_all)
        .await
        .unwrap_or_else(|error| Err(format!("hardware probe failed: {error}")));
    let data = match probed {
        Ok(channels) => WebHardware {
            channels: channels
                .into_iter()
                .map(|channel| WebHwChannel {
                    driver: channel.driver.word().to_string(),
                    index: channel.index,
                    name: channel.name,
                    detail: channel.detail,
                })
                .collect(),
            error: String::new(),
        },
        Err(error) => WebHardware {
            channels: Vec::new(),
            error,
        },
    };
    if let Ok(mut cache) = api.hardware.lock() {
        cache.generation = generation;
        cache.data = Some(data.clone());
    }
    Json(data)
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
    Pause {
        on: bool,
    },
    ClearTrace,
    OpenProject {
        path: String,
    },
    OpenLog {
        path: String,
    },
    SelectSignal {
        ch: u8,
        id: u32,
        ext: bool,
        name: String,
    },
    UnselectSignal {
        ch: u8,
        id: u32,
        ext: bool,
        name: String,
    },
    AttachHardware {
        bus: u8,
        driver: String,
        index: i32,
    },
    DetachHardware {
        bus: u8,
    },
    RefreshHardware,
    SetBusMode {
        real: bool,
    },
    AddBus,
    RemoveBus {
        ch: u8,
    },
    RenameBus {
        ch: u8,
        name: String,
    },
    SetBusRates {
        ch: u8,
        bitrate_kbps: u32,
        fd_data_kbps: u32,
    },
    LoadDbc {
        ch: u8,
        path: String,
    },
    SaveProject {
        path: Option<String>,
    },
    SetSysVar {
        namespace: String,
        name: String,
        value: f64,
    },
    DefineSysVar {
        namespace: String,
        name: String,
        init: f64,
        min: Option<f64>,
        max: Option<f64>,
        unit: String,
        comment: String,
        old_namespace: Option<String>,
        old_name: Option<String>,
    },
    DeleteSysVar {
        namespace: String,
        name: String,
    },
    AddMonitor {
        ch: u8,
        id: u32,
        ext: bool,
        name: String,
    },
    RemoveMonitor {
        ch: u8,
        id: u32,
        ext: bool,
        name: String,
    },
    ClearMonitor,
    EditMonitor {
        ch: u8,
        id: u32,
        ext: bool,
        name: String,
        digits: Option<usize>,
        rule_on: Option<bool>,
        rising: Option<bool>,
        threshold: Option<f64>,
        color: Option<usize>,
    },
    AddData {
        ch: u8,
        id: u32,
        ext: bool,
        name: String,
    },
    RemoveData {
        ch: u8,
        id: u32,
        ext: bool,
        name: String,
    },
    ClearData,
    SetDataVisible {
        ch: u8,
        id: u32,
        ext: bool,
        name: String,
        visible: bool,
    },
    AddState {
        ch: u8,
        id: u32,
        ext: bool,
        name: String,
    },
    RemoveState {
        ch: u8,
        id: u32,
        ext: bool,
        name: String,
    },
    ClearState,
    SetStateWindow {
        seconds: f64,
    },
    SetEntryActive {
        ch: u8,
        id: u32,
        on: bool,
    },
    SetEntryCycle {
        ch: u8,
        id: u32,
        cycle_us: u64,
    },
    SetEntryFd {
        ch: u8,
        id: u32,
        fd: bool,
    },
    SetEntryHex {
        ch: u8,
        id: u32,
        text: String,
    },
    SetEntrySource {
        ch: u8,
        id: u32,
        src: WebSrc,
    },
    ClearEntrySource {
        ch: u8,
        id: u32,
        name: String,
    },
    AddEntry {
        ch: u8,
        id: u32,
    },
    RemoveEntry {
        ch: u8,
        id: u32,
    },
    SimulateBusNodes {
        ch: u8,
    },
    AddTrigger {
        kind: String,
    },
    SetTriggerEnabled {
        index: usize,
        on: bool,
    },
    RemoveTrigger {
        index: usize,
    },
    RearmTriggers,
    EditTrigger {
        index: usize,
        kind: String,
        ch: u8,
        id: u32,
        ext: bool,
        signal: String,
        threshold: f64,
        rising: bool,
        bus: u8,
        slot: u16,
        key: String,
        action: String,
        send_ch: u8,
        send_id: u32,
    },
    SetTriggerLimits {
        pre_frames: usize,
        post_frames: u32,
        marker_cap: usize,
    },
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
    catalog: Arc<Mutex<WebCatalog>>,
    project: Option<String>,
    replay: Option<String>,
) {
    let mut app = App::headless();
    prepare_session(&mut app, project.as_deref(), replay.as_deref());
    // The web plot window is independent of the desktop layout. Ask the
    // core for the 5 ms stress stride directly instead of opening a
    // hidden Graphics window, which would dirty (and later save) config.
    app.plot_stride_us = Some(5_000);
    if project.is_none()
        && replay.is_none()
        && let Ok(mut store) = signals.lock()
    {
        for spec in &SIGNALS {
            store.select(&app, spec.ch, spec.id, false, spec.name);
        }
    }
    let mut catalog_sig = String::new();
    refresh_catalog(&app, &catalog, &mut catalog_sig);
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
                WebCommand::OpenProject { path } => {
                    if app.snap.measuring {
                        app.stop();
                    }
                    let keys = signals.lock().map(|store| store.keys()).unwrap_or_default();
                    for key in keys {
                        app.send(BusCommand::Unsubscribe { key });
                    }
                    if let Ok(mut store) = signals.lock() {
                        store.clear_selection();
                    }
                    app.send(BusCommand::ClearTrace);
                    app.send(BusCommand::ClearAggregates);
                    let layout = read_project_layout(std::path::Path::new(&path));
                    app.open_project_path(std::path::Path::new(&path));
                    if !layout.is_empty() {
                        app.layout_cache = layout;
                    }
                    // Same as the bootstrap load: applying the live layout
                    // is part of opening the project, not an unsaved edit.
                    app.mark_clean();
                    app.log_path.clear();
                    app.log_info = None;
                    app.send(BusCommand::SetRunMode(Mode::Virtual));
                    reset_visual_stores(&trace, &mut series, &mut rate_window);
                }
                WebCommand::OpenLog { path } => {
                    if app.snap.measuring {
                        app.stop();
                    }
                    app.load_log(&path);
                    if app.log_path == path && !app.status.contains("failed") {
                        let keys = signals.lock().map(|store| store.keys()).unwrap_or_default();
                        for key in &keys {
                            app.subscribe(key.clone());
                        }
                        if let Ok(mut store) = signals.lock() {
                            store.reset();
                        }
                        app.send(BusCommand::ClearTrace);
                        app.send(BusCommand::ClearAggregates);
                        app.send(BusCommand::SetRunMode(Mode::Replay));
                        reset_visual_stores(&trace, &mut series, &mut rate_window);
                        app.start_selected();
                    }
                }
                WebCommand::SelectSignal { ch, id, ext, name } => {
                    let key = SigKey::can(ch, id, ext, name.clone());
                    let already = signals
                        .lock()
                        .map(|store| store.is_selected(&key))
                        .unwrap_or(false);
                    if !already {
                        app.subscribe(key);
                        if let Ok(mut store) = signals.lock() {
                            store.select(&app, ch, id, ext, &name);
                        }
                    }
                }
                WebCommand::UnselectSignal { ch, id, ext, name } => {
                    let key = SigKey::can(ch, id, ext, name);
                    let selected = signals
                        .lock()
                        .map(|store| store.is_selected(&key))
                        .unwrap_or(false);
                    if selected {
                        app.send(BusCommand::Unsubscribe { key: key.clone() });
                        if let Ok(mut store) = signals.lock() {
                            store.unselect(&key);
                        }
                    }
                }
                WebCommand::AttachHardware { bus, driver, index } => {
                    let Some(driver) = parse_driver(&driver) else {
                        app.status = format!("unknown hardware driver: {driver}");
                        continue;
                    };
                    let Some(channel) = app.snap.channels.get(bus as usize) else {
                        app.status = format!("bus {bus} does not exist");
                        continue;
                    };
                    let (arb, data) = (channel.bitrate_kbps, channel.fd_data_kbps);
                    app.hw_pick.insert(bus, (driver, index));
                    app.set_hardware_channel(bus, driver, index, arb, Some(data));
                }
                WebCommand::DetachHardware { bus } => {
                    app.detach_hardware(bus);
                }
                WebCommand::RefreshHardware => {
                    app.send(BusCommand::RefreshHardware);
                }
                WebCommand::SetBusMode { real } => {
                    app.send(BusCommand::SetBusMode { real });
                }
                WebCommand::AddBus => {
                    if app.snap.measuring {
                        app.stop();
                    }
                    app.add_channel();
                }
                WebCommand::RemoveBus { ch } => {
                    if app.snap.measuring {
                        app.stop();
                    }
                    app.remove_channel(ch as usize);
                    if let Ok(mut store) = signals.lock() {
                        store.remap_after_remove(ch);
                    }
                    if let Ok(mut store) = trace.lock() {
                        store.reset();
                    }
                    series.clear();
                    rate_window.clear();
                }
                WebCommand::RenameBus { ch, name } => {
                    app.send(BusCommand::SetChannelConfig {
                        ch,
                        name: Some(name),
                        dbc_path: None,
                        bitrate_kbps: None,
                        fd_data_kbps: None,
                        node_roles: None,
                    });
                }
                WebCommand::SetBusRates {
                    ch,
                    bitrate_kbps,
                    fd_data_kbps,
                } => {
                    app.send(BusCommand::SetChannelConfig {
                        ch,
                        name: None,
                        dbc_path: None,
                        bitrate_kbps: Some(bitrate_kbps),
                        fd_data_kbps: Some(fd_data_kbps),
                        node_roles: None,
                    });
                }
                WebCommand::SaveProject { path } => {
                    let target = path
                        .filter(|value| !value.trim().is_empty())
                        .map(std::path::PathBuf::from)
                        .or_else(|| app.project_path.clone());
                    match target {
                        Some(target) => {
                            app.save_project(Some(target));
                        }
                        None => {
                            app.status = "save needs a project path".to_string();
                        }
                    }
                }
                WebCommand::SetSysVar {
                    namespace,
                    name,
                    value,
                } => {
                    app.send(BusCommand::SetSysVar {
                        namespace,
                        name,
                        value,
                    });
                }
                WebCommand::DefineSysVar {
                    namespace,
                    name,
                    init,
                    min,
                    max,
                    unit,
                    comment,
                    old_namespace,
                    old_name,
                } => match checked_sysvar(namespace, name, init, min, max, unit, comment) {
                    Ok(def) => {
                        // A rename deletes the old key first, like the
                        // desktop editor: leaving it would keep a phantom
                        // variable scripts could still write.
                        if let (Some(old_ns), Some(old_name)) = (old_namespace, old_name)
                            && (old_ns != def.namespace || old_name != def.name)
                        {
                            app.send(BusCommand::DeleteSysVar {
                                namespace: old_ns,
                                name: old_name,
                            });
                        }
                        app.send(BusCommand::DefineSysVar(def));
                    }
                    Err(error) => app.status = format!("sysvar: {error}"),
                },
                WebCommand::DeleteSysVar { namespace, name } => {
                    app.send(BusCommand::DeleteSysVar { namespace, name });
                }
                WebCommand::AddMonitor { ch, id, ext, name } => {
                    app.set_win_signal(
                        crate::workspace::PopupTarget::Monitor,
                        SigKey::can(ch, id, ext, name),
                        true,
                    );
                }
                WebCommand::RemoveMonitor { ch, id, ext, name } => {
                    app.set_win_signal(
                        crate::workspace::PopupTarget::Monitor,
                        SigKey::can(ch, id, ext, name),
                        false,
                    );
                }
                WebCommand::ClearMonitor => {
                    let keys: Vec<SigKey> = app
                        .monitor_view
                        .rows
                        .iter()
                        .map(|row| row.key.clone())
                        .collect();
                    for key in keys {
                        app.set_win_signal(crate::workspace::PopupTarget::Monitor, key, false);
                    }
                }
                WebCommand::EditMonitor {
                    ch,
                    id,
                    ext,
                    name,
                    digits,
                    rule_on,
                    rising,
                    threshold,
                    color,
                } => {
                    edit_monitor_row(
                        &mut app,
                        &SigKey::can(ch, id, ext, name),
                        digits,
                        rule_on,
                        rising,
                        threshold,
                        color,
                    );
                }
                WebCommand::AddData { ch, id, ext, name } => {
                    if app.data_windows.is_empty() {
                        app.new_data_window();
                    }
                    app.set_win_signal(
                        crate::workspace::PopupTarget::Data(0),
                        SigKey::can(ch, id, ext, name),
                        true,
                    );
                }
                WebCommand::RemoveData { ch, id, ext, name } => {
                    app.set_win_signal(
                        crate::workspace::PopupTarget::Data(0),
                        SigKey::can(ch, id, ext, name),
                        false,
                    );
                }
                WebCommand::ClearData => {
                    let keys: Vec<SigKey> = app
                        .data_windows
                        .first()
                        .map(|window| {
                            window
                                .signals
                                .iter()
                                .map(|signal| signal.key.clone())
                                .collect()
                        })
                        .unwrap_or_default();
                    for key in keys {
                        app.set_win_signal(crate::workspace::PopupTarget::Data(0), key, false);
                    }
                }
                WebCommand::SetDataVisible {
                    ch,
                    id,
                    ext,
                    name,
                    visible,
                } => {
                    let key = SigKey::can(ch, id, ext, name);
                    if let Some(signal) = app
                        .data_windows
                        .first_mut()
                        .and_then(|window| window.signals.iter_mut().find(|row| row.key == key))
                    {
                        signal.visible = visible;
                    }
                }
                WebCommand::AddState { ch, id, ext, name } => {
                    if app.state_trackers.is_empty() {
                        app.new_state_window();
                    }
                    app.set_win_signal(
                        crate::workspace::PopupTarget::State(0),
                        SigKey::can(ch, id, ext, name),
                        true,
                    );
                }
                WebCommand::RemoveState { ch, id, ext, name } => {
                    app.set_win_signal(
                        crate::workspace::PopupTarget::State(0),
                        SigKey::can(ch, id, ext, name),
                        false,
                    );
                }
                WebCommand::ClearState => {
                    let keys: Vec<SigKey> = app
                        .state_trackers
                        .first()
                        .map(|window| {
                            window
                                .signals
                                .iter()
                                .map(|signal| signal.key.clone())
                                .collect()
                        })
                        .unwrap_or_default();
                    for key in keys {
                        app.set_win_signal(crate::workspace::PopupTarget::State(0), key, false);
                    }
                }
                WebCommand::SetStateWindow { seconds } => {
                    if let Some(window) = app.state_trackers.first_mut() {
                        window.time_window_s = seconds.clamp(0.1, 3600.0);
                    }
                }
                WebCommand::SetEntryActive { ch, id, on } => {
                    app.send(BusCommand::SetEntryActive { ch, id, on });
                }
                WebCommand::SetEntryCycle { ch, id, cycle_us } => {
                    app.send(BusCommand::SetEntryCycle { ch, id, cycle_us });
                }
                WebCommand::SetEntryFd { ch, id, fd } => {
                    app.send(BusCommand::SetEntryFd { ch, id, fd });
                }
                WebCommand::SetEntryHex { ch, id, text } => {
                    app.send(BusCommand::SetEntryHex { ch, id, text });
                }
                WebCommand::SetEntrySource { ch, id, src } => {
                    let Some(kind) = crate::sim::KINDS
                        .iter()
                        .copied()
                        .find(|kind| kind.label().eq_ignore_ascii_case(&src.kind))
                    else {
                        app.status = format!("unknown source kind: {}", src.kind);
                        continue;
                    };
                    let name = src.name.trim().to_string();
                    if name.is_empty() {
                        app.status = "source needs a signal name".to_string();
                        continue;
                    }
                    app.send(BusCommand::SetEntrySource {
                        ch,
                        id,
                        src: crate::sim::ValueSrc {
                            name,
                            kind,
                            lo: src.lo,
                            hi: src.hi,
                            period_us: src.period_us,
                            phase_us: src.phase_us,
                            seq: src.seq,
                            seed: src.seed,
                            redraw_us: src.redraw_us,
                        },
                    });
                }
                WebCommand::ClearEntrySource { ch, id, name } => {
                    app.send(BusCommand::ClearEntrySource { ch, id, name });
                }
                WebCommand::AddEntry { ch, id } => {
                    app.send(BusCommand::AddEntry { ch, id });
                }
                WebCommand::RemoveEntry { ch, id } => {
                    app.send(BusCommand::RemoveEntry { ch, id });
                }
                WebCommand::AddTrigger { kind } => {
                    if let Some(kind) = parse_trigger_kind(&kind) {
                        app.apply_trigger_cmds(vec![TriggerCmd::Add(kind)]);
                    }
                }
                WebCommand::SetTriggerEnabled { index, on } => {
                    app.apply_trigger_cmds(vec![TriggerCmd::SetEnabled { index, on }]);
                }
                WebCommand::RemoveTrigger { index } => {
                    app.apply_trigger_cmds(vec![TriggerCmd::Remove(index)]);
                }
                WebCommand::RearmTriggers => {
                    app.apply_trigger_cmds(vec![TriggerCmd::Rearm]);
                }
                WebCommand::EditTrigger {
                    index,
                    kind,
                    ch,
                    id,
                    ext,
                    signal,
                    threshold,
                    rising,
                    bus,
                    slot,
                    key,
                    action,
                    send_ch,
                    send_id,
                } => {
                    let cond = match kind.as_str() {
                        "signal" => TriggerCond::SignalCross {
                            ch,
                            id,
                            ext,
                            signal,
                            threshold,
                            rising,
                        },
                        "id" => TriggerCond::IdPresent { ch, id },
                        "error" => TriggerCond::ErrorFrame { ch },
                        "timeout" => TriggerCond::CycleTimeout { ch, id },
                        "fr_frame" => TriggerCond::FrFramePresent { bus, slot },
                        "fr_signal" => TriggerCond::FrSignalCross {
                            bus,
                            slot,
                            signal,
                            threshold,
                            rising,
                        },
                        "sysvar" => TriggerCond::SysVar {
                            key,
                            threshold,
                            rising,
                        },
                        _ => {
                            app.status = format!("unknown trigger kind: {kind}");
                            continue;
                        }
                    };
                    let action = match action.as_str() {
                        "start_recording" => TriggerAction::StartRecording,
                        "stop_recording" => TriggerAction::StopRecording,
                        "send" => TriggerAction::Send {
                            ch: send_ch,
                            id: send_id,
                        },
                        "clear_trace" => TriggerAction::ClearTrace,
                        "insert_marker" => TriggerAction::InsertMarker,
                        _ => {
                            app.status = format!("unknown trigger action: {action}");
                            continue;
                        }
                    };
                    if index < app.snap.triggers.len() {
                        app.apply_trigger_cmds(vec![TriggerCmd::Edit {
                            index,
                            cond,
                            action,
                        }]);
                    }
                }
                WebCommand::SetTriggerLimits {
                    pre_frames,
                    post_frames,
                    marker_cap,
                } => {
                    app.apply_trigger_cmds(vec![
                        TriggerCmd::SetPreFrames(pre_frames),
                        TriggerCmd::SetPostFrames(post_frames),
                        TriggerCmd::SetMarkerCap(marker_cap),
                    ]);
                }
                WebCommand::SimulateBusNodes { ch } => {
                    let nodes = app
                        .snap
                        .channels
                        .get(ch as usize)
                        .and_then(|channel| channel.dbc.as_ref())
                        .map(|db| db.nodes.clone())
                        .unwrap_or_default();
                    for node in nodes {
                        app.set_node_role(ch, &node, NodeRole::Simulated);
                    }
                }
                WebCommand::LoadDbc { ch, path } => {
                    if app.snap.measuring {
                        app.stop();
                    }
                    app.open_dbc_for(ch as usize, path);
                    let (keep, drop) = signals
                        .lock()
                        .map(|mut store| {
                            store.reset();
                            let drop = store.prune_missing(&app);
                            (store.keys(), drop)
                        })
                        .unwrap_or_default();
                    for key in keep {
                        app.subscribe(key);
                    }
                    for key in drop {
                        app.send(BusCommand::Unsubscribe { key });
                    }
                    if let Ok(mut store) = trace.lock() {
                        store.reset();
                    }
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
            refresh_catalog(&app, &catalog, &mut catalog_sig);
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

    fn reset(&mut self) {
        self.primed = false;
        self.log.clear();
        self.next_seq = 0;
        self.last_total = 0;
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

#[derive(Clone)]
struct SelectedSignal {
    key: SigKey,
    name: String,
    bus: String,
    unit: String,
    color: usize,
}

const SIGNAL_PALETTE: usize = 8;

/// Incremental cache of the chosen signals. Unlike a Graphics window this is
/// a session-level selection: real projects start empty and the demo selects
/// its eight DBC signals once. The frontend asks for a seed and then only
/// for samples newer than its last timestamp.
#[derive(Default)]
struct SignalStore {
    epoch: u64,
    primed: bool,
    selected: Vec<SelectedSignal>,
    buffers: Vec<SignalBuffer>,
    last_t: Vec<u64>,
}

impl SignalStore {
    fn is_selected(&self, key: &SigKey) -> bool {
        self.selected.iter().any(|signal| &signal.key == key)
    }

    fn keys(&self) -> Vec<SigKey> {
        self.selected
            .iter()
            .map(|signal| signal.key.clone())
            .collect()
    }

    /// Adds a selection without subscribing; the command handler owns the
    /// bus subscription so this stays a pure cache operation.
    fn select(&mut self, app: &App, ch: u8, id: u32, ext: bool, name: &str) {
        let key = SigKey::can(ch, id, ext, name);
        if self.is_selected(&key) {
            return;
        }
        let color = (0..SIGNAL_PALETTE)
            .find(|color| !self.selected.iter().any(|signal| signal.color == *color))
            .unwrap_or(self.selected.len() % SIGNAL_PALETTE);
        self.selected.push(SelectedSignal {
            key,
            name: name.to_string(),
            bus: bus_name(app, ch),
            unit: signal_unit(app, ch, id, ext, name),
            color,
        });
        self.primed = false;
    }

    fn unselect(&mut self, key: &SigKey) {
        if let Some(pos) = self.selected.iter().position(|signal| &signal.key == key) {
            self.selected.remove(pos);
        }
        self.primed = false;
    }

    /// CAN channel indices shift down when a bus is removed. Drop signals on
    /// the removed bus and renumber the rest to match the core's remap.
    fn remap_after_remove(&mut self, removed: u8) {
        self.selected.retain_mut(|signal| {
            if let SigKey::Can { ch, .. } = &mut signal.key {
                if *ch == removed {
                    return false;
                }
                if *ch > removed {
                    *ch -= 1;
                }
            }
            true
        });
        self.primed = false;
    }

    /// A loaded database can replace the signals under a selection. Drop the
    /// ones it no longer declares and return their keys for unsubscription.
    fn prune_missing(&mut self, app: &App) -> Vec<SigKey> {
        let mut dropped = Vec::new();
        self.selected.retain(|signal| {
            let exists = match &signal.key {
                SigKey::Can { ch, id, ext, name } => app
                    .snap
                    .channels
                    .get(*ch as usize)
                    .and_then(|channel| channel.dbc.as_ref())
                    .and_then(|db| db.messages.get(&(*id, *ext)))
                    .is_some_and(|message| {
                        message
                            .signals
                            .iter()
                            .any(|candidate| &candidate.name == name)
                    }),
                _ => true,
            };
            if !exists {
                dropped.push(signal.key.clone());
            }
            exists
        });
        if !dropped.is_empty() {
            self.primed = false;
        }
        dropped
    }

    /// Drops selected signals and buffers; used when the bus set changes.
    fn clear_selection(&mut self) {
        self.selected.clear();
        self.primed = false;
    }

    /// Keeps the selection but forgets sampled history; used when a new log
    /// starts on the same project.
    fn reset(&mut self) {
        self.primed = false;
    }

    fn sync(&mut self, app: &App) {
        for signal in &mut self.selected {
            signal.bus = bus_name(app, key_channel(&signal.key));
            if let Some(sub) = signal_sub(app, &signal.key) {
                signal.unit = sub.unit.clone();
            }
        }
        let mut reset = !self.primed;
        if self.primed {
            for (i, signal) in self.selected.iter().enumerate() {
                let Some(sub) = signal_sub(app, &signal.key) else {
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
            self.last_t = vec![0; self.selected.len()];
            for (i, signal) in self.selected.iter().enumerate() {
                let sub = signal_sub(app, &signal.key);
                let meta = signal_meta(signal);
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

        for (i, signal) in self.selected.iter().enumerate() {
            let Some(sub) = signal_sub(app, &signal.key) else {
                continue;
            };
            let Some(buffer) = self.buffers.get_mut(i) else {
                continue;
            };
            buffer.meta = signal_meta(signal);
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

    /// Includes selected signals with no samples yet; the lane has to exist
    /// before the first frame arrives.
    fn seed(&self) -> Vec<SignalSeries> {
        self.buffers
            .iter()
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

fn key_channel(key: &SigKey) -> u8 {
    match key {
        SigKey::Can { ch, .. } => *ch,
        SigKey::Fr { bus, .. } | SigKey::Eth { bus, .. } => *bus,
    }
}

fn bus_name(app: &App, ch: u8) -> String {
    app.snap
        .channels
        .get(ch as usize)
        .map(|channel| channel.name.clone())
        .unwrap_or_else(|| format!("CAN{}", ch + 1))
}

fn signal_unit(app: &App, ch: u8, id: u32, ext: bool, name: &str) -> String {
    app.snap
        .channels
        .get(ch as usize)
        .and_then(|channel| channel.dbc.as_ref())
        .and_then(|db| db.messages.get(&(id, ext)))
        .and_then(|message| message.signals.iter().find(|signal| signal.name == name))
        .map(|signal| signal.unit.clone())
        .unwrap_or_default()
}

fn signal_sub<'a>(app: &'a App, key: &SigKey) -> Option<&'a SubView> {
    app.snap.subs.iter().find(|sub| &sub.key == key)
}

fn signal_meta(signal: &SelectedSignal) -> SignalSeries {
    SignalSeries {
        key: format!(
            "{}:{}:{}",
            key_channel(&signal.key),
            key_id(&signal.key),
            signal.name
        ),
        name: signal.name.clone(),
        bus: signal.bus.clone(),
        unit: signal.unit.clone(),
        color: signal.color,
        points: Vec::new(),
    }
}

fn key_id(key: &SigKey) -> u32 {
    match key {
        SigKey::Can { id, .. } => *id,
        SigKey::Fr { slot, .. } => *slot as u32,
        SigKey::Eth { .. } => 0,
    }
}

/// Build the DBC signal tree for the picker. Only CAN is exposed for now;
/// the store and commands carry the same key format so FlexRay/Ethernet can
/// join later without changing the frontend contract.
#[derive(Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
struct WebCatalog {
    channels: Vec<WebCatalogChannel>,
}

#[derive(Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
struct WebCatalogChannel {
    name: String,
    messages: Vec<WebCatalogMessage>,
}

#[derive(Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
struct WebCatalogMessage {
    id: u32,
    extended: bool,
    name: String,
    signals: Vec<WebCatalogSignal>,
}

#[derive(Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
struct WebCatalogSignal {
    key: String,
    name: String,
    unit: String,
    min: f64,
    max: f64,
}

fn channel_signature(app: &App) -> String {
    let parts: Vec<String> = app
        .snap
        .channels
        .iter()
        .map(|channel| {
            let table = channel
                .dbc
                .as_ref()
                .map(|db| Arc::as_ptr(db) as usize)
                .unwrap_or(0);
            format!("{}:{}:{table}", channel.name, channel.dbc_paths.join(","))
        })
        .collect();
    parts.join(";")
}

fn refresh_catalog(app: &App, catalog: &Arc<Mutex<WebCatalog>>, signature: &mut String) {
    let next = channel_signature(app);
    if next == *signature {
        return;
    }
    *signature = next;
    if let Ok(mut slot) = catalog.lock() {
        *slot = build_catalog(app);
    }
}

fn build_catalog(app: &App) -> WebCatalog {
    WebCatalog {
        channels: app
            .snap
            .channels
            .iter()
            .enumerate()
            .map(|(ch, channel)| WebCatalogChannel {
                name: channel.name.clone(),
                messages: channel
                    .dbc
                    .as_ref()
                    .map(|db| {
                        db.order
                            .iter()
                            .filter_map(|&(id, ext)| {
                                let message = db.messages.get(&(id, ext))?;
                                Some(WebCatalogMessage {
                                    id,
                                    extended: ext,
                                    name: message.name.clone(),
                                    signals: message
                                        .signals
                                        .iter()
                                        .map(|signal| WebCatalogSignal {
                                            key: format!("{ch}:{id}:{}", signal.name),
                                            name: signal.name.clone(),
                                            unit: signal.unit.clone(),
                                            min: signal.min,
                                            max: signal.max,
                                        })
                                        .collect(),
                                })
                            })
                            .collect()
                    })
                    .unwrap_or_default(),
            })
            .collect(),
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
    project_path: String,
    dirty: bool,
    log: String,
    log_info: String,
    run_mode: String,
    replay_pos_us: Option<u64>,
    replay_total_us: Option<u64>,
    recent_projects: Vec<String>,
    recent_logs: Vec<String>,
    buses: Vec<WebBus>,
    real_bus: bool,
    hw_refresh: u64,
    tx: Vec<WebTx>,
    triggers: Vec<WebTrigger>,
    trigger_limits: WebTriggerLimits,
    sysvars: Vec<WebSysVar>,
    monitor: Vec<WebMonitorRow>,
    data: Vec<WebDataRow>,
    state_rows: Vec<WebStateRow>,
    state_window_s: f64,
    stats: Vec<WebStatsRow>,
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
struct WebBus {
    index: u8,
    name: String,
    dbc_paths: Vec<String>,
    bitrate_kbps: u32,
    fd_data_kbps: u32,
    attached: Option<WebHwAttachment>,
}

#[derive(Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
struct WebHwAttachment {
    driver: String,
    adapter: i32,
    kbps: u32,
    fd: bool,
    can_tx: bool,
    tx_fail: Option<WebTxFail>,
}

#[derive(Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
struct WebTxFail {
    reason: String,
    count: u64,
}

#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct WebSrc {
    name: String,
    kind: String,
    lo: f64,
    hi: f64,
    period_us: u64,
    phase_us: u64,
    seq: Vec<f64>,
    seed: u64,
    redraw_us: u64,
}

#[derive(Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
struct WebTx {
    channel: u8,
    bus: String,
    id: u32,
    name: String,
    node: String,
    active: bool,
    gate_open: bool,
    fd: bool,
    cycle_us: u64,
    data_text: String,
    sent_text: String,
    muted: bool,
    srcs: Vec<WebSrc>,
}

#[derive(Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
struct WebTrigger {
    summary: String,
    action: String,
    kind: String,
    enabled: bool,
    fired: u64,
    level: bool,
    last_fire_us: u64,
    ch: u8,
    id: u32,
    ext: bool,
    signal: String,
    threshold: f64,
    rising: bool,
    bus: u8,
    slot: u16,
    var_key: String,
    action_kind: String,
    send_ch: u8,
    send_id: u32,
}

#[derive(Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
struct WebTriggerLimits {
    pre_frames: usize,
    post_frames: u32,
    marker_cap: usize,
}

#[derive(Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
struct WebStatsRow {
    label: String,
    bus: String,
    count: u64,
    min_ms: String,
    avg_ms: String,
    max_ms: String,
    jitter_ms: String,
    share: String,
    len: u8,
}

#[derive(Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
struct WebMonitorRow {
    ch: u8,
    id: u32,
    ext: bool,
    name: String,
    label: String,
    bus: String,
    value: f64,
    unit: String,
    digits: usize,
    rule_on: bool,
    rising: bool,
    threshold: f64,
    color: usize,
}

#[derive(Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
struct WebDataRow {
    ch: u8,
    id: u32,
    ext: bool,
    name: String,
    bus: String,
    value: String,
    unit: String,
    raw: String,
    min: String,
    avg: String,
    max: String,
    frac: f64,
    visible: bool,
}

#[derive(Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
struct WebStateBand {
    start: f64,
    end: f64,
    label: String,
    color: usize,
}

#[derive(Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
struct WebStateRow {
    ch: u8,
    id: u32,
    ext: bool,
    name: String,
    bus: String,
    current: String,
    bands: Vec<WebStateBand>,
}

#[derive(Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
struct WebSysVar {
    namespace: String,
    name: String,
    value: f64,
    init: f64,
    min: Option<f64>,
    max: Option<f64>,
    unit: String,
    comment: String,
}

#[derive(Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
struct WebHwChannel {
    driver: String,
    index: i32,
    name: String,
    detail: Option<String>,
}

#[derive(Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
struct WebHardware {
    channels: Vec<WebHwChannel>,
    error: String,
}

#[derive(Default)]
struct HardwareCache {
    generation: u64,
    data: Option<WebHardware>,
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
    let buses: Vec<WebBus> =
        snap.channels
            .iter()
            .enumerate()
            .map(|(index, ch)| WebBus {
                index: index as u8,
                name: ch.name.clone(),
                dbc_paths: ch.dbc_paths.clone(),
                bitrate_kbps: ch.bitrate_kbps,
                fd_data_kbps: ch.fd_data_kbps,
                attached: snap.hw.iter().find(|hw| hw.bus == index as u8).map(|hw| {
                    WebHwAttachment {
                        driver: hw.driver.word().to_string(),
                        adapter: hw.adapter,
                        kbps: hw.kbps,
                        fd: hw.fd,
                        can_tx: hw.can_tx,
                        tx_fail: hw.tx_fail.as_ref().map(|(reason, count)| WebTxFail {
                            reason: reason.clone(),
                            count: *count,
                        }),
                    }
                }),
            })
            .collect();

    let mut tx: Vec<WebTx> = snap
        .tx
        .iter()
        .map(|entry| WebTx {
            channel: entry.channel,
            bus: snap
                .channels
                .get(entry.channel as usize)
                .map(|channel| channel.name.clone())
                .unwrap_or_else(|| format!("CAN{}", entry.channel + 1)),
            id: entry.id,
            name: entry.name.clone(),
            node: entry.node.clone(),
            active: entry.active,
            gate_open: entry.gate_open,
            fd: entry.fd,
            cycle_us: entry.cycle_us,
            data_text: entry.data_text.clone(),
            sent_text: entry.sent_text.clone(),
            muted: entry.muted,
            srcs: entry
                .srcs
                .iter()
                .map(|src| WebSrc {
                    name: src.name.clone(),
                    kind: src.kind.label().to_string(),
                    lo: src.lo,
                    hi: src.hi,
                    period_us: src.period_us,
                    phase_us: src.phase_us,
                    seq: src.seq.clone(),
                    seed: src.seed,
                    redraw_us: src.redraw_us,
                })
                .collect(),
        })
        .collect();
    tx.sort_by_key(|entry| (entry.channel, entry.id));

    let total_frames: u64 = snap.aggs.iter().map(|agg| agg.count).sum();
    let ms = |value: f64, count: u64| {
        if count >= 2 {
            format!("{:.2}", value / 1000.0)
        } else {
            "-".to_string()
        }
    };
    let stats: Vec<WebStatsRow> = snap
        .aggs
        .iter()
        .map(|agg| {
            let name = snap
                .channels
                .get(agg.channel as usize)
                .and_then(|channel| channel.dbc.as_ref())
                .and_then(|db| db.message_name(agg.id))
                .unwrap_or("-");
            let id = if agg.extended {
                format!("{:08X}x", agg.id)
            } else {
                format!("{:03X}", agg.id)
            };
            WebStatsRow {
                label: format!("{id}  {name}"),
                bus: snap
                    .channels
                    .get(agg.channel as usize)
                    .map(|channel| channel.name.clone())
                    .unwrap_or_else(|| format!("CAN{}", agg.channel + 1)),
                count: agg.count,
                min_ms: ms(agg.min_us, agg.count),
                avg_ms: ms(agg.cycle_us, agg.count),
                max_ms: ms(agg.max_us, agg.count),
                jitter_ms: ms(agg.jitter_us, agg.count),
                share: format!(
                    "{:.1}%",
                    if total_frames > 0 {
                        agg.count as f64 / total_frames as f64 * 100.0
                    } else {
                        0.0
                    }
                ),
                len: agg.len,
            }
        })
        .collect();

    let triggers: Vec<WebTrigger> = snap
        .triggers
        .iter()
        .enumerate()
        .map(|(index, trigger)| {
            let mut view = WebTrigger {
                summary: app.trigger_summary(index),
                action: match &trigger.action {
                    TriggerAction::StartRecording => "start recording".to_string(),
                    TriggerAction::StopRecording => "stop recording".to_string(),
                    TriggerAction::Send { id, .. } => format!("send 0x{id:X}"),
                    TriggerAction::ClearTrace => "clear trace".to_string(),
                    TriggerAction::InsertMarker => "insert marker".to_string(),
                },
                kind: trigger_kind(&trigger.cond).to_string(),
                enabled: trigger.enabled,
                fired: trigger.fired,
                level: trigger.level,
                last_fire_us: trigger.last_fire_t_us,
                action_kind: match trigger.action {
                    TriggerAction::StartRecording => "start_recording",
                    TriggerAction::StopRecording => "stop_recording",
                    TriggerAction::Send { .. } => "send",
                    TriggerAction::ClearTrace => "clear_trace",
                    TriggerAction::InsertMarker => "insert_marker",
                }
                .to_string(),
                ..Default::default()
            };
            match &trigger.cond {
                TriggerCond::SignalCross {
                    ch,
                    id,
                    ext,
                    signal,
                    threshold,
                    rising,
                } => {
                    view.ch = *ch;
                    view.id = *id;
                    view.ext = *ext;
                    view.signal = signal.clone();
                    view.threshold = *threshold;
                    view.rising = *rising;
                }
                TriggerCond::IdPresent { ch, id } | TriggerCond::CycleTimeout { ch, id } => {
                    view.ch = *ch;
                    view.id = *id;
                }
                TriggerCond::ErrorFrame { ch } => view.ch = *ch,
                TriggerCond::FrFramePresent { bus, slot } => {
                    view.bus = *bus;
                    view.slot = *slot;
                }
                TriggerCond::FrSignalCross {
                    bus,
                    slot,
                    signal,
                    threshold,
                    rising,
                } => {
                    view.bus = *bus;
                    view.slot = *slot;
                    view.signal = signal.clone();
                    view.threshold = *threshold;
                    view.rising = *rising;
                }
                TriggerCond::SysVar {
                    key,
                    threshold,
                    rising,
                } => {
                    view.var_key = key.clone();
                    view.threshold = *threshold;
                    view.rising = *rising;
                }
            }
            if let TriggerAction::Send { ch, id } = trigger.action {
                view.send_ch = ch;
                view.send_id = id;
            }
            view
        })
        .collect();

    let sysvars: Vec<WebSysVar> = snap
        .sysvars
        .iter()
        .map(|v| WebSysVar {
            namespace: v.def.namespace.clone(),
            name: v.def.name.clone(),
            value: v.value,
            init: v.def.init,
            min: v.def.min,
            max: v.def.max,
            unit: v.def.unit.clone(),
            comment: v.def.comment.clone(),
        })
        .collect();

    // The web's value table mirrors the first desktop Data window. Missing
    // values keep the row but print dashes, so a freshly added signal does
    // not flicker in and out before its first frame.
    let data: Vec<WebDataRow> = app
        .data_windows
        .first()
        .map(|window| {
            window
                .signals
                .iter()
                .filter_map(|signal| {
                    let SigKey::Can { ch, id, ext, name } = &signal.key else {
                        return None;
                    };
                    let sub = signal_sub(app, &signal.key);
                    let (value, unit, raw, min, avg, max, frac) = match sub {
                        Some(sub) => {
                            let frac = match app.declared_range(&signal.key) {
                                Some((lo, hi)) if hi > lo => {
                                    ((sub.latest - lo) / (hi - lo)).clamp(0.0, 1.0)
                                }
                                _ if sub.max > sub.min => {
                                    ((sub.latest - sub.min) / (sub.max - sub.min)).clamp(0.0, 1.0)
                                }
                                _ => 0.0,
                            };
                            (
                                sub.label.clone().unwrap_or_else(|| {
                                    crate::dbc::fmt_decoded(&sub.type_tag, sub.latest)
                                }),
                                sub.unit.clone(),
                                sub.last_raw.to_string(),
                                crate::dbc::fmt_decoded(&sub.type_tag, sub.min),
                                crate::dbc::fmt_decoded(&sub.type_tag, sub.avg),
                                crate::dbc::fmt_decoded(&sub.type_tag, sub.max),
                                frac,
                            )
                        }
                        None => (
                            "-".to_string(),
                            String::new(),
                            "-".to_string(),
                            "-".to_string(),
                            "-".to_string(),
                            "-".to_string(),
                            0.0,
                        ),
                    };
                    Some(WebDataRow {
                        ch: *ch,
                        id: *id,
                        ext: *ext,
                        name: name.clone(),
                        bus: bus_name(app, *ch),
                        value,
                        unit,
                        raw,
                        min,
                        avg,
                        max,
                        frac,
                        visible: signal.visible,
                    })
                })
                .collect()
        })
        .unwrap_or_default();

    let state_window_s = app
        .state_trackers
        .first()
        .map(|window| window.time_window_s)
        .unwrap_or(20.0);
    let state_rows: Vec<WebStateRow> = app
        .state_trackers
        .first()
        .map(|window| {
            let tw = window.time_window_s.max(0.1);
            let right_s = app.plot_now_s();
            let left_s = (right_s - tw).max(0.0);
            let lo = (left_s * 1e6) as u64;
            let hi = (right_s * 1e6) as u64;
            window
                .signals
                .iter()
                .filter_map(|signal| {
                    let SigKey::Can { ch, id, ext, name } = &signal.key else {
                        return None;
                    };
                    let (current, bands) = match signal_sub(app, &signal.key) {
                        Some(sub) => {
                            let held = sub.history.at(lo);
                            let samples: Vec<(u64, f64)> =
                                sub.history.range(lo, hi).copied().collect();
                            state_bands_from(held, &samples, lo, hi)
                        }
                        None => ("-".to_string(), Vec::new()),
                    };
                    Some(WebStateRow {
                        ch: *ch,
                        id: *id,
                        ext: *ext,
                        name: name.clone(),
                        bus: bus_name(app, *ch),
                        current,
                        bands,
                    })
                })
                .collect()
        })
        .unwrap_or_default();

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

    // The web monitor covers CAN rows; a project's FlexRay/Ethernet rows are
    // left untouched in the file and simply stay desktop-only for now.
    let monitor: Vec<WebMonitorRow> = app
        .monitor_view
        .rows
        .iter()
        .filter_map(|row| {
            let SigKey::Can { ch, id, ext, name } = &row.key else {
                return None;
            };
            let (value, unit) = signal_sub(app, &row.key)
                .map(|sub| (sub.latest, sub.unit.clone()))
                .unwrap_or((0.0, String::new()));
            Some(WebMonitorRow {
                ch: *ch,
                id: *id,
                ext: *ext,
                name: name.clone(),
                label: row.label.clone(),
                bus: bus_name(*ch),
                value,
                unit,
                digits: row.digits,
                rule_on: row.rule_on,
                rising: row.rising,
                threshold: row.threshold,
                color: row.color,
            })
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
        project: app
            .project_path
            .as_ref()
            .and_then(|path| path.file_name())
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_default(),
        project_path: app
            .project_path
            .as_ref()
            .map(|path| path.to_string_lossy().into_owned())
            .unwrap_or_default(),
        dirty: app.is_dirty(),
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
        recent_projects: app.recent_projects.clone(),
        recent_logs: app.recent_log.clone(),
        buses,
        real_bus: snap.real_bus,
        hw_refresh: snap.hw_refresh,
        tx,
        triggers,
        trigger_limits: WebTriggerLimits {
            pre_frames: app.limits.pre_frames,
            post_frames: app.limits.post_frames,
            marker_cap: app.limits.marker_cap,
        },
        sysvars,
        monitor,
        data,
        state_rows,
        state_window_s,
        stats,
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
        assert!(!state.stats.is_empty(), "statistics share the aggregates");

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

    /// Loading a project through the web keeps the file's desktop layout and
    /// leaves the workspace clean: the plot's fine stride is frontend state,
    /// not a config edit that a later save would write back.
    #[test]
    fn a_web_project_load_keeps_the_desktop_layout_and_stays_clean() {
        let mut app = App::headless();
        prepare_session(&mut app, Some("assets/Sample.rxproj"), None);
        assert!(
            !app.layout_cache.is_empty(),
            "the top-level dock layout survives the headless load"
        );
        assert!(!app.is_dirty(), "web startup must not edit desktop config");
        app.plot_stride_us = Some(5_000);
        assert!(!app.is_dirty(), "the plot stride is frontend state");
    }

    /// The web form's rules match the desktop editor, so a bad shape is
    /// rejected at the host boundary instead of becoming a broken variable.
    #[test]
    fn web_sysvar_definitions_follow_the_desktop_rules() {
        let ok = checked_sysvar(
            " Demo ".into(),
            "Setpoint".into(),
            100.0,
            Some(0.0),
            Some(500.0),
            " rpm ".into(),
            "target".into(),
        )
        .expect("valid definition");
        assert_eq!(ok.namespace, "Demo");
        assert_eq!(ok.unit, "rpm");
        assert!(
            checked_sysvar(
                String::new(),
                "x".into(),
                0.0,
                None,
                None,
                String::new(),
                String::new()
            )
            .is_err()
        );
        assert!(
            checked_sysvar(
                "A::B".into(),
                "x".into(),
                0.0,
                None,
                None,
                String::new(),
                String::new()
            )
            .is_err()
        );
        assert!(
            checked_sysvar(
                "A".into(),
                "x".into(),
                0.0,
                Some(5.0),
                Some(1.0),
                String::new(),
                String::new()
            )
            .is_err()
        );
    }

    /// Definitions and live values reach the web state through the same
    /// core commands the desktop manager uses.
    #[test]
    fn the_state_lists_system_variables_and_live_values() {
        let mut app = App::headless();
        app.send(BusCommand::DefineSysVar(
            checked_sysvar(
                "Demo".into(),
                "Setpoint".into(),
                100.0,
                Some(0.0),
                Some(500.0),
                "rpm".into(),
                "target speed".into(),
            )
            .expect("definition"),
        ));
        app.send(BusCommand::SetSysVar {
            namespace: "Demo".into(),
            name: "Setpoint".into(),
            value: 900.0,
        });
        app.settle();
        let state = build_state(&app, &VecDeque::new(), 0.0);
        assert_eq!(state.sysvars.len(), 1);
        let v = &state.sysvars[0];
        assert_eq!(
            (v.namespace.as_str(), v.name.as_str()),
            ("Demo", "Setpoint")
        );
        assert_eq!(v.value, 500.0, "writes clamp into the declared bounds");
        assert_eq!((v.min, v.max), (Some(0.0), Some(500.0)));
        assert_eq!(v.unit, "rpm");
    }

    /// Monitor rows carry their identity, live value and coloring rule to
    /// the web; edits stay in row state and removal unsubscribes the signal.
    #[test]
    fn the_state_lists_monitor_rows_and_edits_them() {
        let key = SigKey::can(0, 0x100, false, "EngineSpeed");
        let mut app = App::headless();
        app.set_win_signal(crate::workspace::PopupTarget::Monitor, key.clone(), true);
        app.settle();
        let state = build_state(&app, &VecDeque::new(), 0.0);
        assert_eq!(state.monitor.len(), 1);
        assert_eq!(state.monitor[0].name, "EngineSpeed");
        assert_eq!(state.monitor[0].label, "EngineSpeed");
        assert_eq!(state.monitor[0].digits, 1, "desktop default");

        edit_monitor_row(
            &mut app,
            &key,
            Some(2),
            Some(true),
            Some(false),
            Some(87.5),
            Some(3),
        );
        let state = build_state(&app, &VecDeque::new(), 0.0);
        let row = &state.monitor[0];
        assert_eq!(row.digits, 2);
        assert!(row.rule_on);
        assert!(!row.rising);
        assert_eq!(row.threshold, 87.5);
        assert_eq!(row.color, 3);

        app.set_win_signal(crate::workspace::PopupTarget::Monitor, key, false);
        app.settle();
        let state = build_state(&app, &VecDeque::new(), 0.0);
        assert!(state.monitor.is_empty());
    }

    /// The value table mirrors the first Data window: selected rows, their
    /// live strings and the visibility toggle all reach the web state.
    #[test]
    fn the_state_lists_data_rows_and_visibility() {
        let key = SigKey::can(0, 0x100, false, "EngineSpeed");
        let mut app = App::headless();
        app.new_data_window();
        app.set_win_signal(crate::workspace::PopupTarget::Data(0), key.clone(), true);
        app.settle();
        let state = build_state(&app, &VecDeque::new(), 0.0);
        assert_eq!(state.data.len(), 1);
        let row = &state.data[0];
        assert_eq!(row.name, "EngineSpeed");
        assert!(row.visible);
        assert!((0.0..=1.0).contains(&row.frac));

        app.data_windows[0].signals[0].visible = false;
        let state = build_state(&app, &VecDeque::new(), 0.0);
        assert!(!state.data[0].visible);

        app.set_win_signal(crate::workspace::PopupTarget::Data(0), key, false);
        app.settle();
        let state = build_state(&app, &VecDeque::new(), 0.0);
        assert!(state.data.is_empty());
    }

    /// The demo's generator sources reach the web with their shape and
    /// timing, not just a count, so the editor can round-trip them.
    #[test]
    fn the_state_lists_generator_sources() {
        let mut app = App::headless();
        seed_demo(&mut app);
        for _ in 0..20 {
            app.update();
        }
        let state = build_state(&app, &VecDeque::new(), 0.0);
        let src = state
            .tx
            .iter()
            .flat_map(|entry| &entry.srcs)
            .find(|src| src.name == "EngineSpeed")
            .expect("the demo drives EngineSpeed");
        assert_eq!(src.kind, "Sine");
        assert!(src.period_us > 0);
        assert!(src.hi > src.lo);
    }

    /// Visible samples collapse into one band per change of the quantized
    /// value; empty columns carry the last state forward.
    #[test]
    fn state_bands_split_on_value_changes() {
        let (current, bands) = state_bands_from(
            Some(0.0),
            &[(10_000, 0.0), (20_000, 1.0), (30_000, 1.0)],
            0,
            40_000,
        );
        assert_eq!(current, "1");
        assert_eq!(bands.len(), 2);
        assert_eq!(bands[0].start, 0.0);
        assert_eq!(bands[0].end, 0.5);
        assert_eq!(bands[0].label, "0");
        assert_eq!(bands[1].start, 0.5);
        assert_eq!(bands[1].end, 1.0);
        assert_eq!(bands[1].label, "1");
    }

    /// The first State Tracker window feeds the web: identity, bus and the
    /// window length reach the state, so the tab can draw and resize it.
    #[test]
    fn the_state_lists_state_tracker_rows_and_window() {
        let key = SigKey::can(0, 0x100, false, "EngineSpeed");
        let mut app = App::headless();
        app.new_state_window();
        app.set_win_signal(crate::workspace::PopupTarget::State(0), key.clone(), true);
        app.settle();
        let state = build_state(&app, &VecDeque::new(), 0.0);
        assert_eq!(state.state_rows.len(), 1);
        assert_eq!(state.state_rows[0].name, "EngineSpeed");
        assert_eq!(state.state_rows[0].bus, "CAN1");
        assert_eq!(state.state_window_s, 20.0);

        app.set_win_signal(crate::workspace::PopupTarget::State(0), key, false);
        app.settle();
        let state = build_state(&app, &VecDeque::new(), 0.0);
        assert!(state.state_rows.is_empty());
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

    /// Selecting a signal creates its lane metadata even before the first
    /// sample, and unselecting removes it again.
    #[test]
    fn the_store_tracks_the_selected_signals() {
        let app = App::headless();
        let key = SigKey::can(0, 0x100, false, "EngineSpeed");
        let mut store = SignalStore::default();
        store.select(&app, 0, 0x100, false, "EngineSpeed");
        assert!(store.is_selected(&key));
        store.sync(&app);
        let seeded = store.seed();
        assert_eq!(seeded.len(), 1);
        assert_eq!(seeded[0].name, "EngineSpeed");
        assert_eq!(seeded[0].unit, "rpm");
        store.unselect(&key);
        store.sync(&app);
        assert!(store.seed().is_empty());
    }

    /// The Triggers panel reads the live rule list and its fire state.
    #[test]
    fn the_state_lists_triggers() {
        let mut app = App::headless();
        app.apply_trigger_cmds(vec![
            TriggerCmd::Add(TriggerKind::Id),
            TriggerCmd::Edit {
                index: 0,
                cond: TriggerCond::ErrorFrame { ch: 1 },
                action: TriggerAction::InsertMarker,
            },
            TriggerCmd::SetPreFrames(128),
            TriggerCmd::SetPostFrames(16),
            TriggerCmd::SetMarkerCap(64),
        ]);
        let state = build_state(&app, &VecDeque::new(), 0.0);
        assert_eq!(state.triggers.len(), 1);
        assert!(state.triggers[0].enabled);
        assert_eq!(state.triggers[0].kind, "error");
        assert_eq!(state.triggers[0].ch, 1);
        assert_eq!(state.triggers[0].action_kind, "insert_marker");
        assert!(!state.triggers[0].summary.is_empty());
        assert_eq!(state.trigger_limits.pre_frames, 128);
        assert_eq!(state.trigger_limits.post_frames, 16);
        assert_eq!(state.trigger_limits.marker_cap, 64);
    }

    /// The generator panel reads live TxView rows, including their cycle,
    /// payload and node gate.
    #[test]
    fn the_state_lists_generator_entries() {
        let app = App::headless();
        let state = build_state(&app, &VecDeque::new(), 0.0);
        assert!(!state.tx.is_empty(), "the bootstrap generator has entries");
        let entry = state
            .tx
            .iter()
            .find(|entry| entry.channel == 0 && entry.id == 0x100)
            .expect("EngineStatus is in the sample generator");
        assert_eq!(entry.name, "EngineStatus");
        assert!(entry.cycle_us > 0);
        assert_eq!(entry.bus, "CAN1");
    }

    /// Reloading a database must retire selections it no longer declares,
    /// otherwise the lane can never be removed from the picker.
    #[test]
    fn a_database_reload_drops_missing_signals() {
        let app = App::headless();
        let mut store = SignalStore::default();
        store.select(&app, 0, 0x100, false, "NoSuchSignal");
        let dropped = store.prune_missing(&app);
        assert_eq!(dropped.len(), 1);
        assert!(store.selected.is_empty());
    }

    /// Removing a CAN bus shifts every higher channel down; selected signal
    /// keys must follow or their curves point at the wrong bus.
    #[test]
    fn removing_a_bus_remaps_the_selection() {
        let app = App::headless();
        let mut store = SignalStore::default();
        store.select(&app, 0, 0x100, false, "EngineSpeed");
        store.select(&app, 1, 0x64, false, "EngSpeed");
        assert_eq!(store.selected.len(), 2);
        store.remap_after_remove(0);
        assert_eq!(store.selected.len(), 1);
        assert_eq!(store.selected[0].name, "EngSpeed");
        assert_eq!(key_channel(&store.selected[0].key), 0);
        assert_eq!(key_id(&store.selected[0].key), 0x64);
    }

    /// The Buses panel gets its rows from the snapshot, including the DBC
    /// names and the absence of an attachment.
    #[test]
    fn the_state_lists_buses_and_hardware_fields() {
        let app = App::headless();
        let state = build_state(&app, &VecDeque::new(), 0.0);
        assert_eq!(state.buses.len(), 2);
        assert_eq!(state.buses[0].name, "CAN1");
        assert!(
            state.buses[0]
                .dbc_paths
                .iter()
                .any(|path| path.contains("sample.dbc"))
        );
        assert!(state.buses.iter().all(|bus| bus.attached.is_none()));
        assert!(parse_driver("Vector").is_some());
        assert!(parse_driver("Kvaser").is_some());
        assert!(parse_driver("other").is_none());
    }

    /// The picker catalog comes from the loaded DBCs, not a hard-coded list.
    #[test]
    fn the_catalog_lists_the_dbc_signals() {
        let app = App::headless();
        let catalog = build_catalog(&app);
        let message = catalog
            .channels
            .iter()
            .flat_map(|channel| &channel.messages)
            .find(|message| message.name == "EngineStatus")
            .expect("EngineStatus is in sample.dbc");
        assert!(
            message
                .signals
                .iter()
                .any(|signal| signal.name == "EngineSpeed" && signal.unit == "rpm")
        );
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
        for spec in &SIGNALS {
            signals.select(&app, spec.ch, spec.id, false, spec.name);
        }
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
