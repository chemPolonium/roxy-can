//! Experimental web frontend host.
//!
//! The bus core is unchanged: this module starts an `App` on a manual drive
//! in its own thread, publishes a small JSON snapshot, and serves the Vite
//! build from `web/dist`. It is a spike, not a second supported frontend --
//! the desktop ImGui path stays the product.

use std::collections::VecDeque;
use std::hash::{Hash, Hasher};
use std::sync::mpsc::{Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use axum::body::Bytes;
use axum::extract::{DefaultBodyLimit, Query, State};
use axum::http::header::{CONTENT_DISPOSITION, CONTENT_TYPE};
use axum::http::{HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
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
/// Browser uploads are buffered in memory before landing in the temp folder.
/// This covers DBC/ARXML/project files and ordinary logs; bigger recordings
/// still go through a typed host path.
const UPLOAD_LIMIT: usize = 256 * 1024 * 1024;

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
    let scripts = Arc::new(Mutex::new(Vec::new()));
    {
        let shared = shared.clone();
        let trace = trace.clone();
        let signals = signals.clone();
        let catalog = catalog.clone();
        let scripts = scripts.clone();
        std::thread::Builder::new()
            .name("roxy-web-core".to_string())
            .spawn(move || {
                app_loop(
                    rx,
                    SharedStores {
                        shared,
                        trace,
                        signals,
                        catalog,
                        scripts,
                    },
                    project,
                    replay,
                )
            })
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
            .route("/nodes", get(scripts_handler))
            .route("/command", post(command))
            .route(
                "/upload",
                post(upload).layer(DefaultBodyLimit::max(UPLOAD_LIMIT)),
            )
            .route("/download", get(download).delete(discard_download))
            .route("/recording", get(recording_download))
            .with_state(ApiState {
                shared,
                trace,
                signals,
                catalog,
                hardware,
                scripts,
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

async fn scripts_handler(State(api): State<ApiState>) -> Json<Vec<WebScript>> {
    Json(
        api.scripts
            .lock()
            .map(|scripts| scripts.clone())
            .unwrap_or_default(),
    )
}

/// Fingerprint of everything the scripts dialog shows. Source edits are
/// infrequent, but they must reach the editor without shipping every
/// script on every 10 Hz state poll; `/api/nodes` is refreshed only when
/// this changes.
fn scripts_signature(app: &App) -> u64 {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    for node in app.snap.nodes.iter() {
        node.id.hash(&mut hasher);
        node.name.hash(&mut hasher);
        node.source.hash(&mut hasher);
        node.enabled.hash(&mut hasher);
        node.running.hash(&mut hasher);
        node.errored.hash(&mut hasher);
        node.attached.hash(&mut hasher);
        node.log.len().hash(&mut hasher);
        node.log.last().hash(&mut hasher);
    }
    hasher.finish()
}

fn scripts_snapshot(app: &App) -> Vec<WebScript> {
    app.snap
        .nodes
        .iter()
        .map(|node| WebScript {
            id: node.id,
            name: node.name.clone(),
            channel: node.channel,
            attached_bus: node.attached.as_ref().map(|(bus, _)| *bus),
            attached_node: node.attached.as_ref().map(|(_, name)| name.clone()),
            enabled: node.enabled,
            running: node.running,
            errored: node.errored,
            file_path: node.file_path.clone().unwrap_or_default(),
            source: node.source.clone(),
            log: node.log.clone(),
        })
        .collect()
}

/// Runs the desktop exporter for a web export request. Trace picks ASC or
/// PCAP by the path's extension; the others write CSV/TXT. Exporters target
/// the first matching desktop window, which starts as an All-buses view.
fn export_file(app: &mut App, kind: &str, path: &str) {
    if path.trim().is_empty() {
        app.status = "export needs a path".to_string();
        return;
    }
    match kind {
        "trace" => app.export_trace(0, path),
        "messages" => app.export_messages_csv(0, path),
        "stats" => app.export_stats_csv(0, path),
        "write" => app.export_write_txt(path),
        "data" => app.export_data_csv(0, path),
        "state" => app.export_state_csv(0, path),
        "graphics" => app.export_graphics_csv(0, path),
        "spec" => app.export_spec_csv(path),
        other => app.status = format!("unknown export kind: {other}"),
    }
}

/// Seeds the chart from a project's first Graphics window. The desktop
/// project persists curve selection there; the web keeps its own lane
/// metadata but must not start empty when the same project is opened.
fn seed_chart_selection(app: &mut App, store: &mut SignalStore) {
    let keys: Vec<SigKey> = app
        .graphics
        .first()
        .map(|window| {
            window
                .signals
                .iter()
                .filter(|signal| signal.visible)
                .map(|signal| signal.key.clone())
                .collect()
        })
        .unwrap_or_default();
    for key in keys {
        if let SigKey::Can { ch, id, ext, name } = &key {
            let (ch, id, ext, name) = (*ch, *id, *ext, name.clone());
            store.select(app, ch, id, ext, &name);
            app.subscribe(SigKey::can(ch, id, ext, name));
        }
    }
}

/// Selecting a chart lane also records it on the first Graphics window, so
/// project save/load carries the selection. The panel stays the source of
/// truth for lane colors; `store` is updated by the caller.
fn chart_select(app: &mut App, ch: u8, id: u32, ext: bool, name: &str) {
    chart_select_key(app, SigKey::can(ch, id, ext, name));
}

fn chart_select_key(app: &mut App, key: SigKey) {
    if app.graphics.is_empty() {
        app.new_graphics_window();
    }
    app.set_win_signal(
        crate::workspace::PopupTarget::Graphics(0),
        key.clone(),
        true,
    );
    app.subscribe(key);
}

fn chart_unselect(app: &mut App, key: &SigKey) {
    app.set_win_signal(
        crate::workspace::PopupTarget::Graphics(0),
        key.clone(),
        false,
    );
    // Demo lanes never lived in a Graphics window; prune still drops the
    // subscription unless another observer (Monitor/Data/State) uses it.
    app.prune_signal(key);
}

/// Replaces a State row's custom value-definition bands. Cuts are sorted
/// and deduplicated; names are padded so every band has a label.
fn set_state_rule(app: &mut App, key: &SigKey, cuts: Vec<f64>, names: Vec<String>) {
    let Some(window) = app.state_trackers.first_mut() else {
        app.status = "state rule needs a State Tracker window".to_string();
        return;
    };
    let mut cuts: Vec<f64> = cuts.into_iter().filter(|value| value.is_finite()).collect();
    cuts.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    cuts.dedup_by(|a, b| (*a - *b).abs() < f64::EPSILON);
    let mut names = names;
    names.truncate(cuts.len() + 1);
    while names.len() < cuts.len() + 1 {
        names.push(format!("state {}", names.len()));
    }
    window.rules.insert(
        key.clone(),
        crate::observe::StateRule {
            colors: vec![None; cuts.len() + 1],
            cuts,
            names,
        },
    );
}

fn clear_state_rule(app: &mut App, key: &SigKey) {
    if let Some(window) = app.state_trackers.first_mut() {
        window.rules.remove(key);
    }
}

fn pct_text(v: f64) -> String {
    format!("{:.2}", v * 100.0)
}

fn opt_pct_text(v: Option<f64>) -> String {
    v.map(pct_text).unwrap_or_else(|| "-".to_string())
}

fn num_text(v: f64) -> String {
    format!("{v:.0}")
}

fn opt_num_text(v: Option<f64>) -> String {
    v.map(num_text).unwrap_or_else(|| "-".to_string())
}

fn us_ms_text(us: u64) -> String {
    format!("{:.3}", us as f64 / 1000.0)
}

fn opt_ms_text(us: Option<u64>) -> String {
    us.map(us_ms_text).unwrap_or_else(|| "-".to_string())
}

fn opt_ms_f_text(us: Option<f64>) -> String {
    us.map(|v| format!("{:.3}", v / 1000.0))
        .unwrap_or_else(|| "-".to_string())
}

fn opt_num_u_text(v: Option<u64>) -> String {
    v.map(|x| x.to_string()).unwrap_or_else(|| "-".to_string())
}

fn class_stat_rows(
    rows: &mut Vec<WebBusStatRow>,
    load: &crate::load::BusLoad,
    label: &str,
    class: crate::load::FrameClass,
) {
    let (min, max, avg) = load.rate_stats(class);
    rows.push(WebBusStatRow {
        label: format!("{label} [n/s]"),
        current: num_text(load.class_rate(class)),
        min: opt_num_text(min),
        max: opt_num_text(max),
        avg: opt_num_text(avg),
    });
    rows.push(WebBusStatRow {
        label: format!("{label} [total]"),
        current: load.class_total(class).to_string(),
        min: "-".to_string(),
        max: "-".to_string(),
        avg: "-".to_string(),
    });
}

/// The CAN section of the desktop Bus Statistics window, one row per
/// statistic. The reference's hardware-only rows have no meaning here.
fn bus_stat_rows(load: &crate::load::BusLoad) -> Vec<WebBusStatRow> {
    use crate::load::FrameClass;
    let mut rows = Vec::new();
    let (min, max, avg) = load.load_stats();
    rows.push(WebBusStatRow {
        label: "Busload [%]".to_string(),
        current: pct_text(load.load()),
        min: opt_pct_text(min),
        max: opt_pct_text(max),
        avg: opt_pct_text(avg),
    });
    let (last, min, max, avg) = load.send_dist_us();
    rows.push(WebBusStatRow {
        label: "Min. Send Dist. [ms]".to_string(),
        current: opt_ms_text(last),
        min: opt_ms_text(min),
        max: opt_ms_text(max),
        avg: opt_ms_f_text(avg),
    });
    rows.push(WebBusStatRow {
        label: "Bursts [total]".to_string(),
        current: load.bursts_total().to_string(),
        min: "-".to_string(),
        max: "-".to_string(),
        avg: "-".to_string(),
    });
    let (last, min, max, avg) = load.burst_time_us();
    rows.push(WebBusStatRow {
        label: "Burst Time [ms]".to_string(),
        current: us_ms_text(last),
        min: opt_ms_text(min),
        max: opt_ms_text(max),
        avg: opt_ms_f_text(avg),
    });
    let (cur, min, max, avg) = load.frames_per_burst();
    rows.push(WebBusStatRow {
        label: "Frames per Burst".to_string(),
        current: cur.to_string(),
        min: opt_num_u_text(min),
        max: opt_num_u_text(max),
        avg: opt_num_text(avg),
    });
    class_stat_rows(&mut rows, load, "Std. Data", FrameClass::StdData);
    class_stat_rows(&mut rows, load, "Ext. Data", FrameClass::ExtData);
    class_stat_rows(&mut rows, load, "Std. Remote", FrameClass::StdRemote);
    class_stat_rows(&mut rows, load, "Ext. Remote", FrameClass::ExtRemote);
    let (min, max, avg) = load.rate_stats(FrameClass::Error);
    rows.push(WebBusStatRow {
        label: "Errorframes [n/s]".to_string(),
        current: num_text(load.class_rate(FrameClass::Error)),
        min: opt_num_text(min),
        max: opt_num_text(max),
        avg: opt_num_text(avg),
    });
    rows.push(WebBusStatRow {
        label: "Errorframes [total]".to_string(),
        current: load.errors.to_string(),
        min: "-".to_string(),
        max: "-".to_string(),
        avg: "-".to_string(),
    });
    rows.push(WebBusStatRow {
        label: "Chip State".to_string(),
        current: "Simulated".to_string(),
        min: "-".to_string(),
        max: "-".to_string(),
        avg: "-".to_string(),
    });
    rows
}

/// Turns visible samples into at most one band per pixel column (360) using
/// explicit cut points: the band index is the state, its name the label.
fn state_bands_custom(
    held: Option<f64>,
    samples: &[(u64, f64)],
    lo: u64,
    hi: u64,
    rule: &crate::observe::StateRule,
) -> (String, Vec<WebStateBand>) {
    const COLS: usize = 360;
    let span = hi.saturating_sub(lo).max(1);
    let mut values: Vec<Option<f64>> = vec![None; COLS];
    for &(t, v) in samples {
        let idx =
            (((t.saturating_sub(lo)) as u128 * COLS as u128 / span as u128) as usize).min(COLS - 1);
        values[idx] = Some(v);
    }
    let mut last = held;
    for slot in &mut values {
        if let Some(v) = *slot {
            last = Some(v);
        } else {
            *slot = last;
        }
    }
    let label_of = |v: f64| {
        let band = rule.band(v);
        rule.names
            .get(band)
            .cloned()
            .unwrap_or_else(|| format!("band {band}"))
    };
    let mut bands = Vec::new();
    let mut open: Option<(usize, usize)> = None;
    for (i, slot) in values.iter().enumerate() {
        let band = slot.map(|v| rule.band(v));
        match open {
            Some((open_band, _)) if Some(open_band) == band => {}
            Some((open_band, start)) => {
                bands.push(WebStateBand {
                    start: start as f64 / COLS as f64,
                    end: i as f64 / COLS as f64,
                    label: rule
                        .names
                        .get(open_band)
                        .cloned()
                        .unwrap_or_else(|| format!("band {open_band}")),
                    color: open_band % 8,
                });
                open = band.map(|band| (band, i));
            }
            None => open = band.map(|band| (band, i)),
        }
    }
    if let Some((open_band, start)) = open {
        bands.push(WebStateBand {
            start: start as f64 / COLS as f64,
            end: 1.0,
            label: rule
                .names
                .get(open_band)
                .cloned()
                .unwrap_or_else(|| format!("band {open_band}")),
            color: open_band % 8,
        });
    }
    let current = last.map(label_of).unwrap_or_else(|| "-".to_string());
    (current, bands)
}

fn parse_id_filter(text: &str) -> Vec<(u32, bool)> {
    text.split([',', ' ', ';'])
        .filter(|part| !part.is_empty())
        .map(|raw| {
            let token = raw.trim();
            match token.strip_suffix(['x', 'X']) {
                Some(hex) => (u32::from_str_radix(hex, 16).unwrap_or(0), true),
                None => (u32::from_str_radix(token, 16).unwrap_or(0), false),
            }
        })
        .filter(|&(id, _)| id != 0)
        .collect()
}

/// Web twin of the desktop record-filter parser: CAN hex ids (an `x`
/// suffix marks extended) and `FRn:slot` entries, comma/space separated.
fn parse_record_filter(text: &str) -> crate::recorder::RecordFilter {
    let mut filter = crate::recorder::RecordFilter::default();
    for raw in text.split([',', ' ', ';']) {
        let token = raw.trim();
        if token.is_empty() {
            continue;
        }
        match token
            .strip_prefix(['f', 'F'])
            .and_then(|rest| rest.strip_prefix(['r', 'R']))
        {
            Some(rest) => {
                let Some((bus, slot)) = rest.split_once(':') else {
                    continue;
                };
                if let (Ok(bus), Ok(slot)) = (bus.trim().parse::<u8>(), slot.trim().parse::<u16>())
                {
                    filter.fr.push((bus, slot));
                }
            }
            None => filter.can.extend(parse_id_filter(token)),
        }
    }
    filter
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
    scripts: Arc<Mutex<Vec<WebScript>>>,
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
    let probed = tokio::task::spawn_blocking(|| {
        (
            crate::hw::enumerate_all(),
            crate::hw::vector::enumerate_flexray(),
        )
    })
    .await
    .unwrap_or_else(|error| {
        (
            Err(format!("hardware probe failed: {error}")),
            Err(format!("hardware probe failed: {error}")),
        )
    });
    let (can, fr) = probed;
    let (channels, error) = match can {
        Ok(channels) => (
            channels
                .into_iter()
                .map(|channel| WebHwChannel {
                    driver: channel.driver.word().to_string(),
                    index: channel.index,
                    name: channel.name,
                    detail: channel.detail,
                })
                .collect(),
            String::new(),
        ),
        Err(error) => (Vec::new(), error),
    };
    let (fr_channels, fr_error) = match fr {
        Ok(list) => {
            let mut counts = std::collections::BTreeMap::new();
            for channel in &list {
                *counts.entry(channel.name.clone()).or_insert(0u32) += 1;
            }
            (
                list.into_iter()
                    .map(|channel| WebHwChannel {
                        driver: "Vector".to_string(),
                        index: channel.index,
                        name: channel.name.clone(),
                        detail: (counts.get(&channel.name).copied().unwrap_or(0) > 1)
                            .then(|| format!("dev {}", channel.hw_index)),
                    })
                    .collect(),
                String::new(),
            )
        }
        Err(error) => (Vec::new(), error),
    };
    let data = WebHardware {
        channels,
        fr_channels,
        error,
        fr_error,
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
struct UploadQuery {
    name: Option<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct UploadReply {
    path: String,
    name: String,
    size: usize,
}

#[derive(Deserialize)]
struct DownloadQuery {
    name: Option<String>,
}

/// Files the browser exchanges with the host live in the per-user data
/// folder, not the temp folder: a project saved after picking a DBC must
/// still find that DBC after the next reboot.
fn web_data_dir(sub: &str) -> std::path::PathBuf {
    let root = std::env::var_os("LOCALAPPDATA")
        .map(std::path::PathBuf::from)
        .or_else(|| std::env::var_os("XDG_DATA_HOME").map(std::path::PathBuf::from))
        .or_else(|| {
            std::env::var_os("HOME")
                .map(|home| std::path::PathBuf::from(home).join(".local").join("share"))
        })
        .unwrap_or_else(std::env::temp_dir);
    root.join("roxy-can").join(sub)
}

fn upload_dir() -> std::path::PathBuf {
    web_data_dir("web-uploads")
}

fn download_dir() -> std::path::PathBuf {
    web_data_dir("web-downloads")
}

fn to_download_dir(file: &str) -> std::path::PathBuf {
    download_dir().join(upload_name(file))
}

/// A bare file name from the export or save dialog is written to the
/// download folder so the browser can fetch it back. Paths with a directory
/// part are the user's own host paths and stay untouched.
fn download_file_path(path: &str) -> String {
    let trimmed = path.trim();
    if std::path::Path::new(trimmed).components().count() != 1 {
        return trimmed.to_string();
    }
    let _ = std::fs::create_dir_all(download_dir());
    to_download_dir(trimmed).to_string_lossy().into_owned()
}

/// Serves only files in the export folder; the name is sanitized the same
/// way the export path was, so a caller cannot walk out of that folder.
async fn download(Query(query): Query<DownloadQuery>) -> Result<Response, (StatusCode, String)> {
    let name = upload_name(query.name.as_deref().unwrap_or(""));
    let path = to_download_dir(&name);
    let read = path.clone();
    let bytes = tokio::task::spawn_blocking(move || std::fs::read(&read))
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("download task: {e}"),
            )
        })?
        .map_err(|_| (StatusCode::NOT_FOUND, "export is not ready".to_string()))?;
    let mut response = bytes.into_response();
    response.headers_mut().insert(
        CONTENT_TYPE,
        HeaderValue::from_static("application/octet-stream"),
    );
    let disposition =
        HeaderValue::from_str(&format!("attachment; filename=\"{name}\"")).map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("download name: {e}"),
            )
        })?;
    response
        .headers_mut()
        .insert(CONTENT_DISPOSITION, disposition);
    Ok(response)
}

/// Serves the file the recorder finished last. The path comes from the
/// core's own snapshot, never from the query, and a live recording is
/// refused until its writer has closed the trailer.
async fn recording_download(State(api): State<ApiState>) -> Result<Response, (StatusCode, String)> {
    let (path, recording) = api
        .shared
        .lock()
        .map(|state| (state.record_file.clone(), state.recording))
        .unwrap_or_default();
    if recording {
        return Err((
            StatusCode::CONFLICT,
            "stop recording before downloading".to_string(),
        ));
    }
    if path.is_empty() {
        return Err((StatusCode::NOT_FOUND, "no recording yet".to_string()));
    }
    let read = path.clone();
    let bytes = tokio::task::spawn_blocking(move || std::fs::read(&read))
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("recording task: {e}"),
            )
        })?
        .map_err(|e| (StatusCode::NOT_FOUND, format!("recording: {e}")))?;
    let file = std::path::Path::new(&path)
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("recording.asc");
    let name = upload_name(file);
    let mut response = bytes.into_response();
    response.headers_mut().insert(
        CONTENT_TYPE,
        HeaderValue::from_static("application/octet-stream"),
    );
    let disposition =
        HeaderValue::from_str(&format!("attachment; filename=\"{name}\"")).map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("download name: {e}"),
            )
        })?;
    response
        .headers_mut()
        .insert(CONTENT_DISPOSITION, disposition);
    Ok(response)
}

