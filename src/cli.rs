//! The headless command line: replay a CAN log with no window, record it
//! to a fresh file, dump the message statistics, or syntax-check node
//! scripts -- the same bus core the GUI drives, hand-cranked against the
//! wall clock on the manual drive.

use crate::app::App;

/// What the command line asks for.
#[derive(Debug)]
pub enum Cli {
    /// No headless flags: hand the session to the workspace window.
    Gui,
    /// Print this text and exit successfully.
    Help(String),
    /// Run the replay headless.
    Run(CliOpts),
    /// Compile node scripts and report; exit non-zero on any failure.
    CheckScripts(Vec<String>),
}

#[derive(Debug)]
pub struct CliOpts {
    /// The log to replay, when `--replay` was given.
    pub replay: Option<String>,
    /// The project to simulate, when `--project` was given.
    pub project: Option<String>,
    /// Profile name under the project's `profiles/` directory, when
    /// `--profile` was given: role overrides layered on the project.
    pub profile: Option<String>,
    pub speed: f64,
    /// Wall-clock seconds to run before stopping; the whole log when None
    /// (replay only -- a project simulation must be bounded).
    pub duration_s: Option<f64>,
    /// Write the message-statistics CSV here when the run ends.
    pub stats_csv: Option<String>,
}

pub fn usage() -> &'static str {
    "roxy-can -- CAN bus analysis

  roxy-can                       open the workspace window (default)
  roxy-can --replay <log> ...    replay a CAN log without any window
  roxy-can --project <p> ...     simulate a saved project without any window
  roxy-can --check-script <f>    compile node scripts, no window needed
                                 (repeat the flag for more files; non-zero
                                 exit when any script fails)

run options
  --replay <path>    log to replay (.asc or .blf)
  --project <path>   project (.rxproj) to simulate: generators and script
                     nodes run on the virtual bus (needs --duration)
  --profile <name>   overlay profiles/<name>.toml from the project
                     directory onto the project's node roles (needs
                     --project); every unknown bus/node/role is an error
  --speed <n>        playback rate, 1.0 = real time (replay only, default 1.0)
  --duration <s>     stop after this many wall-clock seconds
                     (replay default: run to the end of the log;
                      project: required, a simulation has no end)
  --stats <path>     write the message-statistics CSV when the run ends
  -h, --help         this text"
}

pub fn parse_args(args: &[String]) -> Result<Cli, String> {
    if args.is_empty() {
        return Ok(Cli::Gui);
    }
    if args.iter().any(|a| a == "-h" || a == "--help") {
        return Ok(Cli::Help(usage().to_string()));
    }
    let mut replay = None;
    let mut project = None;
    let mut profile = None;
    let mut speed = 1.0f64;
    let mut duration_s = None;
    let mut stats_csv = None;
    let mut scripts = Vec::new();
    let mut i = 0;
    while i < args.len() {
        // Reads the value after `flag`, refusing an empty or missing one.
        fn value(args: &[String], i: &mut usize, name: &str) -> Result<String, String> {
            *i += 1;
            args.get(*i)
                .filter(|s| !s.is_empty())
                .cloned()
                .ok_or_else(|| format!("{name} needs a value"))
        }
        match args[i].as_str() {
            "--replay" => replay = Some(value(args, &mut i, "--replay")?),
            "--project" => project = Some(value(args, &mut i, "--project")?),
            "--profile" => profile = Some(value(args, &mut i, "--profile")?),
            "--check-script" => scripts.push(value(args, &mut i, "--check-script")?),
            "--speed" => {
                let raw = value(args, &mut i, "--speed")?;
                speed = raw
                    .parse()
                    .map_err(|_| format!("--speed wants a number, got `{raw}`"))?;
                if !speed.is_finite() || speed <= 0.0 {
                    return Err("--speed must be a positive number".to_string());
                }
            }
            "--duration" => {
                let raw = value(args, &mut i, "--duration")?;
                let d: f64 = raw
                    .parse()
                    .map_err(|_| format!("--duration wants a number of seconds, got `{raw}`"))?;
                if !d.is_finite() || d <= 0.0 {
                    return Err("--duration must be a positive number of seconds".to_string());
                }
                duration_s = Some(d);
            }
            "--stats" => stats_csv = Some(value(args, &mut i, "--stats")?),
            other => return Err(format!("unknown flag `{other}`")),
        }
        i += 1;
    }
    if !scripts.is_empty() {
        if replay.is_some() || project.is_some() {
            return Err(
                "`--check-script` runs headless on its own; drop `--replay`/`--project`"
                    .to_string(),
            );
        }
        return Ok(Cli::CheckScripts(scripts));
    }
    if replay.is_some() && project.is_some() {
        return Err("`--replay` and `--project` are mutually exclusive".to_string());
    }
    if profile.is_some() && project.is_none() {
        return Err("`--profile` overlays a `--project`; give both".to_string());
    }
    if project.is_some() && duration_s.is_none() {
        return Err(
            "`--project` simulates until told to stop; `--duration` is required".to_string(),
        );
    }
    if replay.is_none() && project.is_none() {
        return Err("`--replay <log>` or `--project <project>` is required".to_string());
    }
    Ok(Cli::Run(CliOpts {
        replay,
        project,
        profile,
        speed,
        duration_s,
        stats_csv,
    }))
}

