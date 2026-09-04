//! Headless integration tests (主线阶段 2 验收): the whole application --
//! commands in, snapshot out -- runs a complete measurement and export
//! without any UI. These tests drive `advance_clock` + `tick` directly the
//! way the frame loop does, with a synthetic clock, so they never touch
//! imgui, winit or the real wall clock.

use crate::app::{App, Mode};
use crate::can::frame::{CanFrame, Direction, FrameFlags, MAX_CAN_FD_LEN};
use crate::log::AscWriter;
use crate::sim::{SrcKind, ValueSrc};

/// What `update` does for the frame loop, minus the imgui-facing text
/// gate: advance the bus clock to synthetic `now`, then run one step.
fn step(app: &mut App, now: u64) {
    app.advance_clock(now);
    app.tick(now);
}

fn rpm_frame(t_us: u64, rpm: f64) -> CanFrame {
    let raw = (rpm / 0.25) as u16;
    let mut f = CanFrame {
        t_us,
        channel: 0,
        id: 0x100,
        extended: false,
        len: 2,
        data: [0; MAX_CAN_FD_LEN],
        dir: Direction::Rx,
        flags: FrameFlags::NONE,
    };
    f.data[0] = (raw & 0xFF) as u8;
    f.data[1] = (raw >> 8) as u8;
    f
}

fn write_test_log(name: &str, frames: usize, step_us: u64) -> std::path::PathBuf {
    let path = std::env::temp_dir().join(name);
    let mut w = AscWriter::new(&path.to_string_lossy()).unwrap();
    for i in 0..frames {
        w.write(&rpm_frame(i as u64 * step_us, 1000.0)).unwrap();
    }
    w.finish().unwrap();
    path
}

#[test]
fn a_full_virtual_run_composes_through_commands_and_snapshots() {
    let mut app = App::headless();
    app.start_virtual();
    // Drive one DBC-known entry by a sine, subscribe its signal, all via
    // the command path. 0x100 EngineSpeed: factor 0.25, range 0..8000.
    app.send(crate::bus::BusCommand::SetEntryActive {
        ch: 0,
        id: 0x100,
        on: true,
    });
    app.send(crate::bus::BusCommand::SetEntrySource {
        ch: 0,
        id: 0x100,
        src: ValueSrc::new("EngineSpeed", SrcKind::Sine, 0.0, 8000.0),
    });
    app.send(crate::bus::BusCommand::Subscribe {
        key: (0, 0x100, "EngineSpeed".to_string()),
    });

    // Three simulated seconds at 1 ms. 0x100 has no declared cycle in
    // sample.dbc, so the entry runs on the default 100 ms period.
    let mut now = 1_000;
    for _ in 0..3_000 {
        now += 1_000;
        step(&mut app, now);
    }

    // Frames flowed, the aggregate folded them, the signal cache sampled.
    assert!(
        app.snap.frame_counter >= 25 && app.snap.frame_counter <= 35,
        "{}",
        app.snap.frame_counter
    );
    let agg = app
        .snap
        .aggs
        .iter()
        .find(|a| a.channel == 0 && a.id == 0x100)
        .expect("0x100 aggregated");
    assert_eq!(agg.count, app.snap.frame_counter);
    assert!(
        agg.cycle_us > 90_000.0 && agg.cycle_us < 110_000.0,
        "{}",
        agg.cycle_us
    );
    let sub = app
        .sub_view(&(0, 0x100, "EngineSpeed".to_string()))
        .expect("subscribed");
    assert!(
        sub.history.len() >= 20,
        "expected 50 ms-stride samples, got {}",
        sub.history.len()
    );
    // The waveform really drove the payload: the latest raw value is a
    // 0.25-rpm step, so a sine on 0..8000 must have produced varied bytes.
    assert!(
        sub.min < sub.max,
        "waveform never moved: {}..{}",
        sub.min,
        sub.max
    );
    // The virtual run has no replay timeline.
    assert_eq!(app.replay_position(), None);
}

#[test]
fn a_headless_recording_writes_a_readable_log() {
    let mut app = App::headless();
    let path = std::env::temp_dir().join("roxy_can_headless_record.asc");
    app.record_path_buf = path.to_string_lossy().to_string();
    app.start_virtual();
    app.send(crate::bus::BusCommand::SetEntryActive {
        ch: 0,
        id: 0x100,
        on: true,
    });
    app.toggle_record();
    assert!(app.recorder.recording);

    let mut now = 1_000;
    for _ in 0..500 {
        now += 2_000;
        step(&mut app, now);
    }
    app.stop();
    // Stop keeps the record checkbox armed by design; the file handle
    // itself is closed (the frames below read it back from disk).

    // The recorder derives a dated path from the base name; `last_record`
    // holds what was actually written.
    let actual = std::path::PathBuf::from(app.recorder.last_record.clone());
    let mut stream = crate::log::open_stream(&actual).expect("recorded file reopens");
    let frames = std::iter::from_fn(|| stream.next_frame()).count();
    // Slots anchored at t=0, every 100 ms, and the run reaches sim
    // 1_001 ms: slots 0, 100k .. 1_000k -- eleven frames.
    assert_eq!(frames, 11, "recorded one frame per generator slot");
}

