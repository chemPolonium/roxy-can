
use std::collections::VecDeque;
use std::hash::{Hash, Hasher};
use std::sync::mpsc::{Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use axum::body::Bytes;
use axum::extract::{DefaultBodyLimit, Query, State};
use axum::http::header::{CACHE_CONTROL, CONTENT_DISPOSITION, CONTENT_TYPE};
use axum::http::{HeaderValue, StatusCode, Uri};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use rust_embed::RustEmbed;
use serde::{Deserialize, Serialize};
use std::io::IsTerminal;

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
/// Sample generator period; `ROXY_WEB_CYCLE_US` can shorten it for stress runs.
const SAMPLE_CYCLE_US: u64 = 50_000;
/// Browser uploads are buffered in memory before landing in the temp folder.
/// This covers DBC/ARXML/project files and ordinary logs; bigger recordings
/// still go through a typed host path.
const UPLOAD_LIMIT: usize = 256 * 1024 * 1024;

/// Sample curves: real DBC signals, driven by the generator's own value
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

const SAMPLE_SIGNALS: [SignalSpec; 8] = [
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
pub fn run(args: &[String], open: bool) -> Result<(), String> {
    let port = args
        .iter()
        .position(|arg| arg == "--port")
        .and_then(|i| args.get(i + 1))
        .and_then(|p| p.parse::<u16>().ok())
        .unwrap_or(PORT);
    let project = flag_value(args, "--project");
    let replay = flag_value(args, "--replay");
    let profile = flag_value(args, "--profile");
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
                    profile,
                )
            })
            .map_err(|error| format!("Could not start the core thread: {error}"))?;
    }

    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .map_err(|error| format!("Could not start the async runtime: {error}"))?;
    runtime.block_on(async move {
        let api = Router::new()
            .route("/state", get(state))
            .route("/catalog", get(catalog_handler))
            .route("/hardware", get(hardware_handler))
            .route("/fs/list", get(fs_list_handler))
            .route("/fs/mkdir", post(create_folder_handler))
            .route("/fs/rename", post(rename_folder_handler))
            .route("/fs/delete", post(delete_folder_handler))
            .route("/nodes", get(scripts_handler))
            .route("/script-help", get(script_help_handler))
            .route(
                "/debug",
                get(debug_view_handler).post(debug_command_handler),
            )
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
        let app = Router::new().nest("/api", api).fallback(static_file);
        let listener = tokio::net::TcpListener::bind(("127.0.0.1", port))
            .await
            .map_err(|error| {
                if error.kind() == std::io::ErrorKind::AddrInUse {
                    format!(
                        "Could not listen on 127.0.0.1:{port}: the port is already in use. \
                         Close the other program or pass --port <n>."
                    )
                } else {
                    format!("Could not listen on 127.0.0.1:{port}: {error}")
                }
            })?;
        let url = format!("http://127.0.0.1:{port}");
        println!("roxy-can {} is serving on {url}", env!("CARGO_PKG_VERSION"));
        let interactive = std::io::stdout().is_terminal();
        let elevated = process_is_elevated();
        if should_open_browser(open, elevated, interactive) {
            println!("Opening the roxy-can web UI in your default browser…");
            match open_browser(&url) {
                Ok(()) => println!("If it did not open, visit {url}. Press Ctrl+C to stop."),
                Err(error) => println!(
                    "Could not launch the browser ({error}). Open {url} yourself. \
                     Press Ctrl+C to stop."
                ),
            }
        } else if open {
            // Opening a browser is only safe from an ordinary interactive
            // console. An elevated process starts an elevated browser, and a
            // captured or sandboxed console (an agent run, `cargo run > log`)
            // breaks the browser's single-instance hand-off; both end in the
            // browser's "running elevated" prompt, so those runs print the
            // address instead.
            if elevated {
                println!(
                    "Running elevated: the browser is not opened automatically \
                     (it would start with administrator rights)."
                );
            } else {
                println!(
                    "Running without an interactive console: the browser is not \
                     opened automatically."
                );
            }
            println!("Open {url} in your browser. Press Ctrl+C to stop.");
        } else {
            println!("Open {url} in your browser. Press Ctrl+C to stop.");
        }
        axum::serve(listener, app)
            .with_graceful_shutdown(async {
                let _ = tokio::signal::ctrl_c().await;
            })
            .await
            .map_err(|error| format!("HTTP server stopped: {error}"))
    })
}

/// The Vite bundle lives inside the binary in release builds, so the exe can
/// be copied anywhere on its own.
#[derive(RustEmbed)]
#[folder = "web/dist"]
struct Assets;

/// Hashed assets never change under the same name; `index.html` must be
/// revalidated so an updated binary's new bundle is picked up.
async fn static_file(uri: Uri) -> Response {
    let requested = uri.path().trim_start_matches('/');
    let path = if requested.is_empty() {
        "index.html"
    } else {
        requested
    };
    let (served, file) = match Assets::get(path) {
        Some(file) => (path, file),
        None if !requested.contains('.') => match Assets::get("index.html") {
            Some(file) => ("index.html", file),
            None => return StatusCode::NOT_FOUND.into_response(),
        },
        None => return StatusCode::NOT_FOUND.into_response(),
    };
    let mime = mime_guess::from_path(served).first_or_octet_stream();
    let cache = if served == "index.html" {
        "no-cache"
    } else {
        "public, max-age=31536000, immutable"
    };
    (
        StatusCode::OK,
        [(CONTENT_TYPE, mime.as_ref()), (CACHE_CONTROL, cache)],
        file.data.into_owned(),
    )
        .into_response()
}

/// A double-click has no terminal to copy the URL from, so opening the browser
/// is how it reaches the UI. `--web` and `ROXY_WEB_NO_OPEN=1` keep scripted
/// runs silent.
fn open_browser(url: &str) -> std::io::Result<()> {
    #[cfg(target_os = "windows")]
    let mut command = {
        let mut command = std::process::Command::new("cmd");
        command.args(["/C", "start", "", url]);
        command
    };
    #[cfg(target_os = "macos")]
    let mut command = {
        let mut command = std::process::Command::new("open");
        command.arg(url);
        command
    };
    #[cfg(all(unix, not(target_os = "macos")))]
    let mut command = {
        let mut command = std::process::Command::new("xdg-open");
        command.arg(url);
        command
    };
    command.spawn().map(|_| ())
}

