
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

