//! Headless integration tests (主线阶段 2 验收): the whole application --
//! commands in, snapshot out -- runs a complete measurement and export
//! without any UI. These tests drive `advance_clock` + `tick` directly with a
//! synthetic clock, so they never touch the web host or the real wall clock.

use crate::app::{App, Mode, NodeRole};
use crate::can::frame::{CanFrame, Direction, FrameFlags, MAX_CAN_FD_LEN};
use crate::log::AscWriter;
use crate::observe::SigKey;
use crate::sim::{SrcKind, ValueSrc};

/// Advance the bus clock to synthetic `now`, then run one step.
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
    // 角色开关：EngineECU 模拟，0x100 才能发送。
    app.set_node_role(0, "EngineECU", NodeRole::Simulated);
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
        key: SigKey::can(0, 0x100, false, "EngineSpeed"),
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
        .sub_view(&SigKey::can(0, 0x100, false, "EngineSpeed"))
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
    // 角色开关：EngineECU 模拟，0x100 才能发送。
    app.set_node_role(0, "EngineECU", NodeRole::Simulated);
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
    // 角色开关：EngineECU 模拟，0x100 才能发送。
    app.set_node_role(0, "EngineECU", NodeRole::Simulated);
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
    // 角色开关：EngineECU 模拟，0x100 才能发送。
    app.set_node_role(0, "EngineECU", NodeRole::Simulated);
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
    // 角色开关：EngineECU 模拟，0x100 才能发送。
    app.set_node_role(0, "EngineECU", NodeRole::Simulated);
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
    // 角色允许发送（EngineECU 模拟 + 条目启用）。
    app.set_node_role(0, "EngineECU", NodeRole::Simulated);
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

/// Script nodes on the THREADED drive: the node runtime runs inside the
/// core thread, so prints and sends must cross the snapshot boundary back
/// to the frontend. Same shape as the headless test, live clock.
#[test]
fn the_threaded_core_runs_script_nodes() {
    let mut app = App::new();
    app.send(crate::bus::BusCommand::AddNode {
        name: "beacon".into(),
        channel: 0,
        attached: None,
    });
    // Commands are async on the threaded drive: wait for the core to
    // apply the add and publish the node before addressing it by id.
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while app.snap.nodes.is_empty() && std::time::Instant::now() < deadline {
        std::thread::sleep(std::time::Duration::from_millis(2));
        app.update();
    }
    assert!(
        !app.snap.nodes.is_empty(),
        "node never appeared in the snapshot"
    );
    let id = app.snap.nodes[0].id;
    app.send(crate::bus::BusCommand::SetNodeSource {
        id,
        source: r#"
            let n = 0;
            on start { print("up"); }
            on timer 50 {
                n = n + 1;
                print("tick", n);
                send(0x123, n);
            }
        "#
        .to_string(),
    });
    app.start_virtual();

    // The core thread arms the timer on its own clock; poll until at
    // least two ticks have been printed and two frames sent.
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while std::time::Instant::now() < deadline {
        std::thread::sleep(std::time::Duration::from_millis(5));
        app.update();
        let node = app.snap.nodes.iter().find(|n| n.id == id);
        let Some(node) = node else { continue };
        let ticks = node.log.iter().filter(|l| l.starts_with("tick ")).count();
        if ticks >= 2 {
            break;
        }
    }
    let node = &app.snap.nodes.iter().find(|n| n.id == id).expect("node");
    assert!(!node.errored, "node errored: {:?}", node.log);
    assert_eq!(node.log[0], "up");
    let ticks = node.log.iter().filter(|l| l.starts_with("tick ")).count();
    assert!(ticks >= 2, "expected periodic ticks: {:?}", node.log);
    let sent = app
        .snap
        .trace
        .iter()
        .filter(|f| f.id == 0x123 && matches!(f.dir, Direction::Tx))
        .count();
    assert!(sent >= 2, "timer frames hit the bus: {sent}");
    app.stop();
}

/// Panel actions cross the web command -> bus command -> node handler path
/// on the threaded drive; the handler's send joins the normal trace walk.
#[test]
fn the_threaded_core_runs_panel_actions() {
    let mut app = App::new();
    app.send(crate::bus::BusCommand::AddNode {
        name: "panel".into(),
        channel: 0,
        attached: None,
    });
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while app.snap.nodes.is_empty() && std::time::Instant::now() < deadline {
        std::thread::sleep(std::time::Duration::from_millis(2));
        app.update();
    }
    assert!(!app.snap.nodes.is_empty(), "node never appeared");
    let id = app.snap.nodes[0].id;
    app.send(crate::bus::BusCommand::SetNodeSource {
        id,
        source: r#"
            on panel_action("Go") {
                print("panel go");
                send(0x123, 0x5A);
            }
        "#
        .to_string(),
    });
    app.start_virtual();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while std::time::Instant::now() < deadline {
        std::thread::sleep(std::time::Duration::from_millis(5));
        app.update();
        if app.snap.nodes.iter().any(|node| node.running) {
            break;
        }
    }
    app.send(crate::bus::BusCommand::PanelAction {
        name: "Go".into(),
        value: None,
    });

    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while std::time::Instant::now() < deadline {
        std::thread::sleep(std::time::Duration::from_millis(5));
        app.update();
        let logged = app
            .snap
            .nodes
            .iter()
            .find(|node| node.id == id)
            .is_some_and(|node| node.log.iter().any(|line| line == "panel go"));
        let sent = app
            .snap
            .trace
            .iter()
            .any(|frame| frame.id == 0x123 && matches!(frame.dir, Direction::Tx));
        if logged && sent {
            break;
        }
    }
    let node = app
        .snap
        .nodes
        .iter()
        .find(|node| node.id == id)
        .expect("node");
    assert!(
        node.log.iter().any(|line| line == "panel go"),
        "panel handler did not run: {:?}",
        node.log
    );
    assert!(
        app.snap
            .trace
            .iter()
            .any(|frame| frame.id == 0x123 && matches!(frame.dir, Direction::Tx)),
        "panel handler's send never reached the trace"
    );
    app.stop();
}

/// `frame(...)` end to end: a script node sets physical fields on two
/// independent instances and sends each once; the DBC turns them into payload
/// bytes, with unwritten signals at physical 0 rather than raw 0.
#[test]
fn script_frame_instances_send_physical_values() {
    let mut app = App::headless();
    app.send(crate::bus::BusCommand::AddNode {
        name: "frame-sender".into(),
        channel: 0,
        attached: None,
    });
    let id = app.snap.nodes[0].id;
    app.send(crate::bus::BusCommand::SetNodeSource {
        id,
        source: r#"
            on timer 50 {
                let first = frame(EngineStatus);
                first.EngineSpeed = 1000;
                first.EngineTemp = 80;
                send_frame(first);
                let second = frame(EngineStatus);
                second.EngineSpeed = 2000;
                send_frame(second);
            }
        "#
        .to_string(),
    });
    app.send(crate::bus::BusCommand::SetNodeEnabled { id, on: true });
    app.start_virtual();
    let mut now = 1_000;
    for _ in 0..6 {
        now += 20_000;
        app.advance_clock(now);
        app.tick(now);
    }
    let rows: Vec<&crate::can::frame::CanFrame> = app
        .snap
        .trace
        .iter()
        .filter(|frame| frame.id == 0x100 && !frame.extended)
        .collect();
    assert!(
        rows.iter().any(|frame| {
            frame.dir == crate::can::frame::Direction::Tx && frame.data[..3] == [0xA0, 0x0F, 0x78]
        }),
        "EngineSpeed 1000 / EngineTemp 80 encoded: {:?}",
        rows.iter().map(|f| &f.data[..3]).collect::<Vec<_>>()
    );
    assert!(
        rows.iter().any(|frame| {
            frame.dir == crate::can::frame::Direction::Tx
                // EngineSpeed 2000 -> 8000 raw; unwritten EngineTemp is
                // physical 0 -> raw 40 because of the -40 offset.
                && frame.data[..3] == [0x40, 0x1F, 0x28]
        }),
        "the second instance kept its own fields"
    );
    let node = &app.snap.nodes[0];
    assert!(node.running, "{}", node.log.join("\n"));
    app.stop();
}