/// A bare launch opens the UI in the default browser, but only from an
/// ordinary interactive console: `--web` runs, elevated processes and
/// captured stdout (agent runs, redirected logs) print the address instead.
fn should_open_browser(open: bool, elevated: bool, interactive: bool) -> bool {
    open && !elevated && interactive
}

/// Whether this process holds an elevated token. Opening a browser from an
/// elevated process can start an elevated browser instance, and the user's
/// next normal launch then blocks on the browser's elevation-mismatch prompt
/// (Edge on Windows). A bare launch skips the auto-open in that case.
#[cfg(target_os = "windows")]
fn process_is_elevated() -> bool {
    use std::ffi::c_void;
    use windows_sys::Win32::Foundation::{CloseHandle, HANDLE};
    use windows_sys::Win32::Security::{
        GetTokenInformation, TOKEN_ELEVATION, TOKEN_QUERY, TokenElevation,
    };
    use windows_sys::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};

    let mut token: HANDLE = 0;
    // SAFETY: GetCurrentProcess returns a valid pseudo handle and the token
    // handle is closed before returning.
    unsafe {
        if OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) == 0 {
            return false;
        }
        let mut elevation = TOKEN_ELEVATION { TokenIsElevated: 0 };
        let mut size = 0u32;
        let ok = GetTokenInformation(
            token,
            TokenElevation,
            &mut elevation as *mut TOKEN_ELEVATION as *mut c_void,
            std::mem::size_of::<TOKEN_ELEVATION>() as u32,
            &mut size,
        );
        CloseHandle(token);
        ok != 0 && elevation.TokenIsElevated != 0
    }
}