#[test]
fn a_headless_export_reports_the_run() {
    let mut app = App::headless();
    app.start_virtual();
    app.send(crate::bus::BusCommand::SetEntryActive {
        ch: 0,
        id: 0x100,
        on: true,
    });
    let mut now = 1_000;
    for _ in 0..1_000 {
        now += 2_000;
        step(&mut app, now);
    }

    let path = std::env::temp_dir().join("roxy_can_headless_stats.csv");
    app.export_stats_csv(0, &path.to_string_lossy());
    let csv = std::fs::read_to_string(&path).unwrap();
    assert!(csv.starts_with("bus,id,name,count"), "{csv}");
    assert!(csv.contains("EngineStatus"), "{csv}");
}

#[test]
fn a_headless_replay_runs_the_log_and_mutes_the_twin() {
    let log = write_test_log("roxy_can_headless_replay.asc", 100, 10_000);

    let mut app = App::headless();
    // The generator entry for 0x100 is on -- the log carries the same id,
    // so replay must silence it and deliver exactly the log's frames.
    app.send(crate::bus::BusCommand::SetEntryActive {
        ch: 0,
        id: 0x100,
        on: true,
    });
    app.log_path = log.to_string_lossy().to_string();
    app.replay();
    assert!(matches!(app.mode, Mode::Replay));

    // 100 frames at 10 ms log-time: 1.5 s of wall clock at 1 ms covers the
    // whole log with margin.
    let mut now = 1_000;
    for _ in 0..1_500 {
        now += 1_000;
        step(&mut app, now);
    }

    assert_eq!(app.snap.frame_counter, 100, "log frames only, twin muted");
    let (pos, _dur) = app.replay_position().expect("replay has a timeline");
    assert!(pos > 0.5, "playhead advanced, at {pos} s");
}

#[test]
fn the_deadline_follows_the_next_generator_slot() {
    let mut app = App::headless();
    app.start_virtual();
    app.send(crate::bus::BusCommand::SetEntryActive {
        ch: 0,
        id: 0x100,
        on: true,
    });
    // 0x100 runs on the default 100 ms period, anchored at sim 0: the
    // first slot is due immediately.
    assert_eq!(app.next_deadline(1_000_000), Some(1_000_000));
    // Run 50 ms of steps; the next slot then sits 50 ms out.
    let mut now = 0;
    for _ in 0..50 {
        now += 1_000;
        app.advance_clock(now);
        app.tick(now);
    }
    assert_eq!(app.next_deadline(now), Some(now + 50_000));
    // An idle (all-off) bus has no deadline at all.
    app.send(crate::bus::BusCommand::SetEntryActive {
        ch: 0,
        id: 0x100,
        on: false,
    });
    assert_eq!(app.next_deadline(now), None);
}

#[test]
fn step_to_advances_the_clocks_and_the_bus() {
    let mut app = App::headless();
    app.start_virtual();
    app.send(crate::bus::BusCommand::SetEntryActive {
        ch: 0,
        id: 0x100,
        on: true,
    });
    let mut now = 0;
    let mut status = String::new();
    // One wake per second, as an event loop would: six wakes, each step_to
    // must still emit every slot the wall clock passed.
    for _ in 0..6 {
        now += 1_000_000;
        let stride = app.wanted_stride_us();
        let (tol, grace) = (app.spec_tol_pct, app.spec_grace);
        let done = app.step_to(now, stride, tol, grace, &mut status);
        assert!(!done);
        app.refresh_snapshot();
    }
    // Slots at 0, 100k .. 6_000k = 61 frames across six 1 s wakes.
    assert_eq!(app.snap.frame_counter, 61, "{}", app.snap.frame_counter);
}

/// The real deal: `App::new` runs the core on its own thread, driven by
/// nothing but wall time and the commands it is sent. The polling loop
/// tolerates any scheduling; the assertions are about liveness.
#[test]
fn the_threaded_core_serves_frames_on_its_own_thread() {
    let mut app = App::new();
    app.send(crate::bus::BusCommand::SetEntryActive {
        ch: 0,
        id: 0x100,
        on: true,
    });
    app.start_virtual();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while std::time::Instant::now() < deadline && app.snap.frame_counter < 10 {
        std::thread::sleep(std::time::Duration::from_millis(2));
        app.update();
    }
    assert!(
        app.snap.frame_counter >= 10,
        "the threaded core never produced frames: {}",
        app.snap.frame_counter
    );
    // Dropping the app drops the command sender, which is the core
    // thread's signal to exit. Nothing to assert directly -- a leaked
    // thread would only surface under a leak checker -- but releasing
    // here is the contract the loop implements.
    drop(app);
}