/// Frame instances work for FlexRay and Ethernet too; both stay in this
/// session and encode the physical fields through their descriptions.
#[test]
fn script_frame_instances_send_flexray_and_ethernet() {
    let mut app = App::headless();
    assert_eq!(
        app.load_cluster_description(crate::test_assets::path("PowerTrain.arxml"), Some(0)),
        Some(0)
    );
    assert_eq!(
        app.load_eth_description(crate::test_assets::eth_arxml(), Some(0)),
        Some(0)
    );
    app.send(crate::bus::BusCommand::AddNode {
        name: "frame-sender".into(),
        channel: 0,
        attached: None,
    });
    let id = app.snap.nodes[0].id;
    app.send(crate::bus::BusCommand::SetNodeSource {
        id,
        source: r#"
            on timer 50 {
                let f = frame(FR0.Frame_25_0_2);
                f.EngSpeed = 3000;
                send_frame(f);
                let e = frame(ETH0.EthFrame);
                e.Sig = 5;
                send_frame(e);
            }
        "#
        .to_string(),
    });
    app.send(crate::bus::BusCommand::SetNodeEnabled { id, on: true });
    app.start_virtual();
    let mut now = 1_000;
    for _ in 0..6 {
        now += 20_000;
        app.advance_clock(now);
        app.tick(now);
    }
    let fr = app
        .snap
        .fr_trace
        .iter()
        .find(|row| row.bus == 0 && row.slot == 25)
        .expect("the FR frame instance reached the trace");
    let db = app.fr_db(0).expect("cluster");
    let ix = db.frame_ix_at(25, fr.cycle, 2).expect("frame resolves");
    assert!(
        db.decode_signals(ix, &fr.payload)
            .iter()
            .any(|signal| signal.name == "EngSpeed" && (signal.phys - 3000.0).abs() < 1e-9),
        "EngSpeed physical field encoded"
    );
    let eth = app
        .snap
        .eth_trace
        .iter()
        .find(|row| row.bus == 0)
        .expect("the Ethernet frame instance reached the trace");
    let eth_db = std::sync::Arc::clone(&app.eth_buses.get(&0).expect("matrix").db);
    let flow = crate::eth::db::EthFlow {
        vlan: eth.vlan,
        dst: eth.dst,
        src: eth.src,
        ethertype: eth.ethertype,
        payload_len: eth.len(),
    };
    let decoded = eth_db.decode(&flow, &eth.payload, false).expect("decodes");
    assert!(
        decoded
            .signals
            .iter()
            .any(|signal| signal.name == "Sig" && (signal.physical - 5.0).abs() < 1e-9),
        "Sig physical field encoded"
    );
    app.stop();
}

/// Static frame checks catch a typo before the first timer fires.
#[test]
fn script_frame_checks_report_unknown_paths_and_fields() {
    let mut app = App::headless();
    app.send(crate::bus::BusCommand::AddNode {
        name: "typo".into(),
        channel: 0,
        attached: None,
    });
    let id = app.snap.nodes[0].id;
    app.send(crate::bus::BusCommand::SetNodeSource {
        id,
        source: r#"
            on timer 50 {
                let f = frame(EngineStatus);
                f.NoSuchSignal = 1;
                send_frame(f);
                let g = frame(NoSuchMessage);
                send_frame(g);
            }
        "#
        .to_string(),
    });
    app.send(crate::bus::BusCommand::SetNodeEnabled { id, on: true });
    app.start_virtual();
    let mut now = 1_000;
    for _ in 0..3 {
        now += 20_000;
        app.advance_clock(now);
        app.tick(now);
    }
    let log = app.snap.nodes[0].log.join("\n");
    assert!(
        log.contains("signal not found") && log.contains("NoSuchSignal"),
        "{log}"
    );
    assert!(log.contains("NoSuchMessage"), "{log}");
    app.stop();
}

/// `on start` has no frame context; its sends are rejected consistently for
/// raw frames and frame instances, instead of half of them firing on the
/// first timer.
#[test]
fn script_start_sends_are_rejected() {
    let mut app = App::headless();
    app.send(crate::bus::BusCommand::AddNode {
        name: "starter".into(),
        channel: 0,
        attached: None,
    });
    let id = app.snap.nodes[0].id;
    app.send(crate::bus::BusCommand::SetNodeSource {
        id,
        source: r#"
            on start {
                send(0x123, 0x01);
                let f = frame(EngineStatus);
                f.EngineSpeed = 1000;
                send_frame(f);
            }
        "#
        .to_string(),
    });
    app.send(crate::bus::BusCommand::SetNodeEnabled { id, on: true });
    app.start_virtual();
    let mut now = 1_000;
    for _ in 0..3 {
        now += 20_000;
        app.advance_clock(now);
        app.tick(now);
    }
    let log = app.snap.nodes[0].log.join("\n");
    assert!(log.contains("sends are ignored in on start"), "{log}");
    assert!(
        !app.snap
            .trace
            .iter()
            .any(|frame| frame.id == 0x123 && frame.dir == crate::can::frame::Direction::Tx),
        "the raw start send never reaches the bus"
    );
    app.stop();
}

/// The script node end to end, headless: add, edit source, start, and
/// watch `print` land in the log and `send` land on the bus -- including
/// the node reacting to its own frame.
#[test]
fn a_script_node_prints_sends_and_keeps_time() {
    let mut app = App::headless();
    app.send(crate::bus::BusCommand::AddNode {
        name: "gen".into(),
        channel: 0,
        attached: None,
    });
    let id = app.snap.nodes[0].id;
    app.send(crate::bus::BusCommand::SetNodeSource {
        id,
        source: r#"
            let n = 0;
            on start { print("up"); }
            on timer 50 {
                n = n + 1;
                print("tick", n);
                send(0x123, 0xAB);
            }
            on message 0x123 { print("echo"); }
        "#
        .to_string(),
    });
    app.send(crate::bus::BusCommand::SetNodeEnabled { id, on: true });
    app.start_virtual();

    let mut now = 1_000;
    for _ in 0..10 {
        now += 20_000;
        app.advance_clock(now);
        app.tick(now);
    }

    let node = &app.snap.nodes[0];
    assert_eq!(node.name, "gen");
    assert!(node.running, "the node compiled and started");
    assert_eq!(node.log[0], "up");
    assert!(
        node.log.iter().any(|l| l == "tick 1"),
        "the timer fired: {:?}",
        node.log
    );
    assert!(
        node.log.iter().any(|l| l == "echo"),
        "the node saw its own frame: {:?}",
        node.log
    );
    // Node frames are real bus traffic: aggregated and counted.
    let sent = app
        .trace
        .iter()
        .filter(|f| f.id == 0x123 && matches!(f.dir, Direction::Tx))
        .count();
    assert!(sent >= 2, "timer frames hit the bus: {sent}");
    assert!(app.aggs.contains_key(&(
        crate::model::ClusterId(app.snap.cluster_ids.can[0]),
        0x123,
        false
    )));

    // Disabling silences the node without removing it.
    app.send(crate::bus::BusCommand::SetNodeEnabled { id, on: false });
    let before = app.snap.frame_counter;
    now += 100_000;
    app.advance_clock(now);
    app.tick(now);
    assert_eq!(
        app.snap.frame_counter, before,
        "a disabled node stays quiet"
    );
    app.stop();
}