#[cfg(not(target_os = "windows"))]
fn process_is_elevated() -> bool {
    false
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

/// Replaces the live session with a project file and rebuilds every derived
/// view (trace, chart selection, log state) the same way for the session
/// picker and the generated sample.
fn open_project_session(
    app: &mut App,
    path: &std::path::Path,
    signals: &Arc<Mutex<SignalStore>>,
    trace: &Arc<Mutex<TraceStore>>,
    series: &mut VecDeque<WebSeriesPoint>,
    rate_window: &mut VecDeque<(Instant, u64)>,
) {
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
    app.clear_message_expanded();
    app.open_project_path(path);
    // Same as the bootstrap load: applying the live layout is part of
    // opening the project, not an unsaved edit.
    app.mark_clean();
    if let Ok(mut store) = signals.lock() {
        seed_chart_selection(app, &mut store);
    }
    app.log_path.clear();
    app.log_info = None;
    app.send(BusCommand::SetRunMode(Mode::Virtual));
    reset_visual_stores(trace, series, rate_window);
}

/// Validates a system-variable definition coming from the web form. The core
/// applies the same rules; the host is the boundary, so the web path checks
/// them again instead of trusting the page.
#[allow(clippy::too_many_arguments)]
fn checked_sysvar(
    namespace: String,
    name: String,
    init: f64,
    min: Option<f64>,
    max: Option<f64>,
    unit: String,
    comment: String,
    label_sets: Vec<crate::bus::SysVarLabelSet>,
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
    let label_sets = checked_label_sets(label_sets)?;
    Ok(crate::bus::SysVarDef {
        namespace,
        name,
        init,
        min,
        max,
        unit: unit.trim().to_string(),
        comment: comment.trim().to_string(),
        label_sets,
        band_set_id: None,
        labels: Vec::new(),
    })
}

fn checked_label_sets(
    sets: Vec<crate::bus::SysVarLabelSet>,
) -> Result<Vec<crate::bus::SysVarLabelSet>, String> {
    if sets.len() > 8 {
        return Err("at most 8 label sets are allowed".into());
    }
    let mut seen_ids = std::collections::HashSet::new();
    let mut seen_names = std::collections::HashSet::new();
    let mut defaults = 0usize;
    let mut result = Vec::with_capacity(sets.len());
    for mut set in sets {
        set.id = set.id.trim().to_string();
        set.name = set.name.trim().to_string();
        if set.id.is_empty() || set.id.chars().count() > 64 {
            return Err("label set id must be 1–64 characters".into());
        }
        if set.name.is_empty() || set.name.chars().count() > 40 {
            return Err("label set name must be 1–40 characters".into());
        }
        if !seen_ids.insert(set.id.clone()) {
            return Err(format!("duplicate label set id \"{}\"", set.id));
        }
        if !seen_names.insert(set.name.clone()) {
            return Err(format!("duplicate label set name \"{}\"", set.name));
        }
        if set.bands.len() > 64 {
            return Err("at most 64 bands are allowed per set".into());
        }
        let mut bands = Vec::with_capacity(set.bands.len());
        for mut band in set.bands {
            if let Some(upper) = band.upper
                && !upper.is_finite()
            {
                return Err("band bounds must be finite numbers".into());
            }
            band.text = band.text.trim().to_string();
            if band.text.chars().count() > 80 {
                return Err("band text is limited to 80 characters".into());
            }
            if let Some(color) = band.color
                && color > 63
            {
                return Err("band color slot must be 0–63".into());
            }
            bands.push(band);
        }
        if bands.is_empty() {
            return Err(format!(
                "label set \"{}\" needs at least one band",
                set.name
            ));
        }
        if bands.last().is_some_and(|band| band.upper.is_some()) {
            return Err(format!(
                "label set \"{}\" needs an open-ended final band",
                set.name
            ));
        }
        bands.sort_by(|a, b| match (a.upper, b.upper) {
            (Some(left), Some(right)) => left
                .total_cmp(&right)
                .then_with(|| a.inclusive.cmp(&b.inclusive)),
            (Some(_), None) => std::cmp::Ordering::Less,
            (None, Some(_)) => std::cmp::Ordering::Greater,
            (None, None) => std::cmp::Ordering::Equal,
        });
        for pair in bands.windows(2) {
            if pair[0].upper == pair[1].upper && pair[0].inclusive == pair[1].inclusive {
                return Err(format!("duplicate band bound in set \"{}\"", set.name));
            }
        }
        let open = bands.iter().filter(|band| band.upper.is_none()).count();
        if open != 1 {
            return Err(format!(
                "label set \"{}\" needs exactly one open-ended band",
                set.name
            ));
        }
        set.bands = bands;
        if set.is_default {
            defaults += 1;
        }
        result.push(set);
    }
    if defaults > 1 {
        return Err("only one label set can be the default".into());
    }
    if defaults == 0
        && let Some(first) = result.first_mut()
    {
        first.is_default = true;
    }
    Ok(result)
}

/// Applies the editable parts of a Monitor row (not its signal). The row is
/// UI/project state, so the web edit lands in place; only adding/removing a
/// signal touches the bus.
fn edit_monitor_row(
    app: &mut App,
    key: &str,
    digits: Option<usize>,
    bands: Option<Vec<crate::band::Band>>,
) -> Result<(), String> {
    let Some(row) = app
        .monitor_view
        .rows
        .iter_mut()
        .find(|row| row.key.text() == key)
    else {
        return Err("monitor row not found".to_string());
    };
    if let Some(bands) = bands {
        if bands.is_empty() {
            return Err("monitor needs at least one band".to_string());
        }
        for band in &bands {
            if band.upper.is_some_and(|upper| !upper.is_finite()) {
                return Err("monitor bands must be finite".to_string());
            }
            if band.color.is_some_and(|color| color > 63) {
                return Err("monitor color slot must be 0–63".to_string());
            }
        }
        if bands.last().is_some_and(|band| band.upper.is_some()) {
            return Err("monitor needs an open-ended final band".to_string());
        }
        for pair in bands.windows(2) {
            match (pair[0].upper, pair[1].upper) {
                (Some(left), Some(right)) if right < left => {
                    return Err("monitor bands must be ordered".to_string());
                }
                (Some(left), Some(right))
                    if left == right && pair[0].inclusive == pair[1].inclusive =>
                {
                    return Err("monitor bands need unique bounds".to_string());
                }
                (Some(left), Some(right)) if left == right && pair[0].inclusive => {
                    return Err("at one bound, < must come before ≤".to_string());
                }
                _ => {}
            }
        }
        row.bands = bands;
    }
    if let Some(digits) = digits {
        row.digits = digits.min(3);
    }
    Ok(())
}

/// Six-significant-digit state identity, the same quantization the State
/// Tracker uses: tiny numeric jitter does not split a band.
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

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct WebScriptFunction {
    name: &'static str,
    usage: &'static str,
}

fn script_help_items() -> Vec<WebScriptFunction> {
    crate::script::SCRIPT_HELP
        .iter()
        .map(|(name, usage)| WebScriptFunction { name, usage })
        .collect()
}

/// The script editor's completion vocabulary. Names and call shapes live in
/// the language module, so a new builtin cannot be documented twice.
async fn script_help_handler() -> Json<Vec<WebScriptFunction>> {
    Json(script_help_items())
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct WebDebugValue {
    name: String,
    value: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct WebDebugFrame {
    function: String,
    line: u32,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct WebDebug {
    enabled: bool,
    node: Option<u64>,
    paused: bool,
    line: Option<u32>,
    function: Option<String>,
    breakpoints: Vec<u32>,
    locals: Vec<WebDebugValue>,
    globals: Vec<WebDebugValue>,
    stack: Vec<WebDebugFrame>,
    note: Option<String>,
}

impl From<crate::script::debug::DebugView> for WebDebug {
    fn from(view: crate::script::debug::DebugView) -> Self {
        Self {
            enabled: view.enabled,
            node: view.node,
            paused: view.paused,
            line: view.line,
            function: view.function,
            breakpoints: view.breakpoints,
            locals: view
                .locals
                .into_iter()
                .map(|value| WebDebugValue {
                    name: value.name,
                    value: value.value,
                })
                .collect(),
            globals: view
                .globals
                .into_iter()
                .map(|value| WebDebugValue {
                    name: value.name,
                    value: value.value,
                })
                .collect(),
            stack: view
                .stack
                .into_iter()
                .map(|frame| WebDebugFrame {
                    function: frame.function,
                    line: frame.line,
                })
                .collect(),
            note: view.note,
        }
    }
}

#[derive(Deserialize)]
#[serde(tag = "action", rename_all = "snake_case")]
enum WebDebugCommand {
    Arm { node_id: u64, lines: Vec<u32> },
    SetBreakpoints { lines: Vec<u32> },
    Continue,
    Step,
    Pause,
    Stop,
}

async fn debug_view_handler() -> Json<WebDebug> {
    Json(crate::script::debug::view().into())
}

async fn debug_command_handler(Json(command): Json<WebDebugCommand>) -> Json<WebDebug> {
    let view = match command {
        WebDebugCommand::Arm { node_id, lines } => crate::script::debug::arm(node_id, lines),
        WebDebugCommand::SetBreakpoints { lines } => crate::script::debug::set_breakpoints(lines),
        WebDebugCommand::Continue => crate::script::debug::resume(false),
        WebDebugCommand::Step => crate::script::debug::resume(true),
        WebDebugCommand::Pause => crate::script::debug::pause(),
        WebDebugCommand::Stop => crate::script::debug::stop(),
    };
    Json(view.into())
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
        if let Some(issue) = &node.issue {
            issue.line.hash(&mut hasher);
            issue.col.hash(&mut hasher);
            issue.message.hash(&mut hasher);
        } else {
            0u8.hash(&mut hasher);
        }
        node.attached.hash(&mut hasher);
        node.log.len().hash(&mut hasher);
        node.log.last().hash(&mut hasher);
        for test in &node.tests {
            test.name.hash(&mut hasher);
            test.runs.hash(&mut hasher);
            test.failures.hash(&mut hasher);
            test.line.hash(&mut hasher);
        }
        for name in &node.panel_actions {
            name.hash(&mut hasher);
        }
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
            issue: node.issue.as_ref().map(|issue| WebScriptIssue {
                line: issue.line,
                col: issue.col,
                message: issue.message.clone(),
            }),
            file_path: node.file_path.clone().unwrap_or_default(),
            source: node.source.clone(),
            tests: node
                .tests
                .iter()
                .map(|test| WebScriptTest {
                    name: test.name.clone(),
                    runs: test.runs,
                    failures: test.failures,
                    line: test.line,
                    time_s: test.time_s,
                })
                .collect(),
            log: node.log.clone(),
            panel_actions: node.panel_actions.clone(),
        })
        .collect()
}

/// Runs the core exporter for a web export request. Trace picks ASC or PCAP
/// by the path's extension; the others write CSV/TXT. Exporters target the
/// first matching observer window, which starts as an All-buses view.
/// Adopts a browser workspace only if it belongs to the current project
/// revision; a debounce from the previous project is dropped.
fn set_web_workspace(app: &mut App, rev: u64, workspace: serde_json::Value) {
    if rev == app.workspace_rev {
        app.web_workspace = Some(workspace);
    }
}

fn export_file(app: &mut App, kind: &str, path: &str, range: Option<(u64, u64)>) {
    if path.trim().is_empty() {
        app.fail("export needs a path");
        return;
    }
    match kind {
        "trace" => match range {
            Some((from, to)) => app.export_trace_range(0, path, from, to),
            None => app.export_trace(0, path),
        },
        "messages" => app.export_messages_csv(0, path),
        "stats" => app.export_stats_csv(0, path),
        "write" => app.export_write_txt(path),
        "data" => app.export_data_csv(0, path),
        "state" => app.export_state_csv(0, path),
        "graphics" => app.export_graphics_csv(0, path),
        "spec" => app.export_spec_csv(path),
        other => app.fail(format!("unknown export kind: {other}")),
    }
}

/// Seeds the chart from a project's first Graphics window. The project file
/// persists curve selection there; the web keeps its own lane metadata but
/// must not start empty when the same project is opened.
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
        store.select_key(app, key.clone());
        app.subscribe(key);
    }
}

