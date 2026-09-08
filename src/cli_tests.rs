//! CLI tests: argument parsing is pure, and the run itself goes through a
//! real log on disk at high playback speed so the wall-clock loop finishes
//! in milliseconds.

use super::{convert_log, Cli, CliOpts, parse_args, run};
use crate::can::frame::{CanFrame, Direction, FrameFlags, MAX_CAN_FD_LEN};
use crate::log::AscWriter;

fn flag_set(args: &[&str]) -> Vec<String> {
    args.iter().map(|s| s.to_string()).collect()
}

fn opts_of(cli: &Cli) -> &CliOpts {
    match cli {
        Cli::Run(o) => o,
        other => panic!("expected a run, got {other:?}"),
    }
}

#[test]
fn no_arguments_open_the_window() {
    assert!(matches!(parse_args(&[]).unwrap(), Cli::Gui));
}

#[test]
fn help_wins_over_the_rest_of_the_line() {
    for form in ["-h", "--help"] {
        let cli = parse_args(&flag_set(&[form, "--nonsense"])).unwrap();
        let Cli::Help(text) = cli else {
            panic!("expected help for {form}");
        };
        assert!(text.contains("--replay"), "{text}");
    }
}

#[test]
fn a_full_flag_set_parses() {
    let cli = parse_args(&flag_set(&[
        "--replay",
        "run.asc",
        "--speed",
        "2.5",
        "--duration",
        "10",
        "--stats",
        "out.csv",
    ]))
    .unwrap();
    let o = opts_of(&cli);
    assert_eq!(o.replay.as_deref(), Some("run.asc"));
    assert_eq!(o.speed, 2.5);
    assert_eq!(o.duration_s, Some(10.0));
    assert_eq!(o.stats_csv.as_deref(), Some("out.csv"));
}

#[test]
fn defaults_apply_when_flags_are_absent() {
    let cli = parse_args(&flag_set(&["--replay", "x.blf"])).unwrap();
    let o = opts_of(&cli);
    assert_eq!(o.speed, 1.0);
    assert_eq!(o.duration_s, None);
    assert_eq!(o.stats_csv, None);
}

#[test]
fn usage_errors_name_their_flag() {
    let cases: &[(&[&str], &str)] = &[
        (&["--nonsense"], "unknown flag `--nonsense`"),
        (&["--replay"], "--replay needs a value"),
        (&["--speed"], "--speed needs a value"),
        (&["--speed", "abc"], "--speed wants a number"),
        (&["--speed", "0"], "--speed must be a positive number"),
        (&["--duration", "-1"], "--duration must be a positive"),
        (&["--replay", "a.asc", "--nope"], "unknown flag `--nope`"),
        (
            &["--replay", "a.asc", "--speed", "0"],
            "--speed must be a positive number",
        ),
        (&["--speed", "2"], "--replay"), // run needs its log
        (
            &["--check-script", "a.capl", "--replay", "a.asc"],
            "drop `--replay`",
        ),
        (
            &["--replay", "a.asc", "--profile", "ci"],
            "--profile` overlays a `--project`",
        ),
    ];
    for (args, needle) in cases {
        let err = parse_args(&flag_set(args)).unwrap_err();
        assert!(err.contains(needle), "`{err}` should mention `{needle}`");
    }
}

/// The profile rides on a project: accepted with one, stored for run().
#[test]
fn the_profile_flag_travels_with_the_project() {
    let cli = parse_args(&flag_set(&[
        "--project",
        "net.rxproj",
        "--duration",
        "5",
        "--profile",
        "ci",
    ]))
    .unwrap();
    let o = opts_of(&cli);
    assert_eq!(o.profile.as_deref(), Some("ci"));
    assert_eq!(o.project.as_deref(), Some("net.rxproj"));
}

#[test]
fn check_script_flags_collect_files() {
    let cli = parse_args(&flag_set(&["--check-script", "a.capl"])).unwrap();
    match cli {
        Cli::CheckScripts(files) => assert_eq!(files, ["a.capl"]),
        other => panic!("expected CheckScripts, got {other:?}"),
    }
    let cli = parse_args(&flag_set(&[
        "--check-script",
        "a.capl",
        "--check-script",
        "b.capl",
    ]))
    .unwrap();
    match cli {
        Cli::CheckScripts(files) => assert_eq!(files, ["a.capl", "b.capl"]),
        other => panic!("expected CheckScripts, got {other:?}"),
    }
}