/// Runs a replay or a project simulation on the manual drive against the
/// real wall clock: the same `advance_clock` + `tick` lap the GUI's frame
/// loop performs, at a 1 ms cadence. A late lap only batches frames
/// (backfill covers the samples), so sleep granularity costs smoothness,
/// never data.
pub fn run(opts: &CliOpts) -> Result<String, String> {
    let mut app = App::headless();
    let replaying = opts.project.is_none();
    let mut profile_summary: Option<String> = None;
    if let Some(project) = &opts.project {
        // A project simulation: generators and script nodes run on the
        // virtual bus, exactly as the GUI would drive them.
        app.open_project_path(std::path::Path::new(project));
        if app.project_path.as_deref() != Some(std::path::Path::new(project)) {
            return Err(app.status.clone());
        }
        // A GUI project open deliberately leaves every generator entry
        // muted (opening a project must not start traffic). A headless
        // run is the explicit exception: re-arm the entries the project
        // saved as active, from the file itself.
        let file =
            std::fs::read_to_string(project).map_err(|e| format!("project read failed: {e}"))?;
        let cfg: crate::config::ProjectFile =
            serde_json::from_str(&file).map_err(|e| format!("project parse failed: {e}"))?;
        app.start_virtual();
        for t in &cfg.config.tx {
            if t.active {
                app.send(crate::bus::BusCommand::SetEntryActive {
                    ch: t.channel,
                    id: t.id,
                    on: true,
                });
            }
        }
        // The profile overlays role declarations on the freshly opened
        // project and WINS over the saved entry states -- it is the more
        // specific, later declaration. A refused profile aborts the run
        // before any traffic.
        if let Some(name) = &opts.profile {
            let dir = std::path::Path::new(project)
                .parent()
                .unwrap_or(std::path::Path::new("."));
            let summary = crate::profile::apply_profile(&mut app, dir, name)?;
            app.settle();
            profile_summary = Some(summary);
        }
    } else {
        let log = opts.replay.as_deref().expect("validated");
        app.load_log(log);
        // A failed load leaves `log_path` untouched and explains itself in
        // the status line.
        if app.log_path != log {
            return Err(app.status.clone());
        }
        // Set before `replay`, whose StartReplay carries the frontend's
        // speed. (No --record by design: the core drops Record state on
        // replay starts, since recording a replay would only duplicate the
        // log.)
        app.set_replay_speed(opts.speed);
        app.replay();
    }

    let t0 = std::time::Instant::now();
    let mut saw_timeline = false;
    let mut reason = if replaying {
        "end of log"
    } else {
        "duration limit"
    };
    loop {
        let now = t0.elapsed().as_micros() as u64;
        app.advance_clock(now);
        app.tick(now);
        // The playhead advances with the wall clock even past the last
        // frame, so this fires for every log -- empty ones on the first lap.
        if replaying {
            match app.replay_position() {
                Some((pos, dur)) => {
                    saw_timeline = true;
                    if pos >= dur {
                        break;
                    }
                }
                None => {
                    if saw_timeline {
                        reason = "source stopped";
                        break;
                    }
                }
            }
        }
        if let Some(limit) = opts.duration_s
            && t0.elapsed().as_secs_f64() >= limit
        {
            reason = "duration limit";
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(1));
    }
    if replaying && !saw_timeline {
        return Err("the replay never produced a timeline (is the log empty?)".to_string());
    }
    app.stop();

    if let Some(csv) = &opts.stats_csv {
        app.export_stats_csv(0, csv);
        if !app.status.starts_with("exported") {
            return Err(format!("stats export failed: {}", app.status));
        }
    }

    let mut report = if replaying {
        let (pos, dur) = app.replay_position().unwrap_or((0.0, 0.0));
        format!(
            "replay finished ({reason})\n  log        : {}\n  frames     : {}\n  playhead   : {:.3} / {:.3} s at {}x\n",
            opts.replay.as_deref().unwrap_or_default(),
            app.snap.frame_counter,
            pos.min(dur),
            dur,
            opts.speed,
        )
    } else {
        format!(
            "simulation finished ({reason})\n  project    : {}\n  frames     : {}\n  sim clock  : {:.3} s\n",
            opts.project.as_deref().unwrap_or_default(),
            app.snap.frame_counter,
            app.snap.sim_t_us as f64 / 1e6,
        )
    };
    if let Some(summary) = &profile_summary {
        report.push_str(&format!("  profile    : {summary}\n"));
    }
    report.push_str(&format!(
        "  wall time  : {:.3} s\n",
        t0.elapsed().as_secs_f64()
    ));
    if let Some(csv) = &opts.stats_csv {
        report.push_str(&format!("  stats csv  : {csv}\n"));
    }
    Ok(report)
}