/// Selecting a chart lane also records it on the first Graphics window, so
/// project save/load carries the selection. The panel stays the source of
/// truth for lane colors; `store` is updated by the caller.
fn checked_band_set(
    id: String,
    name: String,
    bands: Vec<crate::band::Band>,
) -> Result<crate::band::BandSet, String> {
    let id = id.trim().to_string();
    let name = name.trim().to_string();
    if id.is_empty() || id.chars().count() > 64 {
        return Err("band set id must be 1–64 characters".into());
    }
    if name.is_empty() || name.chars().count() > 40 {
        return Err("band set name must be 1–40 characters".into());
    }
    let mut bands = bands;
    if bands.is_empty() || bands.len() > 64 {
        return Err("band set needs 1–64 bands".into());
    }
    for band in &mut bands {
        if band.upper.is_some_and(|upper| !upper.is_finite()) {
            return Err("band bounds must be finite".into());
        }
        band.text = band.text.trim().to_string();
        if band.text.chars().count() > 80 {
            return Err("band text is limited to 80 characters".into());
        }
        if band.color.is_some_and(|color| color > 63) {
            return Err("band color slot must be 0–63".into());
        }
    }
    bands.sort_by(|a, b| match (a.upper, b.upper) {
        (Some(left), Some(right)) => left
            .total_cmp(&right)
            .then_with(|| a.inclusive.cmp(&b.inclusive)),
        (Some(_), None) => std::cmp::Ordering::Less,
        (None, Some(_)) => std::cmp::Ordering::Greater,
        (None, None) => std::cmp::Ordering::Equal,
    });
    if bands.iter().filter(|band| band.upper.is_none()).count() != 1 {
        return Err("band set needs exactly one open-ended final band".into());
    }
    if bands.last().is_some_and(|band| band.upper.is_some()) {
        return Err("the open-ended band must be last".into());
    }
    if bands
        .windows(2)
        .any(|pair| pair[0].upper == pair[1].upper && pair[0].inclusive == pair[1].inclusive)
    {
        return Err("band set needs unique bound/mode pairs".into());
    }
    Ok(crate::band::BandSet { id, name, bands })
}

#[cfg(test)]
fn chart_select(app: &mut App, ch: u8, id: u32, ext: bool, name: &str) {
    let key = app.can_key(ch, id, ext, name.to_string());
    chart_select_key(app, key);
}

/// Refreshes the core's chart-keep set from the session selection. Called
/// just before an observer drops a row: the chart owns that subscription even
/// when the lane never entered the project's Graphics window (a session-only
/// web pick), so pruning must not unmount it.
fn sync_chart_keep(app: &mut App, signals: &std::sync::Mutex<SignalStore>) {
    if let Ok(store) = signals.lock() {
        app.set_session_chart_keep(store.keys());
    }
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
    // Session-only web lanes may not live in a Graphics window; prune still
    // drops the subscription unless another observer (Monitor/Data/State) uses
    // it.
    app.prune_signal(key);
}

/// Selects a signal as a plotted lane or a cursor-only read signal. Both
/// keep the bus subscription; only the lane state differs. Switching a
/// plotted signal to read removes its Graphics row but the refreshed chart
/// keep set protects the subscription from the prune.
fn set_signal_mode(app: &mut App, store: &mut SignalStore, key: SigKey, read: bool) {
    let known = store.is_selected(&key);
    if known && store.is_read(&key) == read {
        return;
    }
    if !known {
        store.select_key_mode(app, key.clone(), read);
        if read {
            app.subscribe(key);
        } else {
            chart_select_key(app, key);
        }
        app.set_session_chart_keep(store.keys());
        return;
    }
    store.select_key_mode(app, key.clone(), read);
    if read {
        app.set_session_chart_keep(store.keys());
        chart_unselect(app, &key);
    } else {
        chart_select_key(app, key);
        app.set_session_chart_keep(store.keys());
    }
}