/// The full node signal workflow: sig() reads the live published value,
/// and the node logs it — verifying DBC + script + runtime integration.
#[test]
fn a_script_node_reads_signals_and_logs_them() {
    let mut app = App::headless();
    // 角色开关：EngineECU 模拟，0x100 才能发送。
    app.set_node_role(0, "EngineECU", NodeRole::Simulated);
    app.send(crate::bus::BusCommand::AddNode {
        name: "monitor".into(),
        channel: 0,
        attached: None,
    });
    let id = app.snap.nodes[0].id;
    app.send(crate::bus::BusCommand::SetNodeSource {
        id,
        source: r#"
                on message 0x100 {
                    let rpm = sig(0x100, "EngineSpeed");
                    print("rpm:", rpm);
                }
            "#
        .to_string(),
    });
    app.send(crate::bus::BusCommand::SetNodeEnabled { id, on: true });
    // Activate the generator so 0x100 frames flow and the node's
    // `on message 0x100` handler fires.
    app.send(crate::bus::BusCommand::SetEntryActive {
        ch: 0,
        id: 0x100,
        on: true,
    });
    app.start_virtual();

    let mut now = 1_000;
    for _ in 0..10 {
        now += 20_000;
        app.advance_clock(now);
        app.tick(now);
    }

    let node = &app.snap.nodes.iter().find(|n| n.id == id).expect("node");
    assert!(
        node.log.iter().any(|l| l.starts_with("rpm:")),
        "sig() read live values: {:?}",
        node.log
    );
    app.stop();
}

/// The responder pattern end to end: the node watches for a frame and
/// verifies that the script compiles with `frame_byte` / `frame_dlc`
/// builtins and that periodic sends land on the bus.
#[test]
fn a_script_node_responds_to_a_diagnostic_request() {
    let mut app = App::headless();
    app.send(crate::bus::BusCommand::AddNode {
        name: "responder".into(),
        channel: 0,
        attached: None,
    });
    let id = app.snap.nodes[0].id;
    app.send(crate::bus::BusCommand::SetNodeSource {
        id,
        source: r#"
                on start { print("responder up"); }
                on timer 50 {
                    send(0x200, 0xE0, 0x2E);
                }
                on message 0x7E0 {
                    print("req", frame_byte(0), frame_dlc());
                }
            "#
        .to_string(),
    });
    app.send(crate::bus::BusCommand::SetNodeEnabled { id, on: true });
    app.start_virtual();

    let mut now = 1_000;
    for _ in 0..10 {
        now += 20_000;
        app.advance_clock(now);
        app.tick(now);
    }

    let node = &app.snap.nodes.iter().find(|n| n.id == id).expect("node");
    assert!(node.running, "node compiled and started");
    // Periodic timer sends landed on the bus.
    let sent = app
        .snap
        .trace
        .iter()
        .filter(|f| f.id == 0x200 && matches!(f.dir, Direction::Tx))
        .count();
    assert!(sent >= 2, "timer frames hit the bus: {sent}");
    app.stop();
}

