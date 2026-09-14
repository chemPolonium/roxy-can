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
    CheckScripts(ScriptCheck),
    /// Transcode a log to ASC: `--convert <in> <out>`. A pure stream
    /// transform -- no bus, no clock, no window.
    Convert(String, String),
    /// List the Kvaser channels canlib sees: a quick way to check the
    /// driver and adapters before using the hardware link.
    KvaserProbe,
    /// List the Vector channels vxlapi sees with their bus types, and --
    /// when a FlexRay-capable channel is present -- open it RX-only and
    /// drain it briefly: the FR-2 field diagnostic.
    VectorProbe,
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

/// Inputs of the `--check-script` headless gate: the scripts to check,
/// optionally the DBCs (repeatable, merged in order) to run the
/// database-backed assembly checks against, and optionally the DBC node
/// name the scripts act as -- which turns on the send-set and reverse
/// implementation checks.
#[derive(Debug, Default)]
pub struct ScriptCheck {
    pub scripts: Vec<String>,
    pub dbcs: Vec<String>,
    pub node: Option<String>,
}

pub fn usage() -> &'static str {
    "roxy-can -- CAN bus analysis

  roxy-can                       open the workspace window (default)
  roxy-can --replay <log> ...    replay a CAN log without any window
  roxy-can --project <p> ...     simulate a saved project without any window
  roxy-can --check-script <f>    compile node scripts, no window needed
                                 (repeat the flag for more files; non-zero
                                 exit when any script fails). With
                                 --dbc/--node the DBC-backed assembly
                                 checks (signal refs, send set vs the
                                 node's declarations, reverse check,
                                 set_sig ranges) run here too
  roxy-can --convert <in> <out>  transcode a log (.asc/.blf) to ASC
  roxy-can --kvaser-probe        list the Kvaser channels canlib sees
  roxy-can --vector-probe        list Vector channels with bus types; opens
                                 a FlexRay channel rx-only when present

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
  --dbc <path>       DBC for --check-script's assembly checks (repeatable,
                     merged in order, first library wins on clashes)
  --node <name>      the DBC node name the checked scripts act as; turns
                     on the send-set and reverse implementation checks
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
    let mut convert: Option<(String, String)> = None;
    let mut kvaser_probe = false;
    let mut vector_probe = false;
    let mut speed = 1.0f64;
    let mut duration_s = None;
    let mut stats_csv = None;
    let mut scripts = Vec::new();
    let mut script_check = ScriptCheck::default();
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
            "--convert" => {
                let input = value(args, &mut i, "--convert input")?;
                let output = value(args, &mut i, "--convert output")?;
                convert = Some((input, output));
            }
            "--kvaser-probe" => kvaser_probe = true,
            "--vector-probe" => vector_probe = true,
            "--check-script" => scripts.push(value(args, &mut i, "--check-script")?),
            "--dbc" => script_check.dbcs.push(value(args, &mut i, "--dbc")?),
            "--node" => script_check.node = Some(value(args, &mut i, "--node")?),
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
        script_check.scripts = scripts;
        return Ok(Cli::CheckScripts(script_check));
    } else if !script_check.dbcs.is_empty() || script_check.node.is_some() {
        return Err("`--dbc`/`--node` belong to `--check-script`".to_string());
    } else if let Some((input, output)) = convert {
        if replay.is_some() || project.is_some() || profile.is_some() {
            return Err(
                "`--convert` transcodes a log on its own; drop the other run flags".to_string(),
            );
        }
        if !output.to_ascii_lowercase().ends_with(".asc") {
            return Err("`--convert` writes ASC; the output must end in .asc".to_string());
        }
        return Ok(Cli::Convert(input, output));
    }
    if kvaser_probe {
        if replay.is_some() || project.is_some() || profile.is_some() {
            return Err("`--kvaser-probe` runs on its own; drop the other run flags".to_string());
        }
        return Ok(Cli::KvaserProbe);
    }
    if vector_probe {
        if replay.is_some() || project.is_some() || profile.is_some() {
            return Err("`--vector-probe` runs on its own; drop the other run flags".to_string());
        }
        return Ok(Cli::VectorProbe);
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
        // The profile overlays role declarations (and the hardware
        // mapping) on the freshly opened project. A refused profile
        // aborts the run before any traffic.
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

/// Transcodes any readable log (ASC or BLF) into ASC: a pure stream
/// transform with no bus, clock, or recorder involvement -- which is why
/// the old "replay drops recording state" blocker does not apply. Frames
/// cross with their own timestamps, classes, and payloads.
pub fn convert_log(input: &str, output: &str) -> Result<String, String> {
    // Same-file conversion truncates the input while the stream may be
    // mmap-reading it -- refuse rather than corrupt.
    if std::fs::canonicalize(input).ok() == std::fs::canonicalize(output).ok()
        && std::fs::canonicalize(input).is_ok()
    {
        return Err("--convert: input and output are the same file".to_string());
    }
    let mut stream = crate::log::open_stream(std::path::Path::new(input))
        .map_err(|e| format!("log load failed: {e}"))?;
    let mut w =
        crate::log::AscWriter::new(output).map_err(|e| format!("open output failed: {e}"))?;
    let mut n: u64 = 0;
    while let Some(t) = stream.peek_t() {
        let Some(f) = stream.next_frame() else {
            break;
        };
        let _ = t;
        w.write(&f).map_err(|e| format!("write failed: {e}"))?;
        n += 1;
    }
    w.finish().map_err(|e| format!("close failed: {e}"))?;
    Ok(format!(
        "converted {n} frame(s)\n  input : {input}\n  output: {output}"
    ))
}

/// Lists the Kvaser channels canlib discovers: index, name, and whether
/// the channel is the driver's virtual one. The quickest check that the
/// driver and adapters are usable before touching the hardware link.
pub fn kvaser_probe() -> Result<String, String> {
    match crate::hw::kvaser::enumerate() {
        Ok(channels) => {
            let mut s = format!("{} Kvaser channel(s):\n", channels.len());
            for c in &channels {
                s.push_str(&format!("  ch{}: {}\n", c.index, c.name));
            }
            Ok(s)
        }
        Err(e) => Err(e),
    }
}

/// Lists the Vector channels vxlapi discovers with their bus types. When
/// a FlexRay-capable channel is present (a VN7640 port), opens it
/// RX-only with a zeroed cluster configuration and drains it for two
/// seconds, printing every frame -- the FR-2 field diagnostic. The zero
/// configuration is a probe, not a working setup: real reception needs
/// the target network's cluster parameters.
pub fn vector_probe() -> Result<String, String> {
    let channels = crate::hw::vector::enumerate()?;
    let mut s = format!("{} Vector channel(s):\n", channels.len());
    for c in &channels {
        s.push_str(&format!(
            "  ch{}: {} [can:{} flexray:{}]\n",
            c.index, c.name, c.can, c.flexray
        ));
    }
    let fr = crate::hw::vector::enumerate_flexray()?;
    if fr.is_empty() {
        s.push_str("no FlexRay-capable channel present\n");
        return Ok(s);
    }
    for c in &fr {
        s.push_str(&format!("opening ch{} rx-only...\n", c.index));
            match crate::hw::vector::flexray::FlexRayChannel::open_rx(
            c.index,
            &crate::hw::vector::flexray::XLfrClusterConfig::default(),
        ) {
            Ok(mut ch) => {
                s.push_str("  opened; draining 2 s with the zeroed cluster config\n");
                match ch.channel_config() {
                    Ok(cfg) => {
                        s.push_str(&format!(
                            "  channel cfg: status=0x{:08X} cfgMode={} baudrate={} gMacroPerCycle={} gdMacrotick={} staticSlots={} payloadStatic={}\n",
                            cfg.status,
                            cfg.cfg_mode,
                            cfg.cluster.baudrate,
                            cfg.cluster.g_macro_per_cycle,
                            cfg.cluster.gd_macrotick,
                            cfg.cluster.g_number_of_static_slots,
                            cfg.cluster.g_payload_length_static,
                        ));
                        if cfg.status & crate::hw::vector::flexray::FR_CHANNEL_CFG_STATUS_VALID_CLUSTER_CFG == 0 {
                            s.push_str("  note: no VALID_CLUSTER_CFG bit -- the driver holds no usable cluster config, frames will not arrive\n");
                        }
                    }
                    Err(e) => s.push_str(&format!("  channel config read failed: {e}\n")),
                }
                let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
                let mut frames = 0usize;
                while std::time::Instant::now() < deadline {
                    if let Some(f) = ch.try_read() {
                        frames += 1;
                        s.push_str(&format!(
                            "  frame slot={} cycle={} len={} crc=0x{:04X}\n",
                            f.slot,
                            f.cycle,
                            f.payload.len(),
                            f.header_crc
                        ));
                    } else {
                        std::thread::sleep(std::time::Duration::from_millis(5));
                    }
                }
                s.push_str(&format!("  drained {frames} frame(s)\n"));
            }
            Err(e) => s.push_str(&format!("  {e}\n")),
        }
    }
    Ok(s)
}

/// Compiles each node script and reports the outcome per file. A file that
/// fails to read or compile prints its error and makes the whole run fail,
/// so a CI job or a pre-save hook can refuse broken scripts. Success prints
/// one `ok` line per script plus the static facts (response mapping, send
/// / receive sets). With `--dbc`/`--node` the database-backed assembly
/// checks the GUI runs at node start execute here too, printed under the
/// script that raised them.
pub fn check_scripts(check: &ScriptCheck) -> Result<String, String> {
    // Merge `--dbc` files in order into one table, matching the GUI's
    // multi-database loading semantics (first library wins on clashes).
    let dbc = if check.dbcs.is_empty() {
        None
    } else {
        let mut table: Option<crate::dbc::SymbolTable> = None;
        for path in &check.dbcs {
            let text = std::fs::read_to_string(path).map_err(|e| format!("{path}: {e}"))?;
            let parsed =
                crate::dbc::load_dbc_str(&text).map_err(|e| format!("{path}: {e}"))?;
            match &mut table {
                Some(t) => crate::dbc::absorb(t, parsed),
                None => table = Some(parsed),
            }
        }
        table
    };
    let mut lines = Vec::new();
    let mut failed = false;
    for path in &check.scripts {
        match std::fs::read_to_string(path) {
            Ok(src) => match crate::script::compile(&src) {
                Ok(script) => {
                    lines.push(format!(
                        "ok      {path} ({} handlers)",
                        script.handlers.len()
                    ));
                    // The static response mapping (R2): events → timers →
                    // replies, derived from the compiled script itself.
                    for row in script.response_map() {
                        if row.armed_by.is_empty() {
                            continue; // a timer nobody arms: not a response
                        }
                        let replies = row
                            .sends
                            .iter()
                            .map(|(id, ext)| format!("{id:#X}{}", if *ext { "x" } else { "" }))
                            .collect::<Vec<_>>()
                            .join(", ");
                        lines.push(format!(
                            "        response: {} arms \"{}\" -> sends {}",
                            row.armed_by.join(", "),
                            row.timer_label,
                            replies
                        ));
                    }
                    // R2 静态事实表：发送集与接收集一目了然。
                    let sends = script
                        .send_refs
                        .iter()
                        .map(|(_, id, ext)| format!("{id:#X}{}", if *ext { "x" } else { "" }))
                        .collect::<Vec<_>>()
                        .join(", ");
                    let recvs = script
                        .recv_refs
                        .iter()
                        .map(|(id, ext)| format!("{id:#X}{}", if *ext { "x" } else { "" }))
                        .collect::<Vec<_>>()
                        .join(", ");
                    lines.push(format!(
                        "        sends: {}",
                        if sends.is_empty() { "-".to_string() } else { sends }
                    ));
                    lines.push(format!(
                        "        receives: {}",
                        if recvs.is_empty() { "-".to_string() } else { recvs }
                    ));
                    // The assembly checks, same code path as the GUI's
                    // node start: only exit-code-relevant failure is the
                    // compile above; [check]/[info] lines are the report.
                    if let Some(db) = &dbc {
                        for line in crate::node::dbc_checks(&script, db, check.node.as_deref()) {
                            lines.push(format!("        {line}"));
                        }
                    }
                }
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