/// The crash the first threaded launch actually produced: startup
/// restores the last project, and the restore path must cross the thread
/// boundary too. A distinctive workspace is saved from the manual drive,
/// then restored into a threaded one.
#[test]
fn the_threaded_core_restores_a_saved_project_at_startup() {
    let mut saved = App::headless();
    saved.channels[0].name = "Powertrain".to_string();
    saved.refresh_snapshot();
    saved.add_signal_trigger();
    let path = std::env::temp_dir().join("roxy_can_threaded_restore.rxproj");
    assert!(
        saved.save_project(Some(path.clone())),
        "save writes the file"
    );

    let mut app = App::new();
    app.open_project_path(&path);
    app.settle();
    assert_eq!(
        app.channel_name(0),
        "Powertrain",
        "bus declarations ride the restore"
    );
    assert_eq!(
        app.snap.triggers.len(),
        1,
        "the trigger rule list is on the bus side, so it must arrive via SetTriggers"
    );
    drop(app);
    std::fs::remove_file(&path).ok();
}

/// Replay on the threaded drive, including the pause edge: the thread's
/// pause stamping must freeze the log (playhead and frame flow) and
/// resume it without losing the position.
#[test]
fn the_threaded_core_replays_a_log_and_survives_a_pause() {
    // Replay material: a short recorded log from the manual drive.
    let mut src = App::headless();
    src.record_path_buf = std::env::temp_dir()
        .join("roxy_can_threaded_replay_src")
        .to_string_lossy()
        .to_string();
    src.toggle_record();
    src.start_virtual();
    src.send(crate::bus::BusCommand::SetEntryActive {
        ch: 0,
        id: 0x100,
        on: true,
    });
    // Six seconds of log: enough that pausing after ~1 s still leaves
    // plenty of tail for the resume phase to keep playing.
    let mut now = 0;
    for _ in 0..3000 {
        now += 2_000;
        step(&mut src, now);
    }
    src.stop();
    let log = src.recorder.last_record.clone();
    assert!(!log.is_empty(), "the source log was recorded");

    let mut app = App::new();
    app.load_log(&log);
    app.replay();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(8);
    while std::time::Instant::now() < deadline && app.snap.frame_counter < 10 {
        std::thread::sleep(std::time::Duration::from_millis(2));
        app.update();
    }
    assert!(
        app.snap.frame_counter >= 10,
        "the threaded replay never produced frames: {}",
        app.snap.frame_counter
    );

    // Pause: the frame flow stops (an in-flight frame or two may land
    // after the command crosses, hence the tolerance).
    app.send(crate::bus::BusCommand::SetTracePaused(true));
    let frozen = app.snap.frame_counter;
    std::thread::sleep(std::time::Duration::from_millis(150));
    app.update();
    let paused_at = app.snap.frame_counter;
    assert!(
        paused_at >= frozen && paused_at <= frozen + 2,
        "pause must stop the replay: {frozen} -> {paused_at}"
    );

    // Resume: playback continues from where it stood.
    app.send(crate::bus::BusCommand::SetTracePaused(false));
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while std::time::Instant::now() < deadline && app.snap.frame_counter < paused_at + 10 {
        std::thread::sleep(std::time::Duration::from_millis(2));
        app.update();
    }
    assert!(
        app.snap.frame_counter >= paused_at + 10,
        "resume never continued the replay: {} (paused at {paused_at})",
        app.snap.frame_counter
    );
    drop(app);
    std::fs::remove_file(&log).ok();
}

/// Recording on the threaded drive: the draft stem must cross via
/// `SetRecordPath` before `ToggleRecord` lands, and the dated file must
/// re-open as a readable log.
#[test]
fn the_threaded_core_records_to_the_dated_file() {
    let mut app = App::new();
    let base = std::env::temp_dir().join("roxy_can_threaded_record");
    app.record_path_buf = base.to_string_lossy().to_string();
    app.toggle_record();
    app.start_virtual();
    app.send(crate::bus::BusCommand::SetEntryActive {
        ch: 0,
        id: 0x100,
        on: true,
    });
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while std::time::Instant::now() < deadline && app.snap.frame_counter < 10 {
        std::thread::sleep(std::time::Duration::from_millis(2));
        app.update();
    }
    app.stop();
    app.settle();

    let actual = app.snap.last_record.clone();
    assert!(
        actual.contains("roxy_can_threaded_record"),
        "the draft stem must reach the recorder through SetRecordPath: {actual}"
    );
    let mut stream =
        crate::log::open_stream(std::path::Path::new(&actual)).expect("the recorded file reopens");
    let frames = std::iter::from_fn(|| stream.next_frame()).count();
    assert!(frames >= 10, "expected frames on disk, got {frames}");
    drop(app);
    std::fs::remove_file(&actual).ok();
}