/// The checker is the automation story for `.capl` files: a broken script
/// must name its file and its compile error, a good one must pass.
#[test]
fn check_scripts_compile_and_report() {
    use super::check_scripts;
    let dir = std::env::temp_dir();
    let good = dir.join("roxy_can_check_good.capl");
    let bad = dir.join("roxy_can_check_bad.capl");
    let missing = dir.join("roxy_can_check_missing.capl");
    std::fs::write(&good, "on start { print(\"up\"); }").unwrap();
    std::fs::write(&bad, "on timer 0 { }").unwrap();

    let ok = check_scripts(&[good.to_string_lossy().to_string()]).unwrap();
    assert!(ok.contains("ok"), "{ok}");
    assert!(ok.contains("1 handlers"), "{ok}");

    let report = check_scripts(&[
        bad.to_string_lossy().to_string(),
        missing.to_string_lossy().to_string(),
    ])
    .unwrap_err();
    assert!(report.contains("positive"), "{report}");
    assert!(
        report.contains("(os error"),
        "missing file explains itself: {report}"
    );

    std::fs::remove_file(&good).ok();
    std::fs::remove_file(&bad).ok();
}

fn write_log(name: &str, frames: usize, step_us: u64) -> std::path::PathBuf {
    let path = std::env::temp_dir().join(name);
    let mut w = AscWriter::new(&path.to_string_lossy()).unwrap();
    for i in 0..frames {
        let mut f = CanFrame {
            t_us: i as u64 * step_us,
            channel: 0,
            id: 0x100,
            extended: false,
            len: 2,
            data: [0; MAX_CAN_FD_LEN],
            dir: Direction::Rx,
            flags: FrameFlags::NONE,
        };
        f.data[0] = (i % 256) as u8;
        w.write(&f).unwrap();
    }
    w.finish().unwrap();
    path
}

fn tmp(name: &str) -> String {
    std::env::temp_dir()
        .join(name)
        .to_string_lossy()
        .into_owned()
}

#[test]
fn a_cli_replay_runs_the_log_and_exports() {
    let log = write_log("roxy_can_cli_replay.asc", 100, 10_000);
    let stats = tmp("roxy_can_cli_stats.csv");
    let report = run(&CliOpts {
        replay: Some(log.to_string_lossy().into_owned()),
        project: None,
        profile: None,
        speed: 50.0, // a 1 s log finishes in ~20 ms of wall clock
        duration_s: None,
        stats_csv: Some(stats.clone()),
    })
    .unwrap();

    assert!(report.contains("frames     : 100"), "{report}");
    assert!(report.contains("end of log"), "{report}");
    assert!(report.contains("at 50x"), "{report}");

    let csv = std::fs::read_to_string(&stats).unwrap();
    assert!(csv.starts_with("bus,id,name,count"), "{csv}");
    assert!(
        report.contains("stats csv"), // no stats path → no report line
        "{report}"
    );

    std::fs::remove_file(&log).ok();
    std::fs::remove_file(&stats).ok();
}

#[test]
fn the_duration_flag_stops_before_the_log_ends() {
    let log = write_log("roxy_can_cli_duration.asc", 100, 10_000);
    let started = std::time::Instant::now();
    let report = run(&CliOpts {
        replay: Some(log.to_string_lossy().into_owned()),
        project: None,
        profile: None,
        speed: 1.0,
        duration_s: Some(0.05),
        stats_csv: None,
    })
    .unwrap();
    assert!(
        started.elapsed() < std::time::Duration::from_secs(5),
        "the duration limit must actually stop the run"
    );
    assert!(report.contains("duration limit"), "{report}");
    let frames: u64 = report
        .lines()
        .find_map(|l| l.strip_prefix("  frames     : "))
        .expect("the report counts frames")
        .parse()
        .unwrap();
    assert!(
        (1..100).contains(&frames),
        "some frames flowed but not the whole log: {frames}"
    );
    std::fs::remove_file(&log).ok();
}

#[test]
fn a_missing_log_reports_instead_of_running() {
    let err = run(&CliOpts {
        replay: Some(tmp("roxy_can_cli_no_such_log.asc")),
        project: None,
        profile: None,
        speed: 1.0,
        duration_s: None,
        stats_csv: None,
    })
    .unwrap_err();
    assert!(err.contains("log load failed"), "{err}");
}