/// Applies a Signals context-menu action. Watching only adds a reader;
/// removing a chart lane leaves any observer that already watches it alone.
fn chart_signal_action(app: &mut App, store: &mut SignalStore, key: &SigKey, action: &str) -> bool {
    match action {
        "monitor" => app.set_win_signal(crate::workspace::PopupTarget::Monitor, key.clone(), true),
        "data" => {
            if app.data_windows.is_empty() {
                app.new_data_window();
            }
            app.set_win_signal(crate::workspace::PopupTarget::Data(0), key.clone(), true);
        }
        "state" => {
            if app.state_trackers.is_empty() {
                app.new_state_window();
            }
            app.set_win_signal(crate::workspace::PopupTarget::State(0), key.clone(), true);
        }
        "remove" => {
            store.unselect(key);
            app.set_session_chart_keep(store.keys());
            chart_unselect(app, key);
        }
        _ => return false,
    }
    true
}

/// Moves a chart lane in both the session cache and the first Graphics
/// window. The window order is what a project save persists, while the cache
/// order is what the web UI draws.
fn chart_move(app: &mut App, store: &mut SignalStore, key: &SigKey, to: usize) {
    if !store.move_selected(key, to) {
        return;
    }
    let order = store.chart_keys();
    let Some(window) = app.graphics.first_mut() else {
        return;
    };
    let Some(position) = window.signals.iter().position(|signal| &signal.key == key) else {
        return;
    };
    let moved = window.signals.remove(position);
    // Insert relative to the selected neighbours, leaving hidden rows in
    // place around the reordered web lanes.
    let insert_at = order
        .get(to + 1)
        .and_then(|next| window.signals.iter().position(|signal| &signal.key == next))
        .or_else(|| {
            to.checked_sub(1)
                .and_then(|previous| order.get(previous))
                .and_then(|previous| {
                    window
                        .signals
                        .iter()
                        .position(|signal| &signal.key == previous)
                        .map(|index| index + 1)
                })
        })
        .unwrap_or(window.signals.len());
    window.signals.insert(insert_at, moved);
}