async fn discard_download(
    Query(query): Query<DownloadQuery>,
) -> Result<StatusCode, (StatusCode, String)> {
    let name = upload_name(query.name.as_deref().unwrap_or(""));
    let path = to_download_dir(&name);
    tokio::task::spawn_blocking(move || match std::fs::remove_file(&path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error),
    })
    .await
    .map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("discard task: {e}"),
        )
    })?
    .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, format!("discard: {e}")))?;
    Ok(StatusCode::NO_CONTENT)
}

fn upload_name(raw: &str) -> String {
    let base = std::path::Path::new(raw)
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("");
    let mut cleaned: String = base
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-') {
                c
            } else {
                '_'
            }
        })
        .collect();
    cleaned.truncate(120);
    if cleaned.is_empty() || cleaned.chars().all(|c| c == '.') {
        "upload".to_string()
    } else {
        cleaned
    }
}

/// Saves a browser-picked file into the OS temp folder and returns the host
/// path the rest of the API can load. Only the file name is trusted: any
/// directory part is dropped and the stamped prefix keeps two uploads from
/// overwriting each other.
async fn upload(
    Query(query): Query<UploadQuery>,
    body: Bytes,
) -> Result<Json<UploadReply>, (StatusCode, String)> {
    let dir = upload_dir();
    std::fs::create_dir_all(&dir).map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("upload folder: {e}"),
        )
    })?;
    let name = upload_name(query.name.as_deref().unwrap_or("upload"));
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let path = dir.join(format!("{stamp}_{name}"));
    let size = body.len();
    let write = path.clone();
    tokio::task::spawn_blocking(move || std::fs::write(&write, &body))
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("upload task: {e}"),
            )
        })?
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("upload write: {e}"),
            )
        })?;
    Ok(Json(UploadReply {
        path: path.to_string_lossy().into_owned(),
        name,
        size,
    }))
}

