#[test]
fn debug_default_model_availability() {
    let app = App::headless();
    let f = rx_frame(0, 0x100, 2, FrameFlags::NONE);
    let comm = app.model_comms[0].as_ref().unwrap();
    let frames: Vec<String> = comm
        .frames
        .values()
        .map(|frame| format!("{} {:?}", frame.name, frame.address))
        .collect();
    let db_ok = app.model_dbs[0].is_some();
    let frame = comm.frames_in_cluster(app.can_cluster_ids[0]).find(|frame| {
        matches!(frame.address, crate::model::FrameAddress::Can { id, extended, .. } if id == 0x100 && !extended)
    });
    let plan_ok =
        frame.is_some_and(|frame| app.model_plans[0].as_ref().unwrap().get(frame.id).is_some());
    let frame = frame.unwrap();
    let plan = app.model_plans[0]
        .as_ref()
        .unwrap()
        .get(frame.id)
        .unwrap()
        .clone();
    let decoded = crate::model::decode_payload(comm, &plan, &f.data[..f.len as usize]);
    assert!(
        app.model_can_all_values(&f).is_some(),
        "model missing: db_ok={db_ok} frames={frames:?} plan_ok={plan_ok} reads={} decoded={decoded:?} signals={}",
        plan.reads.len(),
        comm.signals.len()
    );
}

fn quiet_app() -> App {
    let mut app = App::headless();
    app.start_virtual();
    // Only frames built by the test may reach the bus statistics.
    app.tx_list.retain(|t| t.channel != 0);
    app
}

/// The load view's whole point: the same traffic reads differently at
/// different bitrates, and each number agrees with a hand calculation.
/// 100 classic 8-byte frames over one second are 111 bits each, 222 碌s of
/// wire time at 500 kbit/s -- 2.22 % of the bus.
#[test]
fn bus_load_matches_the_hand_calculation() {
    let mut app = quiet_app();
    let frames: Vec<CanFrame> = (0..100)
        .map(|i| rx_frame((i + 1) * 10_000, 0x100, 8, FrameFlags::NONE))
        .collect();
    receive(&mut app, 1_000_000, frames);
    assert!((app.bus_loads[0].frame_rate() - 100.0).abs() < 1e-9);
    assert!(
        (app.bus_loads[0].load() - 0.0222).abs() < 1e-9,
        "got {}, expected 2.22 %",
        app.bus_loads[0].load()
    );
    app.stop();
}

/// A 64-byte BRS payload clocks out of the data phase, so the same frame
/// stream is far cheaper at a 2 Mbit/s data phase than at 500 kbit/s --
/// the acceptance case from the capability backlog, and exactly what a
/// frame-counting "load" would get wrong.
#[test]
fn a_brs_payload_gets_cheaper_at_a_faster_data_phase() {
    let mut app = quiet_app();
    let frames: Vec<CanFrame> = (0..100)
        .map(|i| {
            rx_frame(
                (i + 1) * 10_000,
                0x200,
                64,
                FrameFlags::FD.union(FrameFlags::BRS),
            )
        })
        .collect();
    // 55 arbitration bits at 500 kbit/s + 552 data bits at 2 Mbit/s =
    // 110 + 276 = 386 碌s per frame -> 3.86 % load.
    receive(&mut app, 1_000_000, frames.clone());
    assert!(
        (app.bus_loads[0].load() - 0.0386).abs() < 1e-9,
        "got {}, expected 3.86 %",
        app.bus_loads[0].load()
    );
    // Same frames, data phase throttled to the arbitration rate: all 607
    // bits at 500 kbit/s = 1214 碌s -> 12.14 %.
    app.set_can_bitrates_for_test(0, None, Some(500));
    for load in &mut app.bus_loads {
        load.clear();
    }
    receive(&mut app, 3_000_000, frames);
    assert!(
        (app.bus_loads[0].load() - 0.1214).abs() < 1e-9,
        "got {}, expected 12.14 %",
        app.bus_loads[0].load()
    );
    app.stop();
}

/// Error frames never enter per-message aggregation, but the bus view
/// must still report them: they occupy the bus and are the thing you are
/// usually hunting.
#[test]
fn error_frames_are_counted_per_bus() {
    let mut app = quiet_app();
    receive(
        &mut app,
        1_000,
        vec![rx_frame(1_000, 0x300, 0, FrameFlags::ERROR)],
    );
    assert_eq!(app.bus_loads[0].errors, 1);
    receive(
        &mut app,
        2_000,
        vec![rx_frame(2_000, 0x300, 0, FrameFlags::ERROR)],
    );
    assert_eq!(app.bus_loads[0].errors, 2);
    assert!(
        app.bus_loads[1].errors == 0,
        "bus 1 saw nothing and says so"
    );
    app.stop();
}

/// A fresh run must not inherit the previous run's load: the window is
/// cleared with the aggregates it accompanies.
#[test]
fn restarting_measurement_clears_the_bus_windows() {
    let mut app = quiet_app();
    receive(
        &mut app,
        1_000,
        vec![rx_frame(1_000, 0x100, 8, FrameFlags::NONE)],
    );
    assert!(app.bus_loads[0].load() > 0.0);
    app.reset_run();
    assert_eq!(app.bus_loads[0].load(), 0.0);
    assert_eq!(app.bus_loads[0].errors, 0);
}

/// A 0x100 frame carrying `rpm` on EngineSpeed (sample.dbc: factor 0.25,
/// little-endian 16 bit at bit 0).
fn rpm_frame(t_us: u64, rpm: f64) -> CanFrame {
    let raw = (rpm / 0.25) as u16;
    let mut f = rx_frame(t_us, 0x100, 2, FrameFlags::NONE);
    f.data[0] = (raw & 0xFF) as u8;
    f.data[1] = (raw >> 8) as u8;
    f
}

#[test]
fn a_signal_crossing_fires_on_the_crossing_not_the_level() {
    let mut app = quiet_app();
    let base = std::env::temp_dir().join("roxy_can_trigger_cross.asc");
    // The recorder opens through the trigger action, so the core's record
    // path must be set by command: the frontend draft alone would leave it
    // empty, and the derived CWD name collides with parallel tests.
    app.send(crate::bus::BusCommand::SetRecordPath(
        base.to_string_lossy().into_owned(),
    ));
    app.triggers.push(Trigger::new(
        TriggerCond::SignalCross {
            ch: 0,
            id: 0x100,
            ext: false,
            signal: "EngineSpeed".to_string(),
            threshold: 3000.0,
            rising: true,
        },
        TriggerAction::StartRecording,
    ));

    receive(&mut app, 10_000, vec![rpm_frame(10_000, 1000.0)]);
    assert!(!app.recorder.recording, "below threshold: nothing fires");

    receive(&mut app, 20_000, vec![rpm_frame(20_000, 3000.0)]);
    assert!(app.recorder.recording, "the crossing itself fires");

    receive(&mut app, 30_000, vec![rpm_frame(30_000, 3500.0)]);
    assert_eq!(app.triggers[0].fired, 1, "staying above is not an edge");

    receive(&mut app, 40_000, vec![rpm_frame(40_000, 1000.0)]);
    receive(&mut app, 50_000, vec![rpm_frame(50_000, 3200.0)]);
    assert_eq!(app.triggers[0].fired, 2, "re-crossing re-arms");
    assert_eq!(app.triggers[0].last_fire_t_us, 50_000);

    // The recording opens with the pre-trigger context (the below-
    // threshold 10 ms frame), then the crossing frame and everything after.
    app.recorder.close();
    let text = std::fs::read_to_string(&app.recorder.last_record).unwrap();
    let times: Vec<u64> = crate::log::asc::parse_asc(&text)
        .iter()
        .map(|f| f.t_us)
        .collect();
    assert_eq!(
        times,
        vec![10_000, 20_000, 30_000, 40_000, 50_000],
        "pre-trigger context first, then the firing frame and on"
    );
    std::fs::remove_file(&app.recorder.last_record).ok();
}

#[test]
fn an_id_present_trigger_latches_and_can_stop_a_recording() {
    let mut app = quiet_app();
    let base = std::env::temp_dir().join("roxy_can_trigger_id.asc");
    app.send(crate::bus::BusCommand::SetRecordPath(
        base.to_string_lossy().into_owned(),
    ));
    app.triggers.push(Trigger::new(
        TriggerCond::IdPresent { ch: 0, id: 0x777 },
        TriggerAction::StopRecording,
    ));
    app.recorder.recording = true;
    app.recorder.open().unwrap();

    receive(
        &mut app,
        10_000,
        vec![rx_frame(10_000, 0x100, 8, FrameFlags::NONE)],
    );
    assert!(app.recorder.recording, "an unwatched id changes nothing");

    receive(
        &mut app,
        20_000,
        vec![rx_frame(20_000, 0x777, 8, FrameFlags::NONE)],
    );
    assert_eq!(app.triggers[0].fired, 1);
    // The stop rolls on for the post frames, then the recorder closes.
    let fillers: Vec<CanFrame> = (0..40)
        .map(|i| rx_frame(21_000 + i as u64 * 1_000, 0x100, 8, FrameFlags::NONE))
        .collect();
    receive(&mut app, 100_000, fillers);
    assert!(
        !app.recorder.recording,
        "the watched id stopped the recording after the roll"
    );
    assert_eq!(app.triggers[0].fired, 1, "presence latches for the run");
    app.recorder.close();
    std::fs::remove_file(&app.recorder.last_record).ok();
}

/// The record file grows while the measurement runs.
///
/// Everything used to wait for the buffer: the file appeared with its header and
/// stayed that size until Stop, so on a bench doing ten frames a second a
/// recording looked like it was not writing at all -- and a process that died
/// took the whole session with it. The tail of every step now pushes what the
/// step recorded down to disk, which is what this asserts: the frame is readable
/// from the file while the recorder is still open.
#[test]
fn the_record_file_grows_before_anyone_presses_stop() {
    let mut app = quiet_app();
    let base = std::env::temp_dir().join("roxy_can_live_record.asc");
    let _ = std::fs::remove_file(&base);
    app.send(crate::bus::BusCommand::SetRecordPath(
        base.to_string_lossy().into_owned(),
    ));
    app.recorder.recording = true;
    app.recorder.open().unwrap();

    receive(
        &mut app,
        10_000,
        vec![rx_frame(10_000, 0x100, 8, FrameFlags::NONE)],
    );

    let text = std::fs::read_to_string(&app.recorder.last_record).expect("on disk");
    let frames = crate::log::asc::parse_asc(&text);
    assert!(
        frames.iter().any(|f| f.id == 0x100),
        "the step's frames are already in the file: {text:?}"
    );

    app.recorder.close();
    std::fs::remove_file(&app.recorder.last_record).ok();
}

/// Opening a recording never abandons the one that is open.
///
/// `Recorder::open` used to overwrite its own writer. Today no call path
/// reaches it with a file open, but the cost of getting that wrong is silent:
/// the abandoned BLF loses the objects still in its buffer and the abandoned ASC
/// its trailer, and neither says anything.
#[test]
fn opening_a_second_recording_finishes_the_first_file() {
    let dir = std::env::temp_dir();
    let mut app = quiet_app();
    app.recorder.record_path = dir.join("roxy_can_first").to_string_lossy().into_owned();
    let first = app.recorder.open().expect("first opens");
    app.recorder
        .write(&rx_frame(10_000, 0x100, 8, FrameFlags::NONE));

    app.recorder.record_path = dir.join("roxy_can_second").to_string_lossy().into_owned();
    let second = app.recorder.open().expect("second opens");
    assert_ne!(first, second, "the two files are two files");

    let text = std::fs::read_to_string(&first).expect("the first is on disk");
    assert!(
        text.contains("End TriggerBlock"),
        "the first file was closed, not dropped: {text:?}"
    );
    assert!(
        crate::log::asc::parse_asc(&text)
            .iter()
            .any(|f| f.id == 0x100),
        "and it kept the frame: {text:?}"
    );

    app.recorder.close();
    std::fs::remove_file(&first).ok();
    std::fs::remove_file(&second).ok();
}

/// The manual re-arm for appearance watches: the edge blanks the trace
/// ring but leaves aggregates, spec memory and the recorder alone.
#[test]
fn a_clear_trace_trigger_empties_the_ring_and_nothing_else() {
    let mut app = quiet_app();
    app.triggers.push(Trigger::new(
        TriggerCond::IdPresent { ch: 0, id: 0x777 },
        TriggerAction::ClearTrace,
    ));
    // Keep the recorder off the repo root: a named temp path, recording.
    let base = std::env::temp_dir().join("roxy_can_clear_trace.asc");
    app.send(crate::bus::BusCommand::SetRecordPath(
        base.to_string_lossy().into_owned(),
    ));
    app.recorder.recording = true;
    app.recorder.open().unwrap();

    receive(
        &mut app,
        10_000,
        vec![rx_frame(10_000, 0x100, 8, FrameFlags::NONE)],
    );
    assert!(app.trace.len() == 1, "traffic landed");
    assert!(app.snap.frame_counter == 1);
    assert!(app.recorder.recording, "the recorder is untouched");

    receive(
        &mut app,
        20_000,
        vec![rx_frame(20_000, 0x777, 8, FrameFlags::NONE)],
    );
    // The evaluator runs before the frame folds in: the ring was emptied
    // and then the triggering frame itself landed.
    assert_eq!(app.trace.len(), 1, "cleared, then the trigger frame lands");
    assert_eq!(app.trace.iter().next().map(|f| f.id), Some(0x777));
    assert!(
        app.aggs.contains_key(&(
            crate::model::ClusterId(app.snap.cluster_ids.can[0]),
            0x100,
            false
        )),
        "aggregates survive the clear"
    );
    assert!(app.recorder.recording, "still recording after the clear");

    // The ring keeps working afterwards. The ClearTrace action also
    // re-armed the condition's own latch: the next occurrence fires,
    // clears, and its frame lands alone again.
    receive(
        &mut app,
        30_000,
        vec![rx_frame(30_000, 0x777, 8, FrameFlags::NONE)],
    );
    assert_eq!(app.trace.len(), 1, "cleared, then the trigger frame lands");
    assert_eq!(app.trace.iter().next().map(|f| f.id), Some(0x777));
    assert_eq!(
        app.triggers[0].fired, 2,
        "the watch re-armed itself on every clear"
    );
    app.recorder.close();
    std::fs::remove_file(&app.recorder.last_record).ok();
}

#[test]
fn an_error_frame_trigger_latches_once_per_run() {
    let mut app = quiet_app();
    let base = std::env::temp_dir().join("roxy_can_trigger_err.asc");
    app.send(crate::bus::BusCommand::SetRecordPath(
        base.to_string_lossy().into_owned(),
    ));
    app.triggers.push(Trigger::new(
        TriggerCond::ErrorFrame { ch: 0 },
        TriggerAction::StartRecording,
    ));

    let mut other_bus = rx_frame(10_000, 0x300, 0, FrameFlags::ERROR);
    other_bus.channel = 1;
    receive(&mut app, 10_000, vec![other_bus]);
    assert!(
        !app.recorder.recording,
        "error frames on another bus are not ours"
    );

    receive(
        &mut app,
        20_000,
        vec![rx_frame(20_000, 0x300, 0, FrameFlags::ERROR)],
    );
    assert!(app.recorder.recording, "our first error frame fires");

    receive(
        &mut app,
        30_000,
        vec![rx_frame(30_000, 0x300, 0, FrameFlags::ERROR)],
    );
    assert_eq!(app.triggers[0].fired, 1, "latches: one fire per run");
    app.recorder.close();
    std::fs::remove_file(&app.recorder.last_record).ok();
}