/// Replaces a State row's custom value-definition bands. Cuts are sorted
/// and deduplicated; names are padded so every band has a label.
fn set_state_rule(
    app: &mut App,
    key: &SigKey,
    bands: Vec<crate::band::Band>,
    band_set_id: Option<String>,
) {
    let Some(window) = app.state_trackers.first_mut() else {
        app.fail("state rule needs a State Tracker window");
        return;
    };
    window.rules.insert(
        key.clone(),
        crate::observe::StateRule { bands, band_set_id },
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

/// The CAN section of Bus Statistics, one row per statistic. The reference's
/// hardware-only rows have no meaning here.
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
        rule.label(band)
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
                    label: rule.label(open_band),
                    color: rule.color(open_band),
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
            label: rule.label(open_band),
            color: rule.color(open_band),
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

/// Record-filter parser for the web form: CAN hex ids (an `x` suffix marks
/// extended) and `FRn:slot` entries, comma/space separated.
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
        TriggerCond::DerivedSignal { .. } => "derived",
        TriggerCond::PayloadMatch { .. } => "payload",
        TriggerCond::EthFlow { .. } => "eth_flow",
        TriggerCond::FrTimeout { .. } => "fr_timeout",
        TriggerCond::E2eIssue { .. } => "e2e",
        TriggerCond::SomeIp { .. } => "someip",
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
        "derived" => Some(TriggerKind::Derived),
        "payload" => Some(TriggerKind::Payload),
        "eth_flow" => Some(TriggerKind::EthFlow),
        "fr_timeout" => Some(TriggerKind::FrTimeout),
        "e2e" => Some(TriggerKind::E2e),
        "someip" => Some(TriggerKind::SomeIp),
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

/// Bring up the requested session: a saved project, a replay log, or an
/// empty workspace. A project without a log opens stopped; a log starts
/// playback; no flags starts with no buses and no measurement.
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
        if project.is_some() {
            app.mark_clean();
        }
        return;
    }
    if project.is_some() {
        // Loading applies the project's live layout, so a fresh open has
        // nothing unsaved.
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
    let (eth_channels, eth_error) = match crate::hw::Hardware::eth_interfaces() {
        Ok(list) => (
            list.into_iter()
                .map(|iface| WebEthChannel {
                    name: if iface.description.is_empty() {
                        iface.device.clone()
                    } else {
                        format!("{} · {}", iface.device, iface.description)
                    },
                    device: iface.device,
                })
                .collect(),
            String::new(),
        ),
        Err(error) => (Vec::new(), error),
    };
    let data = WebHardware {
        channels,
        fr_channels,
        eth_channels,
        error,
        fr_error,
        eth_error,
    };
    if let Ok(mut cache) = api.hardware.lock() {
        cache.generation = generation;
        cache.data = Some(data.clone());
    }
    Json(data)
}

#[derive(Default, Deserialize)]
struct FsListQuery {
    path: Option<String>,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct FsRoot {
    name: String,
    path: String,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct FsEntry {
    name: String,
    path: String,
    dir: bool,
    size: u64,
    modified_ms: u64,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct FsList {
    path: String,
    parent: Option<String>,
    roots: Vec<FsRoot>,
    entries: Vec<FsEntry>,
    truncated: bool,
}

fn clean_path(path: &std::path::Path) -> String {
    let text = path.to_string_lossy();
    text.strip_prefix("\\\\?\\").unwrap_or(&text).to_string()
}

fn fs_roots() -> Vec<FsRoot> {
    let mut roots = Vec::new();
    let mut seen = std::collections::HashSet::new();
    let mut push = |name: &str, path: std::path::PathBuf| {
        let path = clean_path(&path);
        if seen.insert(path.to_ascii_lowercase()) {
            roots.push(FsRoot {
                name: name.to_string(),
                path,
            });
        }
    };
    if let Some(home) = std::env::var_os("USERPROFILE").or_else(|| std::env::var_os("HOME")) {
        push("Home", std::path::PathBuf::from(home));
    }
    if let Ok(cwd) = std::env::current_dir() {
        push("Working folder", cwd);
    }
    #[cfg(windows)]
    {
        for letter in b'C'..=b'Z' {
            let root = format!("{}:\\", letter as char);
            if std::path::Path::new(&root).exists() {
                push(
                    &format!("{}:", letter as char),
                    std::path::PathBuf::from(root),
                );
            }
        }
    }
    #[cfg(not(windows))]
    {
        push("/", std::path::PathBuf::from("/"));
    }
    roots
}

fn list_directory(requested: Option<&str>) -> Result<FsList, String> {
    let requested = requested
        .map(str::trim)
        .filter(|path| !path.is_empty())
        .map(std::path::PathBuf::from)
        .or_else(|| {
            std::env::var_os("USERPROFILE")
                .or_else(|| std::env::var_os("HOME"))
                .map(std::path::PathBuf::from)
        })
        .or_else(|| std::env::current_dir().ok())
        .ok_or_else(|| "no folder to list".to_string())?;
    let current = std::fs::canonicalize(&requested)
        .map_err(|error| format!("cannot open {}: {error}", clean_path(&requested)))?;
    if !current.is_dir() {
        return Err(format!("not a folder: {}", clean_path(&current)));
    }
    let mut entries = Vec::new();
    let mut truncated = false;
    if let Ok(read) = std::fs::read_dir(&current) {
        for entry in read.flatten() {
            if entries.len() >= 2000 {
                truncated = true;
                break;
            }
            let Ok(meta) = entry.metadata() else {
                continue;
            };
            let name = entry.file_name().to_string_lossy().into_owned();
            entries.push(FsEntry {
                name,
                path: clean_path(&entry.path()),
                dir: meta.is_dir(),
                size: if meta.is_file() { meta.len() } else { 0 },
                modified_ms: meta
                    .modified()
                    .ok()
                    .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
                    .map(|duration| duration.as_millis() as u64)
                    .unwrap_or(0),
            });
        }
    }
    entries.sort_by(|a, b| {
        b.dir.cmp(&a.dir).then_with(|| {
            a.name
                .to_ascii_lowercase()
                .cmp(&b.name.to_ascii_lowercase())
        })
    });
    Ok(FsList {
        path: clean_path(&current),
        parent: current.parent().map(clean_path),
        roots: fs_roots(),
        entries,
        truncated,
    })
}

async fn fs_list_handler(
    Query(query): Query<FsListQuery>,
) -> Result<Json<FsList>, (StatusCode, String)> {
    list_directory(query.path.as_deref())
        .map(Json)
        .map_err(|error| (StatusCode::BAD_REQUEST, error))
}

#[derive(Deserialize)]
struct CreateFolderRequest {
    path: String,
    name: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct NewFolder {
    path: String,
    name: String,
}

fn valid_folder_name(name: &str) -> Result<&str, String> {
    let name = name.trim();
    if name.is_empty() {
        return Err("folder name is empty".to_string());
    }
    if name == "." || name == ".." || name.ends_with(' ') || name.ends_with('.') {
        return Err("folder name is not valid".to_string());
    }
    if name.chars().any(|ch| {
        ch.is_control() || matches!(ch, '/' | '\\' | '<' | '>' | ':' | '"' | '|' | '?' | '*')
    }) {
        return Err("folder name contains an invalid character".to_string());
    }
    Ok(name)
}

fn create_folder(parent: &str, name: &str) -> Result<NewFolder, String> {
    let name = valid_folder_name(name)?;
    let parent = std::fs::canonicalize(parent).map_err(|error| {
        format!(
            "cannot open {}: {error}",
            clean_path(std::path::Path::new(parent))
        )
    })?;
    if !parent.is_dir() {
        return Err(format!("not a folder: {}", clean_path(&parent)));
    }
    let path = parent.join(name);
    if path.exists() {
        return Err(format!("folder already exists: {}", clean_path(&path)));
    }
    std::fs::create_dir(&path).map_err(|error| format!("cannot create folder: {error}"))?;
    Ok(NewFolder {
        path: clean_path(&path),
        name: name.to_string(),
    })
}

async fn create_folder_handler(
    Json(request): Json<CreateFolderRequest>,
) -> Result<Json<NewFolder>, (StatusCode, String)> {
    create_folder(&request.path, &request.name)
        .map(Json)
        .map_err(|error| (StatusCode::BAD_REQUEST, error))
}

#[derive(Deserialize)]
struct RenameFolderRequest {
    path: String,
    name: String,
}

#[derive(Deserialize)]
struct DeleteFolderRequest {
    path: String,
}

fn rename_folder(path: &str, name: &str) -> Result<NewFolder, String> {
    let name = valid_folder_name(name)?;
    let path =
        std::fs::canonicalize(path).map_err(|error| format!("cannot open {path}: {error}"))?;
    if !path.is_dir() {
        return Err(format!("not a folder: {}", clean_path(&path)));
    }
    let parent = path
        .parent()
        .ok_or_else(|| "cannot rename a root folder".to_string())?;
    let target = parent.join(name);
    if target == path {
        return Ok(NewFolder {
            path: clean_path(&path),
            name: name.to_string(),
        });
    }
    if target.exists() {
        return Err(format!("folder already exists: {}", clean_path(&target)));
    }
    std::fs::rename(&path, &target).map_err(|error| format!("cannot rename folder: {error}"))?;
    Ok(NewFolder {
        path: clean_path(&target),
        name: name.to_string(),
    })
}

fn delete_folder(path: &str) -> Result<(), String> {
    let path =
        std::fs::canonicalize(path).map_err(|error| format!("cannot open {path}: {error}"))?;
    if !path.is_dir() {
        return Err(format!("not a folder: {}", clean_path(&path)));
    }
    if path.parent().is_none() {
        return Err("cannot delete a root folder".to_string());
    }
    match std::fs::remove_dir(&path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::DirectoryNotEmpty => {
            Err("folder is not empty; remove its contents first".to_string())
        }
        Err(error) => Err(format!("cannot delete folder: {error}")),
    }
}

async fn rename_folder_handler(
    Json(request): Json<RenameFolderRequest>,
) -> Result<Json<NewFolder>, (StatusCode, String)> {
    rename_folder(&request.path, &request.name)
        .map(Json)
        .map_err(|error| (StatusCode::BAD_REQUEST, error))
}

async fn delete_folder_handler(
    Json(request): Json<DeleteFolderRequest>,
) -> Result<StatusCode, (StatusCode, String)> {
    delete_folder(&request.path)
        .map(|()| StatusCode::NO_CONTENT)
        .map_err(|error| (StatusCode::BAD_REQUEST, error))
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
async fn recording_download(
    State(api): State<ApiState>,
    Query(query): Query<std::collections::HashMap<String, String>>,
) -> Result<Response, (StatusCode, String)> {
    let want_eth = query.get("eth").is_some_and(|value| value == "1");
    let (path, eth_path, recording) = api
        .shared
        .lock()
        .map(|state| {
            (
                state.record_file.clone(),
                state.record_eth_file.clone(),
                state.recording,
            )
        })
        .unwrap_or_default();
    let path = if want_eth && !eth_path.is_empty() {
        eth_path
    } else {
        path
    };
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

fn default_true() -> bool {
    true
}

fn default_mask() -> u8 {
    0xFF
}

fn default_someip_port() -> u16 {
    30490
}

/// Where one log channel should replay. `"same"` removes the table entry
/// instead of pinning today's bus, so a project with no deliberate mapping
/// stays portable; `"drop"` is an explicit no-replay that survives a reopen.
#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
enum ReplayChannelChoice {
    Word(String),
    Bus(u8),
}

#[derive(Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
#[allow(clippy::large_enum_variant)]
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
        target: ReplayChannelChoice,
    },
    RecordToggle {
        name: Option<String>,
        format: Option<u8>,
    },
    SetRecordSettings {
        name: String,
        format: u8,
    },
    SetRecordFilter {
        text: String,
    },
    ClearTrace,
    /// Reset the Messages counters without dropping the Trace ring.
    ClearMessages,
    /// Open or close every Messages row's signal children.
    SetAllMessagesExpanded {
        on: bool,
    },
    /// Open or close one Messages row's signal children.
    SetMessageExpanded {
        key: String,
        on: bool,
    },
    /// Fire `on panel_action` handlers by name on the next measurement step.
    /// `value` is handed to a handler that declares a parameter.
    RunPanelAction {
        name: String,
        #[serde(default)]
        value: Option<f64>,
    },
    /// Write one DBC signal from a Panel setpoint. `source` is the observer
    /// key `ch:id:signal` the catalog hands out.
    PanelWriteSignal {
        source: String,
        value: f64,
    },
    /// Send one raw CAN frame once from a Panel button. `physical` encodes
    /// `signals` against the catalog message instead; `data` is ignored then.
    PanelSendFrame {
        channel: u8,
        id: u32,
        extended: bool,
        data: Vec<u8>,
        #[serde(default)]
        physical: bool,
        #[serde(default)]
        signals: std::collections::BTreeMap<String, f64>,
    },
    PanelSendFrFrame {
        bus: u8,
        slot: u16,
        data: Vec<u8>,
        #[serde(default)]
        physical: bool,
        #[serde(default)]
        signals: std::collections::BTreeMap<String, f64>,
    },
    PanelSendEthFrame {
        bus: u8,
        frame: String,
        data: Vec<u8>,
        #[serde(default)]
        physical: bool,
        #[serde(default)]
        signals: std::collections::BTreeMap<String, f64>,
    },
    PanelSendSomeIp {
        bus: u8,
        service: u16,
        method: u16,
        client: u16,
        session: u16,
        message_type: u8,
        data: Vec<u8>,
        #[serde(default)]
        values: std::collections::BTreeMap<String, f64>,
        #[serde(default)]
        texts: std::collections::BTreeMap<String, String>,
        #[serde(default)]
        transport: String,
        #[serde(default)]
        host: String,
        #[serde(default = "default_someip_port")]
        port: u16,
    },
    OpenProject {
        path: String,
    },
    CreateSampleProject {
        parent: String,
    },
    SetRunMode {
        mode: String,
    },
    OpenLog {
        path: String,
        /// Load and start playback; false parks the log at its beginning.
        #[serde(default = "default_true")]
        play: bool,
    },
    AddReplayFolder {
        path: String,
    },
    AddReplayFile {
        path: String,
    },
    RemoveReplaySource {
        path: String,
        folder: bool,
    },
    RefreshReplaySources,
    SelectSignal {
        ch: u8,
        id: u32,
        ext: bool,
        name: String,
        /// Read-only selection: subscribe and show at cursors, no lane.
        #[serde(default)]
        read: bool,
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
        #[serde(default)]
        read: bool,
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
        #[serde(default)]
        read: bool,
    },
    UnselectEthSignal {
        bus: u8,
        frame: String,
        name: String,
    },
    SelectDerivedSignal {
        id: String,
        name: String,
        #[serde(default)]
        read: bool,
    },
    UnselectDerivedSignal {
        id: String,
        name: String,
    },
    MoveSignal {
        key: String,
        to: usize,
    },
    ChartSignal {
        key: String,
        action: String,
    },
    AttachHardware {
        bus: u8,
        driver: String,
        index: i32,
    },
    DetachHardware {
        bus: u8,
    },
    AttachEthPort {
        bus: u8,
        device: String,
    },
    DetachEthPort {
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
    AddBus {
        #[serde(default)]
        kind: String,
    },
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
    RemoveFrBus {
        bus: u8,
    },
    RemoveEthBus {
        bus: u8,
    },
    /// Lists the clusters of an uploaded ARXML so the UI can ask which one
    /// belongs to the bus the user picked.
    ScanArxml {
        path: String,
    },
    /// Binds one description to a bus. `kind` is `can`, `flexray` or
    /// `ethernet`; `path` is a DBC/ARXML/FIBEX file. A DBC is `kind: can`
    /// with a `.dbc` path.
    AddBusDescription {
        bus: u8,
        kind: String,
        path: String,
        #[serde(default)]
        cluster: String,
    },
    RemoveBusDescription {
        bus: u8,
        kind: String,
        path: String,
        #[serde(default)]
        cluster: String,
    },
    SaveProject {
        path: Option<String>,
        /// Panel pages to store inside the project, in `.rxpanel` format.
        #[serde(default)]
        panels: Vec<serde_json::Value>,
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
        /// Optional trace interval, in microseconds; ignored by non-trace kinds.
        #[serde(default)]
        from_us: Option<u64>,
        #[serde(default)]
        to_us: Option<u64>,
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
        #[serde(default)]
        label_sets: Vec<crate::bus::SysVarLabelSet>,
        #[serde(default)]
        band_set_id: Option<String>,
        old_namespace: Option<String>,
        old_name: Option<String>,
    },
    DeleteSysVar {
        namespace: String,
        name: String,
    },
    DefineBandSet {
        id: String,
        name: String,
        bands: Vec<crate::band::Band>,
        old_id: Option<String>,
    },
    DeleteBandSet {
        id: String,
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
    /// Edit one Monitor row: decimals and optional value bands. `key` is the
    /// canonical `sig:` key; omitted fields keep their current value.
    ConfigureMonitor {
        key: String,
        digits: Option<usize>,
        bands: Option<Vec<crate::band::Band>>,
        #[serde(default)]
        band_set_id: Option<String>,
    },
    /// One or two Monitor rows per line.
    SetMonitorColumns {
        columns: u8,
    },
    /// Reorder Monitor rows: the `sig:` key moves to slot `to` (index in the
    /// post-removal list, clamped by the core).
    MoveMonitorRow {
        key: String,
        to: usize,
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
    /// Reorder the first Data window's rows, same semantics as the Monitor
    /// move.
    MoveDataRow {
        key: String,
        to: usize,
    },
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
        bands: Vec<crate::band::Band>,
        #[serde(default)]
        band_set_id: Option<String>,
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
    SetEntryBrs {
        ch: u8,
        id: u32,
        brs: bool,
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
    AddReplayBlock {
        channel: u8,
        name: String,
        path: String,
        node: Option<String>,
    },
    SetReplayBlock {
        id: u64,
        channel: u8,
        name: String,
        path: String,
        node: Option<String>,
        ids: Vec<(u32, bool)>,
    },
    SetReplayBlockEnabled {
        id: u64,
        on: bool,
    },
    RemoveReplayBlock {
        id: u64,
    },
    /// Clone replay block `id` as a new disabled "… copy".
    DuplicateReplayBlock {
        id: u64,
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
        /// Optional seed fields from the Trace/chart context menus.
        #[serde(default)]
        ch: Option<u8>,
        #[serde(default)]
        id: Option<u32>,
        #[serde(default)]
        bus: Option<u8>,
        #[serde(default)]
        slot: Option<u16>,
        /// `SigKey::text()` for a chart-lane signal; the backend resolves
        /// the cluster to a bus index.
        #[serde(default)]
        signal_key: Option<String>,
    },
    SetTriggerEnabled {
        index: usize,
        on: bool,
    },
    RemoveTrigger {
        index: usize,
    },
    /// Clone rule `index` onto the end; the copy starts disarmed.
    DuplicateTrigger {
        index: usize,
    },
    RearmTriggers,
    RearmTrigger {
        index: usize,
    },
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
        derived_id: String,
        action: String,
        send_ch: u8,
        send_id: u32,
        #[serde(default)]
        sys_var_key: String,
        #[serde(default)]
        sys_var_value: Option<f64>,
        #[serde(default)]
        script_name: String,
        #[serde(default)]
        script_value: Option<f64>,
        #[serde(default)]
        send_frame: String,
        #[serde(default)]
        send_data: String,
        #[serde(default)]
        someip_method: u16,
        #[serde(default)]
        someip_client: u16,
        #[serde(default)]
        someip_session: u16,
        #[serde(default)]
        someip_type: u8,
        /// SOME/IP condition filters; absent = wildcard.
        #[serde(default)]
        cond_someip_method: Option<u16>,
        #[serde(default)]
        cond_someip_type: Option<u8>,
        #[serde(default)]
        someip_values: std::collections::BTreeMap<String, f64>,
        #[serde(default)]
        someip_transport: String,
        #[serde(default)]
        someip_host: String,
        #[serde(default = "default_someip_port")]
        someip_port: u16,
        #[serde(default)]
        offset: u8,
        #[serde(default = "default_mask")]
        mask: u8,
        #[serde(default)]
        value: u8,
        #[serde(default)]
        vlan: Option<u16>,
        #[serde(default)]
        cooldown_ms: u32,
        #[serde(default)]
        max_fires: u32,
        #[serde(default)]
        pre_frames: Option<u32>,
        #[serde(default)]
        post_frames: Option<u32>,
    },
    SetTriggerLimits {
        pre_frames: usize,
        post_frames: u32,
        marker_cap: usize,
    },
    /// Adopt the browser's workspace for the current project revision. A
    /// late debounce from the previous project carries its old revision and
    /// is dropped instead of overwriting the newly loaded file.
    SetWebWorkspace {
        rev: u64,
        workspace: serde_json::Value,
    },
    /// Decode another ASC/BLF with the current descriptions and keep its
    /// selected CAN, FlexRay and Ethernet signals as a comparison baseline.
    LoadReference {
        path: String,
    },
    ClearReference,
    /// Replace the derived-channel definitions. The core samples the curves;
    /// the snapshot carries the definitions back to the editor.
    SetDerived {
        defs: Vec<crate::derived::DerivedDef>,
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