/// The 2026-09-09 role-model batch on the THREADED drive: role switches
/// and replay blocks must cross the command boundary and act inside the
/// core thread, observable through the snapshot.
#[test]
fn the_threaded_core_honors_node_roles_and_replay_blocks() {
    let mut app = App::new();

    // Simulating EngineECU creates its entries (0x100 among them); the
    // per-entry switches stay off — enable 0x100 explicitly (new model:
    // 角色 = 开关，条目开关 = 自定义).
    app.send(crate::bus::BusCommand::SetNodeRole {
        ch: 0,
        node: "EngineECU".to_string(),
        role: NodeRole::Simulated,
    });
    app.send(crate::bus::BusCommand::SetEntryActive {
        ch: 0,
        id: 0x100,
        on: true,
    });
    app.start_virtual();

    let zero_hundred = |app: &App| {
        app.snap
            .trace
            .iter()
            .filter(|f| f.id == 0x100 && matches!(f.dir, Direction::Tx))
            .count()
    };
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while std::time::Instant::now() < deadline && zero_hundred(&app) < 2 {
        std::thread::sleep(std::time::Duration::from_millis(5));
        app.update();
    }
    assert!(
        zero_hundred(&app) >= 2,
        "a Simulated role must drive traffic on the threaded core"
    );

    // Leaving the simulation silences the entry; the snapshot's TxView
    // and the bus's role map both read the new state.
    app.send(crate::bus::BusCommand::SetNodeRole {
        ch: 0,
        node: "EngineECU".to_string(),
        role: NodeRole::Absent,
    });
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while std::time::Instant::now() < deadline {
        app.update();
        let inactive = app
            .snap
            .tx
            .iter()
            .find(|t| t.channel == 0 && t.id == 0x100)
            .is_some_and(|t| !t.active);
        if inactive {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    assert_eq!(
        app.snap.channels[0].role_of("EngineECU"),
        NodeRole::Absent,
        "the snapshot reads the role"
    );

    // A replay block enabled mid-run streams its recorded frames onto
    // the live bus -- the block's fixture carries only id 0x777.
    let log = std::env::temp_dir().join("roxy_can_threaded_block.asc");
    let mut w = AscWriter::new(&log.to_string_lossy()).unwrap();
    for t in 0..4u64 {
        let mut f = CanFrame {
            t_us: t * 10_000,
            channel: 0,
            id: 0x777,
            extended: false,
            len: 1,
            data: [0; MAX_CAN_FD_LEN],
            dir: Direction::Rx,
            flags: FrameFlags::NONE,
        };
        f.data[0] = t as u8;
        w.write(&f).unwrap();
    }
    w.finish().unwrap();
    app.send(crate::bus::BusCommand::AddReplayBlock {
        name: "threaded blk".to_string(),
        channel: 0,
        path: log.to_string_lossy().into_owned(),
        node_filter: None,
        attached: None,
        ids: Vec::new(),
    });
    app.settle();
    let block_id = app.snap.blocks[0].id;
    app.send(crate::bus::BusCommand::SetReplayBlockEnabled {
        id: block_id,
        on: true,
    });
    let seven = |app: &App| {
        app.snap
            .trace
            .iter()
            .filter(|f| f.id == 0x777 && matches!(f.dir, Direction::Tx))
            .count()
    };
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while std::time::Instant::now() < deadline && seven(&app) < 2 {
        std::thread::sleep(std::time::Duration::from_millis(5));
        app.update();
    }
    assert!(
        seven(&app) >= 2,
        "an enabled replay block must stream on the threaded core"
    );

    app.stop();
    std::fs::remove_file(&log).ok();
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
    // 角色开关允许发送：EngineECU 模拟，0x100 才能上内部总线被录进日志。
    src.set_node_role(0, "EngineECU", NodeRole::Simulated);
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
    // 角色开关允许发送：EngineECU 模拟，0x100 才能发送。
    app.set_node_role(0, "EngineECU", NodeRole::Simulated);
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
    let mut keys: Vec<crate::observe::SigKey> = Vec::new();
    let db = app.channel_dbc(0).expect("sample.dbc loaded");
    for (id, ext) in &db.order {
        let msg = db.messages.get(&(*id, *ext)).expect("message table");
        for s in &msg.signals {
            keys.push(SigKey::can(0, *id, *ext, s.name.clone()));
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

/// The same kind of probe on the FlexRay side: what one refresh of the Trace /
/// Messages / Statistics readouts costs with a full FlexRay ring behind it.
/// Not a pass/fail test -- run it and read the numbers:
///
/// `cargo test --release perf_flexray_readouts_under_load -- --ignored --nocapture`
///
/// The load is the generated two-cluster recording
/// replayed the way the screen replays it (60 072 frames over 50.6 s, both
/// cluster descriptions loaded), so the ring sits at its cap like a long run
/// would. Each Trace shape is timed twice: the refresh that rebuilds the row
/// cache from scratch, and the ones after it -- which is what a running replay
/// pays per gate tick. Measured here (`--release`, 10 Hz text gate):
///
/// ```text
///                    rebuild   steady
/// frames only         6.2 ms    1.4 us
/// FR signals expanded 7.9 ms    2.0 us
/// filter just edited  5.8 ms    1.9 us
/// ```
///
/// Four changes sit behind those numbers, each measured against the one before
/// it. Before the row cache could extend itself, every column was the rebuild
/// figure at 7 ms / 70 ms per refresh -- a FlexRay replay with the signal
/// expansion open spent most of every 100 ms gate re-walking the ring. Then the
/// expanded rebuild came down from 51 ms to 20 ms when a child row stopped
/// carrying formatted text and started carrying its value: that text alone
/// measures 56 ms for the ring's 216 000 child rows
/// ([`crate::headless_tests::perf_flexray_child_build_split`]), for a table that
/// draws a few dozen rows at a time. And from 20 ms to 7.9 ms once the child
/// rows stopped being entries of the list at all: a frame row now carries the
/// *count* of its children -- a binary search over the description's
/// precomputed signal end bits -- and the draw resolves an ordinal against the
/// frame row it belongs to.
///
/// The expanded view therefore costs 1.7 ms more than the collapsed one on a
/// ring at its cap, and the list holds 50 000 entries either way. It did not:
/// `MAX_CACHED_ROWS` counted child rows too, so an expanded Trace silently
/// dropped older frames than the collapsed one showed.
#[test]
#[ignore = "measurement probe"]
fn perf_flexray_readouts_under_load() {
    use std::time::Instant;
    let blf = std::path::Path::new(crate::test_assets::logging_blf_two());
    let arxml = crate::test_assets::path("PowerTrain.arxml");
    let fibex = crate::test_assets::path("PowerTrain_v2.xml");
    if !blf.exists()
        || !std::path::Path::new(arxml).exists()
        || !std::path::Path::new(fibex).exists()
    {
        eprintln!("assets absent -- skipped");
        return;
    }
    let mut app = App::headless();
    assert_eq!(app.load_cluster_description(arxml, Some(0)), Some(0));
    assert_eq!(app.load_cluster_description(fibex, Some(1)), Some(1));
    // Drive it the way the screen does: a replay of the real two-cluster
    // recording, one millisecond per core lap. Feeding `ingest_fr_row` by hand
    // would leave the published ring view behind, and the readouts read that
    // view.
    app.log_path = blf.to_string_lossy().to_string();
    app.replay();
    let t0 = Instant::now();
    let mut now = 0u64;
    for _ in 0..52_000 {
        now += 1_000;
        step(&mut app, now);
    }
    let replay_ms = t0.elapsed().as_millis() as f64;
    let (fed, ring, aggs) = (
        app.snap.frame_counter as usize,
        app.snap.fr_trace.len(),
        app.snap.fr_aggs.len(),
    );
    const ROUNDS: f64 = 40.0;

    // The first refresh after the ring stopped growing walks everything; the
    // ones behind it are the steady state a running replay actually lives in,
    // so they are timed apart -- their difference is what the incremental
    // cache buys.
    let timed = |app: &mut App, f: fn(&mut App)| -> (f64, f64) {
        app.text_fresh = true;
        let t0 = Instant::now();
        f(app);
        let first = t0.elapsed().as_micros() as f64;
        let t1 = Instant::now();
        for _ in 1..ROUNDS as u32 {
            app.text_fresh = true;
            f(app);
        }
        (first, t1.elapsed().as_micros() as f64 / (ROUNDS - 1.0))
    };
    let (trace_first, trace_plain) = timed(&mut app, |a| a.sync_trace_rows(0));
    let n_plain = app.trace_windows[0].rows.len();
    app.trace_windows[0].fr_expand = true;
    let (expand_first, trace_expand) = timed(&mut app, |a| a.sync_trace_rows(0));
    let n_expand = app.trace_windows[0].rows.len();
    app.trace_windows[0].fr_expand = false;
    app.trace_windows[0].filter = "zz-nothing-matches".to_string();
    // A filter edit is a rebuild by design: the first number is that walk over
    // the whole ring, the second the steady state behind it.
    let (trace_filter_first, trace_none) = timed(&mut app, |a| a.sync_trace_rows(0));
    app.trace_windows[0].filter = String::new();

    let (_, msgs) = timed(&mut app, |a| a.sync_msg_text(0));
    let n_msgs = app.msg_windows[0].text_rows.len();
    let (_, stats) = timed(&mut app, |a| a.sync_stats_text(0));
    let n_stats = app.stats_windows[0].text_rows.len();

    eprintln!();
    eprintln!(
        "=== FlexRay readouts under load ({fed} frames replayed in {replay_ms:.0} ms of wall clock, ring {ring} rows, {aggs} slot tallies) ==="
    );
    eprintln!(
        "first refresh rebuilds the cache; the rest is what a running replay pays per gate tick ({} Hz):",
        app.text_rate_hz
    );
    eprintln!(
        "Trace, frames only   : {trace_first:9.1} us first | {trace_plain:8.1} us  ({} rows cached)",
        n_plain
    );
    eprintln!(
        "Trace, FR signals on : {expand_first:9.1} us first | {trace_expand:8.1} us  ({n_expand} rows cached)"
    );
    eprintln!(
        "Trace, filter edited : {trace_filter_first:9.1} us first | {trace_none:8.1} us  (nothing matched)"
    );
    eprintln!("Messages             :   (rebuild n/a)  | {msgs:8.1} us  ({n_msgs} rows)");
    eprintln!("Statistics           :   (rebuild n/a)  | {stats:8.1} us  ({n_stats} rows)");
}

/// Where an expanded Trace rebuild's time goes, so the fix can be aimed rather
/// than guessed: resolving which frame each row carries, reading the signal
/// values out of the payloads, building the cache rows, and formatting the
/// values as text. Not a pass/fail test -- run it and read the numbers:
///
/// `cargo test --release perf_flexray_child_build_split -- --ignored --nocapture`
///
/// Every round folds something out of the values it read into a printed sink.
/// Without that, LLVM deletes the arithmetic and the line reports the cost of
/// the loop counter alone -- which is how one run of this probe claimed the
/// whole walk cost 4 ns a row. Measured here (`--release`, 50 000-row ring,
/// 216 661 child rows, 88 B per cached row):
///
/// ```text
/// resolve frames:     0.9 ms   which frame each row carries
/// read values    :    4.2 ms   every signal's bits and physical value
/// build rows     :    5.2 ms   the 50 000 cache rows, each with its child count
/// format text    :   56.4 ms   the value text nobody had asked to see
/// ```
///
/// The first three are roughly what an expanded rebuild pays now (7.9 ms,
/// against the 51 ms all four used to cost, and 6.2 ms for a collapsed one --
/// see [`crate::headless_tests::perf_flexray_readouts_under_load`]). The last is
/// left to the rows actually drawn, and "read values" with it: the list keeps a
/// count per frame row, not the values, so a rebuild walks the description's
/// precomputed signal end bits rather than the ring's payloads.
#[test]
#[ignore = "measurement probe"]
fn perf_flexray_child_build_split() {
    use crate::workspace::TraceRow;
    use std::cell::Cell;
    use std::time::Instant;
    let blf = std::path::Path::new(crate::test_assets::logging_blf_two());
    let arxml = crate::test_assets::path("PowerTrain.arxml");
    let fibex = crate::test_assets::path("PowerTrain_v2.xml");
    if !blf.exists() {
        eprintln!("assets absent -- skipped");
        return;
    }
    let mut app = App::headless();
    app.load_cluster_description(arxml, Some(0));
    app.load_cluster_description(fibex, Some(1));
    app.log_path = blf.to_string_lossy().to_string();
    app.replay();
    let mut now = 0u64;
    for _ in 0..52_000 {
        now += 1_000;
        step(&mut app, now);
    }
    let ring: Vec<_> = app.snap.fr_trace.iter().collect();
    eprintln!("ring {} rows", ring.len());
    let sink = Cell::new(0u64);
    let kids = Cell::new(0usize);
    let frames = Cell::new(0usize);
    let rounds = |f: &mut dyn FnMut()| -> f64 {
        let t = Instant::now();
        f();
        t.elapsed().as_micros() as f64
    };
    let resolve = rounds(&mut || {
        let (mut acc, mut n) = (0u64, 0usize);
        for r in &ring {
            let d = app.fr_db(r.bus).unwrap();
            if let Some(ix) = d.frame_ix_at(r.slot, r.cycle, r.ab) {
                acc += ix as u64;
                n += 1;
            }
        }
        sink.set(acc);
        frames.set(n);
    });
    // What the deferred walk costs: the same frame lookups plus every signal's
    // bits read out of its payload, no text.
    let values = rounds(&mut || {
        let (mut acc, mut n) = (0u64, 0usize);
        for r in &ring {
            let d = app.fr_db(r.bus).unwrap();
            let Some(ix) = d.frame_ix_at(r.slot, r.cycle, r.ab) else {
                continue;
            };
            for (child_ix, raw, phys) in d.frame_values(ix, &r.payload) {
                acc += raw ^ phys.to_bits() ^ child_ix as u64;
                n += 1;
            }
        }
        sink.set(acc);
        kids.set(n);
    });
    // The same walk, with each frame turned into one cache row carrying its
    // child count -- what the rebuild pays for the row list now.
    let mut rows: Vec<TraceRow> = Vec::with_capacity(60_000);
    let build = rounds(&mut || {
        rows.clear();
        let mut acc = 0u64;
        for r in &ring {
            let d = app.fr_db(r.bus).unwrap();
            let kids = match d.frame_ix_at(r.slot, r.cycle, r.ab) {
                Some(ix) => d.frame_values(ix, &r.payload).count() as u32,
                None => 0,
            };
            acc += kids as u64;
            rows.push(TraceRow::Fr((**r).clone(), kids));
        }
        sink.set(acc);
        kids.set(rows.iter().map(|r| r.kids() as usize).sum());
    });
    // The text, which is what the rebuild used to do for all of them.
    let decode = rounds(&mut || {
        let mut acc = 0u64;
        for r in &ring {
            let d = app.fr_db(r.bus).unwrap();
            if let Some(ix) = d.frame_ix_at(r.slot, r.cycle, r.ab) {
                for (name, text) in d.decode(ix, &r.payload) {
                    acc += (name.len() + text.len()) as u64;
                }
            }
        }
        sink.set(acc);
    });
    eprintln!(
        "resolve frames: {resolve:9.1} us ({} of {} rows)",
        frames.get(),
        ring.len()
    );
    eprintln!(
        "read values   : {values:9.1} us ({} child rows)",
        kids.get()
    );
    eprintln!(
        "build rows    : {build:9.1} us ({} cache rows, {} table rows, {} B each)",
        ring.len(),
        kids.get(),
        std::mem::size_of::<TraceRow>()
    );
    eprintln!(
        "format text   : {decode:9.1} us ({} child rows)",
        kids.get()
    );
    eprintln!(
        "sink (kept so nothing above is optimised out): {}",
        sink.get()
    );
    app.stop();
}

/// The Bus Statistics window reads its rollups from the snapshot like every
/// other window: traffic on a bus must show up as per-class totals and a
/// nonzero load figure without touching live bus state.
#[test]
fn the_bus_statistics_read_the_snapshot() {
    let mut app = App::headless();
    // 角色开关：EngineECU 模拟，0x100 才能发送。
    app.set_node_role(0, "EngineECU", NodeRole::Simulated);
    app.start_virtual();
    app.send(crate::bus::BusCommand::SetEntryActive {
        ch: 0,
        id: 0x100,
        on: true,
    });
    let mut now = 1_000;
    for _ in 0..500 {
        now += 2_000;
        app.advance_clock(now);
        app.tick(now);
    }
    let load = app
        .snap
        .bus_loads
        .first()
        .expect("bus 0 has rollups in the snapshot");
    assert!(
        load.class_total(crate::load::FrameClass::StdData) > 0,
        "generated frames reached the load rollups"
    );
    assert!(load.load() > 0.0, "wire time shows up in the windowed load");
    app.stop();
}

/// The Send-now button: one frame off the schedule. The entry keeps its
/// active flag, nothing further is sent by it, and a request made while
/// stopped is dropped rather than shelved for some future run.
#[test]
fn send_now_fires_exactly_one_frame_off_the_schedule() {
    let mut app = App::headless();
    // 角色开关：EngineECU 模拟，0x100 才能发送。
    app.set_node_role(0, "EngineECU", NodeRole::Simulated);
    app.start_virtual();
    app.send(crate::bus::BusCommand::SetEntryActive {
        ch: 0,
        id: 0x100,
        on: false,
    });
    let mut now = 1_000;
    let laps = |app: &mut App, n: usize, now: &mut u64| {
        for _ in 0..n {
            *now += 10_000;
            app.advance_clock(*now);
            app.tick(*now);
        }
    };
    laps(&mut app, 5, &mut now);
    assert_eq!(app.snap.frame_counter, 0, "the entry is off: silence");

    app.send(crate::bus::BusCommand::SendNow { ch: 0, id: 0x100 });
    laps(&mut app, 1, &mut now);
    assert_eq!(app.snap.frame_counter, 1, "exactly one frame went out");
    laps(&mut app, 5, &mut now);
    assert_eq!(app.snap.frame_counter, 1, "no schedule picks up afterwards");
    assert!(
        !app.snap.tx.iter().any(|t| t.id == 0x100 && t.active),
        "send-now never flips the entry on"
    );

    // While stopped the request is dropped, not shelved for the next run.
    app.stop();
    app.send(crate::bus::BusCommand::SendNow { ch: 0, id: 0x100 });
    laps(&mut app, 2, &mut now);
    assert_eq!(app.snap.frame_counter, 1, "a stopped bus drops the request");
    app.start_virtual();
    laps(&mut app, 3, &mut now);
    assert_eq!(
        app.snap.frame_counter, 0,
        "nothing fires into the fresh run either"
    );
    app.stop();
}

/// A Panel setpoint writes a physical signal: the payload changes, the
/// signal's waveform source is dropped so the value holds, and exactly one
/// frame goes out even though the generator entry is off.
#[test]
fn panel_setpoints_write_a_dbc_signal() {
    let mut app = App::headless();
    app.set_node_role(0, "EngineECU", NodeRole::Simulated);
    app.start_virtual();
    app.send(crate::bus::BusCommand::SetEntryActive {
        ch: 0,
        id: 0x100,
        on: false,
    });
    let mut now = 1_000;
    let laps = |app: &mut App, n: usize, now: &mut u64| {
        for _ in 0..n {
            *now += 10_000;
            app.advance_clock(*now);
            app.tick(*now);
        }
    };
    laps(&mut app, 3, &mut now);
    let entry = |app: &App| {
        app.snap
            .tx
            .iter()
            .find(|t| t.channel == 0 && t.id == 0x100)
            .cloned()
            .expect("0x100 entry")
    };
    let before = entry(&app).data_text.clone();
    // Drive the signal with a waveform first: the write must pin it.
    app.send(crate::bus::BusCommand::SetEntrySource {
        ch: 0,
        id: 0x100,
        src: crate::sim::ValueSrc::new("EngineSpeed", crate::sim::SrcKind::Sine, 0.0, 5000.0),
    });
    laps(&mut app, 1, &mut now);
    assert!(
        entry(&app).srcs.iter().any(|src| src.name == "EngineSpeed"),
        "the waveform drives EngineSpeed before the write"
    );

    app.send(crate::bus::BusCommand::PanelWriteSignal {
        key: crate::observe::SigKey::can(0, 0x100, false, "EngineSpeed"),
        value: 1234.5,
    });
    laps(&mut app, 1, &mut now);
    assert_eq!(app.snap.frame_counter, 1, "one frame from the write");
    assert_ne!(entry(&app).data_text, before, "the payload changed");
    assert!(
        !entry(&app).srcs.iter().any(|src| src.name == "EngineSpeed"),
        "the written signal is held, not driven by its waveform"
    );

    // Out-of-range input is clamped into the DBC bounds.
    let table = crate::dbc::load_dbc_str(crate::sample_data::SAMPLE_DBC).expect("sample database");
    let max = table
        .message_of(0x100)
        .and_then(|message| {
            message
                .signals
                .iter()
                .find(|signal| signal.name == "EngineSpeed")
        })
        .expect("EngineSpeed")
        .max;
    let len = table
        .message_of(0x100)
        .expect("EngineStatus")
        .dlc
        .min(crate::can::frame::MAX_CAN_FD_LEN as u64) as u8;
    app.send(crate::bus::BusCommand::PanelWriteSignal {
        key: crate::observe::SigKey::can(0, 0x100, false, "EngineSpeed"),
        value: max + 10_000.0,
    });
    laps(&mut app, 1, &mut now);
    let mut expected = [0u8; crate::can::frame::MAX_CAN_FD_LEN];
    assert!(table.encode_signal(0x100, "EngineSpeed", max, &mut expected));
    let written = entry(&app);
    assert_eq!(
        written.data_text,
        crate::generator::hex_text(&expected, len),
        "the write stops at the DBC maximum"
    );

    app.send(crate::bus::BusCommand::PanelWriteSignal {
        key: crate::observe::SigKey::can(0, 0x100, false, "NoSuchSignal"),
        value: 1.0,
    });
    laps(&mut app, 1, &mut now);
    assert!(
        app.status.contains("unknown signal"),
        "an unknown signal reports instead of sending: {}",
        app.status
    );
    app.stop();
}

/// A Panel setpoint writes a FlexRay signal the same way: the encoded frame
/// lands in this session's trace, decodes back to the written value, and its
/// readback subscription owns the live value.
#[test]
fn panel_setpoints_write_a_flexray_signal() {
    let mut app = App::headless();
    let arxml = crate::test_assets::path("PowerTrain.arxml");
    assert_eq!(app.load_cluster_description(arxml, Some(0)), Some(0));
    app.start_virtual();
    let mut now = 1_000;
    let laps = |app: &mut App, n: usize, now: &mut u64| {
        for _ in 0..n {
            *now += 10_000;
            app.advance_clock(*now);
            app.tick(*now);
        }
    };
    let key = crate::observe::SigKey::fr(0, 25, "EngSpeed");
    app.subscribe(key.clone());
    laps(&mut app, 1, &mut now);

    let rows = app.snap.fr_trace.len();
    app.send(crate::bus::BusCommand::PanelWriteSignal {
        key: key.clone(),
        value: 3000.0,
    });
    laps(&mut app, 1, &mut now);
    assert_eq!(app.snap.fr_trace.len(), rows + 1, "one FlexRay frame out");
    let row = app.snap.fr_trace.last().expect("the written frame");
    assert_eq!((row.bus, row.slot), (0, 25));
    let db = app.fr_db(0).expect("cluster description");
    let frame_ix = db
        .frame_ix_at(row.slot, row.cycle, row.ab)
        .expect("the row resolves back to its frame");
    assert!(
        db.decode_signals(frame_ix, &row.payload)
            .iter()
            .any(|signal| signal.name == "EngSpeed" && (signal.phys - 3000.0).abs() < 1e-9),
        "the payload decodes to the written value"
    );
    assert_eq!(
        app.sub_view(&key).map(|sub| sub.latest),
        Some(3000.0),
        "the readback subscription sees the frame"
    );

    app.send(crate::bus::BusCommand::PanelWriteSignal {
        key: crate::observe::SigKey::fr(0, 25, "NoSuchSignal"),
        value: 1.0,
    });
    laps(&mut app, 1, &mut now);
    assert!(
        app.status.contains("unknown FlexRay signal"),
        "an unknown FlexRay signal reports instead of sending: {}",
        app.status
    );
    app.stop();
}

/// A Button preset writes its signals in one command: good entries land, and
/// bad keys are reported together instead of aborting the whole preset.
#[test]
fn panel_presets_write_each_signal_and_report_failures() {
    let mut app = App::headless();
    assert_eq!(
        app.load_cluster_description(crate::test_assets::path("PowerTrain.arxml"), Some(0)),
        Some(0)
    );
    assert_eq!(
        app.load_eth_description(crate::test_assets::eth_arxml(), Some(0)),
        Some(0)
    );
    app.start_virtual();
    let mut now = 1_000;
    let laps = |app: &mut App, n: usize, now: &mut u64| {
        for _ in 0..n {
            *now += 10_000;
            app.advance_clock(*now);
            app.tick(*now);
        }
    };
    laps(&mut app, 1, &mut now);
    let (fr_before, eth_before) = (app.snap.fr_trace.len(), app.snap.eth_trace.len());

    app.send(crate::bus::BusCommand::PanelWriteSignals {
        writes: vec![
            ("0:256:EngineSpeed".to_string(), 1234.0),
            ("fr:0:25:EngSpeed".to_string(), 2000.0),
            ("eth:0:EthFrame:Sig".to_string(), 4.5),
            ("nope".to_string(), 1.0),
            ("derived:Test".to_string(), 1.0),
        ],
    });
    laps(&mut app, 1, &mut now);

    assert_eq!(
        app.snap.fr_trace.len(),
        fr_before + 1,
        "the FR entry landed"
    );
    assert_eq!(
        app.snap.eth_trace.len(),
        eth_before + 1,
        "the ETH entry landed"
    );
    assert!(
        app.status.contains("2 write(s) failed")
            && app.status.contains("nope")
            && app.status.contains("derived"),
        "both bad entries are reported together: {}",
        app.status
    );
    app.stop();
}

/// A Panel "Send message" button sends exactly one raw CAN frame, and refuses
/// an oversized ID or a non-CAN-FD payload length instead of padding it.
#[test]
fn panel_buttons_send_one_raw_can_frame() {
    let mut app = App::headless();
    app.start_virtual();
    let mut now = 1_000;
    let laps = |app: &mut App, n: usize, now: &mut u64| {
        for _ in 0..n {
            *now += 10_000;
            app.advance_clock(*now);
            app.tick(*now);
        }
    };
    laps(&mut app, 1, &mut now);

    app.send(crate::bus::BusCommand::PanelSendFrame {
        channel: 0,
        id: 0x7df,
        extended: false,
        data: vec![0x02, 0x10, 0x03],
        physical: false,
        signals: Default::default(),
    });
    laps(&mut app, 1, &mut now);
    let frame = app
        .snap
        .trace
        .iter()
        .rev()
        .find(|frame| frame.id == 0x7df)
        .expect("the raw frame reached the trace");
    assert_eq!(
        (frame.len, frame.dir),
        (3, crate::can::frame::Direction::Tx)
    );
    assert_eq!(&frame.data[..3], &[0x02, 0x10, 0x03]);
    assert!(!frame.flags.contains(crate::can::frame::FrameFlags::FD));

    app.send(crate::bus::BusCommand::PanelSendFrame {
        channel: 0,
        id: 0x123,
        extended: false,
        data: vec![0xAA; 12],
        physical: false,
        signals: Default::default(),
    });
    laps(&mut app, 1, &mut now);
    let fd = app
        .snap
        .trace
        .iter()
        .rev()
        .find(|frame| frame.id == 0x123)
        .expect("the FD frame reached the trace");
    assert_eq!(fd.len, 12);
    assert!(fd.flags.contains(crate::can::frame::FrameFlags::FD));

    app.send(crate::bus::BusCommand::PanelSendFrame {
        channel: 0,
        id: 0x800,
        extended: false,
        data: vec![1],
        physical: false,
        signals: Default::default(),
    });
    laps(&mut app, 1, &mut now);
    assert!(
        app.status.contains("standard IDs stop at 0x7FF"),
        "{}",
        app.status
    );
    app.send(crate::bus::BusCommand::PanelSendFrame {
        channel: 0,
        id: 0x123,
        extended: false,
        data: vec![0; 9],
        physical: false,
        signals: Default::default(),
    });
    laps(&mut app, 1, &mut now);
    assert!(
        app.status.contains("CAN FD data must be"),
        "a 9-byte payload reports instead of padding: {}",
        app.status
    );

    app.stop();
    app.send(crate::bus::BusCommand::PanelSendFrame {
        channel: 0,
        id: 0x7df,
        extended: false,
        data: vec![1],
        physical: false,
        signals: Default::default(),
    });
    laps(&mut app, 1, &mut now);
    assert!(app.status.contains("running measurement"), "{}", app.status);
}

/// Raw FlexRay and Ethernet sends stay inside this session: the FR row lands
/// with its slot payload, the ETH row matches its matrix identity, and wrong
/// addresses or payload lengths report instead of being padded.
#[test]
fn panel_buttons_send_raw_flexray_and_ethernet_frames() {
    let mut app = App::headless();
    assert_eq!(
        app.load_cluster_description(crate::test_assets::path("PowerTrain.arxml"), Some(0)),
        Some(0)
    );
    assert_eq!(
        app.load_eth_description(crate::test_assets::eth_arxml(), Some(0)),
        Some(0)
    );
    app.start_virtual();
    let mut now = 1_000;
    let laps = |app: &mut App, n: usize, now: &mut u64| {
        for _ in 0..n {
            *now += 10_000;
            app.advance_clock(*now);
            app.tick(*now);
        }
    };
    laps(&mut app, 1, &mut now);

    app.send(crate::bus::BusCommand::PanelSendFrFrame {
        bus: 0,
        slot: 25,
        data: vec![0xAA; 26],
        physical: false,
        signals: Default::default(),
    });
    laps(&mut app, 1, &mut now);
    let fr = app.snap.fr_trace.last().expect("the FR row");
    assert_eq!((fr.bus, fr.slot, fr.payload.len()), (0, 25, 26));
    assert_eq!(fr.payload[0], 0xAA);

    app.send(crate::bus::BusCommand::PanelSendEthFrame {
        bus: 0,
        frame: "EthFrame".to_string(),
        data: vec![0x5A; 22],
        physical: false,
        signals: Default::default(),
    });
    laps(&mut app, 1, &mut now);
    let eth = app.snap.eth_trace.last().expect("the ETH row");
    assert_eq!((eth.bus, eth.len(), eth.vlan), (0, 22, Some(100)));
    assert_eq!(eth.payload[0], 0x5A);
    let db = std::sync::Arc::clone(&app.eth_buses.get(&0).expect("matrix").db);
    let flow = crate::eth::db::EthFlow {
        vlan: eth.vlan,
        dst: eth.dst,
        src: eth.src,
        ethertype: eth.ethertype,
        payload_len: eth.len(),
    };
    assert!(db.decode(&flow, &eth.payload, false).is_some());

    app.send(crate::bus::BusCommand::PanelSendFrFrame {
        bus: 0,
        slot: 0,
        data: vec![1],
        physical: false,
        signals: Default::default(),
    });
    laps(&mut app, 1, &mut now);
    assert!(
        app.status.contains("slot must be 1..2047"),
        "{}",
        app.status
    );
    app.send(crate::bus::BusCommand::PanelSendEthFrame {
        bus: 0,
        frame: "EthFrame".to_string(),
        data: vec![1],
        physical: false,
        signals: Default::default(),
    });
    laps(&mut app, 1, &mut now);
    assert!(
        app.status.contains("carries 22 payload bytes"),
        "{}",
        app.status
    );
    app.stop();
}

/// A physical Panel send encodes catalog signals into the declared frame:
/// named fields are clamped and written, untouched ones keep their start
/// value (physical 0 in this database).
#[test]
fn panel_buttons_send_physical_can_signals() {
    let mut app = App::headless();
    app.start_virtual();
    let mut now = 1_000;
    let laps = |app: &mut App, n: usize, now: &mut u64| {
        for _ in 0..n {
            *now += 10_000;
            app.advance_clock(*now);
            app.tick(*now);
        }
    };
    laps(&mut app, 1, &mut now);

    let mut signals = std::collections::BTreeMap::new();
    signals.insert("EngineSpeed".to_string(), 1000.0);
    signals.insert("EngineTemp".to_string(), 80.0);
    app.send(crate::bus::BusCommand::PanelSendFrame {
        channel: 0,
        id: 0x100,
        extended: false,
        data: Vec::new(),
        physical: true,
        signals,
    });
    laps(&mut app, 1, &mut now);
    let frame = app
        .snap
        .trace
        .iter()
        .rev()
        .find(|frame| frame.id == 0x100 && frame.data[..3] == [0xA0, 0x0F, 0x78])
        .expect("the physical frame reached the trace");
    assert_eq!(frame.len, 8, "the declared DLC wins");
    assert_eq!(frame.data[3], 0, "ThrottlePos keeps physical 0");

    app.send(crate::bus::BusCommand::PanelSendFrame {
        channel: 0,
        id: 0x100,
        extended: false,
        data: Vec::new(),
        physical: true,
        signals: [("NotASignal".to_string(), 1.0)].into_iter().collect(),
    });
    laps(&mut app, 1, &mut now);
    assert!(
        app.status.contains("has no signal \"NotASignal\""),
        "{}",
        app.status
    );
    app.stop();
}

/// Physical sends cover FlexRay and Ethernet too: the encoded session frame
/// keeps the declared width and unknown signal names report.
#[test]
fn panel_buttons_send_physical_flexray_and_ethernet_signals() {
    let mut app = App::headless();
    assert_eq!(
        app.load_cluster_description(crate::test_assets::path("PowerTrain.arxml"), Some(0)),
        Some(0)
    );
    assert_eq!(
        app.load_eth_description(crate::test_assets::eth_arxml(), Some(0)),
        Some(0)
    );
    app.start_virtual();
    let mut now = 1_000;
    let laps = |app: &mut App, n: usize, now: &mut u64| {
        for _ in 0..n {
            *now += 10_000;
            app.advance_clock(*now);
            app.tick(*now);
        }
    };
    laps(&mut app, 1, &mut now);

    app.send(crate::bus::BusCommand::PanelSendFrFrame {
        bus: 0,
        slot: 25,
        data: Vec::new(),
        physical: true,
        signals: [("EngSpeed".to_string(), 1234.0)].into_iter().collect(),
    });
    laps(&mut app, 1, &mut now);
    let fr = app.snap.fr_trace.last().expect("the FR row");
    assert_eq!((fr.bus, fr.slot, fr.payload.len()), (0, 25, 26));
    assert!(
        fr.payload.iter().any(|byte| *byte != 0),
        "the signal reached the payload"
    );

    app.send(crate::bus::BusCommand::PanelSendEthFrame {
        bus: 0,
        frame: "EthFrame".to_string(),
        data: Vec::new(),
        physical: true,
        signals: [("Sig".to_string(), 7.0)].into_iter().collect(),
    });
    laps(&mut app, 1, &mut now);
    let eth = app.snap.eth_trace.last().expect("the ETH row");
    assert_eq!((eth.bus, eth.len(), eth.vlan), (0, 22, Some(100)));
    assert!(
        eth.payload.iter().any(|byte| *byte != 0),
        "the signal reached the payload"
    );

    app.send(crate::bus::BusCommand::PanelSendFrFrame {
        bus: 0,
        slot: 25,
        data: Vec::new(),
        physical: true,
        signals: [("NotASignal".to_string(), 1.0)].into_iter().collect(),
    });
    laps(&mut app, 1, &mut now);
    assert!(
        app.status.contains("has no signal \"NotASignal\""),
        "{}",
        app.status
    );
    app.stop();
}

/// A Panel setpoint writes an Ethernet signal too: the encoded frame enters
/// this session's trace, matches the matrix by VLAN/length, and its readback
/// subscription owns the live value. No port is touched.
#[test]
fn panel_setpoints_write_an_ethernet_signal() {
    let mut app = App::headless();
    assert_eq!(
        app.load_eth_description(crate::test_assets::eth_arxml(), Some(0)),
        Some(0)
    );
    app.start_virtual();
    let mut now = 1_000;
    let laps = |app: &mut App, n: usize, now: &mut u64| {
        for _ in 0..n {
            *now += 10_000;
            app.advance_clock(*now);
            app.tick(*now);
        }
    };
    let key = crate::observe::SigKey::eth(0, "EthFrame", "Sig");
    app.subscribe(key.clone());
    laps(&mut app, 1, &mut now);

    let rows = app.snap.eth_trace.len();
    app.send(crate::bus::BusCommand::PanelWriteSignal {
        key: key.clone(),
        value: 5.0,
    });
    laps(&mut app, 1, &mut now);
    assert_eq!(app.snap.eth_trace.len(), rows + 1, "one Ethernet frame out");
    let row = app.snap.eth_trace.last().expect("the written frame");
    assert_eq!(row.bus, 0);
    assert_eq!(row.dir, crate::can::frame::Direction::Tx);
    assert_eq!(row.vlan, Some(100), "the matrix's VLAN");
    assert_eq!(
        row.ethertype,
        crate::eth::EthFrame::SESSION_ETHERTYPE,
        "no declared EtherType, so the session value is used"
    );
    let db = std::sync::Arc::clone(&app.eth_buses.get(&0).expect("matrix").db);
    let flow = crate::eth::db::EthFlow {
        vlan: row.vlan,
        dst: row.dst,
        src: row.src,
        ethertype: row.ethertype,
        payload_len: row.len(),
    };
    let decoded = db
        .decode(&flow, &row.payload, app.eth_length_includes_header)
        .expect("the injected flow matches the matrix");
    assert!(
        decoded
            .signals
            .iter()
            .any(|signal| signal.name == "Sig" && (signal.physical - 5.0).abs() < 1e-9),
        "the payload decodes to the written value"
    );
    assert_eq!(
        app.sub_view(&key).map(|sub| sub.latest),
        Some(5.0),
        "the readback subscription sees the frame"
    );

    app.send(crate::bus::BusCommand::PanelWriteSignal {
        key: crate::observe::SigKey::eth(0, "EthFrame", "NoSuchSignal"),
        value: 1.0,
    });
    laps(&mut app, 1, &mut now);
    assert!(
        app.status.contains("cannot encode"),
        "an unknown Ethernet signal reports instead of sending: {}",
        app.status
    );
    app.stop();
}