/// The editor's one Apply lands as one `EditTrigger`: flipping the crossing
/// direction -- the change the old inline editor could not make stick --
/// updates the row in place, both ways.
#[test]
fn a_trigger_edit_flips_the_crossing_direction() {
    let mut app = quiet_app();
    app.add_signal_trigger();
    assert!(matches!(
        &app.triggers[0].cond,
        TriggerCond::SignalCross { rising: true, .. }
    ));
    let action = app.triggers[0].action.clone();
    app.send(crate::bus::BusCommand::EditTrigger {
        index: 0,
        cond: TriggerCond::SignalCross {
            ch: 0,
            id: 0x100,
            ext: false,
            signal: "EngineSpeed".to_string(),
            threshold: 500.0,
            rising: false,
        },
        action,
        cooldown_us: 0,
        max_fires: 0,
        pre_frames: None,
        post_frames: None,
    });
    match &app.triggers[0].cond {
        TriggerCond::SignalCross {
            threshold, rising, ..
        } => {
            assert!(!*rising, "falling sticks");
            assert_eq!(*threshold, 500.0);
        }
        other => panic!("expected a signal trigger, got {other:?}"),
    }
    let action = app.triggers[0].action.clone();
    let cond = match &app.triggers[0].cond {
        TriggerCond::SignalCross {
            ch,
            id,
            ext,
            signal,
            threshold,
            ..
        } => TriggerCond::SignalCross {
            ch: *ch,
            id: *id,
            ext: *ext,
            signal: signal.clone(),
            threshold: *threshold,
            rising: true,
        },
        other => panic!("expected a signal trigger, got {other:?}"),
    };
    app.send(crate::bus::BusCommand::EditTrigger {
        index: 0,
        cond,
        action,
        cooldown_us: 0,
        max_fires: 0,
        pre_frames: None,
        post_frames: None,
    });
    assert!(matches!(
        &app.triggers[0].cond,
        TriggerCond::SignalCross { rising: true, .. }
    ));
}

/// The timeout condition rides the spec's grace: message 100 in `SPEC_DBC`
/// declares a 100 ms period and the default grace is three periods, so 450 ms
/// of silence convicts while a message never seen stays unjudged. Traffic
/// resuming clears the level, making every dropout a fresh edge.
#[test]
fn a_cycle_timeout_trigger_fires_on_each_dropout() {
    let mut app = spec_app();
    let base = std::env::temp_dir().join("roxy_can_trigger_timeout.asc");
    app.send(crate::bus::BusCommand::SetRecordPath(
        base.to_string_lossy().into_owned(),
    ));
    app.triggers.push(Trigger::new(
        TriggerCond::CycleTimeout { ch: 0, id: 100 },
        TriggerAction::StartRecording,
    ));
    app.triggers.push(Trigger::new(
        TriggerCond::CycleTimeout { ch: 0, id: 0x777 },
        TriggerAction::StartRecording,
    ));

    receive(&mut app, 0, vec![frame_at(0, 100, 8, Direction::Rx)]);
    assert!(!app.recorder.recording, "traffic present: no dropout");
    assert_eq!(app.triggers[0].fired, 0);

    receive(&mut app, 450_000, vec![]);
    assert!(app.recorder.recording, "4.5 silent periods is past grace 3");
    assert_eq!(app.triggers[0].fired, 1);
    assert_eq!(
        app.triggers[1].fired, 0,
        "a message never seen is no opinion, not a dropout"
    );

    receive(
        &mut app,
        500_000,
        vec![frame_at(500_000, 100, 8, Direction::Rx)],
    );
    receive(&mut app, 900_000, vec![]);
    assert_eq!(
        app.triggers[0].fired, 2,
        "resumed then silent again: re-fires"
    );
    app.recorder.close();
    std::fs::remove_file(&app.recorder.last_record).ok();
}

/// A frontend that owns its plots sets the stride directly; observer-window
/// zoom neither tightens nor coarsens it.
#[test]
fn a_plot_stride_override_ignores_the_graphics_windows() {
    let mut app = quiet_app();
    app.graphics[0].time_window_s = 10.0;
    let windowed = app.wanted_stride_us();
    app.plot_stride_us = Some(5_000);
    assert_eq!(app.wanted_stride_us(), 5_000);
    assert!(windowed > 5_000, "the default stride stays window-derived");
}

/// The CANoe Graphics behaviour: zooming into a small window must reveal
/// every signal update. A 100 Hz signal watched through a 0.1 s window
/// samples every frame at the tightened stride, where the old fixed 50 ms
/// stride kept only two points per window.
#[test]
fn a_small_graphics_window_pulls_the_sample_stride_down() {
    let mut app = quiet_app();
    let key = SigKey::can_in(crate::model::ClusterId(1), 0x100, false, "EngineSpeed");
    app.subscribe(key.clone());

    // The default 10 s window keeps the coarse stride: 21 frames at 100 Hz
    // yield at most one point per 50 ms.
    app.graphics[0].time_window_s = 10.0;
    let frames: Vec<CanFrame> = (0..21u64)
        .map(|i| rx_frame(i * 10_000, 0x100, 2, FrameFlags::NONE))
        .collect();
    receive(&mut app, 200_000, frames.clone());
    let coarse = app.subs.get(&key).unwrap().history.len();
    assert!(coarse <= 6, "coarse stride decimates: {coarse} points");

    // Zoom to 0.1 s: the stride drops to 500 µs and every frame lands.
    app.graphics[0].time_window_s = 0.1;
    receive(
        &mut app,
        400_000,
        (21..41u64)
            .map(|i| rx_frame(i * 10_000, 0x100, 2, FrameFlags::NONE))
            .collect(),
    );
    let fine = app.subs.get(&key).unwrap().history.len();
    assert!(
        fine - coarse >= 15,
        "the zoomed window samples nearly every frame: {coarse} -> {fine}"
    );
}

/// The reaction rule in its smallest form: a watched message arrives and one
/// frame from the generator entry goes out, carrying the entry's payload and
/// the triggering frame's own timestamp. One edge, one frame.
#[test]
fn a_send_action_transmits_one_generator_frame() {
    let mut app = quiet_app();
    app.add_tx(0, 0x777);
    let i = app
        .tx_list
        .iter()
        .position(|t| t.channel == 0 && t.id == 0x777)
        .expect("entry added");
    app.tx_list[i].len = 2;
    app.tx_list[i].data[0] = 0xDE;
    app.tx_list[i].data[1] = 0xAD;
    app.triggers.push(Trigger::new(
        TriggerCond::IdPresent { ch: 0, id: 0x555 },
        TriggerAction::Send { ch: 0, id: 0x777 },
    ));

    receive(
        &mut app,
        10_000,
        vec![rx_frame(10_000, 0x555, 8, FrameFlags::NONE)],
    );
    let sent: Vec<CanFrame> = app
        .trace
        .iter()
        .filter(|f| f.id == 0x777 && matches!(f.dir, Direction::Tx))
        .copied()
        .collect();
    assert_eq!(sent.len(), 1, "exactly one reaction frame");
    assert_eq!(
        sent[0].t_us, 10_000,
        "stamped with the triggering frame's clock"
    );
    assert_eq!(
        &sent[0].data[..2],
        &[0xDE, 0xAD],
        "the entry's payload goes out"
    );
    assert!(
        app.aggs.contains_key(&(
            crate::model::ClusterId(app.snap.cluster_ids.can[0]),
            0x777,
            false
        )),
        "the reaction aggregates like any real traffic"
    );

    // IdPresent latches, so the rule does not keep answering.
    receive(&mut app, 20_000, vec![]);
    assert_eq!(
        app.trace.iter().filter(|f| f.id == 0x777).count(),
        1,
        "one edge, one frame -- no repeats"
    );
}

/// Two messages that share a signal name: the reaction Send action copies
/// every same-named signal from the triggering frame into the target
/// entry's payload (a gateway hop), leaving signals without a counterpart
/// -- here `FaultBits` -- at the entry's base value.
const MIRROR_DBC: &str = r#"VERSION "roxy-can mirror test"

NS_ :

BU_: ECU

BO_ 256 MotorStatus: 8 ECU
 SG_ EngineSpeed : 0|16@1+ (0.25,0) [0|8000] "" ECU

BO_ 512 DashMirror: 8 ECU
 SG_ EngineSpeed : 0|16@1+ (0.25,0) [0|8000] "" ECU
 SG_ FaultBits : 16|8@1+ (1,0) [0|255] "" ECU
"#;

#[test]
fn a_send_action_mirrors_same_named_signals_from_the_trigger_frame() {
    let mut app = App::headless();
    app.set_can_table_for_test(
        0,
        Some(std::sync::Arc::new(
            crate::dbc::load_dbc_str(MIRROR_DBC).unwrap(),
        )),
    );
    app.tx_list.retain(|t| t.channel != 0);
    app.start_virtual();
    assert!(
        app.model_can_all_values(&frame_at(0, 0x200, 8, Direction::Rx))
            .is_some(),
        "the reaction target is decoded through the model"
    );
    // The reaction target, inactive so only the reaction itself emits it;
    // its base payload is all zeroes.
    app.add_tx(0, 0x200);
    let key = SigKey::can_in(crate::model::ClusterId(1), 0x200, false, "EngineSpeed");
    app.subscribe(key.clone());
    app.triggers.push(Trigger::new(
        TriggerCond::SignalCross {
            ch: 0,
            id: 0x100,
            ext: false,
            signal: "EngineSpeed".to_string(),
            threshold: 3000.0,
            rising: true,
        },
        TriggerAction::Send { ch: 0, id: 0x200 },
    ));

    receive(&mut app, 10_000, vec![rpm_frame(10_000, 1000.0)]);
    assert_eq!(app.triggers[0].fired, 0, "below threshold: no reaction");
    assert_eq!(
        app.subs.get(&key).expect("subscribed").latest,
        0.0,
        "no 0x200 traffic before the crossing"
    );

    receive(&mut app, 20_000, vec![rpm_frame(20_000, 4000.0)]);
    assert_eq!(app.triggers[0].fired, 1, "the crossing fires");
    let sent: Vec<CanFrame> = app
        .trace
        .iter()
        .filter(|f| f.id == 0x200 && matches!(f.dir, Direction::Tx))
        .copied()
        .collect();
    assert_eq!(sent.len(), 1, "exactly one reaction frame");
    assert_eq!(
        sent[0].t_us, 20_000,
        "stamped with the triggering frame's clock"
    );
    assert_eq!(
        &sent[0].data[..2],
        &[0x80, 0x3E],
        "raw 16000 little-endian: the trigger frame's 4000 rpm"
    );
    assert_eq!(
        sent[0].data[2], 0,
        "FaultBits has no counterpart on 0x100: base value stands"
    );
    assert_eq!(
        app.subs.get(&key).expect("subscribed").latest,
        4000.0,
        "the reaction mirrors the trigger frame's value"
    );
    app.stop();
}

/// What a one-second Graphics window would draw right now: asks for the
/// view's data exactly the way `draw_plot` does (view request, then a cache
/// slice), and reports the plot clock plus the visible slice's point count
/// and right edge. A curve that "breathes" -- shrinking back and forth in
/// the time direction -- shows up as the count or the right edge swinging
/// while the clock only ever moves forward.
fn visible_curve(app: &mut App, key: &crate::observe::SigKey) -> (f64, usize, f64) {
    let t_now = app.plot_now_s();
    let tw = app.graphics[0].time_window_s;
    let t_right = t_now - app.graphics[0].t_offset_s;
    let lo_us = ((t_right - tw).max(0.0) * 1e6) as u64;
    let hi_us = (t_right.max(0.0) * 1e6) as u64;
    let pts = app
        .subs
        .get(key)
        .expect("subscribed")
        .history
        .range(lo_us, hi_us);
    let count = pts.len();
    let right = pts.last().map(|&(t, _)| t as f64 / 1e6).unwrap_or(f64::NAN);
    (t_now, count, right)
}

#[test]
fn the_replay_curve_holds_still_at_a_one_second_window() {
    let (mut app, key, file) = app_with_replayable_recording("breathe_1s", 500);
    app.replay();
    app.set_replay_speed(4.0);
    app.graphics[0].opened = true;
    app.graphics[0].time_window_s = 1.0;

    let (mut prev_right, mut prev_count) = (f64::NAN, 0usize);
    let mut worst_gap = 0.0f64;
    let mut worst_drop = 0isize;
    for _ in 0..80 {
        std::thread::sleep(std::time::Duration::from_millis(11));
        app.update();
        let (t_now, count, right) = visible_curve(&mut app, &key);
        if !right.is_finite() || t_now < 1.2 {
            prev_right = right;
            prev_count = count;
            continue;
        }
        // The window is full from here on: the right end must ride the
        // playhead and the point count must hover at the steady state
        // instead of swinging as points appear and vanish.
        let gap = t_now - right;
        worst_gap = worst_gap.max(gap);
        assert!(
            right >= prev_right - 1e-9,
            "the curve's right end moved backwards: {prev_right} -> {right} at t={t_now}"
        );
        worst_drop = worst_drop.max(prev_count as isize - count as isize);
        assert!(
            (count as isize - prev_count as isize).abs() <= 10,
            "the visible point count swung {prev_count} -> {count} at t={t_now}"
        );
        prev_right = right;
        prev_count = count;
    }
    assert!(
        prev_count >= 80,
        "a 1 s window at 100 Hz holds ~100 points, ended at {prev_count} (worst gap {worst_gap:.3}s, worst drop {worst_drop})"
    );
    app.stop();
    std::fs::remove_file(&file).ok();
}

#[test]
fn the_sim_curve_holds_still_at_a_one_second_window() {
    let mut app = App::headless();
    let key = {
        let db = app.channel_dbc(0).expect("sample DBC loaded");
        let id = db.order[0].0;
        SigKey::can_in(
            crate::model::ClusterId(1),
            id,
            false,
            db.messages[&(id, false)].signals[0].name.clone(),
        )
    };
    app.subscribe(key.clone());
    for tx in &mut app.tx_list {
        tx.active = true;
        tx.cycle_us = 10_000;
    }
    open_all_gates(&mut app);
    app.start_virtual();
    app.graphics[0].opened = true;
    app.graphics[0].time_window_s = 1.0;

    let (mut prev_right, mut prev_count) = (f64::NAN, 0usize);
    for _ in 0..600 {
        app.sim_t_us += 16_667;
        app.tick(app.sim_t_us);
        let (t_now, count, right) = visible_curve(&mut app, &key);
        if !right.is_finite() || t_now < 1.2 {
            prev_right = right;
            prev_count = count;
            continue;
        }
        assert!(
            right >= prev_right - 1e-9,
            "the curve's right end moved backwards: {prev_right} -> {right} at t={t_now}"
        );
        assert!(
            (count as isize - prev_count as isize).abs() <= 10,
            "the visible point count swung {prev_count} -> {count} at t={t_now}"
        );
        prev_right = right;
        prev_count = count;
    }
    assert!(
        prev_count >= 80,
        "a 1 s window at 100 Hz holds ~100 points, ended at {prev_count}"
    );
}

