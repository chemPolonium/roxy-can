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
    app.core.advance_clock(now);
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
    let mut app = App::new();
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
    let mut app = App::new();
    let path = std::env::temp_dir().join("roxy_can_headless_record.asc");
    app.recorder.record_path = path.to_string_lossy().to_string();
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
    let mut app = App::new();
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

    let mut app = App::new();
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
    let mut app = App::new();
    app.start_virtual();
    app.send(crate::bus::BusCommand::SetEntryActive {
        ch: 0,
        id: 0x100,
        on: true,
    });
    // 0x100 runs on the default 100 ms period, anchored at sim 0: the
    // first slot is due immediately.
    assert_eq!(app.core.next_deadline(1_000_000), Some(1_000_000));
    // Run 50 ms of steps; the next slot then sits 50 ms out.
    let mut now = 0;
    for _ in 0..50 {
        now += 1_000;
        app.core.advance_clock(now);
        app.tick(now);
    }
    assert_eq!(app.core.next_deadline(now), Some(now + 50_000));
    // An idle (all-off) bus has no deadline at all.
    app.send(crate::bus::BusCommand::SetEntryActive {
        ch: 0,
        id: 0x100,
        on: false,
    });
    assert_eq!(app.core.next_deadline(now), None);
}

#[test]
fn step_to_advances_the_clocks_and_the_bus() {
    let mut app = App::new();
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
        let done = app
            .core
            .step_to(now, stride, app.spec_tol_pct, app.spec_grace, &mut status);
        assert!(!done);
        app.refresh_snapshot();
    }
    // Slots at 0, 100k .. 6_000k = 61 frames across six 1 s wakes.
    assert_eq!(app.snap.frame_counter, 61, "{}", app.snap.frame_counter);
}
