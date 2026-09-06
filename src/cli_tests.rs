//! CLI tests: argument parsing is pure, and the run itself goes through a
//! real log on disk at high playback speed so the wall-clock loop finishes
//! in milliseconds.

use super::{Cli, CliOpts, parse_args, run};
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
    ];
    for (args, needle) in cases {
        let err = parse_args(&flag_set(args)).unwrap_err();
        assert!(err.contains(needle), "`{err}` should mention `{needle}`");
    }
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
    ];
    for (args, needle) in cases {
        let err = parse_args(&flag_set(args)).unwrap_err();
        assert!(err.contains(needle), "`{err}` should mention `{needle}`");
    }
}