/// Feed EngineSpeed the given (time µs, rpm) pairs in one scripted receive.
fn feed_rpm(app: &mut App, pts: &[(u64, f64)]) {
    let t = pts.last().map(|&(t, _)| t).unwrap_or(0) + 10_000;
    receive(app, t, pts.iter().map(|&(t, v)| rpm_frame(t, v)).collect());
}

/// Subscribe EngineSpeed and hang it in Graphics 1's curve list with the
/// given value-axis policy -- the strategies resolve from the list entry,
/// not the window.
fn gfx_app(mode: YMode) -> (App, crate::observe::SigKey) {
    let mut app = quiet_app();
    let key = SigKey::can_in(crate::model::ClusterId(1), 0x100, false, "EngineSpeed");
    app.subscribe(key.clone());
    app.graphics[0].opened = true;
    app.graphics[0].signals.push(GfxSignal {
        key: key.clone(),
        visible: true,
        y_mode: mode,
    });
    (app, key)
}

#[test]
fn the_y_mode_round_trips_through_a_project() {
    let (mut app, _key) = gfx_app(YMode::FitAll);
    let path = std::env::temp_dir().join("roxy_can_ymode.rxproj");
    assert!(app.save_project(Some(path.clone())));

    let mut restored = App::headless();
    restored.open_project_path(&path);
    assert_eq!(
        restored.graphics[0].signals[0].y_mode,
        YMode::FitAll,
        "the per-signal policy rides the project file"
    );
    assert!(
        restored.graphics[0].y_locks.is_empty(),
        "frozen ranges are session state, never persisted"
    );
    std::fs::remove_file(&path).ok();
}

#[test]
fn the_state_tracker_round_trips_through_a_project() {
    let mut app = App::headless();
    let key = SigKey::can_in(crate::model::ClusterId(1), 0x100, false, "EngineSpeed");
    app.state_trackers[0].signals.push(GfxSignal {
        key: key.clone(),
        visible: true,
        y_mode: YMode::Auto,
    });
    app.state_trackers[0].time_window_s = 60.0;
    app.band_sets.push(crate::band::BandSet {
        id: "speed".to_string(),
        name: "Speed".to_string(),
        bands: vec![
            crate::band::Band {
                upper: Some(1000.0),
                inclusive: false,
                text: "low".to_string(),
                color: Some(0),
            },
            crate::band::Band {
                upper: None,
                inclusive: false,
                text: "high".to_string(),
                color: None,
            },
        ],
    });
    app.state_trackers[0].rules.insert(
        key.clone(),
        crate::observe::StateRule {
            band_set_id: Some("speed".to_string()),
            bands: app.band_sets[0].bands.clone(),
        },
    );
    // A default-mode override pinned on the signal's second key (the one
    // whose value is exactly 3000.0 in the sample database's scale).
    let bits = {
        let q = 3000.0f64;
        let q = if q == 0.0 { 0.0 } else { q };
        q.to_bits()
    };
    app.state_trackers[0]
        .overrides
        .insert(key.clone(), [(bits, [0.9, 0.5, 0.1])].into_iter().collect());
    app.subscribe(key.clone());
    let path = std::env::temp_dir().join("roxy_can_state.rxproj");
    assert!(app.save_project(Some(path.clone())));

    let mut restored = App::headless();
    restored.open_project_path(&path);
    assert_eq!(restored.state_trackers[0].time_window_s, 60.0);
    assert_eq!(restored.state_trackers[0].signals[0].key, key);
    let rule = restored.state_trackers[0]
        .rules
        .get(&key)
        .expect("custom bands ride the project file");
    assert_eq!(rule.band_set_id.as_deref(), Some("speed"));
    let set = restored
        .band_sets
        .iter()
        .find(|set| set.id == "speed")
        .expect("state band set restored");
    assert_eq!(set.bands[0].upper, Some(1000.0));
    assert_eq!(set.bands[0].text, "low");
    assert_eq!(set.bands[0].color, Some(0));
    assert_eq!(
        set.bands[1].color, None,
        "automatic survives the round trip"
    );
    let restored_ov = restored.state_trackers[0]
        .overrides
        .get(&key)
        .and_then(|m| m.get(&bits))
        .copied();
    assert_eq!(
        restored_ov,
        Some([0.9, 0.5, 0.1]),
        "default-mode overrides ride the project file too"
    );
    // A restored signal must be resubscribed, or the band draws no data.
    assert!(restored.subs.contains_key(&key));
    std::fs::remove_file(&path).ok();
}

/// Role declarations land in the snapshot the moment the command is
/// applied -- every role editor (Entities, Network, generator groups)
/// reads this one state.
#[test]
fn node_roles_follow_commands_and_land_in_the_snapshot() {
    let mut app = App::headless();
    app.send(crate::bus::BusCommand::SetNodeRole {
        ch: 0,
        node: "EngineECU".to_string(),
        role: NodeRole::Simulated,
    });
    app.settle();
    let role = app.snap.channels[0]
        .node_roles
        .get("EngineECU")
        .copied()
        .expect("the declaration is in the snapshot");
    assert_eq!(role, NodeRole::Simulated);

    app.send(crate::bus::BusCommand::SetNodeRole {
        ch: 0,
        node: "EngineECU".to_string(),
        role: NodeRole::Absent,
    });
    app.settle();
    assert_eq!(
        app.snap.channels[0].role_of("EngineECU"),
        NodeRole::Absent,
        "the snapshot reads the new role"
    );
}

/// The whole replay-block path, from creation to the wire: an
/// enabled block streams its filtered log traffic into a running
/// simulation, with recorded spacing; nothing flows while the block is
/// disabled, and nothing flows in Replay mode either (the log is already
/// the source there).
#[test]
fn a_replay_block_streams_its_log_into_the_simulation() {
    // Fixture log: 0x100 every 10 ms for 5 frames, plus one 0x999 frame.
    let path = std::env::temp_dir().join("roxy_can_block_e2e.asc");
    let mut w = AscWriter::new(&path.to_string_lossy()).unwrap();
    for i in 0..5u64 {
        let mut f = CanFrame {
            t_us: i * 10_000,
            channel: 0,
            id: 0x100,
            extended: false,
            len: 1,
            data: [0; MAX_CAN_FD_LEN],
            dir: Direction::Rx,
            flags: FrameFlags::NONE,
        };
        f.data[0] = i as u8;
        w.write(&f).unwrap();
    }
    let mut stray = CanFrame {
        t_us: 25_000,
        channel: 0,
        id: 0x999,
        extended: false,
        len: 1,
        data: [0; MAX_CAN_FD_LEN],
        dir: Direction::Rx,
        flags: FrameFlags::NONE,
    };
    stray.data[0] = 0xAA;
    w.write(&stray).unwrap();
    w.finish().unwrap();

    let mut app = App::headless();
    app.tx_list.retain(|t| t.channel != 0);
    app.add_replay_block(
        0,
        "restbus".to_string(),
        path.to_string_lossy().into_owned(),
        None,
        None,
    );
    app.settle();
    // A fresh block starts disabled -- adding a block must not begin
    // transmitting on its own.
    let id = app.snap.blocks.first().expect("block exists").id;
    assert!(
        app.snap
            .blocks
            .first()
            .is_some_and(|b| !b.enabled && b.frames == 0),
        "no queue is loaded until the block is enabled"
    );

    // Filter the block down to 0x100, then enable it: the queue loads
    // against the filter at once.
    app.send(crate::bus::BusCommand::SetReplayBlock {
        id,
        name: "restbus".to_string(),
        channel: 0,
        path: path.to_string_lossy().into_owned(),
        node_filter: None,
        attached: None,
        ids: vec![(0x100, false)],
    });
    app.settle();
    app.set_replay_block_enabled(id, true);
    app.settle();
    let block_view = app.snap.blocks.first().expect("block view");
    assert_eq!(block_view.frames, 5, "the id filter kept only 0x100");

    // Run a 45 ms simulation: every block frame is due by then.
    app.start_virtual();
    app.settle();
    for t in 1..=60 {
        app.advance_clock(t * 1_000);
        app.tick(t * 1_000);
    }
    let agg = app
        .snap
        .aggs
        .iter()
        .find(|a| a.channel == 0 && a.id == 0x100)
        .expect("the block's frames reached the bus");
    assert_eq!(agg.count, 5, "all five frames arrived");
    assert!(
        !app.snap
            .aggs
            .iter()
            .any(|a| a.channel == 0 && a.id == 0x999),
        "the id filter kept the stray frame out"
    );
    let cycles: Vec<u64> = app
        .trace
        .iter()
        .filter(|f| f.channel == 0 && f.id == 0x100)
        .map(|f| f.t_us)
        .collect();
    assert_eq!(
        cycles,
        [0, 10_000, 20_000, 30_000, 40_000],
        "recorded spacing survives the round trip"
    );
    app.stop();

    std::fs::remove_file(&path).ok();
}

/// Editing a replay block rewrites its declaration in place, including
/// the id filter and the DBC node it is attached to.
#[test]
fn a_replay_block_can_be_edited_in_place() {
    let mut app = App::headless();
    app.add_replay_block(0, "old".to_string(), "old.asc".to_string(), None, None);
    app.settle();
    let id = app.snap.blocks[0].id;

    app.set_replay_block(
        id,
        0,
        "new".to_string(),
        "new.asc".to_string(),
        Some("EngineECU".to_string()),
        vec![(0x100, false)],
    );
    app.settle();
    let block = app.snap.blocks.first().expect("block view");
    assert_eq!(block.name, "new");
    assert_eq!(block.path, "new.asc");
    assert_eq!(block.node_filter.as_deref(), Some("EngineECU"));
    assert_eq!(block.ids, vec![(0x100, false)]);
    assert_eq!(block.attached, Some((0, "EngineECU".to_string())));
}

/// In Replay mode the log itself is the source; replay blocks stay
/// silent so no frame is ever delivered twice.
#[test]
fn a_replay_block_stays_silent_in_replay_mode() {
    let path = std::env::temp_dir().join("roxy_can_block_replay.asc");
    let mut w = AscWriter::new(&path.to_string_lossy()).unwrap();
    let mut f = CanFrame {
        t_us: 0,
        channel: 0,
        id: 0x321,
        extended: false,
        len: 1,
        data: [0; MAX_CAN_FD_LEN],
        dir: Direction::Rx,
        flags: FrameFlags::NONE,
    };
    f.data[0] = 1;
    w.write(&f).unwrap();
    w.finish().unwrap();

    let mut app = App::headless();
    app.load_log(&path.to_string_lossy());
    app.add_replay_block(
        0,
        "b".to_string(),
        path.to_string_lossy().into_owned(),
        None,
        None,
    );
    app.settle();
    let id = app.snap.blocks[0].id;
    app.set_replay_block_enabled(id, true);
    app.settle();
    app.set_replay_speed(100.0);
    app.replay();
    app.settle();
    for t in 1..=40 {
        app.advance_clock(t * 1_000);
        app.tick(t * 1_000);
    }
    let count_321 = app
        .snap
        .aggs
        .iter()
        .find(|a| a.channel == 0 && a.id == 0x321)
        .map(|a| a.count)
        .unwrap_or(0);
    assert_eq!(
        count_321, 1,
        "exactly one delivery: the log's own frame, not the block's copy"
    );
    app.stop();
    std::fs::remove_file(&path).ok();
}