/// `--project`: the saved project simulates headless. The generator
/// entries saved active in the project drive the virtual bus, and the
/// stats CSV comes out the far end -- the CI story for simulation
/// setups.
#[test]
fn a_project_simulates_headless_until_the_duration_stops_it() {
    let mut app = crate::app::App::headless();
    app.add_tx(0, 0x100);
    app.settle();
    // Command-driven, so the core (not a stale frontend copy) holds the
    // activated entry the project save must record.
    app.send(crate::bus::BusCommand::SetEntryActive {
        ch: 0,
        id: 0x100,
        on: true,
    });
    app.send(crate::bus::BusCommand::SetEntryCycle {
        ch: 0,
        id: 0x100,
        cycle_us: 10_000,
    });
    app.settle();
    let project = std::env::temp_dir().join("roxy_can_cli_sim.rxproj");
    assert!(app.save_project(Some(project.clone())), "save writes");
    app.stop();

    let stats = tmp("roxy_can_cli_sim_stats.csv");
    let report = run(&CliOpts {
        replay: None,
        project: Some(project.to_string_lossy().into_owned()),
        profile: None,
        speed: 1.0,
        duration_s: Some(0.2),
        stats_csv: Some(stats.clone()),
    })
    .unwrap();
    assert!(report.contains("simulation finished"), "{report}");
    assert!(report.contains("sim clock"), "{report}");

    let frames: u64 = report
        .lines()
        .find_map(|l| l.strip_prefix("  frames     : "))
        .expect("the report counts frames")
        .parse()
        .unwrap();
    assert!(
        (10..500).contains(&frames),
        "a 100 ms entry ran for ~0.2 s of wall clock: {frames}"
    );
    let csv = std::fs::read_to_string(&stats).unwrap();
    assert!(
        csv.contains("EngineStatus"),
        "the stats csv names the simulated message: {csv}"
    );

    std::fs::remove_file(&project).ok();
    std::fs::remove_file(&stats).ok();
}

#[test]
fn a_project_needs_a_duration_and_refuses_a_log() {
    let cases: &[(&[&str], &str)] = &[
        (&["--project", "p.rxproj"], "--duration"),
        (
            &[
                "--project",
                "p.rxproj",
                "--replay",
                "a.asc",
                "--duration",
                "1",
            ],
            "mutually exclusive",
        ),
        (
            &[
                "--convert",
                "a.blf",
                "b.asc",
                "--replay",
                "a.asc",
            ],
            "on its own",
        ),
        (
            &["--convert", "a.blf", "b.csv"],
            "must end in .asc",
        ),
    ];
    for (args, needle) in cases {
        let err = parse_args(&flag_set(args)).unwrap_err();
        assert!(err.contains(needle), "`{err}` should mention `{needle}`");
    }
}

/// `--convert` is a pure stream transform: every frame crosses with its
/// timestamp, id, and payload -- the BLF→ASC story without touching the
/// replay or recording semantics.
#[test]
fn convert_transcodes_a_log_to_asc_losslessly() {
    let src = write_log("roxy_can_convert_in.asc", 7, 1_000);
    let out = tmp("roxy_can_convert_out.asc");
    let report = convert_log(src.to_string_lossy().as_ref(), &out).unwrap();
    assert!(report.contains("7 frame(s)"), "{report}");

    let text = std::fs::read_to_string(&out).unwrap();
    let frames = crate::log::asc::parse_asc(&text);
    assert_eq!(frames.len(), 7, "every frame crossed");
    assert_eq!(frames[3].id, 0x100);
    assert_eq!(frames[3].data[0], 3, "payloads are exact");

    // Round-tripping again yields the same frames (the ASC header's
    // wall-clock date line differs, but no frame data does). CanFrame
    // has no PartialEq, so compare the identity tuple.
    let again = tmp("roxy_can_convert_again.asc");
    convert_log(&out, &again).unwrap();
    let sig = |frames: &[CanFrame]| -> Vec<(u64, u8, u32, bool, u8)> {
        frames
            .iter()
            .map(|f| (f.t_us, f.channel, f.id, f.extended, f.len))
            .collect()
    };
    let first = crate::log::asc::parse_asc(&std::fs::read_to_string(&out).unwrap());
    let second = crate::log::asc::parse_asc(&std::fs::read_to_string(&again).unwrap());
    assert_eq!(sig(&first), sig(&second), "ASC->ASC is frame-stable");
    let payload_stable = first
        .iter()
        .zip(second.iter())
        .all(|(a, b)| a.data == b.data);
    assert!(payload_stable, "payloads are stable too");
    std::fs::remove_file(&src).ok();
    std::fs::remove_file(&out).ok();
    std::fs::remove_file(&again).ok();
}

/// A missing input names the failure instead of writing an empty file.
#[test]
fn convert_reports_a_broken_input() {
    let out = tmp("roxy_can_convert_broken.asc");
    let err = convert_log(tmp("roxy_can_convert_missing.blf").as_str(), &out).unwrap_err();
    assert!(err.contains("log load failed"), "{err}");
    assert!(
        !std::path::Path::new(&out).exists(),
        "no output file for a failed load"
    );
}

/// Same-file conversion is refused before anything is opened: truncating
/// the input under its own mmap reader would corrupt the data.
#[test]
fn convert_refuses_input_equal_to_output() {
    let src = write_log("roxy_can_convert_same.asc", 3, 1_000);
    let err = convert_log(src.to_string_lossy().as_ref(), src.to_string_lossy().as_ref())
        .unwrap_err();
    assert!(err.contains("same file"), "{err}");
    let text = std::fs::read_to_string(&src).unwrap();
    assert!(text.contains("End TriggerBlock"), "the input survived");
    std::fs::remove_file(&src).ok();
}