/// Compiles each node script and reports the outcome per file. A file that
/// fails to read or compile prints its error and makes the whole run fail,
/// so a CI job or a pre-save hook can refuse broken scripts. Success prints
/// one `ok` line per script.
pub fn check_scripts(paths: &[String]) -> Result<String, String> {
    let mut lines = Vec::new();
    let mut failed = false;
    for path in paths {
        match std::fs::read_to_string(path) {
            Ok(src) => match crate::script::compile(&src) {
                Ok(script) => lines.push(format!(
                    "ok      {path} ({} handlers)",
                    script.handlers.len()
                )),
                Err(e) => {
                    lines.push(format!("failed {path}\n  {e}"));
                    failed = true;
                }
            },
            Err(e) => {
                lines.push(format!("failed {path}\n  {e}"));
                failed = true;
            }
        }
    }
    let mut report = lines.join("\n");
    report.push('\n');
    if failed { Err(report) } else { Ok(report) }
}

/// Release builds ship with `windows_subsystem = "windows"`: no console is
/// attached, so a headless run's prints would go nowhere. When a parent
/// console exists (the exe was launched from a terminal), adopt it and
/// point the standard handles at it. Debug builds already have a console
/// and the attach call fails harmlessly.
#[cfg(windows)]
pub fn attach_parent_console() {
    const ATTACH_PARENT_PROCESS: u32 = u32::MAX;
    const GENERIC_WRITE: u32 = 0x4000_0000;
    const FILE_SHARE_WRITE: u32 = 0x0000_0002;
    const OPEN_EXISTING: u32 = 3;
    const STD_OUTPUT_HANDLE: u32 = u32::MAX - 10; // (DWORD)-11
    const STD_ERROR_HANDLE: u32 = u32::MAX - 11; // (DWORD)-12
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn AttachConsole(process: u32) -> i32;
        fn CreateFileW(
            name: *const u16,
            access: u32,
            share: u32,
            security: *const u8,
            disposition: u32,
            flags: u32,
            template: isize,
        ) -> isize;
        fn SetStdHandle(which: u32, handle: isize) -> i32;
    }
    unsafe {
        if AttachConsole(ATTACH_PARENT_PROCESS) == 0 {
            return;
        }
        let mut conout: Vec<u16> = "CONOUT$".encode_utf16().collect();
        conout.push(0);
        let handle = CreateFileW(
            conout.as_ptr(),
            GENERIC_WRITE,
            FILE_SHARE_WRITE,
            std::ptr::null(),
            OPEN_EXISTING,
            0,
            0,
        );
        if handle != -1 {
            SetStdHandle(STD_OUTPUT_HANDLE, handle);
            SetStdHandle(STD_ERROR_HANDLE, handle);
        }
    }
}

#[cfg(not(windows))]
pub fn attach_parent_console() {}

#[cfg(test)]
#[path = "cli_tests.rs"]
mod tests;