/// Replay blocks survive a project round trip: declaration, filters, and
/// the enable bit all come back.
#[test]
fn replay_blocks_round_trip_through_a_project() {
    let path = std::env::temp_dir().join("roxy_can_block_proj.asc");
    let mut w = AscWriter::new(&path.to_string_lossy()).unwrap();
    let mut f = CanFrame {
        t_us: 0,
        channel: 0,
        id: 0x100,
        extended: false,
        len: 1,
        data: [0; MAX_CAN_FD_LEN],
        dir: Direction::Rx,
        flags: FrameFlags::NONE,
    };
    f.data[0] = 1;
    w.write(&f).unwrap();
    w.finish().unwrap();

    let mut app = App::headless();
    app.add_replay_block(
        1,
        "engine_log".to_string(),
        path.to_string_lossy().into_owned(),
        None,
        None,
    );
    app.settle();
    let id = app.snap.blocks[0].id;
    app.set_replay_block_enabled(id, true);
    app.settle();

    let json = serde_json::to_string(&ProjectV2::from_app(&app, None)).unwrap();
    assert!(
        json.contains(r#""blocks""#) && json.contains("engine_log"),
        "the declaration is in the file"
    );
    let mut restored = App::headless();
    serde_json::from_str::<ProjectV2>(&json)
        .unwrap()
        .apply(&mut restored);
    restored.settle();
    let block = restored.snap.blocks.first().expect("block restored");
    assert_eq!(block.name, "engine_log");
    assert_eq!(block.channel, 1);
    assert!(block.enabled, "the enable bit survives");
    assert_eq!(block.frames, 1, "the queue reloaded against the same log");
    std::fs::remove_file(&path).ok();
}

/// `emit_value` turns a script into a derived-signal driver: the stream
/// shows up in the snapshot under the node's name, subscribes like any
/// signal, and carries the computed samples through history.
#[test]
fn emit_value_publishes_a_derived_signal_stream() {
    let mut app = App::headless();
    app.send(crate::bus::BusCommand::AddNode {
        name: "calc".to_string(),
        channel: 0,
        attached: None,
    });
    app.settle();
    let id = app.snap.nodes[0].id;
    app.send(crate::bus::BusCommand::SetNodeSource {
        id,
        source: "on timer 10 { emit_value(\"SpeedKmh\", 36 * 10); }".to_string(),
    });
    app.settle();
    app.start_virtual();
    app.settle();
    for t in 1..=60 {
        app.advance_clock(t * 1_000);
        app.tick(t * 1_000);
    }

    let key = SigKey::can_in(
        crate::model::ClusterId(1),
        crate::app::EMITTED_ID_BASE | id as u32,
        false,
        "SpeedKmh".to_string(),
    );
    assert!(
        app.snap
            .emitted
            .iter()
            .any(|(k, node)| k == &key && node == "calc"),
        "the stream is published for the selection tree"
    );
    let sub = app.subs.get(&key).expect("the stream subscribes itself");
    assert_eq!(sub.latest, 360.0, "the expression was evaluated");
    assert!(
        !sub.history.is_empty(),
        "samples flowed with the timer cadence"
    );

    // Removing the node retires its streams from the selection tree --
    // there is no future emitter for them.
    app.send(crate::bus::BusCommand::RemoveNode { id });
    app.settle();
    assert!(
        app.snap.emitted.is_empty(),
        "a removed node's streams leave the tree"
    );
    app.stop();
}

/// The manager's lifecycle via commands: define with bounds, manual set
/// clamps, and every run start resets the value to the declared init.
/// A sysvar condition sweeps the live registry: the operator raising
/// `@Demo::Setpoint` past the threshold fires the trigger once per
/// crossing -- holding high never refires, dropping low re-arms.
#[test]
fn a_sysvar_condition_fires_when_the_operator_raises_it() {
    let mut app = App::headless();
    app.send(crate::bus::BusCommand::DefineSysVar(
        crate::bus::SysVarDef {
            namespace: "Demo".to_string(),
            name: "Setpoint".to_string(),
            init: 0.0,
            min: Some(0.0),
            max: Some(100.0),
            unit: String::new(),
            comment: String::new(),
            label_sets: Vec::new(),
            band_set_id: None,
            labels: Vec::new(),
        },
    ));
    app.settle();
    app.send(crate::bus::BusCommand::AddTrigger {
        cond: crate::trigger::TriggerCond::SysVar {
            key: "Demo::Setpoint".to_string(),
            threshold: 50.0,
            rising: true,
        },
        action: crate::trigger::TriggerAction::ClearTrace,
        enabled: true,
    });
    app.settle();

    app.start_virtual();
    app.settle();

    fn set(app: &mut App, v: f64) {
        app.send(crate::bus::BusCommand::SetSysVar {
            namespace: "Demo".to_string(),
            name: "Setpoint".to_string(),
            value: v,
        });
        app.settle();
    }
    // The sweep lives inside the measuring step: every read of the
    // trigger needs an explicit clock advance to evaluate.
    let run_to = |app: &mut App, t: u64| {
        app.advance_clock(t);
        app.tick(t);
    };

    set(&mut app, 80.0);
    let t = app.now_us() + 1_000;
    run_to(&mut app, t);
    assert_eq!(app.snap.triggers[0].fired, 1, "the crossing fires once");

    // Holding above the threshold is a level, not repeated edges.
    let next = t + 100_000;
    run_to(&mut app, next);
    assert_eq!(
        app.snap.triggers[0].fired, 1,
        "holding high does not refire"
    );

    // Drop below and raise again: a fresh edge.
    set(&mut app, 10.0);
    let t = app.now_us() + 1_000;
    run_to(&mut app, t);
    set(&mut app, 80.0);
    let t = app.now_us() + 1_000;
    run_to(&mut app, t);
    assert_eq!(
        app.snap.triggers[0].fired, 2,
        "a second crossing fires again"
    );
    app.stop();
}

/// A Monitor row holds the signal's subscription on its own: removing
/// the same signal from a Data window must not cut the Monitor's feed
/// (and with every watcher gone, the feed is pruned).
#[test]
fn monitor_rows_hold_the_subscription_against_data_removal() {
    let mut app = App::headless();
    app.new_data_window();
    let key = SigKey::can_in(crate::model::ClusterId(1), 0x100, false, "EngineStatus");
    app.set_win_signal(PopupTarget::Data(0), key.clone(), true);
    app.set_win_signal(PopupTarget::Monitor, key.clone(), true);
    assert_eq!(app.monitor_view.rows.len(), 1, "the Monitor row was added");

    app.set_win_signal(PopupTarget::Data(0), key.clone(), false);
    app.settle();
    assert!(
        app.sub_view(&key).is_some(),
        "the Monitor row keeps the feed alive"
    );

    app.set_win_signal(PopupTarget::Monitor, key.clone(), false);
    app.settle();
    assert!(
        app.sub_view(&key).is_none(),
        "with every watcher gone the feed is pruned"
    );
}

/// The web's row drag sends one move per drop: the row lands before the
/// slot it was dropped on, and an oversized index clamps to the end.
/// The project snapshot serialises the Vec order, so no extra bookkeeping
/// is needed for the reorder to persist.
#[test]
fn monitor_and_data_rows_reorder_by_key() {
    let mut app = App::headless();
    app.new_data_window();
    let a = SigKey::can_in(crate::model::ClusterId(1), 0x100, false, "EngineSpeed");
    let b = SigKey::can_in(crate::model::ClusterId(1), 0x200, false, "VehicleSpeed");
    app.set_win_signal(PopupTarget::Monitor, a.clone(), true);
    app.set_win_signal(PopupTarget::Monitor, b.clone(), true);
    app.set_win_signal(PopupTarget::Data(0), a.clone(), true);
    app.set_win_signal(PopupTarget::Data(0), b.clone(), true);

    // Move b to slot 0: "to" is the post-removal index, so it lands first.
    app.move_monitor_row(&b, 0);
    assert_eq!(app.monitor_view.rows[0].key, b);
    assert_eq!(app.monitor_view.rows[1].key, a);

    // An unknown key is a no-op; an oversized index clamps to the end.
    let missing = SigKey::can_in(crate::model::ClusterId(1), 0x777, false, "Nope");
    app.move_monitor_row(&missing, 0);
    app.move_monitor_row(&b, 99);
    assert_eq!(app.monitor_view.rows[0].key, a);
    assert_eq!(app.monitor_view.rows[1].key, b);

    // The Data window reorders through the same rule.
    app.move_data_row(&b, 0);
    let window = app.data_windows.first().expect("data window");
    assert_eq!(window.signals[0].key, b);
    assert_eq!(window.signals[1].key, a);

    // Derived keys match by definition id: the wire form drops the name.
    let d1 = SigKey::derived("d1", "Load");
    app.set_win_signal(PopupTarget::Monitor, d1.clone(), true);
    let wire = crate::observe::SigKey::from_text("derived:d1").expect("parses");
    app.move_monitor_row(&wire, 0);
    assert_eq!(app.monitor_view.rows[0].key, d1);
}

#[test]
fn sysvars_define_clamp_and_reset_on_run_start() {
    let mut app = App::headless();
    app.send(crate::bus::BusCommand::DefineSysVar(
        crate::bus::SysVarDef {
            namespace: "Demo".to_string(),
            name: "Setpoint".to_string(),
            init: 100.0,
            min: Some(0.0),
            max: Some(500.0),
            unit: "rpm".to_string(),
            comment: "target speed".to_string(),
            label_sets: Vec::new(),
            band_set_id: None,
            labels: Vec::new(),
        },
    ));
    app.settle();
    assert_eq!(app.snap.sysvars.len(), 1, "defined");
    assert_eq!(app.snap.sysvars[0].value, 100.0, "starts at init");

    app.send(crate::bus::BusCommand::SetSysVar {
        namespace: "Demo".to_string(),
        name: "Setpoint".to_string(),
        value: 900.0,
    });
    app.send(crate::bus::BusCommand::SetSysVar {
        namespace: "Demo".to_string(),
        name: "Setpoint".to_string(),
        value: -3.0,
    });
    app.settle();
    assert_eq!(
        app.snap.sysvars[0].value, 0.0,
        "writes clamp into the declared bounds"
    );

    // A run start rewinds every variable to init.
    app.send(crate::bus::BusCommand::SetSysVar {
        namespace: "Demo".to_string(),
        name: "Setpoint".to_string(),
        value: 250.0,
    });
    app.settle();
    app.start_virtual();
    app.settle();
    assert_eq!(app.snap.sysvars[0].value, 100.0, "run start resets to init");
    app.stop();

    // Deleting removes the definition.
    app.send(crate::bus::BusCommand::DeleteSysVar {
        namespace: "Demo".to_string(),
        name: "Setpoint".to_string(),
    });
    app.settle();
    assert!(app.snap.sysvars.is_empty(), "deleted");
}

/// A script's `sys_set` writes flow through the node into the live value
/// and publish as an observable stream grouped under the namespace, the
/// same tree Data and Graphics browse.
#[test]
fn a_script_sys_set_publishes_the_value_and_the_stream() {
    let mut app = App::headless();
    app.send(crate::bus::BusCommand::DefineSysVar(
        crate::bus::SysVarDef {
            namespace: "Demo".to_string(),
            name: "Setpoint".to_string(),
            init: 10.0,
            min: None,
            max: None,
            unit: String::new(),
            comment: String::new(),
            label_sets: Vec::new(),
            band_set_id: None,
            labels: Vec::new(),
        },
    ));
    app.send(crate::bus::BusCommand::AddNode {
        name: "calc".to_string(),
        channel: 0,
        attached: None,
    });
    app.settle();
    let id = app.snap.nodes[0].id;
    app.send(crate::bus::BusCommand::SetNodeSource {
        id,
        source: "on timer 10 { sys_set(\"Demo::Setpoint\", 7 * 6); }".to_string(),
    });
    app.settle();
    app.start_virtual();
    app.settle();
    for t in 1..=30 {
        app.advance_clock(t * 1_000);
        app.tick(t * 1_000);
    }
    assert_eq!(app.snap.sysvars[0].value, 42.0, "the script write landed");
    let key = SigKey::can_in(
        crate::model::ClusterId(1),
        crate::app::EMITTED_ID_BASE | (crate::bus::SYSVAR_STREAM_ID as u32),
        false,
        "Setpoint".to_string(),
    );
    assert!(
        app.snap
            .emitted
            .iter()
            .any(|(k, owner)| k == &key && owner == "Demo"),
        "the stream shows under its namespace"
    );
    assert_eq!(
        app.subs.get(&key).expect("subscribed").latest,
        42.0,
        "observers see the sample"
    );
    app.stop();
}

/// The Write ring mirrors node prints and command news into one
/// system-wide stream, classified by kind, and the Clear command
/// empties it.
#[test]
fn the_write_ring_collects_node_and_command_lines() {
    let mut app = App::headless();
    app.send(crate::bus::BusCommand::AddNode {
        name: "talker".to_string(),
        channel: 0,
        attached: None,
    });
    app.settle();
    let id = app.snap.nodes[0].id;
    app.send(crate::bus::BusCommand::SetNodeSource {
        id,
        source: "on timer 10 { print(\"beat\"); }".to_string(),
    });
    app.settle();
    app.start_virtual();
    app.settle();
    assert!(
        app.snap
            .write
            .iter()
            .any(|l| { l.kind == crate::bus::WriteKind::Info && l.text.contains("measuring") }),
        "run start news lands as info"
    );
    for t in 1..=20 {
        app.advance_clock(t * 1_000);
        app.tick(t * 1_000);
    }
    let script_lines: Vec<_> = app
        .snap
        .write
        .iter()
        .filter(|l| l.kind == crate::bus::WriteKind::Script && l.text.contains("beat"))
        .collect();
    assert!(
        !script_lines.is_empty(),
        "node prints land in the ring under the node's name"
    );
    assert!(script_lines[0].text.starts_with("[talker]"), "attribution");

    app.send(crate::bus::BusCommand::ClearWrite);
    app.settle();
    assert!(app.snap.write.is_empty(), "Clear empties the ring");
    app.stop();
}

/// What the bar cannot keep, the ring must. A message produced *inside* a step
/// -- a replay reaching the end of its log, a trigger firing, a post-roll
/// ending -- is replaced by the next frame's news, which in a running
/// measurement is a few milliseconds later. The line the operator was reading
/// is gone before they finished it, so it joins the Write ring on its way past
/// the bar.
#[test]
fn a_step_message_stays_in_the_write_window() {
    let mut app = App::headless();
    let src = write_mixed_fr_asc("roxy_can_step_news", 4, 13);
    replay_to_the_end(&mut app, &src);
    std::fs::remove_file(&src).ok();
    assert!(
        app.snap.write.iter().any(|l| {
            l.kind == crate::bus::WriteKind::Info && l.text.starts_with("replay finished")
        }),
        "the message the step produced is still readable: {:?}",
        app.snap
            .write
            .iter()
            .map(|l| l.text.as_str())
            .collect::<Vec<_>>()
    );
}

/// A project whose files moved loads the parts it can and says so -- but the
/// half that failed used to be written to the bar and then overwritten by the
/// "project loaded" line two statements later, so the user never saw it at all.
/// Each problem now stays in the Write ring, and the bar carries the count.
/// The Messages and Statistics tables read names and counts from the unified
/// model; this pins that the snapshot actually carries them for every frame
/// the default workspace's tables declare.
#[test]
fn the_snapshot_carries_model_frame_metadata() {
    let app = App::headless();
    let db = app.snap.channels[0]
        .dbc
        .as_ref()
        .expect("the sample database");
    let (id, message) = db
        .messages
        .iter()
        .find(|(_, message)| !message.signals.is_empty())
        .expect("a described message");
    let meta = app
        .snap
        .model_frames
        .get(&(0, id.0, id.1))
        .expect("the model names the frame");
    assert_eq!(meta.name, message.name);
    assert_eq!(meta.signal_count, message.signals.len());
}

/// Import and plan warnings must be readable after the load; the Write log
/// carries them once per message even when the model is rebuilt repeatedly.
#[test]
fn model_issues_reach_the_write_log_once() {
    let mut app = App::headless();
    let issue = crate::model::Issue::warning(
        crate::model::IssueKind::UnsupportedTransformation,
        "T1 is shown raw",
    );
    app.report_model_issues("CAN1", std::slice::from_ref(&issue));
    app.report_model_issues("CAN1", std::slice::from_ref(&issue));
    app.refresh_snapshot();
    let hits = app
        .snap
        .write
        .iter()
        .filter(|line| line.text.contains("shown raw"))
        .count();
    assert_eq!(hits, 1, "the warning is written once, not per rebuild");
}

/// E2E counters are tracked per message from the model plan and published
/// with the aggregates; the CRC verdict is not claimed.
#[test]
fn e2e_counter_continuity_is_published_per_message() {
    use crate::model::{
        CanParams, Cluster, ClusterId, ClusterKind, Comm, E2eProtection, E2eProtectionId, Frame,
        FrameAddress, FrameId, FramePduMapping, FrameTiming, MappingId, Network, NetworkId,
        NetworkParams, Pdu, PduId, PduKind,
    };
    let mut app = App::headless();
    let mut comm = Comm::default();
    comm.clusters.insert(
        ClusterId(1),
        Cluster {
            id: ClusterId(1),
            kind: ClusterKind::Can,
            name: "CAN1".to_string(),
            aliases: Vec::new(),
            description: String::new(),
            sources: Vec::new(),
            networks: vec![NetworkId(1)],
        },
    );
    comm.networks.insert(
        NetworkId(1),
        Network {
            id: NetworkId(1),
            cluster: ClusterId(1),
            name: "CAN1".to_string(),
            params: NetworkParams::Can(CanParams::default()),
        },
    );
    comm.pdus.insert(
        PduId(1),
        Pdu {
            id: PduId(1),
            name: "P".to_string(),
            length: 8,
            kind: PduKind::Signal,
            header_type: crate::model::ContainerHeaderType::Short,
            cycle_us: None,
            selector: None,
            secured: None,
            transformation: None,
            e2e: Some(E2eProtectionId(1)),
        },
    );
    comm.e2e.insert(
        E2eProtectionId(1),
        E2eProtection {
            id: E2eProtectionId(1),
            name: "E".to_string(),
            profile_name: "Profile1".to_string(),
            profile: Some(1),
            data_id: Some(0x123),
            system_signal: None,
            payload: None,
            counter_offset: None,
            counter_length: None,
            crc_offset: None,
            crc_length: None,
        },
    );
    comm.frame_pdu_mappings.insert(
        MappingId(1),
        FramePduMapping {
            id: MappingId(1),
            frame: FrameId(1),
            pdu: PduId(1),
            start_bit: 0,
            byte_order: crate::model::ByteOrder::LittleEndian,
        },
    );
    comm.frames.insert(
        FrameId(1),
        Frame {
            id: FrameId(1),
            cluster: ClusterId(1),
            network: NetworkId(1),
            name: "Protected".to_string(),
            length: 8,
            address: FrameAddress::Can {
                id: 0x123,
                extended: false,
                fd: false,
            },
            timing: FrameTiming::default(),
            comment: String::new(),
            ethernet: None,
            pdu_mappings: vec![MappingId(1)],
        },
    );
    app.send(crate::bus::BusCommand::SetChannelModel {
        ch: 0,
        comm: Some(std::sync::Arc::new(comm)),
        sources: Vec::new(),
    });
    app.settle();
    app.triggers.push(Trigger::new(
        TriggerCond::E2eIssue {
            ch: 0,
            id: 0x123,
            ext: false,
        },
        TriggerAction::InsertMarker,
    ));
    let frame = |t: u64, counter: u8| {
        let mut f = frame_at(t, 0x123, 8, Direction::Rx);
        f.data[0] = 0xAA;
        f.data[1] = counter;
        f
    };
    receive(
        &mut app,
        10_000,
        vec![
            frame(10_000, 0),
            frame(20_000, 1),
            frame(30_000, 1),
            frame(40_000, 5),
        ],
    );
    let view = app
        .snap
        .e2e
        .get(&(0, 0x123, false))
        .expect("E2E continuity published");
    assert_eq!(view.profile, 1);
    assert_eq!(view.data_id, Some(0x123));
    assert_eq!(view.frames, 4);
    assert_eq!(view.counter, Some(5));
    assert_eq!(view.repeats, 1, "0,1,1 repeats once");
    assert_eq!(view.jumps, 1, "1 -> 5 is a jump");
    assert_eq!(app.snap.markers, [40_000], "the E2E jump fired the rule");
}

/// Frame decoding must find the model through the channel's current cluster
/// id. A project that restored ids like 42 used to fall back to the legacy
/// decoder because the lookup assumed `channel + 1`.
#[test]
fn model_decoding_follows_restored_cluster_ids() {
    let mut app = App::headless();
    let db = app.snap.channels[0]
        .dbc
        .clone()
        .expect("the sample database");
    let ((id, extended), message) = db
        .messages
        .iter()
        .find(|(_, message)| !message.signals.is_empty())
        .expect("a described message");
    app.send(crate::bus::BusCommand::SetClusterIds {
        next: 100,
        can: vec![42, 43],
        fr: Default::default(),
        eth: Default::default(),
    });
    app.settle();
    assert_eq!(app.snap.cluster_ids.can[0], 42);
    let mut frame = frame_at(1_000, *id, message.dlc.max(1) as u8, Direction::Rx);
    frame.extended = *extended;
    assert!(
        app.model_can_all_values(&frame).is_some(),
        "the model is found through the restored cluster id"
    );
}

/// FlexRay and Ethernet ARXML go through FIBEX/EthDb-shaped tables for legacy
/// decoding, but the core must also hold the document's own model -- and keep
/// holding it after a reopen, where `SetClusterIds` rebuilds the adapter
/// models around the restored ids.
#[test]
fn fr_and_eth_arxml_models_reach_the_core_and_survive_reopen() {
    let fr = crate::test_assets::path("PowerTrain.arxml");
    if !std::path::Path::new(fr).exists() {
        println!("asset absent -- skipped");
        return;
    }
    let mut app = App::headless();
    let fr_bus = app
        .load_cluster_description(fr, Some(0))
        .expect("FR description");
    let fr_model = app.fr_models.get(&fr_bus).expect("the FR model");
    let fr_cluster = fr_model.clusters.values().next().expect("one cluster");
    assert!(
        matches!(
            fr_cluster.sources.first(),
            Some(crate::model::ClusterSource::Arxml { .. })
        ),
        "FR model is read from the document: {:?}",
        fr_cluster.sources
    );
    assert_eq!(
        Some(&fr_cluster.id.0),
        app.snap.cluster_ids.fr.get(&fr_bus),
        "and bound to the bus's global id"
    );

    assert!(
        app.snap
            .fr_model_frames
            .values()
            .any(|frame| frame.signal_count > 0),
        "the snapshot carries FR model frame counts"
    );

    let eth_bus = app
        .load_eth_description(crate::test_assets::eth_arxml(), Some(0))
        .expect("Ethernet description");
    let eth_model = app.eth_models.get(&eth_bus).expect("the Ethernet model");
    let eth_cluster = eth_model.clusters.values().next().expect("one cluster");
    assert!(matches!(
        eth_cluster.sources.first(),
        Some(crate::model::ClusterSource::Arxml { .. })
    ));
    assert_eq!(
        Some(&eth_cluster.id.0),
        app.snap.cluster_ids.eth.get(&eth_bus)
    );
    assert!(
        app.snap
            .eth_model_frames
            .get("EthFrame")
            .is_some_and(|frame| frame.signal_count > 0),
        "the snapshot carries Ethernet model frame counts"
    );

    let path = std::env::temp_dir().join(format!(
        "roxy_can_fr_eth_models_{}.rxproj",
        std::process::id()
    ));
    assert!(app.save_project(Some(path.clone())), "save writes the file");
    let mut restored = App::headless();
    restored.open_project_path(&path);
    restored.settle();
    let fr_model = restored
        .fr_models
        .get(&fr_bus)
        .expect("FR model after reopen");
    let fr_cluster = fr_model.clusters.values().next().expect("one cluster");
    assert!(
        matches!(
            fr_cluster.sources.first(),
            Some(crate::model::ClusterSource::Arxml { .. })
        ),
        "the reopen restores the FR document model"
    );
    assert_eq!(fr_cluster.id.0, restored.snap.cluster_ids.fr[&fr_bus]);
    let eth_model = restored
        .eth_models
        .get(&eth_bus)
        .expect("Ethernet model after reopen");
    let eth_cluster = eth_model.clusters.values().next().expect("one cluster");
    assert!(matches!(
        eth_cluster.sources.first(),
        Some(crate::model::ClusterSource::Arxml { .. })
    ));
    assert_eq!(eth_cluster.id.0, restored.snap.cluster_ids.eth[&eth_bus]);
}

#[test]
fn a_project_load_problem_outlives_the_loaded_line() {
    let asset = crate::test_assets::path("PowerTrain.arxml");
    if !std::path::Path::new(asset).exists() {
        println!("asset absent -- skipped");
        return;
    }
    let mut app = App::headless();
    assert_eq!(app.load_cluster_description(asset, Some(0)), Some(0));
    let path = std::env::temp_dir().join(format!(
        "roxy_can_half_loaded_{}.rxproj",
        std::process::id()
    ));
    assert!(app.save_project(Some(path.clone())), "save writes the file");

    // The description is the part that went missing between sessions.
    let mut doc: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
    let fr = doc["clusters"]
        .as_array_mut()
        .expect("clusters")
        .iter_mut()
        .find(|cluster| cluster["kind"] == "flexray")
        .expect("flexray cluster");
    fr["sources"][0]["path"] = "assets/no-such-cluster.xml".into();
    std::fs::write(&path, serde_json::to_string(&doc).unwrap()).unwrap();

    let mut restored = App::headless();
    restored.open_project_path(&path);
    restored.settle();
    assert!(
        restored.status.contains("project loaded")
            && restored.status.contains("1 item(s) failed to load"),
        "the bar says both what worked and how much did not: {}",
        restored.status
    );
    assert!(
        restored.snap.write.iter().any(|l| {
            l.kind == crate::bus::WriteKind::Error
                && l.text.contains("FlexRay description load failed")
        }),
        "and the reason is still there to read: {:?}",
        restored
            .snap
            .write
            .iter()
            .map(|l| l.text.as_str())
            .collect::<Vec<_>>()
    );
    std::fs::remove_file(&path).ok();
}

/// The frontend's own failures carry a reason the operator may need after the
/// bar has moved on -- an OS error, a parser message, the path it happened to.
/// A refused project open is the case where there is nothing else on screen to
/// explain why clicking the file did nothing.
#[test]
fn a_refused_project_open_is_still_readable() {
    let mut app = App::headless();
    let missing = std::env::temp_dir().join(format!(
        "roxy_can_not_a_project_{}.rxproj",
        std::process::id()
    ));
    app.open_project_path(&missing);
    app.settle();
    assert!(
        app.status.starts_with("project read failed"),
        "{}",
        app.status
    );
    assert!(
        app.snap.write.iter().any(|l| {
            l.kind == crate::bus::WriteKind::Error && l.text.starts_with("project read failed")
        }),
        "the refusal outlives the one line on the bar"
    );
}

/// The Write export writes the ring as plain text in display order and
/// shape, honouring the window's per-kind filter -- "what I see is what
/// I save". An empty (or fully filtered) ring refuses rather than
/// writing an empty file.
#[test]
fn the_write_export_honours_the_kind_filter() {
    let mut app = App::headless();
    app.send(crate::bus::BusCommand::AddNode {
        name: "talker".to_string(),
        channel: 0,
        attached: None,
    });
    app.settle();
    let id = app.snap.nodes[0].id;
    app.send(crate::bus::BusCommand::SetNodeSource {
        id,
        source: "on timer 10 { print(\"beat\"); }".to_string(),
    });
    app.settle();
    app.start_virtual();
    app.settle();
    for t in 1..=20 {
        app.advance_clock(t * 1_000);
        app.tick(t * 1_000);
    }
    assert!(app.snap.write.len() >= 2, "info and script lines exist");

    // All kinds on: every line lands in the file.
    let path = std::env::temp_dir().join("roxy_can_write_export_test.txt");
    app.export_write_txt(&path.to_string_lossy());
    let content = std::fs::read_to_string(&path).unwrap();
    assert_eq!(
        content.lines().count(),
        app.snap.write.len(),
        "one line per ring entry"
    );
    assert!(
        content
            .lines()
            .all(|l| l.starts_with('[') && l.contains("] ")),
        "lines carry the wall stamp: {content:?}"
    );

    // Script prints off: they leave the file too.
    app.write_filter[0] = false;
    app.export_write_txt(&path.to_string_lossy());
    let content = std::fs::read_to_string(&path).unwrap();
    assert!(
        !content.contains("beat"),
        "the filtered kind is not exported: {content:?}"
    );
    assert!(!content.is_empty(), "the unfiltered kinds still land");

    // Everything filtered off: refuse, keep the old file.
    let stale = std::fs::read_to_string(&path).unwrap();
    for slot in app.write_filter.iter_mut() {
        *slot = false;
    }
    app.export_write_txt(&path.to_string_lossy());
    assert_eq!(
        std::fs::read_to_string(&path).unwrap(),
        stale,
        "an empty export writes nothing"
    );
    std::fs::remove_file(&path).ok();
    app.stop();
}

/// A script writing an undefined variable gets a warning in its log and
/// the write is dropped; the node keeps running.
#[test]
fn an_undefined_sysvar_write_warns_and_is_dropped() {
    let mut app = App::headless();
    app.send(crate::bus::BusCommand::AddNode {
        name: "loose".to_string(),
        channel: 0,
        attached: None,
    });
    app.settle();
    let id = app.snap.nodes[0].id;
    app.send(crate::bus::BusCommand::SetNodeSource {
        id,
        source: "on timer 10 { sys_set(\"Nowhere::X\", 1); }".to_string(),
    });
    app.settle();
    app.start_virtual();
    app.settle();
    for t in 1..=20 {
        app.advance_clock(t * 1_000);
        app.tick(t * 1_000);
    }
    let node = app.snap.nodes.first().expect("node present");
    assert!(!node.errored, "a bad sysvar write does not stop the node");
    assert!(
        node.log.iter().any(|l| l.contains("Nowhere::X")),
        "the undefined write is named in the log: {:?}",
        node.log
    );
    app.stop();
}

/// System variable definitions persist with the project and restore via
/// the Define path.
#[test]
fn sysvars_survive_a_project_roundtrip() {
    let mut app = App::headless();
    app.band_sets.push(crate::band::BandSet {
        id: "rate".to_string(),
        name: "Rate bands".to_string(),
        bands: vec![crate::band::Band {
            upper: None,
            inclusive: false,
            text: "Nominal".to_string(),
            color: Some(1),
        }],
    });
    app.send(crate::bus::BusCommand::DefineSysVar(
        crate::bus::SysVarDef {
            namespace: "Demo".to_string(),
            name: "Rate".to_string(),
            init: 2.5,
            min: Some(0.0),
            max: Some(10.0),
            unit: "Hz".to_string(),
            comment: String::new(),
            label_sets: Vec::new(),
            band_set_id: Some("rate".to_string()),
            labels: Vec::new(),
        },
    ));
    app.settle();
    let json = serde_json::to_string(&ProjectV2::from_app(&app, None)).unwrap();
    let mut restored = App::headless();
    serde_json::from_str::<ProjectV2>(&json)
        .unwrap()
        .apply(&mut restored);
    restored.settle();
    let v = &restored.snap.sysvars[0];
    assert_eq!(v.def.band_set_id.as_deref(), Some("rate"));
    let set = restored
        .band_sets
        .iter()
        .find(|set| set.id == "rate")
        .expect("band set restored");
    assert_eq!(set.bands[0].text, "Nominal");
    assert_eq!(v.value, 2.5, "value starts at init");
}
/// A handler that sends the frame it receives re-enters itself. The step's
/// reaction budget stops that from wedging the core and leaves one warning.
#[test]
fn a_self_echoing_node_cannot_wedge_a_step() {
    let mut app = quiet_app();
    app.send(crate::bus::BusCommand::AddNode {
        name: "echo".to_string(),
        channel: 0,
        attached: None,
    });
    app.settle();
    let id = app.snap.nodes[0].id;
    app.send(crate::bus::BusCommand::SetNodeSource {
        id,
        source: "on message 0x100 { send(frame_id(), 1); }".to_string(),
    });
    app.send(crate::bus::BusCommand::SetNodeEnabled { id, on: true });
    app.start_virtual();
    app.settle();
    receive(
        &mut app,
        1_000,
        vec![rx_frame(1_000, 0x100, 1, FrameFlags::NONE)],
    );
    assert!(
        app.frame_counter < 5_000,
        "the same-step echo loop is bounded, got {} frames",
        app.frame_counter
    );
    assert!(
        app.snap
            .write
            .iter()
            .any(|line| line.text.contains("reaction limit")),
        "the limit is explained in Write: {:?}",
        app.snap.write
    );
    app.stop();
}

/// A script reads the core's old math channels through `derived("Name")`:
/// the value injected into every channel's host input is the newest sample
/// from the previous lap.
#[test]
fn scripts_read_core_derived_channels_by_name() {
    let mut app = App::headless();
    let a = SigKey::can_in(crate::model::ClusterId(1), 0x100, false, "A");
    app.subscribe_signal(a);
    app.set_derived_defs(vec![crate::derived::DerivedDef {
        id: "d".into(),
        name: "Sum".into(),
        expression: "[A] + 0".into(),
        ..Default::default()
    }]);
    app.send(crate::bus::BusCommand::AddNode {
        name: "reader".to_string(),
        channel: 0,
        attached: None,
    });
    app.settle();
    let id = app.snap.nodes[0].id;
    app.send(crate::bus::BusCommand::SetNodeSource {
        id,
        source: "on timer 1 { print(derived(\"Sum\")); }".to_string(),
    });
    app.settle();
    app.start_virtual();
    app.settle();
    let key = SigKey::derived("d", "Sum");
    {
        let sub = app.subs.get_mut(&key).expect("the lane is subscribed");
        sub.latest = 9.0;
        sub.push_sample(1_000, 9.0, 1);
    }
    for t in 1..=20 {
        app.advance_clock(t * 1_000);
        app.tick(t * 1_000);
    }
    app.settle();
    let node = app.snap.nodes.first().expect("node present");
    assert!(
        !node.errored,
        "the derived builtin compiles: {:?}",
        node.log
    );
    assert!(
        node.log.iter().any(|line| line.contains("9")),
        "the script read the sampled lane: {:?}",
        node.log
    );
    app.stop();
}

/// Frame-driven handlers can publish too: every matching frame becomes a
/// derived-signal sample through the dispatch path (mirrors, conversions).
#[test]
fn emit_value_works_from_message_handlers() {
    let mut app = App::headless();
    app.send(crate::bus::BusCommand::AddNode {
        name: "mirror".to_string(),
        channel: 0,
        attached: None,
    });
    app.settle();
    let id = app.snap.nodes[0].id;
    app.send(crate::bus::BusCommand::SetNodeSource {
        id,
        source: "on message 0x200 { emit_value(\"Seen\", 7); }".to_string(),
    });
    app.settle();
    app.start_virtual();
    app.settle();

    receive(
        &mut app,
        5_000,
        vec![CanFrame {
            t_us: 5_000,
            channel: 0,
            id: 0x200,
            extended: false,
            len: 1,
            data: [0; MAX_CAN_FD_LEN],
            dir: Direction::Rx,
            flags: FrameFlags::NONE,
        }],
    );
    app.settle();

    let key = SigKey::can_in(
        crate::model::ClusterId(1),
        crate::app::EMITTED_ID_BASE | id as u32,
        false,
        "Seen".to_string(),
    );
    let sub = app.subs.get(&key).expect("frame-driven emission published");
    assert_eq!(sub.latest, 7.0, "the handler ran once for the frame");
    app.stop();
}

/// Streams track their owner: a rename rewrites the tree's owner label,
/// a rebinding to another bus re-homes the stream keys, and a handler
/// that errors after emitting discards its partial outputs (an errored
/// callback is atomic -- half a computation is not a sample).
#[test]
fn derived_streams_follow_their_node_across_edits() {
    let mut app = App::headless();
    app.send(crate::bus::BusCommand::AddNode {
        name: "calc".to_string(),
        channel: 0,
        attached: None,
    });
    app.settle();
    let id = app.snap.nodes[0].id;
    app.send(crate::bus::BusCommand::SetNodeSource {
        id,
        source: "on start { emit_value(\"X\", 1); }".to_string(),
    });
    app.send(crate::bus::BusCommand::SetNodeEnabled { id, on: true });
    app.start_virtual();
    app.settle();
    for t in 1..=5u64 {
        app.advance_clock(t * 1_000);
        app.tick(t * 1_000);
    }
    let key = |ch: u8| {
        crate::observe::SigKey::can_in(
            crate::model::ClusterId(ch as u32 + 1),
            crate::app::EMITTED_ID_BASE | id as u32,
            false,
            "X",
        )
    };
    assert!(
        app.snap.emitted.iter().any(|(k, _)| *k == key(0)),
        "the stream opened on the node's bus"
    );

    // Rename: owner label changes, the key does not.
    app.send(crate::bus::BusCommand::SetNodeName {
        id,
        name: "renamed".to_string(),
    });
    app.settle();
    assert!(
        app.snap
            .emitted
            .iter()
            .any(|(k, owner)| *k == key(0) && owner == "renamed"),
        "the tree shows the new owner"
    );
    app.stop();
}

/// An errored handler discards its partial emissions: `emit_value` calls
/// before a runtime error never reach the bus -- half a computation is
/// not a sample.
#[test]
fn an_errored_handler_discards_its_emissions() {
    let mut app = App::headless();
    app.send(crate::bus::BusCommand::AddNode {
        name: "boom".to_string(),
        channel: 0,
        attached: None,
    });
    app.settle();
    let id = app.snap.nodes[0].id;
    app.send(crate::bus::BusCommand::SetNodeSource {
        id,
        // Division by zero is the easiest runtime error.
        source: "on start { emit_value(\"Ghost\", 1); print(1 / 0); }".to_string(),
    });
    app.send(crate::bus::BusCommand::SetNodeEnabled { id, on: true });
    app.start_virtual();
    app.settle();
    for t in 1..=5u64 {
        app.advance_clock(t * 1_000);
        app.tick(t * 1_000);
    }

    let node = app.snap.nodes.iter().find(|n| n.id == id).expect("node");
    assert!(node.errored, "the division tripped the fuse");
    assert!(
        !app.snap.emitted.iter().any(|(k, _)| k.name() == "Ghost"),
        "the partial emission never published"
    );
    app.stop();
}

/// Removing a bus takes its script nodes, replay blocks, and derived
/// streams with it and shifts the survivors' bindings down -- the same
/// remap the tx entries and the windows follow.
#[test]
fn removing_a_bus_remaps_nodes_blocks_and_streams() {
    let mut app = App::headless();
    app.send(crate::bus::BusCommand::AddNode {
        name: "gone".to_string(),
        channel: 0,
        attached: None,
    });
    app.send(crate::bus::BusCommand::AddNode {
        name: "stay".to_string(),
        channel: 1,
        attached: None,
    });
    app.settle();
    let stay_id = app
        .snap
        .nodes
        .iter()
        .find(|n| n.name == "stay")
        .expect("stay node")
        .id;
    app.send(crate::bus::BusCommand::SetNodeSource {
        id: stay_id,
        source: "on start { emit_value(\"X\", 1); }".to_string(),
    });
    app.send(crate::bus::BusCommand::SetNodeEnabled {
        id: stay_id,
        on: true,
    });
    app.add_replay_block(1, "blk".to_string(), String::new(), None, None);
    app.settle();

    app.start_virtual();
    app.settle();
    for t in 1..=10u64 {
        app.advance_clock(t * 1_000);
        app.tick(t * 1_000);
    }
    assert!(
        app.snap.emitted.iter().any(|(k, _)| matches!(
            k,
            SigKey::Can {
                cluster: crate::model::ClusterId(2),
                ..
            }
        )),
        "the surviving node emitted on bus CAN2"
    );

    app.remove_channel(0);
    app.settle();

    let nodes: Vec<&str> = app.snap.nodes.iter().map(|n| n.name.as_str()).collect();
    assert_eq!(nodes, ["stay"], "the removed bus's node went with it");
    assert_eq!(
        app.snap.nodes[0].channel, 0,
        "the survivor's binding shifted down"
    );
    assert_eq!(app.snap.blocks.len(), 1);
    assert_eq!(app.snap.blocks[0].channel, 0, "the block shifted down too");
    assert!(
        app.snap.emitted.iter().all(|(k, _)| matches!(
            k,
            SigKey::Can {
                cluster: crate::model::ClusterId(2),
                ..
            }
        )),
        "derived streams keep their cluster; the bus index moved under them"
    );
    app.stop();
}

/// Triggers bind to a bus by index too: a rule watching the removed bus
/// is meaningless, and a Send reaction aimed at it has no target -- both
/// are dropped; the survivors shift down. A FlexRay rule's index is not in
/// that space at all, so removal neither drops it nor renumbers it.
#[test]
fn removing_a_bus_drops_its_triggers_and_shifts_the_rest() {
    let mut app = App::headless();
    app.triggers.push(Trigger::new(
        TriggerCond::IdPresent { ch: 1, id: 199 },
        TriggerAction::StartRecording,
    ));
    app.triggers.push(Trigger::new(
        TriggerCond::IdPresent { ch: 0, id: 0x100 },
        TriggerAction::StartRecording,
    ));
    app.triggers.push(Trigger::new(
        TriggerCond::IdPresent { ch: 1, id: 200 },
        TriggerAction::Send { ch: 0, id: 0x100 },
    ));
    app.triggers.push(Trigger::new(
        TriggerCond::ErrorFrame { ch: 1 },
        TriggerAction::Send { ch: 1, id: 199 },
    ));
    app.triggers.push(Trigger::new(
        TriggerCond::FrFramePresent { bus: 1, slot: 13 },
        TriggerAction::StartRecording,
    ));
    app.refresh_snapshot();

    app.remove_channel(0);
    app.settle();

    assert_eq!(app.snap.triggers.len(), 3, "two CAN rules and the FR one");
    assert_eq!(app.snap.triggers[0].cond.can_bus(), Some(0), "shifted down");
    assert_eq!(app.snap.triggers[1].cond.can_bus(), Some(0));
    assert_eq!(
        app.snap.triggers[1].action,
        TriggerAction::Send { ch: 0, id: 199 },
        "the Send reaction follows its bus down"
    );
    assert_eq!(
        app.snap.triggers[2].cond.fr_bus(),
        Some(1),
        "a FlexRay bus index is not a CAN channel: it survives as it was"
    );
    assert_eq!(app.snap.triggers[2].cond.can_bus(), None);
}

/// Queues one FlexRay frame on a mock port, steps the core to `t_us`, and
/// pulls the snapshot back. A free function rather than a closure: the closure
/// would hold the app borrowed across the assertions that read it.
fn fr_step(app: &mut App, q: &crate::hw::MockFrHandles, slot: u16, t_us: u64) {
    use crate::hw::vector::flexray::FrFrame;
    q.lock().expect("mock lock").push_back(FrFrame {
        slot,
        cycle: 0,
        payload: vec![0, 0],
        header_crc: 0,
        flags: 0,
    });
    app.advance_clock(t_us);
    app.tick(t_us);
    app.refresh_snapshot();
}

/// A FlexRay arrival arms the same trigger layer a CAN frame does: the edge on
/// the watched slot acts, an arrival of the same slot on another cluster does
/// not, and the presence latch holds until it is re-armed.
#[test]
fn a_flexray_arrival_fires_a_trigger_on_the_watched_bus() {
    let mut app = quiet_app();
    app.tx_list.retain(|t| t.channel != 0);
    let q0 = app.hw.attach_fr_mock(0, 5);
    let q1 = app.hw.attach_fr_mock(1, 6);
    app.start_virtual();
    app.triggers.push(Trigger::new(
        TriggerCond::FrFramePresent { bus: 1, slot: 13 },
        TriggerAction::InsertMarker,
    ));

    fr_step(&mut app, &q0, 13, 10_000);
    assert!(
        app.snap.markers.is_empty(),
        "the watched rule names cluster 1, not this one"
    );

    fr_step(&mut app, &q1, 13, 20_000);
    assert_eq!(
        app.snap.markers,
        [20_000],
        "the edge stamps the bus clock it arrived on"
    );

    fr_step(&mut app, &q1, 13, 30_000);
    assert_eq!(app.snap.markers.len(), 1, "presence latches for the run");

    app.send(crate::bus::BusCommand::RearmTriggers);
    fr_step(&mut app, &q1, 13, 40_000);
    assert_eq!(
        app.snap.markers,
        [20_000, 40_000],
        "re-arming lets the next arrival fire again"
    );
    app.stop();
}

/// A FlexRay slot timeout rides the same grace as the spec verdict: no
/// arrival means no opinion, and silence past grace is a fresh edge.
#[test]
fn a_flexray_slot_timeout_fires_after_grace() {
    let mut app = quiet_app();
    app.tx_list.retain(|t| t.channel != 0);
    app.load_cluster_description(crate::test_assets::path("PowerTrain.arxml"), Some(0));
    let q0 = app.hw.attach_fr_mock(0, 5);
    app.start_virtual();
    app.triggers.push(Trigger::new(
        TriggerCond::FrTimeout { bus: 0, slot: 13 },
        TriggerAction::InsertMarker,
    ));

    let mut status = String::new();
    app.eval_timeout_triggers(10_000, 3, &mut status);
    assert!(app.snap.markers.is_empty(), "never seen: no opinion");

    fr_step(&mut app, &q0, 13, 10_000);
    let mut status = String::new();
    app.eval_timeout_triggers(10_000_000, 3, &mut status);
    app.refresh_snapshot();
    assert_eq!(
        app.snap.markers,
        [10_000_000],
        "silence past grace fires the rule"
    );
    app.stop();
}

/// A FlexRay send action queues the whole entry, not a payload typed into the
/// rule: the generator row's width and encoded signals are what goes out.
#[test]
fn a_trigger_sends_a_whole_flexray_entry() {
    let mut app = quiet_app();
    app.load_cluster_description(crate::test_assets::path("PowerTrain.arxml"), Some(0));
    app.send(crate::bus::BusCommand::AddFrEntry {
        bus: 0,
        slot: 13,
        node: String::new(),
    });
    app.settle();
    let width = app.fr_tx_list[0].len;
    app.triggers.push(Trigger::new(
        TriggerCond::IdPresent { ch: 0, id: 0x700 },
        TriggerAction::SendFr { bus: 0, slot: 13 },
    ));

    receive(
        &mut app,
        10_000,
        vec![rx_frame(10_000, 0x700, 8, FrameFlags::NONE)],
    );
    assert_eq!(app.fr_from_scripts.len(), 1, "one whole entry was queued");
    let queued = app.fr_from_scripts[0].clone();
    assert_eq!((queued.1, queued.2), (0, 13));
    assert_eq!(queued.3.len(), width, "the row's declared width is kept");

    app.advance_clock(20_000);
    app.tick(20_000);
    app.refresh_snapshot();
    assert!(
        app.snap
            .fr_trace
            .iter()
            .any(|row| row.bus == 0 && row.slot == 13),
        "the entry landed in the next generator block: queued={} trace={:?}",
        app.fr_from_scripts.len(),
        app.snap.fr_trace
    );
    app.stop();
}

/// An Ethernet send action injects one whole matrix frame: the definition
/// supplies VLAN/EtherType/length, the rule supplies the payload bytes.
#[test]
fn a_trigger_sends_a_whole_ethernet_frame() {
    let mut app = quiet_app();
    app.measuring = true;
    app.eth_dbs.insert(
        0,
        std::sync::Arc::new(crate::eth::db::EthDb {
            clusters: vec![crate::eth::db::EthCluster {
                name: "ETH1".into(),
                vlans: vec![100],
            }],
            frames: vec![crate::eth::db::EthFrameDef {
                name: "Payload".into(),
                cluster: "ETH1".into(),
                vlan: Some(100),
                length: 4,
                ethertype_hint: Some(0x88B5),
                pdus: vec![],
            }],
            ..Default::default()
        }),
    );
    app.triggers.push(Trigger::new(
        TriggerCond::IdPresent { ch: 0, id: 0x700 },
        TriggerAction::SendEth {
            bus: 0,
            frame: "Payload".into(),
            data: vec![0xAA, 0xBB, 0xCC, 0xDD],
        },
    ));
    receive(
        &mut app,
        10_000,
        vec![rx_frame(10_000, 0x700, 8, FrameFlags::NONE)],
    );
    app.refresh_snapshot();
    let frame = app.snap.eth_trace.last().expect("the injected frame");
    assert_eq!(frame.payload.as_ref(), [0xAA, 0xBB, 0xCC, 0xDD]);
    assert_eq!(frame.vlan, Some(100));
    assert_eq!(frame.ethertype, 0x88B5);
    app.stop();
}

/// A captured SOME/IP datagram is decoded into the service-event ring, not
/// just left as an opaque Ethernet payload.
#[test]
fn someip_messages_reach_the_service_event_ring() {
    let mut app = quiet_app();
    app.triggers.push(crate::trigger::Trigger::new(
        crate::trigger::TriggerCond::SomeIp {
            bus: 0,
            service: 0x1234,
            method: None,
            message_type: None,
        },
        crate::trigger::TriggerAction::InsertMarker,
    ));
    let mut someip = Vec::new();
    someip.extend_from_slice(&0x1234u16.to_be_bytes());
    someip.extend_from_slice(&0x8001u16.to_be_bytes());
    someip.extend_from_slice(&8u32.to_be_bytes());
    someip.extend_from_slice(&1u16.to_be_bytes());
    someip.extend_from_slice(&7u16.to_be_bytes());
    someip.extend_from_slice(&[1, 1, 0x02, 0]);
    let mut udp = Vec::new();
    udp.extend_from_slice(&30501u16.to_be_bytes());
    udp.extend_from_slice(&30490u16.to_be_bytes());
    udp.extend_from_slice(&((8 + someip.len()) as u16).to_be_bytes());
    udp.extend_from_slice(&0u16.to_be_bytes());
    udp.extend_from_slice(&someip);
    let mut ip = vec![0u8; 20];
    ip[0] = 0x45;
    ip[2..4].copy_from_slice(&((20 + udp.len()) as u16).to_be_bytes());
    ip[9] = 17;
    ip[12..16].copy_from_slice(&[10, 0, 0, 1]);
    ip[16..20].copy_from_slice(&[10, 0, 0, 2]);
    ip.extend_from_slice(&udp);
    app.ingest_eth(crate::eth::EthFrame::session(10_000, 0, None, 0x0800, ip));
    app.refresh_snapshot();
    assert_eq!(app.snap.someip.len(), 1);
    let event = &app.snap.someip[0];
    assert_eq!((event.service, event.method), (0x1234, 0x8001));
    assert_eq!(event.message_type, 0x02);
    assert_eq!(event.session, 7);
    assert!(
        app.trigger_summary(0).starts_with("ETH0"),
        "SOME/IP rule must name the Ethernet bus, got {}",
        app.trigger_summary(0)
    );
    assert_eq!(app.snap.markers, [10_000], "the service rule fired");
}

/// A decoded SOME/IP field publishes through the ordinary signal observer as
/// a synthetic Ethernet frame, so it can be plotted like any other signal.
#[test]
fn someip_fields_publish_to_signal_subscribers() {
    let mut app = quiet_app();
    app.ensure_eth_bus(0);
    app.someip_services
        .services
        .push(crate::eth::arxml_service::ServiceDef {
            name: "SpeedService".to_string(),
            id: 0x1234,
            major: 1,
            methods: Vec::new(),
            events: vec![crate::eth::arxml_service::ServiceEvent {
                name: "SpeedEvent".to_string(),
                id: 0x8001,
                fields: vec![crate::eth::arxml_service::ServiceField {
                    name: "speed".to_string(),
                    ty: crate::eth::arxml_service::ServiceType::U16,
                    props: Default::default(),
                }],
            }],
            instances: Vec::new(),
            buses: vec![0],
            issues: Vec::new(),
        });
    app.send(crate::bus::BusCommand::SetSomeIpServices(
        app.someip_services.clone(),
    ));
    app.settle();
    let key = app.eth_key(0, "SOMEIP 0x1234/0x8001".to_string(), "speed".to_string());
    app.subscribe_signal(key.clone());

    let mut someip = Vec::new();
    someip.extend_from_slice(&0x1234u16.to_be_bytes());
    someip.extend_from_slice(&0x8001u16.to_be_bytes());
    someip.extend_from_slice(&10u32.to_be_bytes());
    someip.extend_from_slice(&1u16.to_be_bytes());
    someip.extend_from_slice(&7u16.to_be_bytes());
    someip.extend_from_slice(&[1, 1, 0x02, 0, 0, 7]);
    let mut udp = Vec::new();
    udp.extend_from_slice(&30501u16.to_be_bytes());
    udp.extend_from_slice(&30490u16.to_be_bytes());
    udp.extend_from_slice(&((8 + someip.len()) as u16).to_be_bytes());
    udp.extend_from_slice(&0u16.to_be_bytes());
    udp.extend_from_slice(&someip);
    let mut ip = vec![0u8; 20];
    ip[0] = 0x45;
    ip[2..4].copy_from_slice(&((20 + udp.len()) as u16).to_be_bytes());
    ip[9] = 17;
    ip[12..16].copy_from_slice(&[10, 0, 0, 1]);
    ip[16..20].copy_from_slice(&[10, 0, 0, 2]);
    ip.extend_from_slice(&udp);
    app.ingest_eth(crate::eth::EthFrame::session(10_000, 0, None, 0x0800, ip));
    app.settle();
    assert_eq!(app.subs.get(&key).map(|sub| sub.latest), Some(7.0));
}

/// Hardware RX: frames received on an attached adapter ingest like any
/// bus traffic -- a real node's frames arrive this way.
#[test]
fn hardware_rx_frames_ingest_like_bus_traffic() {
    let mut app = App::headless();
    app.tx_list.retain(|t| t.channel != 0);
    let (_written, incoming) = app.hw.attach_mock(0);
    app.start_virtual();
    app.settle();
    incoming.lock().expect("mock lock").push_back(CanFrame {
        t_us: 0,
        channel: 0,
        id: 0x555,
        extended: false,
        len: 1,
        data: [0; MAX_CAN_FD_LEN],
        dir: Direction::Rx,
        flags: FrameFlags::NONE,
    });
    for t in 1..=20u64 {
        app.advance_clock(t * 1_000);
        app.tick(t * 1_000);
    }
    let agg = app
        .snap
        .aggs
        .iter()
        .find(|a| a.channel == 0 && a.id == 0x555)
        .expect("the wire frame reached the bus");
    assert!(agg.count >= 1, "received hardware frames are ingested");
    app.stop();
}

/// Two FlexRay clusters watched at once: each port's rows arrive stamped with
/// their own bus, and detaching one watch leaves its sibling running. A single
/// watch per machine meant a second port's traffic merged into the first bus --
/// the same slot number on two clusters is two different frames.
#[test]
fn two_flexray_watches_feed_their_own_buses() {
    use crate::hw::vector::flexray::FrFrame;
    let fr = |slot: u16| FrFrame {
        slot,
        cycle: 1,
        payload: vec![slot as u8],
        header_crc: 0,
        flags: 0,
    };
    let mut app = App::headless();
    app.tx_list.retain(|t| t.channel != 0);
    let q0 = app.hw.attach_fr_mock(0, 5);
    let q1 = app.hw.attach_fr_mock(1, 6);
    app.start_virtual();
    q0.lock().expect("mock lock").push_back(fr(13));
    q1.lock().expect("mock lock").push_back(fr(13));
    for t in 1..=4u64 {
        app.advance_clock(t * 1_000);
        app.tick(t * 1_000);
    }
    app.refresh_snapshot();
    let rows = app.snap.fr_aggs.clone();
    assert_eq!(
        rows.iter().map(|a| (a.bus, a.slot)).collect::<Vec<_>>(),
        vec![(0, 13), (1, 13)],
        "one tally per watched bus"
    );

    // Dropping one watch must not take the other with it, and the surviving
    // port keeps delivering.
    app.send(crate::bus::BusCommand::SetFrWatch {
        bus: 1,
        channel_index: None,
        fibex_path: String::new(),
    });
    assert_eq!(
        app.hw.fr_watches.keys().copied().collect::<Vec<_>>(),
        vec![0],
        "only the requested bus was detached"
    );
    q0.lock().expect("mock lock").push_back(fr(14));
    q1.lock().expect("mock lock").push_back(fr(99));
    for t in 5..=8u64 {
        app.advance_clock(t * 1_000);
        app.tick(t * 1_000);
    }
    app.refresh_snapshot();
    let slots: Vec<(u8, u16)> = app.snap.fr_aggs.iter().map(|a| (a.bus, a.slot)).collect();
    assert!(slots.contains(&(0, 14)), "bus 0 keeps feeding: {slots:?}");
    assert!(
        !slots.contains(&(1, 99)),
        "the detached port's queue is nobody's traffic: {slots:?}"
    );

    // The Messages table has to tell the two clusters apart: slot 13 exists on
    // both, so the Bus column is what keeps the rows from looking duplicated,
    // and the header counts the rows the window actually shows.
    app.text_fresh = true;
    app.sync_msg_text(0);
    let win = &app.msg_windows[0];
    let fr: Vec<(&str, &str)> = win
        .text_rows
        .iter()
        .filter(|r| r.bus.starts_with("FR"))
        .map(|r| (r.bus.as_str(), r.label.as_str()))
        .collect();
    assert!(
        fr.iter()
            .any(|(b, l)| *b == "FR0" && l.starts_with("slot 13")),
        "bus 0's slot 13: {fr:?}"
    );
    assert!(
        fr.iter()
            .any(|(b, l)| *b == "FR1" && l.starts_with("slot 13")),
        "bus 1's slot 13, told apart by its bus: {fr:?}"
    );
    assert_eq!(
        win.text_header,
        format!("{} messages", win.text_rows.len()),
        "the header counts what is on screen"
    );
    // Both rows print "slot 13" -- the clusters differ only in the Bus column --
    // so this is the same two-visible-items-on-one-ID shape the CAN side has.
    let keys: Vec<&str> = win.text_rows.iter().map(|r| r.id_key.as_str()).collect();
    let unique = std::collections::HashSet::<&str>::from_iter(keys.iter().copied());
    assert_eq!(
        unique.len(),
        keys.len(),
        "one ID per row, however the labels repeat: {keys:?}"
    );

    // Scoping the window to one cluster leaves the other cluster's rows out --
    // the whole point of numbering the slots by bus in a two-cluster table.
    app.msg_windows[0].scope = SigScope::FrBus(0);
    app.text_fresh = true;
    app.sync_msg_text(0);
    let buses: Vec<String> = app.msg_windows[0]
        .text_rows
        .iter()
        .map(|r| r.bus.clone())
        .collect();
    assert_eq!(buses, vec!["FR0".to_string(), "FR0".to_string()]);

    // A cluster that goes away releases the windows pointed at it: an
    // unexplained empty table is worse than falling back to the widest view.
    // Closing the port alone must not do that -- the 路 and its description are
    // still there to look at.
    app.detach_flexray_watch(0);
    assert_eq!(
        app.msg_windows[0].scope,
        SigScope::FrBus(0),
        "断开 keeps the 路, so the view of it stays"
    );
    app.remove_fr_bus(0);
    assert_eq!(app.msg_windows[0].scope, SigScope::All);
    app.stop();
}

/// One database on two buses puts the same message in the Messages table twice:
/// same id, same name, only the Bus column differs -- which is what a bench
/// with both channels loading one DBC does on its own. A table does not seed
/// item IDs per cell, so an ID taken from the printed label lands on two visible
/// items at once, and the old table UI stopped the window with "2 visible items with
/// conflicting ID". The row therefore carries its own key.
#[test]
fn messages_rows_that_print_one_label_keep_their_own_ids() {
    let mut app = spec_app();
    app.copy_can_table_for_test(0, 1);
    receive(
        &mut app,
        0,
        vec![
            frame_at(0, 100, 8, Direction::Rx),
            CanFrame {
                channel: 1,
                ..frame_at(0, 100, 8, Direction::Rx)
            },
        ],
    );
    app.text_fresh = true;
    app.sync_msg_text(0);
    let rows = &app.msg_windows[0].text_rows;
    let labels: Vec<&str> = rows.iter().map(|r| r.label.as_str()).collect();
    let sets = std::collections::HashSet::<&str>::from_iter(labels.iter().copied());
    assert!(
        labels.len() > sets.len(),
        "the setup must print one label twice: {labels:?}"
    );
    let keys: Vec<&str> = rows.iter().map(|r| r.id_key.as_str()).collect();
    let key_sets = std::collections::HashSet::<&str>::from_iter(keys.iter().copied());
    assert_eq!(
        key_sets.len(),
        keys.len(),
        "one ID per visible row, however the labels repeat: {keys:?} against {labels:?}"
    );
    app.stop();
}

/// The regression as the user sees it. Slot 71 of the bundled ARXML is
/// scheduled to six different frames, one per cycle phase, so a Messages row
/// keyed on the slot changed its frame name on almost every arrival -- the
/// name "would not hold still". The table now holds one row per occupant, each
/// with its own count.
#[test]
fn the_messages_window_lists_each_occupant_of_a_repeated_slot() {
    use crate::hw::vector::flexray::FrFrame;
    let arxml = crate::test_assets::path("PowerTrain.arxml");
    let mut app = quiet_app();
    app.tx_list.retain(|t| t.channel != 0);
    let Ok(bytes) = std::fs::read(arxml) else {
        panic!(
            "{arxml} missing -- it is tracked in the repository, so a broken checkout must fail, not skip"
        );
    };
    let db = crate::fr_db::FrDb::parse(&crate::dbc::text_from_bytes(bytes)).expect("parses");
    app.fr_buses.insert(0, std::sync::Arc::new(db));
    app.push_fr_db_to_core();
    let q = app.hw.attach_fr_mock(0, 5);
    app.start_virtual();
    for (i, cycle) in (0..6u8).enumerate() {
        q.lock().expect("mock lock").push_back(FrFrame {
            slot: 71,
            cycle,
            payload: vec![0; 8],
            header_crc: 0,
            flags: 0,
        });
        let t = (i as u64 + 1) * 1_000_000;
        app.advance_clock(t);
        app.tick(t);
        app.refresh_snapshot();
    }
    app.text_fresh = true;
    app.sync_msg_text(0);
    let rows: Vec<(String, String)> = app.msg_windows[0]
        .text_rows
        .iter()
        .filter(|r| r.bus == "FR0")
        .map(|r| (r.label.clone(), r.count.clone()))
        .collect();
    assert_eq!(rows.len(), 6, "one row per occupant: {rows:?}");
    assert!(
        rows.iter().all(|(_, c)| c == "1"),
        "each frame counted on its own, not six into one: {rows:?}"
    );
    let labels: std::collections::BTreeSet<&str> = rows.iter().map(|(l, _)| l.as_str()).collect();
    assert_eq!(labels.len(), 6, "six distinct names: {labels:?}");
    app.stop();
}

/// The empty FlexRay child row has to say *which* empty it is: a cluster with
/// no description loaded, a description that has no frame for that slot and
/// cycle, and a frame that declares no signals are three different things to
/// act on -- and on a real two-cluster log they arrive mixed in one table.
#[test]
fn an_empty_flexray_message_row_names_its_reason() {
    use crate::hw::vector::flexray::FrFrame;
    let arxml = crate::test_assets::path("PowerTrain.arxml");
    let mut app = quiet_app();
    app.tx_list.retain(|t| t.channel != 0);
    let Ok(bytes) = std::fs::read(arxml) else {
        panic!(
            "{arxml} missing -- it is tracked in the repository, so a broken checkout must fail, not skip"
        );
    };
    let db = crate::fr_db::FrDb::parse(&crate::dbc::text_from_bytes(bytes)).expect("parses");
    // A slot the description schedules but does not name any signals for, when
    // the asset has one; otherwise the case below is the only one to check.
    let silent = db
        .slot_signals()
        .into_iter()
        .find(|(_, _, names)| names.is_empty())
        .map(|(slot, _, _)| slot);
    app.fr_buses.insert(0, std::sync::Arc::new(db));
    // The row's frame is resolved where the row is counted, so the core needs
    // the description too -- a frontend-only copy used to be enough because the
    // name was re-resolved at draw time.
    app.push_fr_db_to_core();
    let q0 = app.hw.attach_fr_mock(0, 5);
    let q1 = app.hw.attach_fr_mock(1, 6);
    app.start_virtual();
    for (q, slot) in [(&q0, 13u16), (&q0, 4_095), (&q1, 13)]
        .into_iter()
        .chain(silent.map(|s| (&q0, s)))
    {
        q.lock().expect("mock lock").push_back(FrFrame {
            slot,
            cycle: 0,
            payload: vec![0; 8],
            header_crc: 0,
            flags: 0,
        });
    }
    for t in 1..=4u64 {
        app.advance_clock(t * 1_000);
        app.tick(t * 1_000);
    }
    app.text_fresh = true;
    app.sync_msg_text(0);
    let note = |bus: &str, slot: u16| {
        app.msg_windows[0]
            .text_rows
            .iter()
            .find(|r| r.bus == bus && r.label.starts_with(&format!("slot {slot}")))
            .and_then(|r| r.empty_note.clone())
    };
    assert_eq!(
        note("FR0", 4_095).as_deref(),
        Some("(FR0 description schedules no frame in slot 4095)"),
        "a slot the description resolves no frame for says so"
    );
    assert_eq!(
        note("FR0", 13),
        None,
        "slot 13 is in this description and decodes, so it carries no note at all"
    );
    assert_eq!(
        note("FR1", 13).as_deref(),
        Some("(FR1 has no cluster description)"),
        "an undescribed cluster says that instead"
    );
    if let Some(slot) = silent {
        assert_eq!(
            note("FR0", slot).as_deref(),
            Some("(frame declares no signals)"),
            "a frame the description does have says a third thing"
        );
    }
    app.stop();
}
/// frames go to the wire alongside the internal bus -- no per-node dial,
/// the bus mode is the switch.
#[test]
fn script_frames_reach_the_wire_in_real_bus_mode() {
    let mut app = App::headless();
    let (written, _incoming) = app.hw.attach_mock(0);
    app.send(crate::bus::BusCommand::AddNode {
        name: "beacon".to_string(),
        channel: 0,
        attached: None,
    });
    app.settle();
    let id = app.snap.nodes[0].id;
    app.send(crate::bus::BusCommand::SetNodeSource {
        id,
        source: "on timer 50 { send(0x777, 1); }".to_string(),
    });
    app.send(crate::bus::BusCommand::SetNodeEnabled { id, on: true });
    app.start_virtual();
    app.settle();
    for t in 1..=300u64 {
        app.advance_clock(t * 1_000);
        app.tick(t * 1_000);
    }
    let wire_count = || {
        written
            .lock()
            .expect("mock lock")
            .iter()
            .filter(|f| f.id == 0x777)
            .count()
    };
    assert!(
        wire_count() >= 2,
        "the script's frames reach the wire: {}",
        wire_count()
    );
    app.stop();
}

/// Removing a bus detaches its hardware and shifts the other buses'
/// attachments down with the bus.
#[test]
fn removing_a_bus_detaches_its_hardware_and_shifts_the_rest() {
    let mut app = App::headless();
    app.hw.attach_mock(0);
    let (_w, _i) = app.hw.attach_mock(1);
    assert!(app.hw.is_attached(1));
    // Each row's picker remembers the port it chose. The survivor must keep
    // naming its own after the shift; the removed row must stop naming one, or
    // a later bus on index 0 opens showing a port somebody chose for its
    // predecessor.
    app.hw_pick.insert(0, (crate::hw::HwDriver::Vector, 3));
    app.hw_pick.insert(1, (crate::hw::HwDriver::Vector, 4));

    app.remove_channel(0);
    app.settle();

    assert!(
        app.hw.is_attached(0) && !app.hw.is_attached(1),
        "the removed bus's hardware went with it, the survivor shifted down"
    );
    assert_eq!(
        app.hw_pick.get(&0).copied(),
        Some((crate::hw::HwDriver::Vector, 4)),
        "the survivor's picker moved down with its bus"
    );
    assert_eq!(app.hw_pick.len(), 1, "and the removed bus is out of it");
}

/// The CANoe-style bus mode switch: Simulated parks every attachment
/// (received frames are dropped, directed wire writes are suppressed)
/// while keeping the configuration; flipping back to Real bus reconnects
/// them without reattaching.
#[test]
fn the_bus_mode_switch_parks_and_reconnects_the_wire() {
    let mut app = App::headless();
    app.tx_list.retain(|t| t.channel != 0);
    let (written, incoming) = app.hw.attach_mock(0);
    app.set_node_role(0, "EngineECU", NodeRole::Simulated);
    app.send(crate::bus::BusCommand::SetEntryActive {
        ch: 0,
        id: 0x100,
        on: true,
    });
    app.start_virtual();
    app.settle();

    // Baseline: Real bus, the node's frames reach the wire.
    let wire_ids = || -> Vec<u32> {
        written
            .lock()
            .expect("mock lock")
            .iter()
            .map(|f| f.id)
            .collect()
    };
    for t in 1..=500u64 {
        app.advance_clock(t * 1_000);
        app.tick(t * 1_000);
    }
    assert!(
        wire_ids().contains(&0x100),
        "real bus: the generator's frames reach the wire"
    );

    // Park it (Simulated): the injected wire frame is dropped and wire
    // writes stop, but the attachment and the switch survive.
    app.send(crate::bus::BusCommand::SetBusMode { real: false });
    app.settle();
    assert!(!app.snap.real_bus, "the snapshot reports the parked mode");
    let tx_parked = written.lock().expect("mock lock").len();
    incoming.lock().expect("mock lock").push_back(CanFrame {
        t_us: 0,
        channel: 0,
        id: 0x555,
        extended: false,
        len: 1,
        data: [0; MAX_CAN_FD_LEN],
        dir: Direction::Rx,
        flags: FrameFlags::NONE,
    });
    for t in 900_000..940_000u64 {
        app.advance_clock(t);
        app.tick(t);
    }
    assert_eq!(
        written.lock().expect("mock lock").len(),
        tx_parked,
        "Simulated: no wire writes while parked"
    );
    assert!(
        !app.snap
            .aggs
            .iter()
            .any(|a| a.channel == 0 && a.id == 0x555),
        "Simulated: wire frames never reach the internal bus"
    );

    // Back to Real bus: the same attachment reconnects -- the queued wire
    // frame from before the flip was drained while parked, but a fresh
    // one lands, and the node's writes resume.
    app.send(crate::bus::BusCommand::SetBusMode { real: true });
    app.settle();
    assert!(app.snap.real_bus);
    incoming.lock().expect("mock lock").push_back(CanFrame {
        t_us: 0,
        channel: 0,
        id: 0x556,
        extended: false,
        len: 1,
        data: [0; MAX_CAN_FD_LEN],
        dir: Direction::Rx,
        flags: FrameFlags::NONE,
    });
    let wire_before = written.lock().expect("mock lock").len();
    for t in 940_000..1_240_000u64 {
        app.advance_clock(t);
        app.tick(t);
    }
    assert!(
        app.snap
            .aggs
            .iter()
            .any(|a| a.channel == 0 && a.id == 0x556),
        "Real bus: fresh wire frames are ingested again"
    );
    assert!(
        written.lock().expect("mock lock").len() > wire_before,
        "Real bus: the node's wire writes resume"
    );
    app.stop();
}

/// A wire that takes nothing used to be invisible: every refused write fell
/// into `.ok()`, the frames stayed on the internal bus, and every view kept
/// scrolling as though the bench were merely quiet. The first refusal is now
/// said once and each one after it counted on the row.
#[test]
fn a_wire_that_refuses_writes_is_said_once_and_counted() {
    let mut app = App::headless();
    app.tx_list.retain(|t| t.channel != 0);
    let (written, _incoming) = app.hw.attach_mock_refusing(0, "XL_ERR_HW_NOTPRESENT");
    app.set_node_role(0, "EngineECU", NodeRole::Simulated);
    app.send(crate::bus::BusCommand::SetEntryActive {
        ch: 0,
        id: 0x100,
        on: true,
    });
    app.start_virtual();
    app.settle();
    for t in 1..=500u64 {
        app.advance_clock(t * 1_000);
        app.tick(t * 1_000);
    }
    app.refresh_snapshot();
    assert!(
        written.lock().expect("mock lock").is_empty(),
        "nothing reached the wire -- that is the fact being reported"
    );
    let (_, n) = app
        .snap
        .hw
        .iter()
        .find(|h| h.bus == 0)
        .and_then(|h| h.tx_fail.clone())
        .expect("the row says the wire refused writes");
    assert!(
        n >= 2,
        "every refusal after the first is counted, not swallowed: {n}"
    );
    let said: Vec<String> = app
        .snap
        .write
        .iter()
        .filter(|l| l.text.contains("transmit refused by the wire"))
        .map(|l| l.text.clone())
        .collect();
    assert_eq!(
        said.len(),
        1,
        "one line for one fault, however many frames it refused: {said:?}"
    );
    assert!(
        said[0].contains("XL_ERR_HW_NOTPRESENT"),
        "the driver's own reason travels with it: {}",
        said[0]
    );
    assert!(
        app.snap
            .write
            .iter()
            .any(|l| l.kind == crate::bus::WriteKind::Error),
        "a dead wire is not news of the informational kind"
    );
}

/// The same drive on a wire that takes everything: no complaint anywhere.
/// Without this half the test above only proves the latch fires, not that it
/// can tell the two situations apart.
#[test]
fn a_wire_that_takes_writes_leaves_no_complaint() {
    let mut app = App::headless();
    app.tx_list.retain(|t| t.channel != 0);
    let (written, _incoming) = app.hw.attach_mock(0);
    app.set_node_role(0, "EngineECU", NodeRole::Simulated);
    app.send(crate::bus::BusCommand::SetEntryActive {
        ch: 0,
        id: 0x100,
        on: true,
    });
    app.start_virtual();
    app.settle();
    for t in 1..=500u64 {
        app.advance_clock(t * 1_000);
        app.tick(t * 1_000);
    }
    app.refresh_snapshot();
    assert!(
        !written.lock().expect("mock lock").is_empty(),
        "the wire took the frames"
    );
    assert!(
        app.snap
            .hw
            .iter()
            .find(|h| h.bus == 0)
            .is_some_and(|h| h.tx_fail.is_none()),
        "and nothing was refused, so the row stays quiet"
    );
    assert!(
        !app.snap
            .write
            .iter()
            .any(|l| l.text.contains("transmit refused by the wire")),
        "and the log says nothing about it"
    );
}

/// The third way the wire fails, and the one the driver admits to only in
/// events: `xlCanTransmitEx` **accepts** the frame, the controller retries it
/// forever because nobody answers, and the sole sign is a stream of non-frame
/// events. Those used to be dropped where no frame was read, so the screen
/// showed a calm stream of Tx and 0 Rx -- indistinguishable from a quiet bus.
#[test]
fn a_wire_that_never_answers_is_told_from_the_events() {
    let mut app = App::headless();
    app.tx_list.retain(|t| t.channel != 0);
    app.hw
        .attach_mock_reporting(0, &[(0x0402, 4001), (0x0409, 16)]);
    app.set_node_role(0, "EngineECU", NodeRole::Simulated);
    app.send(crate::bus::BusCommand::SetEntryActive {
        ch: 0,
        id: 0x100,
        on: true,
    });
    app.start_virtual();
    for t in 1..=50u64 {
        app.advance_clock(t * 1_000);
        app.tick(t * 1_000);
    }
    app.refresh_snapshot();
    let (reason, n) = app
        .snap
        .hw
        .iter()
        .find(|h| h.bus == 0)
        .and_then(|h| h.tx_fail.clone())
        .expect("the events are reported, not dropped");
    assert!(
        reason.contains("0x0402"),
        "the tag the driver actually used travels with it: {reason}"
    );
    assert_eq!(n, 4001, "the count is the driver's own: {n}");
    let said: Vec<String> = app
        .snap
        .write
        .iter()
        .filter(|l| l.text.contains("transmit refused by the wire"))
        .map(|l| l.text.clone())
        .collect();
    assert_eq!(
        said.len(),
        1,
        "4001 refused attempts are one line, not 4001: {said:?}"
    );
}

/// State Trackers ride the same remap: their rows (and the per-key color
/// memory) follow the bus removal just like Graphics and Data rows.
#[test]
fn removing_a_bus_remaps_state_trackers() {
    let mut app = App::headless();
    app.new_state_window();
    let key = SigKey::can_in(crate::model::ClusterId(2), 0x200, false, "VehicleSpeed");
    app.set_win_signal(crate::app::PopupTarget::State(0), key.clone(), true);
    assert!(!app.state_trackers[0].signals.is_empty(), "the row landed");
    app.state_trackers[0]
        .color_slots
        .insert(key.clone(), [(7u64, 0usize)].into_iter().collect());

    app.remove_channel(0);
    app.settle();

    let sig = &app.state_trackers[0].signals[0];
    assert!(
        matches!(
            sig.key,
            SigKey::Can {
                cluster: crate::model::ClusterId(2),
                ..
            }
        ),
        "the row keeps its cluster identity when the bus index shifts"
    );
    let shifted = SigKey::can_in(crate::model::ClusterId(2), 0x200, false, "VehicleSpeed");
    assert!(
        app.state_trackers[0].color_slots.contains_key(&shifted),
        "the color memory stays on the same key"
    );
}

/// The disk-backed trace: frames the ring trimmed are archived to a temp
/// file, and Export Trace replays the archive ahead of the hot ring --
/// the exported ASC covers the whole run, head included.
#[test]
fn the_trace_export_includes_archived_head_frames() {
    let mut app = App::headless();
    app.set_trace_limit(1_000);
    app.start_virtual();
    app.settle();
    let frames: Vec<CanFrame> = (0..1_200u64)
        .map(|i| CanFrame {
            t_us: i * 100,
            channel: 0,
            id: 0x300,
            extended: false,
            len: 1,
            data: {
                let mut d = [0u8; MAX_CAN_FD_LEN];
                d[0] = (i % 256) as u8;
                d
            },
            dir: Direction::Rx,
            flags: FrameFlags::NONE,
        })
        .collect();
    let last_t = 1_199 * 100;
    receive(&mut app, last_t, frames);
    app.settle();

    // The hot ring is capped; the overflow sits in the archive.
    let (dropped, _) = app.snap.trace.head_loss();
    assert!(dropped >= 100, "the limit trimmed the head: {dropped}");
    let (_, archived_n) = app.snap.trace.archive().expect("archive exists");
    assert!(archived_n >= 100, "the trim was archived, not lost");

    let out = std::env::temp_dir().join("roxy_can_export_archived.asc");
    app.export_trace(0, out.to_string_lossy().as_ref());
    assert!(app.status.starts_with("exported"), "{}", app.status);
    let text = std::fs::read_to_string(&out).unwrap();
    let exported = crate::log::asc::parse_asc(&text);
    assert_eq!(exported.len(), 1_200, "archive + hot ring = the whole run");
    assert_eq!(exported[0].t_us, 0, "the head survived via the archive");
    assert_eq!(exported[1_199].t_us, last_t, "the tail is the live ring");
    std::fs::remove_file(&out).ok();
}