/// 阶段 4 验收探针：满载下"发布一帧快照"要花多久。Not a pass/fail
/// test -- run it explicitly and read the numbers:
///
/// `cargo test --release perf_snapshot_publish_under_load -- --ignored --nocapture`
///
/// The load: every signal the sample database declares (six), each history
/// filled with 72 000 points (an hour at the coarse stride), and the trace
/// ring at its full 50 000-frame limit. It times three things per round:
/// what the pre-stage-4 design paid per publish (deep-copy every history,
/// rebuild the flat trace vector), what the current design pays on a lap
/// where every cache gained samples (worst case), and one where nothing
/// changed (idle bus).
#[test]
#[ignore = "measurement probe"]
fn perf_snapshot_publish_under_load() {
    let mut app = App::headless();
    app.start_virtual();

    // Every signal the sample database declares, taken in table order.
    let mut keys: Vec<(u8, u32, String)> = Vec::new();
    let db = app.channel_dbc(0).expect("sample.dbc loaded");
    for id in &db.order {
        let msg = db.messages.get(id).expect("message table");
        for s in &msg.signals {
            keys.push((0, *id, s.name.clone()));
        }
    }
    assert!(keys.len() >= 4, "need a handful of signals to subscribe");
    for key in &keys {
        app.subscribe(key.clone());
    }

    // Fill every history: 72 000 points, 1 ms apart.
    const POINTS: u64 = 72_000;
    for (i, key) in keys.iter().enumerate() {
        let sub = app.subs.get_mut(key).expect("subscribed");
        for k in 0..POINTS {
            sub.push_sample(k * 1_000, (k % 97) as f64 + i as f64, 1_000);
        }
    }
    // Fill the trace ring to the cap.
    for i in 0..crate::app::TRACE_LIMIT {
        app.trace.push(crate::can::frame::CanFrame {
            t_us: i as u64 * 1_000,
            channel: 0,
            id: 0x100,
            extended: false,
            len: 8,
            data: [0; crate::can::frame::MAX_CAN_FD_LEN],
            dir: crate::can::frame::Direction::Rx,
            flags: crate::can::frame::FrameFlags::NONE,
        });
    }
    // One lap settles the first publish (empty view -> full view).
    app.tick(1_000_000);

    const ROUNDS: u64 = 200;

    // The old design, reproduced: what snapshot() deep-copied per publish
    // before the chunked storage existed.
    let t0 = std::time::Instant::now();
    for _ in 0..ROUNDS {
        let mut sink = 0usize;
        for key in &keys {
            let sub = app.subs.get(key).unwrap();
            let flat: Vec<(u64, f64)> = sub.history.iter().copied().collect();
            sink += flat.len();
        }
        let flat_trace: Vec<crate::can::frame::CanFrame> = app.trace.iter().copied().collect();
        sink += flat_trace.len();
        std::hint::black_box(sink);
    }
    let old_publish_us = t0.elapsed().as_micros() as f64 / ROUNDS as f64;

    // Current design, worst case: every cache resampled this lap.
    let mut now = 1_000_000u64;
    let t0 = std::time::Instant::now();
    for _ in 0..ROUNDS {
        now += 20_000;
        for key in &keys {
            if let Some(s) = app.subs.get_mut(key) {
                s.history_dirty = true;
            }
        }
        app.tick(now);
    }
    let dirty_lap_us = t0.elapsed().as_micros() as f64 / ROUNDS as f64;

    // Current design, idle: nothing resampled, publish is pure sharing.
    let t0 = std::time::Instant::now();
    for _ in 0..ROUNDS {
        now += 20_000;
        app.tick(now);
    }
    let idle_lap_us = t0.elapsed().as_micros() as f64 / ROUNDS as f64;

    eprintln!();
    eprintln!(
        "=== snapshot publish under load ({} points/curve, {} curves, {}-frame trace) ===",
        POINTS,
        keys.len(),
        crate::app::TRACE_LIMIT
    );
    eprintln!("old design, deep copy per publish : {old_publish_us:8.1} us");
    eprintln!("now, worst case (all caches dirty): {dirty_lap_us:8.1} us");
    eprintln!("now, idle (nothing resampled)     : {idle_lap_us:8.1} us");
    eprintln!(
        "worst-case ratio vs old design    : {:8.1}x cheaper",
        old_publish_us / dirty_lap_us.max(0.001)
    );
}