#[derive(Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum WebCommand {
    Start,
    Stop,
    Pause {
        on: bool,
    },
    ReplayStep {
        delta_us: u64,
    },
    ReplayPlayTo {
        target_us: u64,
    },
    ReplayPlayFor {
        duration_us: u64,
    },
    SetReplaySpeed {
        speed: f64,
    },
    SetReplayChannel {
        kind: String,
        raw: u8,
        target: Option<u8>,
    },
    RecordToggle {
        name: Option<String>,
        format: Option<u8>,
    },
    SetRecordFilter {
        text: String,
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
    SelectFrSignal {
        bus: u8,
        slot: u16,
        name: String,
    },
    UnselectFrSignal {
        bus: u8,
        slot: u16,
        name: String,
    },
    SelectEthSignal {
        bus: u8,
        frame: String,
        name: String,
    },
    UnselectEthSignal {
        bus: u8,
        frame: String,
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
    AttachFrHardware {
        bus: u8,
        index: i32,
    },
    DetachFrHardware {
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
    SetNodeRole {
        bus: u8,
        node: String,
        simulated: bool,
    },
    LoadFrDescription {
        path: String,
        bus: Option<u8>,
    },
    RemoveFrBus {
        bus: u8,
    },
    LoadEthDescription {
        path: String,
        bus: Option<u8>,
    },
    RemoveEthBus {
        bus: u8,
    },
    LoadDbc {
        ch: u8,
        path: String,
    },
    ImportArxml {
        path: String,
    },
    SaveProject {
        path: Option<String>,
    },
    AddScript {
        name: String,
        channel: u8,
        attached_bus: Option<u8>,
        attached_node: Option<String>,
    },
    SetScriptName {
        id: u64,
        name: String,
    },
    SetScriptSource {
        id: u64,
        source: String,
    },
    SetScriptEnabled {
        id: u64,
        on: bool,
    },
    RemoveScript {
        id: u64,
    },
    SaveScriptFile {
        path: String,
        source: String,
    },
    SetSpecTolerance {
        value: u64,
    },
    SetSpecGrace {
        value: u64,
    },
    ClearSpec,
    Export {
        kind: String,
        path: String,
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
        kind: String,
        ch: u8,
        id: u32,
        ext: bool,
        frame: Option<String>,
        name: String,
    },
    RemoveMonitor {
        kind: String,
        ch: u8,
        id: u32,
        ext: bool,
        frame: Option<String>,
        name: String,
    },
    ClearMonitor,
    EditMonitor {
        kind: String,
        ch: u8,
        id: u32,
        ext: bool,
        frame: Option<String>,
        name: String,
        digits: Option<usize>,
        rule_on: Option<bool>,
        rising: Option<bool>,
        threshold: Option<f64>,
        color: Option<usize>,
    },
    AddData {
        kind: String,
        ch: u8,
        id: u32,
        ext: bool,
        frame: Option<String>,
        name: String,
    },
    RemoveData {
        kind: String,
        ch: u8,
        id: u32,
        ext: bool,
        frame: Option<String>,
        name: String,
    },
    ClearData,
    SetDataVisible {
        kind: String,
        ch: u8,
        id: u32,
        ext: bool,
        frame: Option<String>,
        name: String,
        visible: bool,
    },
    AddState {
        kind: String,
        ch: u8,
        id: u32,
        ext: bool,
        frame: Option<String>,
        name: String,
    },
    RemoveState {
        kind: String,
        ch: u8,
        id: u32,
        ext: bool,
        frame: Option<String>,
        name: String,
    },
    ClearState,
    SetStateWindow {
        seconds: f64,
    },
    SetStateRule {
        kind: String,
        ch: u8,
        id: u32,
        ext: bool,
        frame: Option<String>,
        name: String,
        cuts: Vec<f64>,
        names: Vec<String>,
    },
    ClearStateRule {
        kind: String,
        ch: u8,
        id: u32,
        ext: bool,
        frame: Option<String>,
        name: String,
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
    AddFrEntry {
        bus: u8,
        slot: u16,
    },
    RemoveFrEntry {
        bus: u8,
        slot: u16,
    },
    SetFrEntryActive {
        bus: u8,
        slot: u16,
        on: bool,
    },
    SetFrEntryCycle {
        bus: u8,
        slot: u16,
        cycle_us: u64,
    },
    SetFrEntryHex {
        bus: u8,
        slot: u16,
        text: String,
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
/// The caches the core thread publishes into and the HTTP handlers read.
/// Grouped so `app_loop` keeps a small argument list.
struct SharedStores {
    shared: Arc<Mutex<WebState>>,
    trace: Arc<Mutex<TraceStore>>,
    signals: Arc<Mutex<SignalStore>>,
    catalog: Arc<Mutex<WebCatalog>>,
    scripts: Arc<Mutex<Vec<WebScript>>>,
}

fn app_loop(
    inbox: Receiver<WebCommand>,
    stores: SharedStores,
    project: Option<String>,
    replay: Option<String>,
) {
    let SharedStores {
        shared,
        trace,
        signals,
        catalog,
        scripts,
    } = stores;
    let mut app = App::headless();
    prepare_session(&mut app, project.as_deref(), replay.as_deref());
    if let Ok(mut store) = signals.lock() {
        seed_chart_selection(&mut app, &mut store);
    }
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
    let mut scripts_sig = u64::MAX;

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
                WebCommand::ReplayStep { delta_us } => {
                    app.send(BusCommand::ReplayStep { delta_us });
                }
                WebCommand::ReplayPlayTo { target_us } => {
                    app.send(BusCommand::ReplayPlayTo { target_us });
                }
                WebCommand::ReplayPlayFor { duration_us } => {
                    app.send(BusCommand::ReplayPlayFor { duration_us });
                }
                WebCommand::SetReplaySpeed { speed } => {
                    let speed = if speed.is_finite() && speed > 0.0 {
                        speed
                    } else {
                        1.0
                    };
                    app.replay_speed = speed;
                    app.send(BusCommand::SetReplaySpeed(speed));
                }
                WebCommand::SetReplayChannel { kind, raw, target } => {
                    let Some(kind) = crate::replay_map::LogBusKind::parse(&kind) else {
                        app.status = format!("unknown replay bus kind: {kind}");
                        continue;
                    };
                    let mut map = app.snap.replay_map.clone();
                    map.set(kind, raw, target);
                    app.send(BusCommand::SetReplayMap(map));
                }
                WebCommand::RecordToggle { name, format } => {
                    if let Some(name) = name {
                        app.record_path_buf = name;
                    }
                    if let Some(format) = format {
                        app.record_format = if format == 1 { 1 } else { 0 };
                    }
                    app.toggle_record();
                }
                WebCommand::SetRecordFilter { text } => {
                    app.record_filter_text = text.clone();
                    app.send(BusCommand::SetRecordFilter {
                        filter: parse_record_filter(&text),
                    });
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
                    if let Ok(mut store) = signals.lock() {
                        seed_chart_selection(&mut app, &mut store);
                    }
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
                        chart_select(&mut app, ch, id, ext, &name);
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
                        chart_unselect(&mut app, &key);
                        if let Ok(mut store) = signals.lock() {
                            store.unselect(&key);
                        }
                    }
                }
                WebCommand::SelectFrSignal { bus, slot, name } => {
                    let key = SigKey::fr(bus, slot, name);
                    let already = signals
                        .lock()
                        .map(|store| store.is_selected(&key))
                        .unwrap_or(false);
                    if !already {
                        chart_select_key(&mut app, key.clone());
                        if let Ok(mut store) = signals.lock() {
                            store.select_key(&app, key);
                        }
                    }
                }
                WebCommand::UnselectFrSignal { bus, slot, name } => {
                    let key = SigKey::fr(bus, slot, name);
                    let selected = signals
                        .lock()
                        .map(|store| store.is_selected(&key))
                        .unwrap_or(false);
                    if selected {
                        chart_unselect(&mut app, &key);
                        if let Ok(mut store) = signals.lock() {
                            store.unselect(&key);
                        }
                    }
                }
                WebCommand::SelectEthSignal { bus, frame, name } => {
                    let key = SigKey::eth(bus, frame, name);
                    let already = signals
                        .lock()
                        .map(|store| store.is_selected(&key))
                        .unwrap_or(false);
                    if !already {
                        chart_select_key(&mut app, key.clone());
                        if let Ok(mut store) = signals.lock() {
                            store.select_key(&app, key);
                        }
                    }
                }
                WebCommand::UnselectEthSignal { bus, frame, name } => {
                    let key = SigKey::eth(bus, frame, name);
                    let selected = signals
                        .lock()
                        .map(|store| store.is_selected(&key))
                        .unwrap_or(false);
                    if selected {
                        chart_unselect(&mut app, &key);
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
                WebCommand::AttachFrHardware { bus, index } => {
                    app.attach_fr_watch_on(bus, index);
                }
                WebCommand::DetachFrHardware { bus } => {
                    app.detach_flexray_watch(bus);
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
                WebCommand::ImportArxml { path } => {
                    if app.snap.measuring {
                        app.stop();
                    }
                    app.import_arxml_path(&path);
                }
                WebCommand::SaveProject { path } => {
                    let target = path
                        .filter(|value| !value.trim().is_empty())
                        .map(|value| std::path::PathBuf::from(download_file_path(&value)))
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
                WebCommand::AddScript {
                    name,
                    channel,
                    attached_bus,
                    attached_node,
                } => {
                    let attached = match (attached_bus, attached_node) {
                        (Some(bus), Some(node)) => Some((bus, node)),
                        _ => None,
                    };
                    app.send(BusCommand::AddNode {
                        name,
                        channel,
                        attached,
                    });
                }
                WebCommand::SetScriptName { id, name } => {
                    app.send(BusCommand::SetNodeName { id, name });
                }
                WebCommand::SetScriptSource { id, source } => {
                    app.send(BusCommand::SetNodeSource { id, source });
                }
                WebCommand::SetScriptEnabled { id, on } => {
                    app.send(BusCommand::SetNodeEnabled { id, on });
                }
                WebCommand::RemoveScript { id } => {
                    app.send(BusCommand::RemoveNode { id });
                }
                WebCommand::SaveScriptFile { path, source } => {
                    if path.trim().is_empty() {
                        app.status = "script save needs a path".to_string();
                    } else {
                        match std::fs::write(&path, source) {
                            Ok(()) => app.status = format!("script saved: {path}"),
                            Err(error) => app.status = format!("script save failed: {error}"),
                        }
                    }
                }
                WebCommand::SetSpecTolerance { value } => {
                    app.spec_tol_pct = value.min(100);
                }
                WebCommand::SetSpecGrace { value } => {
                    app.spec_grace = value.clamp(1, 20);
                }
                WebCommand::ClearSpec => {
                    app.send(BusCommand::ClearSpec);
                }
                WebCommand::Export { kind, path } => {
                    export_file(&mut app, &kind, &download_file_path(&path));
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
                WebCommand::AddMonitor {
                    kind,
                    ch,
                    id,
                    ext,
                    frame,
                    name,
                } => {
                    let Some(key) = command_key(&kind, ch, id, ext, frame, name) else {
                        app.status = format!("unknown signal kind: {kind}");
                        continue;
                    };
                    app.set_win_signal(crate::workspace::PopupTarget::Monitor, key, true);
                }
                WebCommand::RemoveMonitor {
                    kind,
                    ch,
                    id,
                    ext,
                    frame,
                    name,
                } => {
                    let Some(key) = command_key(&kind, ch, id, ext, frame, name) else {
                        app.status = format!("unknown signal kind: {kind}");
                        continue;
                    };
                    app.set_win_signal(crate::workspace::PopupTarget::Monitor, key, false);
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
                    kind,
                    ch,
                    id,
                    ext,
                    frame,
                    name,
                    digits,
                    rule_on,
                    rising,
                    threshold,
                    color,
                } => {
                    let Some(key) = command_key(&kind, ch, id, ext, frame, name) else {
                        app.status = format!("unknown signal kind: {kind}");
                        continue;
                    };
                    edit_monitor_row(&mut app, &key, digits, rule_on, rising, threshold, color);
                }
                WebCommand::AddData {
                    kind,
                    ch,
                    id,
                    ext,
                    frame,
                    name,
                } => {
                    let Some(key) = command_key(&kind, ch, id, ext, frame, name) else {
                        app.status = format!("unknown signal kind: {kind}");
                        continue;
                    };
                    if app.data_windows.is_empty() {
                        app.new_data_window();
                    }
                    app.set_win_signal(crate::workspace::PopupTarget::Data(0), key, true);
                }
                WebCommand::RemoveData {
                    kind,
                    ch,
                    id,
                    ext,
                    frame,
                    name,
                } => {
                    let Some(key) = command_key(&kind, ch, id, ext, frame, name) else {
                        app.status = format!("unknown signal kind: {kind}");
                        continue;
                    };
                    app.set_win_signal(crate::workspace::PopupTarget::Data(0), key, false);
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
                    kind,
                    ch,
                    id,
                    ext,
                    frame,
                    name,
                    visible,
                } => {
                    let Some(key) = command_key(&kind, ch, id, ext, frame, name) else {
                        app.status = format!("unknown signal kind: {kind}");
                        continue;
                    };
                    if let Some(signal) = app
                        .data_windows
                        .first_mut()
                        .and_then(|window| window.signals.iter_mut().find(|row| row.key == key))
                    {
                        signal.visible = visible;
                    }
                }
                WebCommand::AddState {
                    kind,
                    ch,
                    id,
                    ext,
                    frame,
                    name,
                } => {
                    let Some(key) = command_key(&kind, ch, id, ext, frame, name) else {
                        app.status = format!("unknown signal kind: {kind}");
                        continue;
                    };
                    if app.state_trackers.is_empty() {
                        app.new_state_window();
                    }
                    app.set_win_signal(crate::workspace::PopupTarget::State(0), key, true);
                }
                WebCommand::RemoveState {
                    kind,
                    ch,
                    id,
                    ext,
                    frame,
                    name,
                } => {
                    let Some(key) = command_key(&kind, ch, id, ext, frame, name) else {
                        app.status = format!("unknown signal kind: {kind}");
                        continue;
                    };
                    app.set_win_signal(crate::workspace::PopupTarget::State(0), key, false);
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
                WebCommand::SetStateRule {
                    kind,
                    ch,
                    id,
                    ext,
                    frame,
                    name,
                    cuts,
                    names,
                } => {
                    let Some(key) = command_key(&kind, ch, id, ext, frame, name) else {
                        app.status = format!("unknown signal kind: {kind}");
                        continue;
                    };
                    set_state_rule(&mut app, &key, cuts, names);
                }
                WebCommand::ClearStateRule {
                    kind,
                    ch,
                    id,
                    ext,
                    frame,
                    name,
                } => {
                    let Some(key) = command_key(&kind, ch, id, ext, frame, name) else {
                        app.status = format!("unknown signal kind: {kind}");
                        continue;
                    };
                    clear_state_rule(&mut app, &key);
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
                WebCommand::SetNodeRole {
                    bus,
                    node,
                    simulated,
                } => {
                    app.send(BusCommand::SetNodeRole {
                        ch: bus,
                        node,
                        role: if simulated {
                            crate::app::NodeRole::Simulated
                        } else {
                            crate::app::NodeRole::Absent
                        },
                    });
                }
                WebCommand::LoadFrDescription { path, bus } => {
                    if app.snap.measuring {
                        app.stop();
                    }
                    app.load_cluster_description(&path, bus);
                }
                WebCommand::RemoveFrBus { bus } => {
                    if app.snap.measuring {
                        app.stop();
                    }
                    app.remove_fr_bus(bus);
                }
                WebCommand::LoadEthDescription { path, bus } => {
                    if app.snap.measuring {
                        app.stop();
                    }
                    app.load_eth_description(&path, bus);
                }
                WebCommand::RemoveEthBus { bus } => {
                    if app.snap.measuring {
                        app.stop();
                    }
                    app.remove_eth_bus(bus);
                }
                WebCommand::AddFrEntry { bus, slot } => {
                    app.add_fr_tx(bus, slot);
                }
                WebCommand::RemoveFrEntry { bus, slot } => {
                    app.send(BusCommand::RemoveFrEntry { bus, slot });
                }
                WebCommand::SetFrEntryActive { bus, slot, on } => {
                    app.send(BusCommand::SetFrEntryActive { bus, slot, on });
                }
                WebCommand::SetFrEntryCycle {
                    bus,
                    slot,
                    cycle_us,
                } => {
                    app.send(BusCommand::SetFrEntryCycle {
                        bus,
                        slot,
                        cycle_us,
                    });
                }
                WebCommand::SetFrEntryHex { bus, slot, text } => {
                    app.send(BusCommand::SetFrEntryHex { bus, slot, text });
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
            let sig = scripts_signature(&app);
            if sig != scripts_sig
                && let Ok(mut slot) = scripts.lock()
            {
                *slot = scripts_snapshot(&app);
                scripts_sig = sig;
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
    /// The FR and Ethernet rings expose no head-loss counter; a shrinking
    /// length is a new run (or a cleared ring), so it triggers a reseed.
    last_fr_len: usize,
    last_eth_len: usize,
    primed: bool,
}

impl TraceStore {
    fn seed_rows(app: &App) -> Vec<WebFrame> {
        let mut rows: Vec<WebFrame> = app
            .snap
            .trace
            .iter()
            .map(|frame| frame_dto(app, frame))
            .chain(app.snap.fr_trace.iter().map(|row| fr_frame_dto(app, row)))
            .chain(
                app.snap
                    .eth_trace
                    .iter()
                    .map(|frame| eth_frame_dto(app, frame)),
            )
            .collect();
        rows.sort_by_key(|row| row.t_us);
        if rows.len() > TRACE_LOG_CAP {
            rows.drain(..rows.len() - TRACE_LOG_CAP);
        }
        rows
    }

    fn sync(&mut self, app: &App) {
        let len = app.snap.trace.len();
        let loss = app.snap.trace.head_loss().0;
        let total = len as u64 + loss;
        let fr_len = app.snap.fr_trace.len();
        let eth_len = app.snap.eth_trace.len();
        let reset = !self.primed
            || total < self.last_total
            || fr_len < self.last_fr_len
            || eth_len < self.last_eth_len;
        if reset {
            self.primed = true;
            self.epoch = self.epoch.wrapping_add(1).max(1);
            self.next_seq = 0;
            self.log.clear();
            for row in Self::seed_rows(app) {
                self.push(row);
            }
        } else {
            let mut rows = Vec::new();
            let new_count = total.saturating_sub(self.last_total) as usize;
            if new_count > 0 {
                rows.extend(
                    app.snap
                        .trace
                        .iter()
                        .rev()
                        .take(new_count)
                        .map(|frame| frame_dto(app, frame)),
                );
            }
            if fr_len > self.last_fr_len {
                let take = fr_len - self.last_fr_len;
                rows.extend(
                    app.snap
                        .fr_trace
                        .iter()
                        .rev()
                        .take(take)
                        .map(|row| fr_frame_dto(app, row)),
                );
            }
            if eth_len > self.last_eth_len {
                let take = eth_len - self.last_eth_len;
                rows.extend(
                    app.snap
                        .eth_trace
                        .iter_rev()
                        .take(take)
                        .map(|frame| eth_frame_dto(app, frame)),
                );
            }
            // The three rings are drained in arrival order within one
            // publish; sorting the small delta keeps the merged table
            // chronological without touching the stored log.
            rows.sort_by_key(|row| row.t_us);
            for row in rows {
                self.push(row);
            }
        }
        self.last_total = total;
        self.last_fr_len = fr_len;
        self.last_eth_len = eth_len;
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
        self.last_fr_len = 0;
        self.last_eth_len = 0;
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
        kind: "can".to_string(),
        addr: if frame.extended {
            format!("0x{:08X}", frame.id)
        } else {
            format!("0x{:03X}", frame.id)
        },
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

fn fr_frame_dto(app: &App, row: &crate::trace::FrRow) -> WebFrame {
    WebFrame {
        kind: "fr".to_string(),
        addr: format!("slot {}", row.slot),
        t_us: row.t_us,
        bus: row.bus,
        bus_name: app.fr_bus_name(row.bus),
        id: row.slot as u32,
        extended: false,
        name: app.fr_row_name(row).unwrap_or("").to_string(),
        dir: "Rx",
        len: row.payload.len().min(u8::MAX as usize) as u8,
        data: hex(&row.payload),
        fd: false,
    }
}

fn eth_frame_dto(app: &App, frame: &crate::eth::EthFrame) -> WebFrame {
    const SHOWN_BYTES: usize = 32;
    let shown = frame.payload.len().min(SHOWN_BYTES);
    let mut data = hex(&frame.payload[..shown]);
    if frame.payload.len() > SHOWN_BYTES {
        data.push_str(" …");
    }
    WebFrame {
        kind: "eth".to_string(),
        addr: format!("0x{:04X}", frame.ethertype),
        t_us: frame.t_us,
        bus: frame.bus,
        bus_name: app.eth_bus_label(frame.bus),
        id: frame.ethertype as u32,
        extended: false,
        name: app.eth_row_name(frame),
        dir: match frame.dir {
            Direction::Rx => "Rx",
            Direction::Tx => "Tx",
        },
        len: frame.payload.len().min(u8::MAX as usize) as u8,
        data,
        fd: false,
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
        self.select_key(app, SigKey::can(ch, id, ext, name));
    }

    fn select_key(&mut self, app: &App, key: SigKey) {
        if self.is_selected(&key) {
            return;
        }
        let name = match &key {
            SigKey::Can { name, .. } | SigKey::Fr { name, .. } | SigKey::Eth { name, .. } => {
                name.clone()
            }
        };
        let bus = signal_bus(app, &key);
        let unit = match &key {
            SigKey::Can { ch, id, ext, name } => signal_unit(app, *ch, *id, *ext, name),
            // FR/Eth units arrive with the first subscription publish.
            SigKey::Fr { .. } | SigKey::Eth { .. } => String::new(),
        };
        let color = (0..SIGNAL_PALETTE)
            .find(|color| !self.selected.iter().any(|signal| signal.color == *color))
            .unwrap_or(self.selected.len() % SIGNAL_PALETTE);
        self.selected.push(SelectedSignal {
            key,
            name,
            bus,
            unit,
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
            signal.bus = signal_bus(app, &signal.key);
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

#[cfg(test)]
fn key_channel(key: &SigKey) -> u8 {
    match key {
        SigKey::Can { ch, .. } => *ch,
        SigKey::Fr { bus, .. } | SigKey::Eth { bus, .. } => *bus,
    }
}

#[cfg(test)]
fn key_id(key: &SigKey) -> u32 {
    match key {
        SigKey::Can { id, .. } => *id,
        SigKey::Fr { slot, .. } => *slot as u32,
        SigKey::Eth { .. } => 0,
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

/// Resolves the address every observer command carries. CAN keys use a
/// message id; FlexRay keys use a slot in the same `id` field with the
/// cluster in `ch`; Ethernet names its frame. One command shape covers the
/// three bus kinds.
fn command_key(
    kind: &str,
    ch: u8,
    id: u32,
    ext: bool,
    frame: Option<String>,
    name: String,
) -> Option<SigKey> {
    match kind {
        "can" => Some(SigKey::can(ch, id, ext, name)),
        "fr" => u16::try_from(id)
            .ok()
            .map(|slot| SigKey::fr(ch, slot, name)),
        "eth" => frame.map(|frame| SigKey::eth(ch, frame, name)),
        _ => None,
    }
}

struct RowAddress {
    kind: &'static str,
    ch: u8,
    id: u32,
    ext: bool,
    frame: Option<String>,
}

/// Observer rows carry one address shape for every bus kind. FlexRay puts
/// its cluster in `ch` and its slot in `id`; Ethernet keeps `ch` and names
/// its frame.
fn row_address(key: &SigKey) -> RowAddress {
    match key {
        SigKey::Can { ch, id, ext, .. } => RowAddress {
            kind: "can",
            ch: *ch,
            id: *id,
            ext: *ext,
            frame: None,
        },
        SigKey::Fr { bus, slot, .. } => RowAddress {
            kind: "fr",
            ch: *bus,
            id: u32::from(*slot),
            ext: false,
            frame: None,
        },
        SigKey::Eth { bus, frame, .. } => RowAddress {
            kind: "eth",
            ch: *bus,
            id: 0,
            ext: false,
            frame: Some(frame.clone()),
        },
    }
}

fn series_key(key: &SigKey) -> String {
    match key {
        SigKey::Can { ch, id, name, .. } => format!("{ch}:{id}:{name}"),
        SigKey::Fr { bus, slot, name } => format!("fr:{bus}:{slot}:{name}"),
        SigKey::Eth { bus, frame, name } => format!("eth:{bus}:{frame}:{name}"),
    }
}

fn signal_bus(app: &App, key: &SigKey) -> String {
    match key {
        SigKey::Can { ch, .. } => bus_name(app, *ch),
        SigKey::Fr { bus, .. } => app.fr_bus_name(*bus),
        SigKey::Eth { bus, .. } => app.eth_bus_label(*bus),
    }
}

fn signal_meta(signal: &SelectedSignal) -> SignalSeries {
    SignalSeries {
        key: series_key(&signal.key),
        name: signal.name.clone(),
        bus: signal.bus.clone(),
        unit: signal.unit.clone(),
        color: signal.color,
        points: Vec::new(),
    }
}

/// Build the signal tree for the picker: every CAN channel with a DBC plus
/// every loaded FlexRay cluster. Keys are the same strings the series
/// endpoint reports, so selection state round-trips without translation.
#[derive(Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
struct WebCatalog {
    channels: Vec<WebCatalogChannel>,
}

#[derive(Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
struct WebCatalogChannel {
    kind: String,
    index: u8,
    name: String,
    messages: Vec<WebCatalogMessage>,
}

#[derive(Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
struct WebCatalogMessage {
    id: u32,
    extended: bool,
    name: String,
    /// Ethernet frames are addressed by name, not by their catalog index.
    #[serde(skip_serializing_if = "Option::is_none")]
    frame: Option<String>,
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
    let fr: Vec<String> = app
        .fr_buses
        .iter()
        .map(|(bus, cfg)| format!("fr{bus}:{}:{}", cfg.path, Arc::as_ptr(&cfg.db) as usize))
        .collect();
    let eth: Vec<String> = app
        .eth_buses
        .iter()
        .map(|(bus, cfg)| format!("eth{bus}:{}:{}", cfg.path, Arc::as_ptr(&cfg.db) as usize))
        .collect();
    parts
        .into_iter()
        .chain(fr)
        .chain(eth)
        .collect::<Vec<_>>()
        .join(";")
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
    let mut channels: Vec<WebCatalogChannel> = app
        .snap
        .channels
        .iter()
        .enumerate()
        .map(|(ch, channel)| WebCatalogChannel {
            kind: "can".to_string(),
            index: ch as u8,
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
                                frame: None,
                                signals: message
                                    .signals
                                    .iter()
                                    .map(|signal| WebCatalogSignal {
                                        key: series_key(&SigKey::can(
                                            ch as u8,
                                            id,
                                            ext,
                                            &signal.name,
                                        )),
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
        .collect();
    for (bus, cfg) in &app.fr_buses {
        let messages = cfg
            .db
            .slot_signals()
            .into_iter()
            .map(|(slot, frame, names)| WebCatalogMessage {
                id: slot as u32,
                extended: false,
                name: frame,
                frame: None,
                signals: names
                    .into_iter()
                    .map(|name| {
                        let meta = cfg.db.slot_signal(slot, &name);
                        WebCatalogSignal {
                            key: series_key(&SigKey::fr(*bus, slot, &name)),
                            name,
                            unit: meta.map(|signal| signal.unit.clone()).unwrap_or_default(),
                            min: meta.map(|signal| signal.min).unwrap_or(0.0),
                            max: meta.map(|signal| signal.max).unwrap_or(0.0),
                        }
                    })
                    .collect(),
            })
            .collect();
        channels.push(WebCatalogChannel {
            kind: "fr".to_string(),
            index: *bus,
            name: app.fr_bus_name(*bus),
            messages,
        });
    }
    for (bus, cfg) in &app.eth_buses {
        let messages = cfg
            .db
            .frames
            .iter()
            .enumerate()
            .map(|(index, frame)| WebCatalogMessage {
                id: index as u32,
                extended: false,
                name: frame.name.clone(),
                frame: Some(frame.name.clone()),
                signals: frame
                    .pdus
                    .iter()
                    .flat_map(|pdu| &pdu.signals)
                    .map(|signal| WebCatalogSignal {
                        key: series_key(&SigKey::eth(*bus, &frame.name, &signal.name)),
                        name: signal.name.clone(),
                        unit: signal.unit.clone(),
                        min: 0.0,
                        max: 0.0,
                    })
                    .collect(),
            })
            .collect();
        channels.push(WebCatalogChannel {
            kind: "eth".to_string(),
            index: *bus,
            name: app.eth_bus_label(*bus),
            messages,
        });
    }
    WebCatalog { channels }
}

#[derive(Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
struct WebState {
    measuring: bool,
    trace_paused: bool,
    recording: bool,
    record_file: String,
    record_filter: String,
    mode: String,
    /// Bus-event marker times in seconds, oldest first.
    markers: Vec<f64>,
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
    replay_speed: f64,
    recent_projects: Vec<String>,
    recent_logs: Vec<String>,
    replay_map: WebReplayMap,
    buses: Vec<WebBus>,
    fr_buses: Vec<WebFrBus>,
    eth_buses: Vec<WebEthBus>,
    real_bus: bool,
    hw_refresh: u64,
    tx: Vec<WebTx>,
    fr_tx: Vec<WebFrTx>,
    triggers: Vec<WebTrigger>,
    trigger_limits: WebTriggerLimits,
    sysvars: Vec<WebSysVar>,
    monitor: Vec<WebMonitorRow>,
    data: Vec<WebDataRow>,
    state_rows: Vec<WebStateRow>,
    state_window_s: f64,
    scripts: Vec<WebScriptInfo>,
    stats: Vec<WebStatsRow>,
    bus_stats: Vec<WebBusStats>,
    spec_rows: Vec<WebSpecRow>,
    spec_tolerance: u64,
    spec_grace: u64,
    spec_periodic: usize,
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
struct WebFrBus {
    bus: u8,
    name: String,
    path: String,
    cluster: String,
    frames: usize,
    attached: bool,
    channel: i32,
}

#[derive(Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
struct WebEthBus {
    bus: u8,
    name: String,
    path: String,
    clusters: usize,
    frames: usize,
}

#[derive(Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
struct WebFrTx {
    bus: u8,
    bus_name: String,
    slot: u16,
    name: String,
    node: String,
    active: bool,
    cycle_us: u64,
    data_text: String,
    sent_text: String,
    muted: bool,
    undescribed: bool,
}

#[derive(Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
struct WebNode {
    name: String,
    simulated: bool,
    entries: usize,
}

#[derive(Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
struct WebReplayRow {
    raw: u8,
    count: u64,
    target: Option<u8>,
}

#[derive(Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
struct WebReplayMap {
    can: Vec<WebReplayRow>,
    fr: Vec<WebReplayRow>,
    eth: Vec<WebReplayRow>,
}

/// Census rows plus configured rows whose channel has not arrived yet, with
/// the target the core would actually use (`None` = no replay).
fn replay_rows(
    census: &[(u8, u64)],
    configured: Vec<(u8, Option<u8>)>,
    effective: impl Fn(u8) -> Option<u8>,
) -> Vec<WebReplayRow> {
    let mut rows: std::collections::BTreeMap<u8, u64> = census.iter().copied().collect();
    for (raw, _) in configured {
        rows.entry(raw).or_insert(0);
    }
    rows.into_iter()
        .map(|(raw, count)| WebReplayRow {
            raw,
            count,
            target: effective(raw),
        })
        .collect()
}

#[derive(Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
struct WebBus {
    index: u8,
    name: String,
    dbc_paths: Vec<String>,
    bitrate_kbps: u32,
    fd_data_kbps: u32,
    nodes: Vec<WebNode>,
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
struct WebSpecRow {
    name: String,
    addr: String,
    bus: String,
    rule: String,
    declared: String,
    measured: String,
    count: u64,
    first: String,
    last: String,
}

#[derive(Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
struct WebBusStatRow {
    label: String,
    current: String,
    min: String,
    max: String,
    avg: String,
}

#[derive(Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
struct WebBusStats {
    name: String,
    kbps: u32,
    load_now: f64,
    load_history: Vec<f64>,
    rows: Vec<WebBusStatRow>,
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
    kind: String,
    ch: u8,
    id: u32,
    ext: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    frame: Option<String>,
    name: String,
    label: String,
    bus: String,
    value: f64,
    has_value: bool,
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
    kind: String,
    ch: u8,
    id: u32,
    ext: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    frame: Option<String>,
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
struct WebStateRule {
    cuts: Vec<f64>,
    names: Vec<String>,
}

#[derive(Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
struct WebStateRow {
    kind: String,
    ch: u8,
    id: u32,
    ext: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    frame: Option<String>,
    name: String,
    bus: String,
    current: String,
    bands: Vec<WebStateBand>,
    rule: Option<WebStateRule>,
}

#[derive(Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
struct WebScript {
    id: u64,
    name: String,
    channel: u8,
    attached_bus: Option<u8>,
    attached_node: Option<String>,
    enabled: bool,
    running: bool,
    errored: bool,
    file_path: String,
    source: String,
    log: Vec<String>,
}

#[derive(Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
struct WebScriptInfo {
    id: u64,
    name: String,
    channel: u8,
    attached_bus: Option<u8>,
    attached_node: Option<String>,
    enabled: bool,
    running: bool,
    errored: bool,
    file_path: String,
    log_len: usize,
    last_log: String,
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
    fr_channels: Vec<WebHwChannel>,
    error: String,
    fr_error: String,
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
    kind: String,
    addr: String,
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
    kind: String,
    addr: String,
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
    let buses: Vec<WebBus> = snap
        .channels
        .iter()
        .enumerate()
        .map(|(index, ch)| WebBus {
            index: index as u8,
            name: ch.name.clone(),
            dbc_paths: ch.dbc_paths.clone(),
            bitrate_kbps: ch.bitrate_kbps,
            fd_data_kbps: ch.fd_data_kbps,
            nodes: ch
                .dbc
                .as_ref()
                .map(|db| {
                    db.nodes
                        .iter()
                        .map(|node| WebNode {
                            name: node.clone(),
                            simulated: ch.role_of(node) == crate::app::NodeRole::Simulated,
                            entries: snap
                                .tx
                                .iter()
                                .filter(|entry| entry.channel == index as u8 && entry.node == *node)
                                .count(),
                        })
                        .collect()
                })
                .unwrap_or_default(),
            attached: snap
                .hw
                .iter()
                .find(|hw| hw.bus == index as u8)
                .map(|hw| WebHwAttachment {
                    driver: hw.driver.word().to_string(),
                    adapter: hw.adapter,
                    kbps: hw.kbps,
                    fd: hw.fd,
                    can_tx: hw.can_tx,
                    tx_fail: hw.tx_fail.as_ref().map(|(reason, count)| WebTxFail {
                        reason: reason.clone(),
                        count: *count,
                    }),
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

    let mut fr_tx: Vec<WebFrTx> = snap
        .fr_tx
        .iter()
        .map(|entry| WebFrTx {
            bus: entry.bus,
            bus_name: app.fr_bus_name(entry.bus),
            slot: entry.slot,
            name: entry.name.clone(),
            node: entry.node.clone(),
            active: entry.active,
            cycle_us: entry.cycle_us,
            data_text: entry.data_text.clone(),
            sent_text: entry.sent_text.clone(),
            muted: entry.muted,
            undescribed: entry.undescribed,
        })
        .collect();
    fr_tx.sort_by_key(|entry| (entry.bus, entry.slot));

    let total_frames: u64 = snap
        .aggs
        .iter()
        .map(|agg| agg.count)
        .chain(snap.fr_aggs.iter().map(|agg| agg.count))
        .chain(snap.eth_aggs.iter().map(|agg| agg.count))
        .sum();
    let share_text = |count: u64| {
        format!(
            "{:.1}%",
            if total_frames > 0 {
                count as f64 / total_frames as f64 * 100.0
            } else {
                0.0
            }
        )
    };
    let ms = |value: f64, count: u64| {
        if count >= 2 {
            format!("{:.2}", value / 1000.0)
        } else {
            "-".to_string()
        }
    };
    let mut stats: Vec<WebStatsRow> = snap
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
                share: share_text(agg.count),
                len: agg.len,
            }
        })
        .collect();
    for agg in &snap.fr_aggs {
        let name = App::fr_frame_name(agg);
        stats.push(WebStatsRow {
            label: if name.is_empty() {
                format!("slot {}", agg.slot)
            } else {
                format!("slot {}  {name}", agg.slot)
            },
            bus: app.fr_bus_name(agg.bus),
            count: agg.count,
            min_ms: ms(agg.min_us, agg.count),
            avg_ms: ms(agg.cycle_us, agg.count),
            max_ms: ms(agg.max_us, agg.count),
            jitter_ms: ms(agg.jitter_us, agg.count),
            share: share_text(agg.count),
            len: agg.payload.len().min(u8::MAX as usize) as u8,
        });
    }
    for agg in &snap.eth_aggs {
        let vlan = agg
            .key
            .vlan
            .map(|vlan| format!("VLAN {vlan}  "))
            .unwrap_or_default();
        stats.push(WebStatsRow {
            label: format!("{vlan}{}", app.eth_flow_name(&agg.key)),
            bus: app.eth_bus_label(agg.key.bus),
            count: agg.count,
            min_ms: ms(agg.min_us, agg.count),
            avg_ms: ms(agg.cycle_us, agg.count),
            max_ms: ms(agg.max_us, agg.count),
            jitter_ms: ms(agg.jitter_us, agg.count),
            share: share_text(agg.count),
            len: agg.key.len.min(u8::MAX as usize) as u8,
        });
    }

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

    let fr_buses: Vec<WebFrBus> = app
        .fr_buses
        .iter()
        .map(|(bus, cfg)| {
            let watch = app.snap.fr_watches.iter().find(|watch| watch.bus == *bus);
            WebFrBus {
                bus: *bus,
                name: app.fr_bus_name(*bus),
                path: cfg.path.clone(),
                cluster: cfg.db.params.name.clone(),
                frames: cfg.db.frames.len(),
                attached: watch.is_some(),
                channel: watch.map(|watch| watch.channel_index).unwrap_or(-1),
            }
        })
        .collect();
    let eth_buses: Vec<WebEthBus> = app
        .eth_buses
        .iter()
        .map(|(bus, cfg)| WebEthBus {
            bus: *bus,
            name: app.eth_bus_label(*bus),
            path: cfg.path.clone(),
            clusters: cfg.db.clusters.len(),
            frames: cfg.db.frames.len(),
        })
        .collect();
    let replay_map = {
        use crate::replay_map::LogBusKind;
        let map = &snap.replay_map;
        WebReplayMap {
            can: replay_rows(
                &snap.replay_census_can,
                map.configured(LogBusKind::Can),
                |raw| map.can_channel(raw, snap.channels.len()),
            ),
            fr: replay_rows(
                &snap.replay_census_fr,
                map.configured(LogBusKind::Fr),
                |raw| map.target(LogBusKind::Fr, raw),
            ),
            eth: replay_rows(
                &snap.replay_census_eth,
                map.configured(LogBusKind::Eth),
                |raw| map.target(LogBusKind::Eth, raw),
            ),
        }
    };

    let bus_stats: Vec<WebBusStats> = snap
        .bus_loads
        .iter()
        .enumerate()
        .filter_map(|(index, load)| {
            let channel = snap.channels.get(index)?;
            Some(WebBusStats {
                name: channel.name.clone(),
                kbps: channel.bitrate_kbps,
                load_now: load.load(),
                load_history: load.history().map(|(_, value)| value).collect(),
                rows: bus_stat_rows(load),
            })
        })
        .collect();

    let spec_rows: Vec<WebSpecRow> = app
        .spec_rows()
        .into_iter()
        .map(|row| WebSpecRow {
            name: row.name,
            addr: row.addr,
            bus: row.bus,
            rule: row.kind.label().to_string(),
            declared: crate::spec::qty(row.kind, row.declared),
            measured: crate::spec::qty(row.kind, row.measured),
            count: row.count,
            first: format!("{:.3} s", row.first_t_us as f64 / 1e6),
            last: format!("{:.3} s", row.last_t_us as f64 / 1e6),
        })
        .collect();
    let spec_periodic: usize = snap
        .channels
        .iter()
        .filter_map(|channel| channel.dbc.as_ref())
        .map(|db| {
            db.messages
                .values()
                .filter(|message| message.cycle_us.is_some_and(|cycle| cycle > 0))
                .count()
        })
        .sum();

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
                .map(|signal| {
                    let RowAddress {
                        kind,
                        ch,
                        id,
                        ext,
                        frame,
                    } = row_address(&signal.key);
                    let name = signal.key.name().to_string();
                    let sub = signal_sub(app, &signal.key);
                    let (value, unit, raw, min, avg, max, frac) = match sub {
                        // An empty subscription has min = inf / max = -inf;
                        // a fresh row shows dashes until its first frame.
                        Some(sub) if sub.min <= sub.max => {
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
                        _ => (
                            "-".to_string(),
                            String::new(),
                            "-".to_string(),
                            "-".to_string(),
                            "-".to_string(),
                            "-".to_string(),
                            0.0,
                        ),
                    };
                    WebDataRow {
                        kind: kind.to_string(),
                        ch,
                        id,
                        ext,
                        frame,
                        name,
                        bus: signal_bus(app, &signal.key),
                        value,
                        unit,
                        raw,
                        min,
                        avg,
                        max,
                        frac,
                        visible: signal.visible,
                    }
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
                .map(|signal| {
                    let RowAddress {
                        kind,
                        ch,
                        id,
                        ext,
                        frame,
                    } = row_address(&signal.key);
                    let name = signal.key.name().to_string();
                    let rule = window.rules.get(&signal.key);
                    let (current, bands) = match signal_sub(app, &signal.key) {
                        Some(sub) => {
                            let held = sub.history.at(lo);
                            let samples: Vec<(u64, f64)> =
                                sub.history.range(lo, hi).copied().collect();
                            match rule {
                                Some(rule) => state_bands_custom(held, &samples, lo, hi, rule),
                                None => state_bands_from(held, &samples, lo, hi),
                            }
                        }
                        None => ("-".to_string(), Vec::new()),
                    };
                    WebStateRow {
                        kind: kind.to_string(),
                        ch,
                        id,
                        ext,
                        frame,
                        name,
                        bus: signal_bus(app, &signal.key),
                        current,
                        bands,
                        rule: rule.map(|rule| WebStateRule {
                            cuts: rule.cuts.clone(),
                            names: rule.names.clone(),
                        }),
                    }
                })
                .collect()
        })
        .unwrap_or_default();

    let scripts: Vec<WebScriptInfo> = snap
        .nodes
        .iter()
        .map(|node| WebScriptInfo {
            id: node.id,
            name: node.name.clone(),
            channel: node.channel,
            attached_bus: node.attached.as_ref().map(|(bus, _)| *bus),
            attached_node: node.attached.as_ref().map(|(_, name)| name.clone()),
            enabled: node.enabled,
            running: node.running,
            errored: node.errored,
            file_path: node.file_path.clone().unwrap_or_default(),
            log_len: node.log.len(),
            last_log: node.log.last().cloned().unwrap_or_default(),
        })
        .collect();

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

    let monitor: Vec<WebMonitorRow> = app
        .monitor_view
        .rows
        .iter()
        .map(|row| {
            let RowAddress {
                kind,
                ch,
                id,
                ext,
                frame,
            } = row_address(&row.key);
            let (value, has_value, unit) = signal_sub(app, &row.key)
                .map(|sub| (sub.latest, sub.min <= sub.max, sub.unit.clone()))
                .unwrap_or((0.0, false, String::new()));
            WebMonitorRow {
                kind: kind.to_string(),
                ch,
                id,
                ext,
                frame,
                name: row.key.name().to_string(),
                label: row.label.clone(),
                bus: signal_bus(app, &row.key),
                value,
                has_value,
                unit,
                digits: row.digits,
                rule_on: row.rule_on,
                rising: row.rising,
                threshold: row.threshold,
                color: row.color,
            }
        })
        .collect();

    let mut messages: Vec<WebMessage> = snap
        .aggs
        .iter()
        .map(|m| {
            let addr = if m.extended {
                format!("0x{:08X}", m.id)
            } else {
                format!("0x{:03X}", m.id)
            };
            WebMessage {
                kind: "can".to_string(),
                addr,
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
            }
        })
        .collect();
    for m in &snap.fr_aggs {
        let name = App::fr_frame_name(m).to_string();
        messages.push(WebMessage {
            kind: "fr".to_string(),
            addr: format!("slot {}", m.slot),
            bus: m.bus,
            bus_name: app.fr_bus_name(m.bus),
            id: m.slot as u32,
            extended: false,
            name: if name.is_empty() {
                format!("slot {}", m.slot)
            } else {
                name
            },
            count: m.count,
            cycle_ms: m.cycle_us / 1000.0,
            len: m.payload.len().min(u8::MAX as usize) as u8,
            data: hex(&m.payload),
        });
    }
    for a in &snap.eth_aggs {
        let vlan = a
            .key
            .vlan
            .map(|vlan| format!("VLAN {vlan}  "))
            .unwrap_or_default();
        messages.push(WebMessage {
            kind: "eth".to_string(),
            addr: format!("0x{:04X}", a.key.ethertype),
            bus: a.key.bus,
            bus_name: app.eth_bus_label(a.key.bus),
            id: a.key.ethertype as u32,
            extended: false,
            name: format!("{vlan}{}", app.eth_flow_name(&a.key)),
            count: a.count,
            cycle_ms: a.cycle_us / 1000.0,
            len: a.key.len.min(u8::MAX as usize) as u8,
            data: String::new(),
        });
    }
    messages.sort_by_key(|m| {
        let order = match m.kind.as_str() {
            "can" => 0,
            "fr" => 1,
            _ => 2,
        };
        (order, m.bus, m.id)
    });

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
        recording: snap.recording,
        record_file: snap.last_record.clone(),
        record_filter: app.record_filter_text.clone(),
        markers: snap
            .markers
            .iter()
            .rev()
            .take(500)
            .rev()
            .map(|t_us| *t_us as f64 / 1e6)
            .collect(),
        mode: match snap.mode {
            Mode::Virtual => "Simulation".to_string(),
            Mode::Replay => "Replay".to_string(),
        },
        frames: snap.frame_counter,
        fps,
        trace_len: snap.trace.len() + snap.fr_trace.len() + snap.eth_trace.len(),
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
        replay_speed: if app.replay_speed.is_finite() {
            app.replay_speed
        } else {
            1e9
        },
        recent_projects: app.recent_projects.clone(),
        recent_logs: app.recent_log.clone(),
        replay_map,
        buses,
        fr_buses,
        eth_buses,
        real_bus: snap.real_bus,
        hw_refresh: snap.hw_refresh,
        tx,
        fr_tx,
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
        scripts,
        stats,
        bus_stats,
        spec_rows,
        spec_tolerance: app.spec_tol_pct,
        spec_grace: app.spec_grace,
        spec_periodic,
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
        let path = std::env::temp_dir().join("roxy_web_layout_project.rxproj");
        let mut source = App::headless();
        source.layout_cache = "[Window][Dockspace]\n".to_string();
        assert!(source.save_project(Some(path.clone())), "fixture saves");
        let mut app = App::headless();
        prepare_session(&mut app, Some(path.to_str().unwrap()), None);
        assert!(
            !app.layout_cache.is_empty(),
            "the top-level dock layout survives the headless load"
        );
        assert!(!app.is_dirty(), "web startup must not edit desktop config");
        app.plot_stride_us = Some(5_000);
        assert!(!app.is_dirty(), "the plot stride is frontend state");
        std::fs::remove_file(&path).ok();
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

    /// Bus nodes carry their role and generator-entry count, so the Buses
    /// dialog can switch simulation per ECU instead of all-or-nothing.
    #[test]
    fn the_state_lists_bus_nodes_and_roles() {
        let mut app = App::headless();
        let node = app.snap.channels[0]
            .dbc
            .as_ref()
            .and_then(|db| db.nodes.first().cloned())
            .expect("sample DBC nodes");
        app.set_node_role(0, &node, crate::app::NodeRole::Simulated);
        app.settle();
        let state = build_state(&app, &VecDeque::new(), 0.0);
        let bus = state
            .buses
            .iter()
            .find(|bus| bus.index == 0)
            .expect("first bus");
        let row = bus
            .nodes
            .iter()
            .find(|row| row.name == node)
            .expect("the node is listed");
        assert!(row.simulated);
        assert!(row.entries > 0, "a simulated node owns generator entries");

        app.set_node_role(0, &node, crate::app::NodeRole::Absent);
        app.settle();
        let state = build_state(&app, &VecDeque::new(), 0.0);
        let bus = state.buses.iter().find(|bus| bus.index == 0).unwrap();
        let row = bus.nodes.iter().find(|row| row.name == node).unwrap();
        assert!(!row.simulated);
    }

    /// Node scripts reach the web with their source, binding and summary;
    /// the editor cache only refreshes when the fingerprint changes.
    #[test]
    fn scripts_reach_the_web_with_source_and_runtime_state() {
        let mut app = App::headless();
        app.send(BusCommand::AddNode {
            name: "Node A".to_string(),
            channel: 0,
            attached: Some((0, "EngineECU".to_string())),
        });
        app.settle();
        let id = app.snap.nodes[0].id;
        let before = scripts_signature(&app);
        app.send(BusCommand::SetNodeSource {
            id,
            source: "on start { print(\"hi\"); }".to_string(),
        });
        app.settle();
        assert_ne!(
            scripts_signature(&app),
            before,
            "source edits refresh /nodes"
        );
        let snapshots = scripts_snapshot(&app);
        assert_eq!(snapshots.len(), 1);
        assert_eq!(snapshots[0].name, "Node A");
        assert!(snapshots[0].source.contains("print"));
        assert_eq!(snapshots[0].attached_node.as_deref(), Some("EngineECU"));
        let state = build_state(&app, &VecDeque::new(), 0.0);
        assert_eq!(state.scripts.len(), 1);
        assert_eq!(state.scripts[0].id, id);
        assert!(state.scripts[0].enabled);
    }

    /// The web export paths write the desktop formats: ASC for trace and
    /// CSV headers for Messages/Statistics/Data/State.
    #[test]
    fn web_exports_write_the_desktop_formats() {
        let mut app = App::headless();
        seed_demo(&mut app);
        for _ in 0..20 {
            app.update();
        }
        chart_select(&mut app, 0, 0x100, false, "EngineSpeed");
        let dir = std::env::temp_dir();
        let files = [
            (
                "trace",
                dir.join("roxy_web_export.asc"),
                "Begin Triggerblock",
            ),
            (
                "messages",
                dir.join("roxy_web_export_messages.csv"),
                "bus,id,name,dir,count",
            ),
            (
                "stats",
                dir.join("roxy_web_export_stats.csv"),
                "bus,id,name,count",
            ),
            (
                "data",
                dir.join("roxy_web_export_data.csv"),
                "bus,signal,value,unit",
            ),
            (
                "graphics",
                dir.join("roxy_web_export_graphics.csv"),
                "time_us,bus,signal,value",
            ),
            ("spec", dir.join("roxy_web_export_spec.csv"), "# database"),
        ];
        for (kind, path, header) in &files {
            export_file(&mut app, kind, path.to_str().unwrap());
            let text = std::fs::read_to_string(path).unwrap_or_default();
            assert!(
                text.contains(header),
                "{kind} export should contain {header:?}, got {:?}",
                text.chars().take(60).collect::<String>()
            );
            std::fs::remove_file(path).ok();
        }
    }

    /// Chart selection is also Graphics-window state, so it survives a
    /// project save and seeds the lanes when the project is opened again.
    #[test]
    fn chart_selection_round_trips_through_the_graphics_window() {
        let mut app = App::headless();
        chart_select(&mut app, 0, 0x100, false, "EngineSpeed");
        let key = SigKey::can(0, 0x100, false, "EngineSpeed");
        assert!(
            app.graphics
                .first()
                .is_some_and(|window| window.signals.iter().any(|row| row.key == key))
        );
        assert!(app.config_snapshot().contains("EngineSpeed"));

        let mut seeded = SignalStore::default();
        seed_chart_selection(&mut app, &mut seeded);
        assert!(seeded.is_selected(&key), "a load repopulates the lanes");

        chart_unselect(&mut app, &key);
        assert!(
            app.graphics
                .first()
                .is_none_or(|window| window.signals.iter().all(|row| row.key != key))
        );
    }

    /// The Bus Statistics tab reads the same BusLoad numbers as the desktop
    /// window: load, send distance, bursts and per-class rates.
    #[test]
    fn the_state_lists_bus_load_statistics() {
        let mut app = App::headless();
        seed_demo(&mut app);
        for _ in 0..30 {
            app.update();
        }
        let state = build_state(&app, &VecDeque::new(), 0.0);
        assert!(!state.bus_stats.is_empty());
        let bus = &state.bus_stats[0];
        assert_eq!(bus.name, "CAN1");
        assert!(!bus.load_history.is_empty(), "the sparkline has buckets");
        assert!(
            bus.rows.iter().any(|row| row.label == "Busload [%]"),
            "the load row is present"
        );
        assert!(
            bus.rows
                .iter()
                .any(|row| row.label == "Errorframes [total]"),
            "error counters are present"
        );
    }

    /// The Specification tab reads the project's tolerance/grace, the
    /// periodic-message count and the latched verdict rows.
    #[test]
    fn the_state_lists_specification_controls() {
        let mut app = App::headless();
        seed_demo(&mut app);
        for _ in 0..20 {
            app.update();
        }
        let state = build_state(&app, &VecDeque::new(), 0.0);
        assert!(state.spec_periodic > 0, "the DBC declares periodic frames");
        assert_eq!(state.spec_tolerance, app.spec_tol_pct);
        assert_eq!(state.spec_grace, app.spec_grace);
        assert!(state.spec_rows.is_empty(), "a clean demo has no verdicts");
    }

    /// Custom cuts replace the quantized value with a band index and use the
    /// band's own name, exactly like the desktop Value Definition editor.
    #[test]
    fn state_custom_bands_use_cut_names() {
        let rule = crate::observe::StateRule {
            cuts: vec![1.0],
            names: vec!["low".to_string(), "high".to_string()],
            colors: vec![None, None],
        };
        let (current, bands) =
            state_bands_custom(Some(0.0), &[(10_000, 0.0), (20_000, 2.0)], 0, 40_000, &rule);
        assert_eq!(current, "high");
        assert_eq!(bands.len(), 2);
        assert_eq!(bands[0].label, "low");
        assert_eq!(bands[1].label, "high");
    }

    /// A saved custom rule reaches the state and can be removed again.
    #[test]
    fn state_rules_reach_the_web_and_clear() {
        let key = SigKey::can(0, 0x100, false, "EngineSpeed");
        let mut app = App::headless();
        app.new_state_window();
        app.set_win_signal(crate::workspace::PopupTarget::State(0), key.clone(), true);
        app.settle();
        set_state_rule(
            &mut app,
            &key,
            vec![1000.0, 5000.0],
            vec!["idle".into(), "cruise".into(), "fast".into()],
        );
        let state = build_state(&app, &VecDeque::new(), 0.0);
        let rule = state.state_rows[0].rule.as_ref().expect("rule saved");
        assert_eq!(rule.cuts, vec![1000.0, 5000.0]);
        assert_eq!(rule.names[2], "fast");

        clear_state_rule(&mut app, &key);
        let state = build_state(&app, &VecDeque::new(), 0.0);
        assert!(state.state_rows[0].rule.is_none());
    }

    /// FR/Eth descriptions loaded into the core appear as bus sections with
    /// their cluster identity and frame counts.
    #[test]
    fn the_state_lists_flexray_and_ethernet_buses() {
        let mut app = App::headless();
        let fr = app
            .load_cluster_description(crate::test_assets::path("PowerTrain_v2.xml"), None)
            .expect("FIBEX fixture loads");
        let eth = app
            .load_eth_description(crate::test_assets::path("PowerTrain.arxml"), None)
            .expect("ARXML fixture loads");
        let state = build_state(&app, &VecDeque::new(), 0.0);
        let fr_bus = state
            .fr_buses
            .iter()
            .find(|bus| bus.bus == fr)
            .expect("FlexRay bus listed");
        assert!(!fr_bus.name.is_empty());
        assert!(fr_bus.frames > 0);
        let eth_bus = state
            .eth_buses
            .iter()
            .find(|bus| bus.bus == eth)
            .expect("Ethernet bus listed");
        assert!(!eth_bus.name.is_empty());
        assert!(eth_bus.path.contains("PowerTrain"));
    }

    /// Replay rows from the FlexRay ring join the web trace alongside CAN,
    /// with a slot address and the cluster's bus label.
    #[test]
    fn the_trace_store_merges_flexray_rows() {
        let mut app = App::headless();
        app.load_cluster_description(crate::test_assets::path("PowerTrain.arxml"), Some(0));
        app.load_cluster_description(crate::test_assets::path("PowerTrain_v2.xml"), Some(1));
        prepare_session(&mut app, None, Some(crate::test_assets::logging_blf_two()));
        for _ in 0..40 {
            std::thread::sleep(Duration::from_millis(5));
            app.update();
        }
        let mut store = TraceStore::default();
        store.sync(&app);
        let rows = store.seed();
        let fr = rows
            .iter()
            .find(|row| row.kind == "fr")
            .expect("FlexRay rows reach the web trace");
        assert!(fr.addr.starts_with("slot "));
        assert!(fr.bus_name.starts_with("FR"));
        assert!(!fr.data.is_empty());
        assert!(
            rows.iter()
                .any(|row| row.kind == "fr" && !row.name.is_empty()),
            "at least one row resolves through its cluster description"
        );
    }

    /// FlexRay generator rows reach the web with their slot, sender and
    /// payload, so the Generator dialog can edit them like CAN entries.
    #[test]
    fn the_state_lists_flexray_generator_entries() {
        let mut app = App::headless();
        let bus = app
            .load_cluster_description(crate::test_assets::path("PowerTrain.arxml"), None)
            .expect("cluster loads");
        app.add_fr_tx(bus, 13);
        app.settle();
        let state = build_state(&app, &VecDeque::new(), 0.0);
        assert!(!state.fr_tx.is_empty(), "the FR entry is listed");
        let row = state.fr_tx.iter().find(|row| row.slot == 13).unwrap();
        assert_eq!(row.bus, bus);
        assert!(row.bus_name.starts_with("FR"));
        assert!(!row.node.is_empty(), "the description names the sender");
    }

    /// The signal catalog exposes the loaded FlexRay cluster's slots and
    /// signals, and a selected FR key keeps the `fr:` series identity.
    #[test]
    fn the_catalog_offers_flexray_signals() {
        let mut app = App::headless();
        let bus = app
            .load_cluster_description(crate::test_assets::path("PowerTrain.arxml"), None)
            .expect("cluster loads");
        let catalog = build_catalog(&app);
        let fr = catalog
            .channels
            .iter()
            .find(|channel| channel.kind == "fr" && channel.index == bus)
            .expect("FR catalog channel");
        assert!(fr.name.starts_with("FR"));
        let message = fr
            .messages
            .iter()
            .find(|message| message.id == 13)
            .expect("slot 13 message");
        let signal = message.signals.first().expect("a signal");
        assert!(signal.key.starts_with("fr:"));

        let mut store = SignalStore::default();
        let key = SigKey::fr(bus, 13, &signal.name);
        app.subscribe(key.clone());
        store.select_key(&app, key);
        store.sync(&app);
        let seeded = store.seed();
        assert!(seeded[0].key.starts_with("fr:"));
        assert_eq!(seeded[0].bus, fr.name);
    }

    /// Ethernet signals enter the same picker and observer tables. The
    /// catalog names the frame because a matrix has no numeric id, and the
    /// series key carries that name end to end.
    #[test]
    fn the_catalog_and_observers_offer_ethernet_signals() {
        use crate::workspace::PopupTarget;
        let mut app = App::headless();
        let bus = app
            .load_eth_description(crate::test_assets::eth_arxml(), Some(0))
            .expect("Ethernet matrix loads");
        let catalog = build_catalog(&app);
        let eth = catalog
            .channels
            .iter()
            .find(|channel| channel.kind == "eth" && channel.index == bus)
            .expect("Ethernet catalog channel");
        assert_eq!(eth.name, "ETH0");
        let message = eth.messages.first().expect("frame");
        assert_eq!(message.name, "EthFrame");
        assert_eq!(message.frame.as_deref(), Some("EthFrame"));
        let signal = message.signals.first().expect("signal");
        assert_eq!(signal.key, "eth:0:EthFrame:Sig");

        let key = SigKey::eth(bus, "EthFrame", "Sig");
        app.set_win_signal(PopupTarget::Monitor, key.clone(), true);
        app.new_data_window();
        app.set_win_signal(PopupTarget::Data(0), key.clone(), true);
        app.new_state_window();
        app.set_win_signal(PopupTarget::State(0), key.clone(), true);
        let state = build_state(&app, &std::collections::VecDeque::new(), 60.0);
        let monitor = state.monitor.first().expect("monitor row");
        assert_eq!(monitor.kind, "eth");
        assert_eq!(monitor.frame.as_deref(), Some("EthFrame"));
        assert_eq!(monitor.bus, "ETH0");
        assert!(!monitor.has_value, "no sample has arrived yet");
        assert_eq!(state.data[0].kind, "eth");
        assert_eq!(state.data[0].value, "-", "no samples yet, so dashes");
        assert_eq!(state.data[0].min, "-");
        assert_eq!(state.state_rows[0].kind, "eth");

        let mut store = SignalStore::default();
        store.select_key(&app, key.clone());
        store.sync(&app);
        assert_eq!(store.seed()[0].key, "eth:0:EthFrame:Sig");
        assert_eq!(store.seed()[0].bus, "ETH0");
    }

    /// The observer tables share one address shape: a FlexRay key keeps its
    /// cluster in `ch` and its slot in `id`, so Monitor/Data/State can list
    /// both bus kinds without a second set of DTOs.
    #[test]
    fn observers_list_flexray_rows_next_to_can() {
        use crate::workspace::PopupTarget;
        let mut app = App::headless();
        let bus = app
            .load_cluster_description(crate::test_assets::path("PowerTrain.arxml"), Some(0))
            .expect("cluster loads");
        let key = SigKey::fr(bus, 13, "CarSpeed");
        app.set_win_signal(PopupTarget::Monitor, key.clone(), true);
        app.new_data_window();
        app.set_win_signal(PopupTarget::Data(0), key.clone(), true);
        app.new_state_window();
        app.set_win_signal(PopupTarget::State(0), key.clone(), true);

        let state = build_state(&app, &std::collections::VecDeque::new(), 60.0);
        let monitor = state.monitor.first().expect("monitor row");
        assert_eq!(monitor.kind, "fr");
        assert_eq!((monitor.ch, monitor.id, monitor.ext), (bus, 13, false));
        assert_eq!(monitor.name, "CarSpeed");
        assert_eq!(monitor.bus, "FR0");
        let data = state.data.first().expect("data row");
        assert_eq!(data.kind, "fr");
        assert_eq!((data.ch, data.id), (bus, 13));
        let state_row = state.state_rows.first().expect("state row");
        assert_eq!(state_row.kind, "fr");
        assert_eq!((state_row.ch, state_row.id), (bus, 13));
    }

    #[test]
    fn uploads_keep_only_a_safe_file_name() {
        assert_eq!(upload_name("C:\\logs\\drive.asc"), "drive.asc");
        assert_eq!(upload_name("/tmp/project.rxproj"), "project.rxproj");
        assert_eq!(upload_name("a b:c?.dbc"), "a_b_c_.dbc");
        assert_eq!(upload_name(".."), "upload");
        assert_eq!(upload_name(""), "upload");
        assert_eq!(upload_name("\u{4e2d}\u{6587}.dbc"), "__.dbc");
    }

    #[test]
    fn bare_file_names_land_in_the_download_folder() {
        let folder = download_dir();
        let path = std::path::PathBuf::from(download_file_path("export.csv"));
        assert_eq!(path.parent(), Some(folder.as_path()));
        assert_eq!(
            path.file_name().and_then(|n| n.to_str()),
            Some("export.csv")
        );
        assert_eq!(download_file_path("logs/export.csv"), "logs/export.csv");
        assert_eq!(download_file_path(""), "");
        let nested = to_download_dir("../../secret.csv");
        assert_eq!(nested.parent(), Some(folder.as_path()));
        assert_eq!(
            nested.file_name().and_then(|n| n.to_str()),
            Some("secret.csv")
        );
    }

    #[test]
    fn flexray_rows_report_the_attached_port() {
        let mut app = App::headless();
        app.load_cluster_description(crate::test_assets::path("PowerTrain.arxml"), Some(0));
        app.hw.attach_fr_mock(0, 5);
        app.refresh_snapshot();
        let state = build_state(&app, &std::collections::VecDeque::new(), 60.0);
        let bus = state.fr_buses.first().expect("FR bus row");
        assert!(bus.attached);
        assert_eq!(bus.channel, 5);
    }

    #[test]
    fn replay_rows_merge_census_and_configured_channels() {
        let rows = replay_rows(&[(2, 5), (0, 9)], vec![(2, None), (4, Some(1))], |raw| {
            if raw == 2 { None } else { Some(1) }
        });
        assert_eq!(
            rows.iter()
                .map(|row| (row.raw, row.count, row.target))
                .collect::<Vec<_>>(),
            [(0, 9, Some(1)), (2, 5, None), (4, 0, Some(1))],
            "census and remembered rows share one sorted table"
        );
        assert_eq!(
            crate::replay_map::LogBusKind::parse("FlexRay"),
            Some(crate::replay_map::LogBusKind::Fr)
        );
        assert!(crate::replay_map::LogBusKind::parse("lin").is_none());
    }

    #[test]
    fn observer_commands_resolve_can_and_flexray_addresses() {
        assert_eq!(
            command_key("can", 1, 0x100, true, None, "EngineSpeed".to_string()),
            Some(SigKey::can(1, 0x100, true, "EngineSpeed"))
        );
        assert_eq!(
            command_key("fr", 2, 13, false, None, "CarSpeed".to_string()),
            Some(SigKey::fr(2, 13, "CarSpeed"))
        );
        assert_eq!(
            command_key(
                "eth",
                0,
                0,
                false,
                Some("EthFrame".to_string()),
                "Sig".to_string()
            ),
            Some(SigKey::eth(0, "EthFrame", "Sig"))
        );
        assert!(
            command_key("eth", 0, 0, false, None, "x".to_string()).is_none(),
            "an Ethernet address without a frame name is incomplete"
        );
        assert!(command_key("fr", 0, 70_000, false, None, "x".to_string()).is_none());
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