/// The composite CI story end to end: a saved project whose script node
/// transmits on a timer runs headless and the node's frames reach the
/// stats export -- no window, no user, full fidelity.
#[test]
fn a_project_node_script_drives_a_headless_simulation() {
    let mut app = crate::app::App::headless();
    app.tx_list.retain(|t| t.channel != 0);
    app.send(crate::bus::BusCommand::SetNodes {
        nodes: vec![crate::config::NodeCfg {
            name: "beacon".to_string(),
            channel: 0,
            source: "on timer 50 { send(0x555, 1); }".to_string(),
            enabled: true,
        }],
    });
    app.settle();
    let project = std::env::temp_dir().join("roxy_can_cli_node.rxproj");
    assert!(app.save_project(Some(project.clone())), "save writes");
    app.stop();

    let report = run(&CliOpts {
        replay: None,
        project: Some(project.to_string_lossy().into_owned()),
        profile: None,
        speed: 1.0,
        duration_s: Some(0.3),
        stats_csv: None,
    })
    .unwrap();
    let frames: u64 = report
        .lines()
        .find_map(|l| l.strip_prefix("  frames     : "))
        .expect("the report counts frames")
        .parse()
        .unwrap();
    assert!(
        frames >= 3,
        "a 50 ms node timer must drive traffic for 0.3 s: {report}"
    );
    std::fs::remove_file(&project).ok();
}

/// `--profile` end to end: a project saved fully muted, a profile that
/// simulates EngineECU, and the overlay's traffic reaching the run. The
/// report names the profile; the project alone would have produced
/// nothing.
#[test]
fn a_profile_overlay_drives_a_muted_project() {
    use std::fs;

    let dir = std::env::temp_dir().join("roxy_can_cli_prof");
    fs::create_dir_all(dir.join("profiles")).unwrap();
    let project = dir.join("net.rxproj");

    // The fixture project: nothing active, nothing simulated.
    let app = crate::app::App::headless();
    let proj = crate::config::ProjectFile {
        version: 1,
        layout: String::new(),
        project: None,
        config: crate::config::Config::from_app(&app, None),
    };
    fs::write(
        &project,
        serde_json::to_string_pretty(&proj).expect("project serializes"),
    )
    .unwrap();
    fs::write(
        dir.join("profiles").join("ci.toml"),
        "[[node]]\nbus = \"CAN1\"\nnode = \"EngineECU\"\nrole = \"Simulated\"\n",
    )
    .unwrap();

    let report = run(&CliOpts {
        replay: None,
        project: Some(project.to_string_lossy().into_owned()),
        profile: Some("ci".to_string()),
        speed: 1.0,
        // The engine entry runs at the 100 ms default cycle; half a
        // second leaves margin even when parallel tests slow the laps.
        duration_s: Some(0.5),
        stats_csv: None,
    })
    .unwrap();
    assert!(report.contains("profile    : profile `ci` applied"), "{report}");
    let frames: u64 = report
        .lines()
        .find_map(|l| l.strip_prefix("  frames     : "))
        .expect("the report counts frames")
        .parse()
        .unwrap();
    assert!(
        frames >= 2,
        "the profile's Simulated node must drive the muted project: {report}"
    );
    fs::remove_dir_all(&dir).ok();
}

/// A profile that names a node the DBC never declared aborts the run
/// before any traffic -- loud failure is the profile contract.
#[test]
fn a_bad_profile_aborts_the_run_before_traffic() {
    use std::fs;

    let dir = std::env::temp_dir().join("roxy_can_cli_prof_bad");
    fs::create_dir_all(dir.join("profiles")).unwrap();
    let project = dir.join("net.rxproj");
    let app = crate::app::App::headless();
    let proj = crate::config::ProjectFile {
        version: 1,
        layout: String::new(),
        project: None,
        config: crate::config::Config::from_app(&app, None),
    };
    fs::write(
        &project,
        serde_json::to_string_pretty(&proj).expect("project serializes"),
    )
    .unwrap();
    fs::write(
        dir.join("profiles").join("typo.toml"),
        "[[node]]\nbus = \"CAN1\"\nnode = \"EngineEcu\"\nrole = \"Simulated\"\n",
    )
    .unwrap();

    let err = run(&CliOpts {
        replay: None,
        project: Some(project.to_string_lossy().into_owned()),
        profile: Some("typo".to_string()),
        speed: 1.0,
        duration_s: Some(0.2),
        stats_csv: None,
    })
    .unwrap_err();
    assert!(err.contains("EngineEcu"), "{err}");
    fs::remove_dir_all(&dir).ok();
}
