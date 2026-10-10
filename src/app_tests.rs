use super::*;
// The tests rely on the parent's imports through `use super::*`; the
// ones app.rs itself no longer needs are imported directly here.
use crate::bus::MAX_TX_CATCHUP;
use crate::observe::SigKey;
use crate::can::frame::{FrameFlags, MAX_CAN_FD_LEN};
use crate::config::Config;
use crate::generator::TxMsg;
use crate::log::AscWriter;
use crate::observe::Subscription;
use crate::observe::YMode;
use crate::sim::ValueSrc;
use crate::source::FrameSource;
use crate::trigger::{Trigger, TriggerAction, TriggerCond};

#[test]
fn record_survives_start_and_writes_frames() {
    let mut app = App::headless();
    let path = std::env::temp_dir().join("roxy_can_record_test.asc");
    app.record_path_buf = path.to_string_lossy().to_string();
    app.toggle_record();
    assert!(app.recorder.recording);
    assert!(
        !app.tx_list.is_empty(),
        "DBC messages pre-populate the generator"
    );
    for tx in &mut app.tx_list {
        tx.active = true;
        tx.cycle_us = 10_000;
    }
    open_all_gates(&mut app);
    open_all_gates(&mut app);
    app.start_virtual();
    assert!(
        app.recorder.recording,
        "Start must not clear the Record checkbox"
    );
    for _ in 0..12 {
        std::thread::sleep(std::time::Duration::from_millis(11));
        app.update();
    }
    app.stop();
    let actual = app.recorder.last_record.clone();
    assert!(actual.ends_with(".asc"), "no record file: {actual}");
    assert!(
        actual.contains("roxy_can_record_test_"),
        "generated name should keep the user base: {actual}"
    );
    let content = std::fs::read_to_string(&actual).unwrap();
    let frames = crate::log::asc::parse_asc(&content);
    assert!(frames.len() >= 10, "expected frames, got {}", frames.len());
    if let Some(dir) = std::path::Path::new(&actual).parent()
        && let Ok(rd) = std::fs::read_dir(dir)
    {
        for e in rd.flatten() {
            let n = e.file_name().to_string_lossy().to_string();
            if n.starts_with("roxy_can_record_test") {
                std::fs::remove_file(e.path()).ok();
            }
        }
    }
}

/// The record filter gates only the FILE: a whitelist of ids keeps the
/// recorded ASC to those frames, while trace and aggregates still see
/// the whole bus.
#[test]
fn the_record_filter_limits_the_file_but_not_the_bus() {
    let mut app = App::headless();
    let path = std::env::temp_dir().join("roxy_can_record_filtered.asc");
    app.record_path_buf = path.to_string_lossy().to_string();
    app.set_record_filter(crate::recorder::RecordFilter::can(vec![(0x100, false)]));
    app.toggle_record();
    // motbus on CAN2: activate one entry whose id is NOT in the whitelist.
    app.add_tx(1, 0x999);
    for tx in &mut app.tx_list {
        tx.active = true;
        tx.cycle_us = 10_000;
    }
    open_all_gates(&mut app);
    app.start_virtual();
    for _ in 0..12 {
        std::thread::sleep(std::time::Duration::from_millis(11));
        app.update();
    }
    app.stop();
    let actual = app.recorder.last_record.clone();
    let content = std::fs::read_to_string(&actual).unwrap();
    let frames = crate::log::asc::parse_asc(&content);
    assert!(
        frames.len() >= 5,
        "the whitelisted id flowed: got {} frame(s)",
        frames.len()
    );
    assert!(
        frames.iter().all(|f| f.id == 0x100),
        "nothing outside the whitelist landed in the file"
    );
    // The bus itself saw the filtered-out traffic: aggregates count it.
    let stray = app
        .snap
        .aggs
        .iter()
        .find(|a| a.channel == 1 && a.id == 0x999)
        .expect("the filtered-out id still reached the bus");
    assert!(stray.count >= 1, "aggregates must not be filtered");
    std::fs::remove_file(&actual).ok();
}

/// The record filter can name a FlexRay slot, and it still gates only the
/// file: three arrivals, one of them on the whitelist, and the Trace ring keeps
/// all three. The CAN side of the same recording is untouched by a slot-only
/// whitelist -- the filter lists what to keep, it does not mute the other bus.
#[test]
fn a_record_filter_can_keep_only_some_flexray_slots() {
    use crate::hw::vector::flexray::FrFrame;
    let mut app = App::headless();
    let path = std::env::temp_dir().join("roxy_can_record_fr_filtered.asc");
    app.record_path_buf = path.to_string_lossy().to_string();
    app.set_record_filter(crate::recorder::RecordFilter {
        can: vec![],
        fr: vec![(0, 13)],
    });
    app.toggle_record();
    let q0 = app.hw.attach_fr_mock(0, 5);
    let q1 = app.hw.attach_fr_mock(1, 6);
    app.start_virtual();
    let frame = |slot: u16| FrFrame {
        slot,
        cycle: 0,
        payload: vec![0x2A],
        header_crc: 0,
        flags: 0,
    };
    {
        let mut g = q0.lock().expect("mock lock");
        g.push_back(frame(13));
        g.push_back(frame(14));
    }
    q1.lock().expect("mock lock").push_back(frame(13));
    for t in (10_000u64..=60_000).step_by(10_000) {
        app.advance_clock(t);
        app.tick(t);
    }
    app.stop();
    let actual = app.recorder.last_record.clone();
    let text = std::fs::read_to_string(&actual).expect("the recording exists");
    let (_, rows) = crate::log::asc::parse_asc_full(&text);
    assert_eq!(
        rows.iter()
            .map(|r| (r.bus, r.slot))
            .collect::<Vec<_>>(),
        vec![(0, 13)],
        "only the whitelisted cluster+slot landed in the file"
    );
    // The two filtered-out arrivals are still in the session: the ring saw them,
    // and so did the per-frame tallies.
    assert_eq!(
        app.snap.fr_trace.len(),
        3,
        "the filter gates the file, never the trace"
    );
    assert_eq!(app.snap.fr_aggs.len(), 3, "one tally per arrival");
    std::fs::remove_file(&actual).ok();
}

/// The filter holds for trigger-started recordings too: the pre-trigger
/// context the trigger drains into the file only ever held whitelisted
/// frames, so a filtered recording stays clean end to end.
#[test]
fn a_trigger_started_recording_follows_the_record_filter() {
    let mut app = App::headless();
    let path = std::env::temp_dir().join("roxy_can_trig_filtered.asc");
    app.record_path_buf = path.to_string_lossy().to_string();
    app.set_record_filter(crate::recorder::RecordFilter::can(vec![(0x100, false)]));
    app.toggle_record();
    app.triggers.push(Trigger::new(
        TriggerCond::IdPresent { ch: 0, id: 0x100 },
        TriggerAction::StartRecording,
    ));
    app.add_tx(0, 0x999);
    for tx in &mut app.tx_list {
        tx.active = true;
        tx.cycle_us = 10_000;
    }
    open_all_gates(&mut app);
    app.start_virtual();
    for _ in 0..12 {
        std::thread::sleep(std::time::Duration::from_millis(11));
        app.update();
    }
    app.stop();

    let actual = app.recorder.last_record.clone();
    let content = std::fs::read_to_string(&actual).unwrap();
    let frames = crate::log::asc::parse_asc(&content);
    assert!(
        frames.len() >= 5,
        "whitelisted frames flowed: {}",
        frames.len()
    );
    assert!(
        frames.iter().all(|f| f.id == 0x100),
        "the pre-trigger context and live tail stay filtered"
    );
    if let Some(dir) = std::path::Path::new(&actual).parent()
        && let Ok(rd) = std::fs::read_dir(dir)
    {
        for e in rd.flatten() {
            let n = e.file_name().to_string_lossy().to_string();
            if n.starts_with("roxy_can_trig_filtered") {
                std::fs::remove_file(e.path()).ok();
            }
        }
    }
}

#[test]
fn replay_after_recorded_simulation_creates_no_second_file() {
    let mut app = App::headless();
    let path = std::env::temp_dir().join("roxy_can_replay_rec_test.asc");
    app.record_path_buf = path.to_string_lossy().to_string();
    app.toggle_record();
    for tx in &mut app.tx_list {
        tx.active = true;
        tx.cycle_us = 10_000;
    }
    open_all_gates(&mut app);
    app.start_virtual();
    for _ in 0..12 {
        std::thread::sleep(std::time::Duration::from_millis(11));
        app.update();
    }
    app.stop();
    let first = app.recorder.last_record.clone();
    assert!(!first.is_empty(), "simulation should have recorded a file");
    app.log_path = first.clone();
    app.replay();
    assert!(!app.recorder.recording, "replay must drop the Record state");
    assert_eq!(
        app.recorder.last_record, first,
        "replay must not open a second record file"
    );
    app.stop();
    std::fs::remove_file(&first).ok();
}

/// The toolbar combo, not the draft's typed extension, picks the record
/// backend: `toggle_record` stamps the selected extension onto the stem
/// it forwards to the recorder.
#[test]
fn the_format_combo_picks_the_record_extension() {
    let mut app = App::headless();
    let base = std::env::temp_dir()
        .join("roxy_can_combo_fmt")
        .to_string_lossy()
        .to_string();
    // A draft typed as .asc forwards .blf when the combo says BLF.
    app.record_path_buf = format!("{base}.asc");
    app.record_format = 1;
    app.toggle_record();
    assert!(app.recorder.recording);
    assert_eq!(
        app.recorder.record_path,
        format!("{base}.blf"),
        "combo BLF must win over the typed .asc"
    );
    app.toggle_record();
    // A bare stem forwards .asc with the combo back at its default.
    app.record_path_buf = base.clone();
    app.record_format = 0;
    app.toggle_record();
    assert_eq!(
        app.recorder.record_path,
        format!("{base}.asc"),
        "combo ASC must stamp .asc"
    );
    app.toggle_record();
}

/// The recording's default home follows the project: the toolbar input
/// holds only the file name ("record"), and arming resolves it into the
/// project's Record/ folder. A draft the user typed themselves is left
/// alone, and an absolute path is honored exactly as typed.
#[test]
fn loading_a_project_points_the_record_draft_at_its_record_folder() {
    let dir = std::env::temp_dir().join("roxy_can_record_home");
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("net.rxproj");
    let mut app = App::headless();
    assert!(app.save_project(Some(path.clone())), "save writes the file");
    assert_eq!(
        app.record_path_buf, "record",
        "the draft shows the bare default name"
    );

    // Arming resolves the name into the project's Record/ folder.
    app.toggle_record();
    assert!(app.recorder.recording);
    let expected = std::path::Path::new(&dir)
        .join("Record")
        .join("record.asc")
        .to_string_lossy()
        .replace('\\', "/");
    let sent = app.recorder.record_path.replace('\\', "/");
    assert_eq!(sent, expected, "relative name lands in Record/");
    app.toggle_record();

    // A fresh load from disk resets the draft to the default name too.
    let mut app = App::headless();
    app.open_project_path(&path);
    assert_eq!(
        app.record_path_buf, "record",
        "draft retargeted on load: {}",
        app.record_path_buf
    );

    // A draft the user typed survives a Save As of the same workspace.
    app.record_path_buf = "D:/mylogs/custom".to_string();
    assert!(app.save_project(Some(path.clone())));
    assert_eq!(
        app.record_path_buf, "D:/mylogs/custom",
        "Save As must not clobber a custom draft"
    );
    app.toggle_record();
    assert_eq!(
        app.recorder.record_path, "D:/mylogs/custom.asc",
        "absolute paths are honored as typed"
    );
    app.toggle_record();
    std::fs::remove_file(&path).ok();
}

#[test]
fn loading_log_does_not_start_replay() {
    let mut app = App::headless();
    let path = std::env::temp_dir().join("roxy_can_load_asc_test.asc");
    app.record_path_buf = path.to_string_lossy().to_string();
    app.toggle_record();
    for tx in &mut app.tx_list {
        tx.active = true;
        tx.cycle_us = 10_000;
    }
    open_all_gates(&mut app);
    app.start_virtual();
    for _ in 0..12 {
        std::thread::sleep(std::time::Duration::from_millis(11));
        app.update();
    }
    app.stop();
    let first = app.recorder.last_record.clone();
    app.load_log(&first);
    assert!(!app.measuring, "loading must not start playback");
    assert!(app.log_info.is_some(), "load should cache a stream summary");
    app.replay();
    assert!(app.measuring, "replay starts on demand");
    assert!(matches!(app.mode, Mode::Replay));
    app.stop();
    std::fs::remove_file(&first).ok();
}

#[test]
fn loading_blf_does_not_start_replay() {
    let mut app = App::headless();
    let path = std::env::temp_dir().join("roxy_can_load_blf_test.blf");
    let bytes = crate::log::blf::tests::minimal_file();
    std::fs::write(&path, &bytes).unwrap();
    app.load_log(&path.to_string_lossy());
    assert!(
        app.log_info.is_some(),
        "BLF load should cache a stream summary, got status {:?}",
        app.status
    );
    assert!(!app.measuring, "loading must not start playback");
    app.replay();
    assert!(app.measuring, "replay starts on demand");
    assert!(matches!(app.mode, Mode::Replay));
    app.stop();
    std::fs::remove_file(&path).ok();
}

#[test]
fn open_dropped_reports_unsupported_for_mf4() {
    let mut app = App::headless();
    app.open_dropped(std::path::Path::new("/tmp/does-not-exist.mf4"));
    assert!(
        app.status.contains("unsupported format: MF4"),
        "MF4 should surface a clear reason, got {:?}",
        app.status
    );
}

#[test]
fn aggregates_frames_per_message_id() {
    let mut app = App::headless();
    let tx = app
        .tx_list
        .iter_mut()
        .find(|t| t.id == 0x100)
        .expect("EngineStatus pre-populated in generator");
    tx.active = true;
    tx.cycle_us = 10_000;
    // 角色开关允许发送：0x100 属 EngineECU。
    app.set_node_role(0, "EngineECU", NodeRole::Simulated);
    app.start_virtual();
    for _ in 0..8 {
        std::thread::sleep(std::time::Duration::from_millis(12));
        app.update();
    }
    let agg = app
        .aggs
        .get(&(0, 0x100, false))
        .expect("EngineStatus aggregated on CAN1");
    assert!(agg.count >= 5, "expected several frames, got {}", agg.count);
    assert!(
        (agg.cycle_us / 1000.0 - 12.0).abs() < 8.0,
        "cycle should track the update cadence, got {}ms",
        agg.cycle_us / 1000.0
    );
    assert!(agg.min_us > 0.0, "min cycle should be recorded");
    assert!(agg.max_us >= agg.min_us, "max cycle >= min cycle");
    app.stop();
}

/// The observer Clear buttons mean "empty the view": the trace ring and
/// the per-message counters blank out while the run keeps going, and the
/// next arrivals re-populate both. (They used to reset the window's
/// filter settings instead, which read as "does nothing".)
#[test]
fn clearing_the_trace_and_message_counters_empties_the_views() {
    let mut app = App::headless();
    app.add_tx(0, 0x777);
    let tx = app.tx_list.last_mut().unwrap();
    tx.cycle_us = 10_000;
    tx.active = true;
    // 0x777 is not in the database: unassigned, no role gate applies.
    app.start_virtual();
    run_sim(&mut app, 5, 10_000);
    assert!(app.trace.len() > 0, "frames on the trace before the clear");
    assert!(
        app.aggs.contains_key(&(0, 0x777, false)),
        "a counter exists before the clear"
    );

    app.send(crate::bus::BusCommand::ClearTrace);
    app.send(crate::bus::BusCommand::ClearAggregates);
    assert_eq!(app.trace.len(), 0, "the trace view is blank");
    assert!(app.aggs.is_empty(), "the counters are blank");

    // Still measuring: the next cycles repopulate both views (the clock
    // keeps advancing -- a manual clear happens mid-run).
    for i in 6..=10u64 {
        app.sim_t_us = i * 10_000;
        app.tick(app.sim_t_us);
    }
    assert!(app.trace.len() > 0, "frames arrive after the clear");
    assert!(
        app.aggs.contains_key(&(0, 0x777, false)),
        "counters resume after the clear"
    );
    app.stop();
}

/// Drives `ticks` steps of the loop, each `step_us` of simulation time
/// apart. `tick` reads `sim_t_us` directly, so no wall clock is involved.
fn run_sim(app: &mut App, ticks: u32, step_us: u64) {
    for i in 1..=ticks {
        app.sim_t_us = u64::from(i) * step_us;
        app.tick(app.sim_t_us);
    }
}

fn slots_of(app: &App, id: u32) -> Vec<u64> {
    app.trace
        .iter()
        .filter(|f| f.id == id)
        .map(|f| f.t_us)
        .collect()
}

#[test]
fn generator_frames_are_spaced_exactly_one_cycle() {
    let mut app = App::headless();
    app.add_tx(0, 0x777);
    let tx = app.tx_list.last_mut().unwrap();
    tx.cycle_us = 20_000;
    tx.active = true;
    app.start_virtual();
    // Ticks land on multiples of 7 ms, which the 20 ms cycle never lines up
    // with: slots must still come out on exact 20 ms boundaries.
    run_sim(&mut app, 12, 7_000);
    assert_eq!(
        slots_of(&app, 0x777),
        vec![0, 20_000, 40_000, 60_000, 80_000]
    );
    let agg = app.aggs.get(&(0, 0x777, false)).expect("aggregate");
    assert_eq!((agg.min_us, agg.max_us), (20_000.0, 20_000.0));
    app.stop();
}

#[test]
fn a_tick_exactly_on_a_slot_does_not_drop_a_cycle() {
    let mut app = App::headless();
    app.add_tx(0, 0x779);
    let tx = app.tx_list.last_mut().unwrap();
    tx.cycle_us = 20_000;
    tx.active = true;
    app.start_virtual();
    // Ticks land precisely on slot boundaries. A slot is due when the
    // clock reaches it, so the tick sitting on 120 ms owns that slot too --
    // nothing is skipped, and nothing waits a tick past its own stamp.
    run_sim(&mut app, 6, 20_000);
    assert_eq!(
        slots_of(&app, 0x779),
        vec![0, 20_000, 40_000, 60_000, 80_000, 100_000, 120_000],
        "one frame per cycle, none skipped at the boundary"
    );
    app.stop();
}

#[test]
fn a_stalled_ui_backfills_the_slots_it_missed() {
    let mut app = App::headless();
    app.add_tx(0, 0x778);
    let tx = app.tx_list.last_mut().unwrap();
    tx.cycle_us = 20_000;
    tx.active = true;
    app.start_virtual();
    app.sim_t_us = 0;
    app.tick(0);
    // Twelve cycles' worth of stall: every missed slot still goes out at
    // its own stamp. Skipping the backlog used to punch a hole into the
    // bus's own timeline, and at fine Graphics strides that hole read as
    // the curve being eaten while the plot slid on.
    app.sim_t_us = 250_000;
    app.tick(250_000);
    assert_eq!(
        slots_of(&app, 0x778),
        (0..13u64).map(|i| i * 20_000).collect::<Vec<_>>(),
        "the backlog was emitted, not skipped"
    );
    assert_eq!(
        app.tx_list.last().unwrap().next_t_us,
        260_000,
        "the schedule resumes past the stall"
    );
    app.stop();
}

#[test]
fn a_stall_backfill_is_bounded_per_tick() {
    let mut app = App::headless();
    app.add_tx(0, 0x778);
    let tx = app.tx_list.last_mut().unwrap();
    tx.cycle_us = 1_000;
    tx.active = true;
    app.start_virtual();
    app.sim_t_us = 0;
    app.tick(0);
    // A 100 s freeze at a 1 ms cycle owes 100 000 slots: one tick takes a
    // bounded bite and the rest streams over the following ticks.
    app.sim_t_us = 100_000_000;
    app.tick(app.sim_t_us);
    assert_eq!(
        slots_of(&app, 0x778).len() as u32,
        MAX_TX_CATCHUP + 1,
        "slot 0 plus the bounded burst"
    );
    assert!(
        app.tx_list.last().unwrap().next_t_us <= app.sim_t_us,
        "still behind, so the next tick keeps streaming"
    );
    app.tick(app.sim_t_us);
    assert_eq!(
        slots_of(&app, 0x778).len() as u32,
        2 * MAX_TX_CATCHUP + 1,
        "each tick takes another bounded bite"
    );
    app.stop();
}

#[test]
fn a_ui_stall_never_punches_a_hole_into_the_sample_timeline() {
    let mut app = App::headless();
    let key = {
        let db = app.channel_dbc(0).expect("sample DBC loaded");
        let id = db.order[0].0;
        SigKey::can(0u8, id, false, db.messages[&(id, false)].signals[0].name.clone())
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
    // Two seconds of ordinary ticks, then the clock jumps a 0.4 s UI
    // freeze in one step, then ordinary ticks resume. Whatever the stall,
    // the sampled timeline must stay on its 10 ms grid.
    for _ in 0..120 {
        app.sim_t_us += 16_667;
        app.tick(app.sim_t_us);
    }
    app.sim_t_us += 400_000;
    app.tick(app.sim_t_us);
    for _ in 0..60 {
        app.sim_t_us += 16_667;
        app.tick(app.sim_t_us);
    }
    let stamps: Vec<u64> = app.subs[&key].history.iter().map(|&(t, _)| t).collect();
    assert!(
        stamps.len() > 200,
        "100 Hz for ~3.4 s, got {}",
        stamps.len()
    );
    for (a, b) in stamps.iter().zip(stamps.iter().skip(1)) {
        assert!(
            b - a <= 20_000,
            "a {} us gap at {:.3}s -- the stall ate the samples",
            b - a,
            *a as f64 / 1e6
        );
    }
    app.stop();
}

#[test]
fn a_pause_freezes_the_simulation_clock_and_its_phase() {
    let mut app = App::headless();
    app.start_virtual();
    app.update();
    let before = app.sim_t_us;
    // Age the wall clock by 400 ms the way a real pause would, then check
    // the simulation clock neither moves during the pause nor absorbs the
    // paused span afterwards.
    app.trace_paused = true;
    app.refresh_snapshot();
    app.t0 = Instant::now() - std::time::Duration::from_millis(400);
    app.update();
    assert_eq!(app.sim_t_us, before, "a paused clock must not advance");
    app.trace_paused = false;
    app.refresh_snapshot();
    app.update();
    assert!(
        app.sim_t_us - before < 50_000,
        "resuming absorbed the paused span: sim advanced {} us",
        app.sim_t_us - before
    );
    app.stop();
}

/// A 16-byte message with one signal in the classic area and one starting at
/// byte 9, so payload widening is testable without an FD asset.
const WIDE_DBC: &str = r#"VERSION "roxy-can test database"

NS_ :

BU_: ECU

BO_ 768 WideMsg: 16 ECU
 SG_ NearSig : 0|16@1+ (1,0) [0|65535] "" ECU
 SG_ FarSig : 72|16@1+ (1,0) [0|65535] "" ECU
"#;

/// Channel 0 on [`WIDE_DBC`] with one active, source-driven `WideMsg`. The
/// default period is one second, so a slot `t` microseconds into the run
/// carries `(t as f64 / 1e6) * hi`.
fn driven_app(signal: &str, kind: crate::sim::SrcKind, lo: f64, hi: f64) -> App {
    let mut app = App::headless();
    app.channels[0].dbc = Some(std::sync::Arc::new(
        crate::dbc::load_dbc_str(WIDE_DBC).expect("wide dbc parses"),
    ));
    app.add_tx(0, 0x300);
    // 测试 DBC 的发送节点 ECU 需要角色开关允许发送。
    app.set_node_role(0, "ECU", NodeRole::Simulated);
    let tx = app.tx_list.last_mut().expect("tx entry added");
    tx.cycle_us = 20_000;
    tx.active = true;
    tx.srcs.push(ValueSrc::new(signal, kind, lo, hi));
    app.start_virtual();
    app
}

fn emitted(app: &App, id: u32) -> Vec<CanFrame> {
    app.trace.iter().filter(|f| f.id == id).copied().collect()
}

fn raw_at(f: &CanFrame, start_bit: u64) -> u64 {
    crate::decode::extract_raw(&f.data, start_bit, 16, false)
}

#[test]
fn a_driven_signal_carries_the_value_of_its_own_timestamp() {
    // Ticks land off the 20 ms slot grid on purpose, so a payload read at
    // the tick instead of at the stamp it carries would come out wrong.
    let mut app = driven_app("NearSig", crate::sim::SrcKind::Ramp, 0.0, 1000.0);
    run_sim(&mut app, 12, 7_000);
    let frames = emitted(&app, 0x300);
    let slots: Vec<u64> = frames.iter().map(|f| f.t_us).collect();
    assert_eq!(slots, vec![0, 20_000, 40_000, 60_000, 80_000]);
    let vals: Vec<u64> = frames.iter().map(|f| raw_at(f, 0)).collect();
    assert_eq!(
        vals,
        vec![0, 20, 40, 60, 80],
        "each frame must hold the ramp value at its own stamp"
    );
    app.stop();
}

#[test]
fn the_wall_clock_cannot_move_a_generated_value() {
    let value_at = |now_us: u64| {
        let mut app = driven_app("NearSig", crate::sim::SrcKind::Ramp, 0.0, 1000.0);
        app.tx_list.last_mut().unwrap().next_t_us = 40_000;
        app.sim_t_us = 45_000;
        app.tick(now_us);
        raw_at(&emitted(&app, 0x300)[0], 0)
    };
    assert_eq!(
        value_at(45_000),
        value_at(9_999_999),
        "payloads must depend on simulation time only"
    );
    assert_eq!(value_at(9_999_999), 40, "the value at the slot stamped");
}

#[test]
fn driving_a_signal_leaves_the_base_payload_alone() {
    let mut app = driven_app("NearSig", crate::sim::SrcKind::Sine, 0.0, 1000.0);
    let base = app.tx_list.last().unwrap().data;
    run_sim(&mut app, 30, 7_000);
    assert!(
        emitted(&app, 0x300).iter().any(|f| raw_at(f, 0) != 0),
        "the source should have moved something by now"
    );
    assert_eq!(
        app.tx_list.last().unwrap().data,
        base,
        "a waveform sample must not become the saved base payload"
    );
    app.stop();
}

#[test]
fn a_driven_signal_past_byte_8_widens_the_frame() {
    let mut app = driven_app("FarSig", crate::sim::SrcKind::Ramp, 0.0, 1000.0);
    let i = app.tx_list.len() - 1;
    assert!(app.set_tx_hex(i, "00 01 02 03 04 05 06 07"));
    app.tx_list[i].next_t_us = 70_000;
    app.sim_t_us = 70_000;
    app.tick(70_000);
    let f = emitted(&app, 0x300).pop().expect("one frame");
    // Bits 72..88 need 11 bytes; 11 is not a legal FD length, so the frame
    // goes out at 12 with the FD flag set.
    assert_eq!(f.len, 12, "widened to the next legal FD length");
    assert!(f.flags.contains(FrameFlags::FD), "widening implies FD");
    assert_eq!(raw_at(&f, 72), 70, "the driven bytes are really there");
    assert_eq!(raw_at(&f, 0), 0x0100, "the base bytes still come through");
    assert_eq!(
        app.tx_list[i].len, 8,
        "only the emitted frame grows, not the base"
    );
    app.stop();
}

#[test]
fn pin_signal_stops_only_that_signal() {
    let mut app = driven_app("NearSig", crate::sim::SrcKind::Ramp, 0.0, 1000.0);
    let i = app.tx_list.len() - 1;
    app.set_source(
        i,
        ValueSrc::new("FarSig", crate::sim::SrcKind::Sine, 0.0, 100.0),
    );
    assert_eq!(app.tx_list[i].srcs.len(), 2);
    assert!(app.pin_signal(i, "NearSig", 250.0));
    let names: Vec<&str> = app.tx_list[i]
        .srcs
        .iter()
        .map(|s| s.name.as_str())
        .collect();
    assert_eq!(names, ["FarSig"], "pinning must not stop the other source");
    assert_eq!(
        crate::decode::extract_raw(&app.tx_list[i].data, 0, 16, false),
        250,
        "pinned into the base"
    );
    assert_eq!(
        app.tx_list[i].data_text,
        "FA 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00"
    );
    assert!(
        !app.pin_signal(i, "NoSuchSignal", 1.0),
        "unknown signal refused"
    );
    app.stop();
}

#[test]
fn a_hex_edit_keeps_the_sources_running() {
    let mut app = driven_app("NearSig", crate::sim::SrcKind::Ramp, 0.0, 1000.0);
    let i = app.tx_list.len() - 1;
    assert!(!app.set_tx_hex(i, "0 zz"), "non-hex must not apply");
    assert_eq!(app.tx_list[i].len, 16, "the rejected edit changed nothing");
    assert!(app.set_tx_hex(i, "11 22 33"));
    assert_eq!(app.tx_list[i].len, 3);
    assert_eq!(app.tx_list[i].data_text, "11 22 33", "text stays canonical");
    assert_eq!(
        app.tx_list[i].srcs.len(),
        1,
        "fixing one byte must not throw away the stimulus setup"
    );
    app.stop();
}

#[test]
fn set_source_replaces_by_name() {
    let mut app = driven_app("NearSig", crate::sim::SrcKind::Ramp, 0.0, 1000.0);
    let i = app.tx_list.len() - 1;
    app.set_source(
        i,
        ValueSrc::new("NearSig", crate::sim::SrcKind::Sine, 0.0, 50.0),
    );
    assert_eq!(app.tx_list[i].srcs.len(), 1, "the same name is one source");
    assert_eq!(app.tx_list[i].srcs[0].hi, 50.0, "and the later one wins");
    app.clear_source(i, "NearSig");
    assert!(app.tx_list[i].srcs.is_empty());
    app.stop();
}

/// A DBC that declares 0 ms means event-triggered; one that declares
/// nothing at all must still get the invented fallback period.
const CYCLE_TEST_DBC: &str = r#"VERSION "roxy-can cycle test"

NS_ :

BU_: ECU

BO_ 4096 EventMsg: 8 ECU
 SG_ S : 0|8@1+ (1,0) [0|0] "" ECU

BO_ 4097 DefaultedMsg: 8 ECU
 SG_ S : 0|8@1+ (1,0) [0|0] "" ECU

BA_DEF_ BO_  "GenMsgCycleTime" INT 0 10000;
BA_DEF_DEF_  "GenMsgCycleTime" 77;
BA_ "GenMsgCycleTime" BO_ 4096 0;
"#;

#[test]
fn new_generator_entries_inherit_the_declared_cycle() {
    let app = App::headless();
    let cycle = |ch: u8, id: u32| {
        app.tx_list
            .iter()
            .find(|t| t.channel == ch && t.id == id)
            .map(|t| t.cycle_us)
    };
    // assets/motbus.dbc:62-63 declare these two explicitly...
    assert_eq!(cycle(1, 0x64), Some(133_000), "EngineData 133ms");
    assert_eq!(cycle(1, 0xC9), Some(50_000), "ABSdata 50ms");
    // ...and its BA_DEF_DEF_ puts 100ms on the rest.
    assert_eq!(cycle(1, 0xC7), Some(100_000), "declared default");
    // sample.dbc declares nothing, so the fallback is what shows up.
    assert_eq!(cycle(0, 0x100), Some(100_000), "no declaration -> fallback");
}

#[test]
fn an_event_triggered_message_is_never_auto_sent() {
    let mut app = App::headless();
    app.channels[0].dbc = Some(std::sync::Arc::new(
        crate::dbc::load_dbc_str(CYCLE_TEST_DBC).unwrap(),
    ));
    app.tx_list.retain(|t| t.channel != 0);
    app.add_tx(0, 4096);
    app.add_tx(0, 4097);
    let i = app.tx_list.len() - 2;
    assert_eq!(
        app.tx_list[i].cycle_us, 0,
        "an explicit 0 is not 'undeclared'"
    );
    assert_eq!(
        app.tx_list[i + 1].cycle_us,
        77_000,
        "the default still applies"
    );
    for t in &mut app.tx_list {
        t.active = true;
    }
    open_all_gates(&mut app);
    app.start_virtual();
    run_sim(&mut app, 20, 10_000);
    assert!(
        slots_of(&app, 4096).is_empty(),
        "event-triggered means no timer"
    );
    assert_eq!(
        slots_of(&app, 4097),
        vec![0, 77_000, 154_000],
        "the declared 77ms period is what runs"
    );
    app.stop();
}

#[test]
fn simulated_node_state_follows_the_bus_it_lives_on() {
    let mut app = App::headless();
    app.channels[0].set_node_role("EngineECU", NodeRole::Simulated);
    app.channels[1].set_node_role("ABS", NodeRole::Simulated);
    app.remove_channel(0);
    assert_eq!(
        app.channels[0].role_of("ABS"),
        NodeRole::Simulated,
        "the survivor keeps its own nodes instead of inheriting the deleted bus's"
    );
    assert_eq!(
        app.channels[0].role_of("EngineECU"),
        NodeRole::Absent,
        "the removed bus's declarations go with it"
    );
}

/// What one bus is actually putting on the wire right now.
fn active_ids(app: &App, ch: u8) -> Vec<u32> {
    let mut ids: Vec<u32> = app
        .tx_list
        .iter()
        .filter(|t| t.channel == ch && t.active)
        .map(|t| t.id)
        .collect();
    ids.sort_unstable();
    ids
}

/// 测试助手（新语义）：声明节点角色为模拟，并启用它名下全部条目。
/// 角色本身不再触碰条目开关——需要旧行为（角色切了就发送）的测试
/// 用这个助手显式补上“启用”一步。
fn simulate_node_and_enable(app: &mut App, ch: u8, node: &str) {
    app.set_node_role(ch, node, NodeRole::Simulated);
    let ids: Vec<u32> = app
        .tx_list
        .iter()
        .filter(|t| t.channel == ch && t.node == node)
        .map(|t| t.id)
        .collect();
    for id in ids {
        app.send(crate::bus::BusCommand::SetEntryActive { ch, id, on: true });
    }
    app.settle();
}

/// 测试助手：把所有总线的所有 DBC 节点角色设为模拟（打开全部开关）。
/// 手动武装（tx.active = true）的测试需要它，否则开关拦截发送。
fn open_all_gates(app: &mut App) {
    for ch in 0..app.snap.channel_count as u8 {
        app.simulate_all_nodes(ch);
    }
    app.settle();
}

fn entry_of(app: &App, ch: u8, id: u32) -> &TxMsg {
    app.tx_list
        .iter()
        .find(|t| t.channel == ch && t.id == id)
        .expect("entry exists")
}

/// 模拟角色补建条目但不发送；逐条启用（SetEntryActive）后只有启用的
/// 条目开始发送，且不影响其他总线。
#[test]
fn ticking_a_node_activates_only_its_own_messages() {
    let mut app = App::headless();
    app.set_node_role(1, "ABS", NodeRole::Simulated);
    // assets/motbus.dbc:31,35,54 -- ABS owns these three, nobody else.
    assert_eq!(
        app.tx_list
            .iter()
            .filter(|t| t.channel == 1 && t.node == "ABS")
            .count(),
        3
    );
    simulate_node_and_enable(&mut app, 1, "ABS");
    assert_eq!(active_ids(&app, 1), [199, 200, 201]);
    assert!(active_ids(&app, 0).is_empty(), "the other bus untouched");
    assert!(
        app.tx_list
            .iter()
            .filter(|t| t.channel == 1 && t.active)
            .all(|t| t.next_t_us == 0),
        "a ticked node starts on the next tick, not one period later"
    );
    assert_eq!(app.node_role(1, "ABS"), NodeRole::Simulated);
    assert_eq!(
        app.node_role(1, "GearBox"),
        NodeRole::Absent,
        "not a side effect"
    );
}

#[test]
fn ticking_a_node_creates_the_entries_it_lacks() {
    let mut app = App::headless();
    app.tx_list.clear();
    app.set_node_role(1, "ABS", NodeRole::Simulated);
    assert_eq!(
        app.tx_list.iter().filter(|t| t.channel == 1).count(),
        3,
        "the generator refills from the DBC"
    );
    assert_eq!(app.tx_list.len(), 3, "and only this node's messages");
    assert_eq!(entry_of(&app, 1, 201).name, "ABSdata");
    assert_eq!(entry_of(&app, 1, 201).cycle_us, 50_000);
}

/// 新模型：角色 = 总开关，条目开/关 = 自定义。模拟创建条目但不改写
/// 任何开关；条目的启停只经由 SetEntryActive。
#[test]
fn simulating_a_node_creates_entries_but_leaves_them_off() {
    let mut app = App::headless();
    app.tx_list.clear();
    app.set_node_role(1, "ABS", NodeRole::Simulated);
    // 条目已补建（inactive），角色开关已打开——但没有任何一条在发送。
    assert_eq!(app.tx_list.iter().filter(|t| t.channel == 1).count(), 3);
    assert!(
        active_ids(&app, 1).is_empty(),
        "模拟不自动发送：条目开关是用户自定义"
    );

    // 逐条启用后按各自周期发送。
    let ids: Vec<u32> = app
        .tx_list
        .iter()
        .filter(|t| t.channel == 1)
        .map(|t| t.id)
        .collect();
    for id in ids {
        app.send(crate::bus::BusCommand::SetEntryActive {
            ch: 1,
            id,
            on: true,
        });
    }
    app.settle();
    assert_eq!(active_ids(&app, 1), [199, 200, 201]);
    assert_eq!(entry_of(&app, 1, 201).name, "ABSdata");
}

#[test]
fn ticking_a_node_never_overwrites_a_tuned_cycle() {
    let mut app = App::headless();
    let tuned = entry_of(&app, 1, 201).cycle_us;
    assert_eq!(tuned, 50_000, "what the DBC declares");
    let i = app
        .tx_list
        .iter()
        .position(|t| t.channel == 1 && t.id == 201)
        .unwrap();
    app.tx_list[i].cycle_us = 250_000;
    app.set_node_role(1, "ABS", NodeRole::Simulated);
    assert_eq!(
        app.tx_list[i].cycle_us, 250_000,
        "a period someone dialed in outlives the click"
    );
    app.send(crate::bus::BusCommand::SetEntryActive {
        ch: 1,
        id: 201,
        on: true,
    });
    assert!(app.tx_list[i].active, "and the entry is switched on");
}

#[test]
fn unticking_a_node_keeps_its_entries_and_their_stimulus() {
    let mut app = App::headless();
    let i = app
        .tx_list
        .iter()
        .position(|t| t.channel == 1 && t.id == 201)
        .unwrap();
    app.set_source(
        i,
        ValueSrc::new("CarSpeed", crate::sim::SrcKind::Ramp, 0.0, 300.0),
    );
    let before = app.tx_list.len();

    // 逐条目开关是自定义：关掉后条目、激励、周期原样保留。
    app.send(crate::bus::BusCommand::SetEntryActive {
        ch: 1,
        id: 201,
        on: false,
    });
    assert!(active_ids(&app, 1).is_empty(), "stopped sending");
    assert_eq!(app.tx_list.len(), before, "entries survive");
    assert_eq!(app.tx_list[i].srcs.len(), 1, "with the waveform attached");
    assert_eq!(app.tx_list[i].cycle_us, 50_000, "and the declared period");

    app.send(crate::bus::BusCommand::SetEntryActive {
        ch: 1,
        id: 201,
        on: true,
    });
    assert_eq!(
        app.tx_list[i].srcs.len(),
        1,
        "ticking it back on does not rebuild the entry"
    );
    assert!(app.tx_list[i].active);
}

/// 角色开关关闭（节点离线/监听）不影响条目开关：条目仍是"启用"，
/// 只是开关不允许它发送；切回模拟即恢复。
#[test]
fn node_gate_does_not_touch_entry_switches() {
    let mut app = App::headless();
    app.set_node_role(1, "ABS", NodeRole::Simulated);
    let ids: Vec<u32> = app
        .tx_list
        .iter()
        .filter(|t| t.channel == 1)
        .map(|t| t.id)
        .collect();
    for id in ids {
        app.send(crate::bus::BusCommand::SetEntryActive {
            ch: 1,
            id,
            on: true,
        });
    }
    app.settle();

    // 切到离线：开关关闭（条目开/关保留），再切回模拟：原样恢复发送。
    app.set_node_role(1, "ABS", NodeRole::Absent);
    app.settle();
    for t in app.snap.tx.iter().filter(|t| t.channel == 1) {
        assert!(!t.gate_open, "offline closes the gate: {}", t.id);
        assert!(t.active, "switches stay as customized: {}", t.id);
    }

    app.set_node_role(1, "ABS", NodeRole::Simulated);
    app.settle();
    for t in app
        .snap
        .tx
        .iter()
        .filter(|t| t.channel == 1 && t.node == "ABS")
    {
        assert!(t.gate_open, "gate reopens: {}", t.id);
    }
}

/// 开关模型下，DBC 消失不影响“角色开关关闭即停发”：生成器用 base
/// 字节就能发送，DBC 只影响解码与补建。
#[test]
fn the_role_gate_silences_a_node_even_after_its_dbc_is_gone() {
    let mut app = App::headless();
    app.set_node_role(1, "ABS", NodeRole::Simulated);
    let ids: Vec<u32> = app
        .tx_list
        .iter()
        .filter(|t| t.channel == 1 && t.node == "ABS")
        .map(|t| t.id)
        .collect();
    for id in &ids {
        app.send(crate::bus::BusCommand::SetEntryActive {
            ch: 1,
            id: *id,
            on: true,
        });
    }
    app.settle();
    assert_eq!(active_ids(&app, 1).len(), 3);

    app.channels[1].dbc = None;
    app.refresh_snapshot();
    app.set_node_role(1, "ABS", NodeRole::Absent);
    app.settle();
    // 开关语义：开关原样保留（自定义不丢），但角色离线即不发送。
    // 只检查 ABS 名下的三条（其余节点的条目本就是关）。
    for id in &ids {
        let t = app
            .tx_list
            .iter()
            .find(|t| t.channel == 1 && t.id == *id)
            .expect("entry survives");
        assert!(t.active, "switch preserved: {id}");
    }
    assert_eq!(app.node_role(1, "ABS"), NodeRole::Absent);
}

#[test]
fn a_receive_only_node_can_still_be_simulated() {
    let mut app = App::headless();
    app.set_node_role(1, "DashBoard", NodeRole::Simulated);
    assert!(active_ids(&app, 1).is_empty(), "it has no messages to send");
    assert_eq!(
        app.node_role(1, "DashBoard"),
        NodeRole::Simulated,
        "the intent is remembered anyway"
    );
}

/// The zero-cost role: a monitor is present and listening, but the bus
/// carries nothing from it -- behaviourally the same as absent today,
/// and a different declaration for the hardware phase to honour.
#[test]
fn a_monitoring_node_declares_presence_without_traffic() {
    let mut app = App::headless();
    // 模拟并启用全部条目，然后切到监听：条目开关原样保留（自定义不
    // 被角色切换改写），开关关闭即停发。
    app.set_node_role(1, "ABS", NodeRole::Simulated);
    let ids: Vec<u32> = app
        .tx_list
        .iter()
        .filter(|t| t.channel == 1 && t.node == "ABS")
        .map(|t| t.id)
        .collect();
    for id in &ids {
        app.send(crate::bus::BusCommand::SetEntryActive {
            ch: 1,
            id: *id,
            on: true,
        });
    }
    app.settle();
    assert_eq!(active_ids(&app, 1).len(), 3);

    app.set_node_role(1, "ABS", NodeRole::Absent);
    assert_eq!(app.node_role(1, "ABS"), NodeRole::Absent);
    assert!(
        [199, 200, 201].iter().all(|id| app
            .tx_list
            .iter()
            .any(|t| t.channel == 1 && t.id == *id && t.active)),
        "its entries survive, ready for a return to Simulated"
    );
    assert!(
        !app.channels[1].node_roles.contains_key("GearBox"),
        "absent stays the stored default: no entry, no traffic"
    );
}

/// The restore chip compares a row against this, so it has to stay the
/// database's own opinion even after the row disagrees with it.
#[test]
fn the_declared_cycle_survives_a_hand_tuned_row() {
    let mut app = App::headless();
    assert_eq!(app.dbc_cycle_us(1, 0xC9), Some(50_000), "ABSdata");
    assert_eq!(
        app.dbc_cycle_us(0, 0x100),
        None,
        "sample.dbc declares nothing"
    );
    assert_eq!(app.dbc_cycle_us(1, 0x5AA), None, "no such message");
    let i = app
        .tx_list
        .iter()
        .position(|t| t.channel == 1 && t.id == 0xC9)
        .unwrap();
    app.tx_list[i].cycle_us = 250_000;
    assert_eq!(
        app.dbc_cycle_us(1, 0xC9),
        Some(50_000),
        "not whatever the row currently says"
    );

    app.channels[0].dbc = Some(std::sync::Arc::new(
        crate::dbc::load_dbc_str(CYCLE_TEST_DBC).unwrap(),
    ));
    app.refresh_snapshot();
    assert_eq!(
        app.dbc_cycle_us(0, 4096),
        Some(0),
        "a declared 0 is event-triggered, not undeclared"
    );
}

#[test]
fn the_cycle_box_accepts_only_whole_milliseconds_in_range() {
    assert_eq!(cycle_from_ms_text("100"), Some(100_000));
    assert_eq!(cycle_from_ms_text("  133 "), Some(133_000));
    assert_eq!(cycle_from_ms_text("0"), Some(0), "0 is event-triggered");
    assert_eq!(
        cycle_from_ms_text("60000"),
        Some(60_000_000),
        "top of the range"
    );
    assert_eq!(cycle_from_ms_text(""), None, "half-deleted text");
    assert_eq!(cycle_from_ms_text("1.5"), None, "no sub-millisecond step");
    assert_eq!(cycle_from_ms_text("-1"), None);
    assert_eq!(cycle_from_ms_text("60001"), None, "past the ceiling");
    assert_eq!(cycle_from_ms_text("abc"), None);
}

/// The parser writes `""` for a transmitter the DBC never assigned, and
/// that matches every unassigned message at once.
const NO_OWNER_DBC: &str = r#"VERSION "roxy-can orphan test"

NS_ :

BU_: ECU

BO_ 4096 Orphan: 8 Vector__XXX
 SG_ S : 0|8@1+ (1,0) [0|0] "" ECU

"#;

#[test]
fn a_node_with_no_name_simulates_nothing() {
    let mut app = App::headless();
    app.channels[0].dbc = Some(std::sync::Arc::new(
        crate::dbc::load_dbc_str(NO_OWNER_DBC).unwrap(),
    ));
    app.tx_list.retain(|t| t.channel != 0);
    app.add_tx(0, 4096);
    assert_eq!(entry_of(&app, 0, 4096).node, "", "unassigned");

    app.set_node_role(0, "", NodeRole::Simulated);
    assert!(
        app.channels[0].node_roles.is_empty(),
        "not even recorded as a tick"
    );
    assert!(
        active_ids(&app, 0).is_empty(),
        "an empty name must not adopt every message without an owner"
    );
}

#[test]
fn tx_generator_emits_frames() {
    let mut app = App::headless();
    app.add_tx(0, 0x777);
    let tx = app.tx_list.last_mut().expect("tx entry added");
    assert_eq!(tx.id, 0x777);
    assert_eq!(tx.channel, 0);
    assert!(!tx.active, "new entries start inactive");
    tx.cycle_us = 20_000;
    tx.active = true;
    app.start_virtual();
    app.update();
    assert!(
        app.trace
            .iter()
            .any(|f| f.id == 0x777 && matches!(f.dir, Direction::Tx)),
        "expected a Tx frame from the generator"
    );
    assert!(
        app.aggs.contains_key(&(0, 0x777, false)),
        "generator frames aggregate"
    );
    app.stop();
}

#[test]
fn export_trace_writes_parseable_asc() {
    let mut app = App::headless();
    for tx in &mut app.tx_list {
        tx.active = true;
        tx.cycle_us = 10_000;
    }
    open_all_gates(&mut app);
    app.start_virtual();
    for _ in 0..6 {
        std::thread::sleep(std::time::Duration::from_millis(11));
        app.update();
    }
    let n = app.trace.len();
    assert!(n > 0, "expected captured frames");
    let path = std::env::temp_dir().join("roxy_can_export_test.asc");
    let path_str = path.to_string_lossy().to_string();
    app.export_trace(0, &path_str);
    let content = std::fs::read_to_string(&path).unwrap();
    let frames = crate::log::asc::parse_asc(&content);
    assert_eq!(frames.len(), n, "exported frame count mismatch");
    std::fs::remove_file(&path).ok();
    app.stop();
}

/// A mixed log: one CAN frame and one FlexRay row per 2 ms step, the row a
/// millisecond after its frame. Both sides therefore have `rows` arrivals at a
/// 2 ms cadence, and the file's line order interleaves the two buses. The
/// FlexRay slot is the caller's, so a test can put the traffic where its own
/// description says a frame belongs.
fn write_mixed_fr_asc(name: &str, rows: u64, slot: u16) -> std::path::PathBuf {
    use crate::trace::FrRow;
    let path = std::env::temp_dir()
        .join(format!("{name}_{}.asc", std::process::id()));
    let mut w = AscWriter::new(&path.to_string_lossy()).unwrap();
    for i in 0..rows {
        let mut f = CanFrame {
            t_us: i * 2_000,
            channel: 0,
            id: 0x100,
            extended: false,
            len: 2,
            data: [0; MAX_CAN_FD_LEN],
            dir: Direction::Rx,
            flags: FrameFlags::NONE,
        };
        f.data[0] = i as u8;
        w.write(&f).unwrap();
        w.write_fr(&FrRow {
            bus: 0,
            t_us: i * 2_000 + 1_000,
            ab: 0,
            slot,
            cycle: i as u8,
            payload: vec![0xF0, i as u8],
            header_crc: 0,
            flags: 0,
            name: None,
        })
        .unwrap();
    }
    w.finish().unwrap();
    path
}

/// Replays `path` on the manual drive until the core has published it.
fn replay_to_the_end(app: &mut App, path: &std::path::Path) {
    app.load_log(&path.to_string_lossy());
    app.replay();
    for _ in 0..8 {
        std::thread::sleep(std::time::Duration::from_millis(11));
        app.update();
    }
    app.stop();
}

/// The trace export is not CAN-only: the FlexRay rows in the ring cross into
/// the ASC file too, and the file stays in time order, so an exported mixed
/// run is still a log rather than two logs glued together.
#[test]
fn the_trace_export_carries_flexray_rows() {
    let mut app = App::headless();
    let src = write_mixed_fr_asc("roxy_can_export_fr_in", 4, 13);
    replay_to_the_end(&mut app, &src);
    assert_eq!(app.snap.fr_trace.len(), 4, "the replay fed the FR ring");

    let out = std::env::temp_dir().join("roxy_can_export_fr_out.asc");
    app.export_trace(0, &out.to_string_lossy());
    std::fs::remove_file(&src).ok();
    let content = std::fs::read_to_string(&out).unwrap();
    std::fs::remove_file(&out).ok();
    let (can, fr) = crate::log::asc::parse_asc_full(&content);
    assert_eq!(can.len(), 4, "the CAN side still crosses");
    assert_eq!(fr.len(), 4, "so does FlexRay");
    assert_eq!(fr[2].slot, 13);
    assert_eq!(fr[2].payload, vec![0xF0, 2]);
    assert_eq!(fr[2].t_us, 5_000);
    assert_eq!(content.matches("Fr RMSG").count(), 4);
}

/// The Message Statistics table carries FlexRay slots with the same columns it
/// computes for CAN -- count, the min/avg/max of the observed intervals and
/// the share of the run -- and the same scope rule: pin the window to a CAN
/// bus and the slots leave, because a slot is not a CAN id.
#[test]
fn the_statistics_window_lists_flexray_slots() {
    let mut app = App::headless();
    let src = write_mixed_fr_asc("roxy_can_stats_fr_in", 4, 13);
    replay_to_the_end(&mut app, &src);
    std::fs::remove_file(&src).ok();

    app.sync_stats_text(0);
    let rows = &app.stats_windows[0].text_rows;
    let fr = rows.iter().find(|r| r.bus == "FR0").expect("a FlexRay row");
    assert_eq!(fr.label, "slot 13");
    assert_eq!(fr.count, "4");
    // Four arrivals 2 ms apart: every interval is the same, so min, avg and
    // max all read 2.00 ms.
    assert_eq!((fr.min.as_str(), fr.avg.as_str(), fr.max.as_str()), ("2.00", "2.00", "2.00"));
    assert_eq!(fr.len, "2");
    assert_eq!(fr.share, "50.0%", "half the run's frames are FlexRay");
    assert!(
        app.stats_windows[0].text_header.starts_with(&format!("{} messages", rows.len())),
        "the header counts the rows it shows: {:?}",
        app.stats_windows[0].text_header
    );

    let csv = std::env::temp_dir().join("roxy_can_stats_fr.csv");
    app.export_stats_csv(0, &csv.to_string_lossy());
    let text = std::fs::read_to_string(&csv).unwrap();
    std::fs::remove_file(&csv).ok();
    assert!(
        text.lines().any(|l| l.starts_with("FR0,13,")),
        "the CSV carries the slot: {text}"
    );

    // A bus-scoped window is a CAN scope.
    app.stats_windows[0].scope = crate::workspace::SigScope::Bus(0);
    app.text_fresh = true;
    app.sync_stats_text(0);
    assert!(
        app.stats_windows[0]
            .text_rows
            .iter()
            .all(|r| r.bus != "FR0"),
        "FlexRay leaves a CAN-bus window"
    );
}

/// The report carries its own premises -- which databases, the tolerance
/// and grace in effect, how many messages declared a period -- plus every
/// latched row. Without the header, the same table means something else
/// at a different threshold setting and cannot be re-checked.
#[test]
fn the_spec_report_export_includes_its_premises_and_rows() {
    let mut app = App::headless();
    app.start_virtual();
    app.tx_list.retain(|t| t.channel != 0);
    receive(
        &mut app,
        1_000,
        vec![rx_frame(1_000, 0x777, 8, FrameFlags::NONE)],
    );
    assert!(
        app.spec
            .rows
            .contains_key(&(0, 0x777, false, crate::spec::Kind::Unknown))
    );
    app.spec_tol_pct = 5;
    app.spec_grace = 4;

    let path = std::env::temp_dir().join("roxy_can_spec_report.csv");
    app.export_spec_csv(path.to_string_lossy().as_ref());
    let content = std::fs::read_to_string(&path).unwrap();

    assert!(
        content.contains("# database,CAN1,assets/sample.dbc"),
        "which database each bus carried"
    );
    assert!(content.contains("# tolerance,+/-5%"));
    assert!(content.contains("# grace,4x declared period"));
    assert!(
        content.contains("# periodic messages declared,"),
        "how many messages had a period to break"
    );
    assert!(
        content.contains("CAN1,777,not in database,Unknown id"),
        "the latched row itself: {content}"
    );
    std::fs::remove_file(&path).ok();
    app.stop();
}

#[test]
fn two_channels_aggregate_separately() {
    let mut app = App::headless();
    assert_eq!(app.channels.len(), 2);
    for (ch, c) in app.channels.iter().enumerate() {
        assert!(c.dbc.is_some(), "CAN{} should load its DBC", ch + 1);
    }
    // 角色开关：两条总线的节点全部模拟，条目才会发送。
    open_all_gates(&mut app);
    assert!(
        app.tx_list.iter().any(|t| t.channel == 0 && t.id == 0x100)
            && app.tx_list.iter().any(|t| t.channel == 1 && t.id == 0xC8),
        "generator pre-populated on both buses"
    );
    for tx in &mut app.tx_list {
        if (tx.channel == 0 && tx.id == 0x100) || (tx.channel == 1 && tx.id == 0xC8) {
            tx.active = true;
            tx.cycle_us = 10_000;
        }
    }
    app.start_virtual();
    for _ in 0..6 {
        std::thread::sleep(std::time::Duration::from_millis(11));
        app.update();
    }
    let a = app.aggs.get(&(0, 0x100, false)).expect("CAN1 aggregate");
    let b = app.aggs.get(&(1, 0xC8, false)).expect("CAN2 aggregate");
    assert!(a.count >= 3, "CAN1 frames: {}", a.count);
    assert!(b.count >= 3, "CAN2 frames: {}", b.count);
    assert!(app.trace.iter().any(|f| f.channel == 1 && f.id == 0xC8));
    app.stop();
}

#[test]
fn csv_exports_match_window_state() {
    let mut app = App::headless();
    let db = app.channels[0].dbc.as_ref().expect("sample DBC loaded");
    let id = db.order[0].0;
    let sig = db.messages[&(id, false)].signals[0].name.clone();
    let key = SigKey::can(0, id, false, sig);
    app.subscribe(key.clone());
    app.graphics[0].signals.push(GfxSignal {
        key: key.clone(),
        visible: true,
        y_mode: YMode::Auto,
    });
    app.data_windows[0].signals.push(GfxSignal {
        key: key.clone(),
        visible: true,
        y_mode: YMode::Auto,
    });
    for tx in &mut app.tx_list {
        tx.active = true;
        tx.cycle_us = 10_000;
    }
    open_all_gates(&mut app);
    app.start_virtual();
    for _ in 0..8 {
        std::thread::sleep(std::time::Duration::from_millis(11));
        app.update();
    }
    let dir = std::env::temp_dir();
    let stats = dir.join("roxy_stats_test.csv");
    app.export_stats_csv(0, &stats.to_string_lossy());
    let s = std::fs::read_to_string(&stats).unwrap();
    assert!(s.lines().count() > 1, "stats should have data rows");

    let msgs = dir.join("roxy_msgs_test.csv");
    app.export_messages_csv(0, &msgs.to_string_lossy());
    let m = std::fs::read_to_string(&msgs).unwrap();
    assert!(m.lines().count() > 1, "messages should have data rows");

    let gfx = dir.join("roxy_gfx_test.csv");
    app.export_graphics_csv(0, &gfx.to_string_lossy());
    let g = std::fs::read_to_string(&gfx).unwrap();
    assert!(g.contains(key.name()), "graphics history names the signal");

    let data = dir.join("roxy_data_test.csv");
    app.export_data_csv(0, &data.to_string_lossy());
    let d = std::fs::read_to_string(&data).unwrap();
    assert!(d.contains(key.name()), "data snapshot names the signal");

    for p in [&stats, &msgs, &gfx, &data] {
        std::fs::remove_file(p).ok();
    }
    app.stop();
}

#[test]
fn trace_filter_matches_by_name_id_and_direction() {
    let app = App::headless();
    let rx = CanFrame {
        t_us: 0,
        channel: 0,
        id: 0x100,
        extended: false,
        len: 8,
        data: [0; MAX_CAN_FD_LEN],
        dir: Direction::Rx,
        flags: FrameFlags::NONE,
    };
    let tx = CanFrame {
        id: 0x320,
        dir: Direction::Tx,
        ..rx
    };
    let rx_ch1 = CanFrame { channel: 1, ..rx };
    let unknown = CanFrame { id: 0x777, ..rx };
    let mut w = app.trace_windows[0].clone();
    assert!(app.trace_match(&w, &rx));
    assert!(app.trace_match(&w, &unknown));

    w.filter = "eng".to_string();
    assert!(app.trace_match(&w, &rx), "name match is case-insensitive");
    assert!(!app.trace_match(&w, &unknown));

    w.filter = "77".to_string();
    assert!(app.trace_match(&w, &unknown), "hex id substring");
    assert!(!app.trace_match(&w, &rx));

    w.filter.clear();
    w.dir = 1;
    assert!(app.trace_match(&w, &rx));
    assert!(!app.trace_match(&w, &tx), "Rx-only filter drops Tx frames");

    w.dir = 0;
    w.dbc_only = true;
    assert!(app.trace_match(&w, &rx));
    assert!(!app.trace_match(&w, &unknown), "DBC-only drops unknown IDs");

    w.dbc_only = false;
    w.scope = SigScope::Bus(0);
    assert!(app.trace_match(&w, &rx), "Bus scope passes its own bus");
    assert!(!app.trace_match(&w, &rx_ch1), "Bus scope drops other buses");

    w.scope = SigScope::Manual;
    w.manual.insert(crate::workspace::Pick::Can { ch: 0, id: 0x320 });
    assert!(
        !app.trace_match(&w, &rx),
        "Manual selection drops unselected IDs"
    );
    assert!(
        app.trace_match(&w, &tx),
        "Manual selection passes the chosen ID"
    );
    w.manual.clear();
    assert!(
        !app.trace_match(&w, &tx),
        "empty Manual selection passes nothing"
    );
    w.scope = SigScope::All;
    assert!(app.trace_match(&w, &rx), "All scope passes everything");
}

/// The row menu's "Filter this ID" has to leave on screen the row it was opened
/// on -- and nothing else. The box's plain text is a substring search, so the
/// old `1AB` also kept `1AB0` and `21AB`; the menu now writes the exact form,
/// with the `x` suffix a 29-bit frame's row needs because the frame class is
/// part of its address.
#[test]
fn the_trace_row_menu_filters_to_the_row_it_was_opened_on() {
    let mut app = App::headless();
    let mut ext = frame_at(1_000, 0x1ABCDEF, 8, Direction::Rx);
    ext.extended = true;
    let std_frame = frame_at(2_000, 0x1AB, 8, Direction::Rx);
    let longer_std = frame_at(3_000, 0x1AB0, 8, Direction::Rx);

    crate::ui::trace::filter_to_id(&mut app, 0, &ext);
    let w = app.trace_windows[0].clone();
    assert!(
        app.trace_match(&w, &ext),
        "the extended row survives its own menu (filter {:?})",
        app.trace_windows[0].filter
    );
    assert!(
        !app.trace_match(&w, &std_frame),
        "and it names one frame class, not every frame holding those digits"
    );

    // A standard frame: the exact form, and the over-match the prefix exists to
    // stop.
    crate::ui::trace::filter_to_id(&mut app, 0, &std_frame);
    assert_eq!(app.trace_windows[0].filter, "id:1AB");
    let w = app.trace_windows[0].clone();
    assert!(app.trace_match(&w, &std_frame));
    assert!(!app.trace_match(&w, &ext), "now the extended twin is the one out");
    let loose = {
        let mut win = app.trace_windows[0].clone();
        win.filter = "1AB".to_string();
        win
    };
    assert!(
        app.trace_match(&loose, &longer_std),
        "the substring search is still a substring search"
    );
    assert!(
        !app.trace_match(&w, &longer_std),
        "and `id:` is what you ask for one id and nothing else"
    );
}

/// `slot:13` is the address the plain text cannot express: a `13` search also
/// matches slot 113, slot 130 and any frame name holding those digits. The two
/// numbering spaces stay apart -- an id names no slot and a slot names no CAN
/// frame -- because translating between them is exactly the confusion the
/// filters exist to avoid.
#[test]
fn the_trace_filter_takes_an_exact_slot_or_id() {
    let app = quiet_app();
    let row = |slot: u16| crate::trace::FrRow {
        bus: 0,
        t_us: 1_000,
        ab: 0,
        slot,
        cycle: 0,
        payload: vec![1],
        header_crc: 0,
        flags: 0,
        name: None,
    };
    let mk = |app: &App, filter: &str| {
        let mut w = app.trace_windows[0].clone();
        w.filter = filter.to_string();
        w.filter_lens()
    };
    let flt = mk(&app, "13");
    assert!(app.trace_fr_match(&flt, &row(13)));
    assert!(
        app.trace_fr_match(&flt, &row(113)),
        "the substring search is still a substring search"
    );
    let flt = mk(&app, "slot:13");
    assert!(app.trace_fr_match(&flt, &row(13)), "slot 13 stays");
    assert!(!app.trace_fr_match(&flt, &row(113)), "slot 113 leaves");
    assert!(!app.trace_fr_match(&flt, &row(130)), "and so does slot 130");
    let can = frame_at(1_000, 0x13, 8, Direction::Rx);
    assert!(
        !app.trace_match_lens(&flt, &can),
        "a slot is not a CAN id, so the CAN rows leave"
    );

    let flt = mk(&app, "id:13");
    assert!(app.trace_match_lens(&flt, &can));
    assert!(
        !app.trace_fr_match(&flt, &row(13)),
        "and an id is not a slot, so the FlexRay rows leave"
    );
    // The frame class is part of a CAN address here, as it is in the ID column.
    let mut ext = frame_at(2_000, 0x13, 8, Direction::Rx);
    ext.extended = true;
    assert!(
        !app.trace_match_lens(&flt, &ext),
        "id:13 means the standard frame"
    );
    let flt = mk(&app, "id:13x");
    assert!(
        app.trace_match_lens(&flt, &ext),
        "id:13x means the extended one"
    );
    assert!(!app.trace_match_lens(&flt, &can));
    // A signal really can be named `Slot`, and `Slot=3` must stay the value
    // condition it has always been -- which is why the exact form uses a colon.
    let flt = mk(&app, "Slot=3");
    assert_eq!(flt.exact, None, "an `=` text is a condition, not an address");
    assert!(!flt.value_conds.is_empty());
    // A half-typed address falls back to the substring search it would have
    // been: the table goes empty and the box still shows exactly what was
    // typed. The alternative -- ignoring a filter that looks armed -- would
    // hide the reason instead of stating it.
    for typo in ["id:", "id:XYZ", "slot:", "slot:70000"] {
        let flt = mk(&app, typo);
        assert_eq!(flt.exact, None, "{typo} is not an address");
        assert_eq!(
            flt.query.as_deref(),
            Some(typo.to_ascii_uppercase().as_str()),
            "{typo} stays the text search it was"
        );
    }
}

/// The row menu's "Clear filter" has to lift *every* condition, including the
/// ones that live in the collapsed 筛选 row: it used to reset the text box, the
/// direction, 仅 DBC and the scope, and quietly leave the payload search, the
/// frame kind and the time range working. A button that names the whole filter
/// and clears half of it is the same lie as a "Clear" that clears one of two
/// tables -- and now that the time range is saved with the project, a reopened
/// session can be hiding rows nothing on screen still shows.
#[test]
fn the_row_menu_clear_filter_lifts_every_condition() {
    let mut app = App::headless();
    let base = app.trace_windows[0].clone();
    /// One filter condition and how to switch it on alone.
    type Case = (&'static str, fn(&mut crate::workspace::TraceWin));
    // One row every condition below can exclude by itself: unknown to the DBC,
    // Rx, classic, empty payload, bus 0, at 9 s.
    let f = frame_at(9_000_000, 0x777, 8, Direction::Rx);
    let cases: [Case; 7] = [
        ("the text box", |w| w.filter = "zzz".into()),
        ("payload", |w| w.payload = "AA BB".into()),
        ("帧类型", |w| w.flags_kind = 2),
        ("仅 DBC", |w| w.dbc_only = true),
        ("方向", |w| w.dir = 2),
        (
            "时间范围",
            |w| {
                w.time_from = "1.0".into();
                w.time_to = "2.5".into();
            },
        ),
        ("作用域", |w| w.scope = SigScope::Bus(1)),
    ];
    for (what, set) in cases {
        let mut w = base.clone();
        set(&mut w);
        app.trace_windows[0] = w;
        assert!(
            !app.trace_match(&app.trace_windows[0], &f),
            "{what} alone hides the row"
        );
        crate::ui::trace::clear_filter(&mut app, 0);
        assert!(
            app.trace_match(&app.trace_windows[0], &f),
            "and the menu's Clear filter lifts {what}"
        );
    }

    // What it must not touch: the window's Manual pick set is a list the
    // operator built, not a condition they typed.
    let mut w = base.clone();
    w.scope = SigScope::Manual;
    w.manual.insert(crate::workspace::Pick::Fr { bus: 0, slot: 13 });
    app.trace_windows[0] = w;
    crate::ui::trace::clear_filter(&mut app, 0);
    assert_eq!(app.trace_windows[0].scope, SigScope::All);
    assert!(
        !app.trace_windows[0].manual.is_empty(),
        "a one-key clear must not quietly eat the curated selection"
    );
}

/// The other half of the same defect: a closed 筛选 row keeps filtering with
/// nothing on screen to say so. The toggle now carries the count and names the
/// conditions on hover, so the report is built from what actually filters --
/// text that fails to parse hides nothing and must not be counted.
#[test]
fn a_closed_filter_row_names_the_conditions_still_working() {
    let mut app = App::headless();
    let w = &mut app.trace_windows[0];
    assert!(w.hidden_conds().is_empty(), "an untouched window hides nothing");
    // Filled in but not filtering: unparseable payload text, an unparsable
    // second, a frame kind left on Any.
    w.payload = "zz".to_string();
    w.time_from = "abc".to_string();
    assert!(
        w.hidden_conds().is_empty(),
        "text that does not filter is not reported as filtering: {:?}",
        w.hidden_conds()
    );
    w.payload = "11 22".to_string();
    w.time_from = "1.0".to_string();
    w.flags_kind = 1;
    w.dbc_only = true;
    assert_eq!(
        w.hidden_conds(),
        ["payload", "帧类型", "仅 DBC", "时间范围"],
        "every condition that is working while the row is closed"
    );
    // The main row's own controls are always visible, so they never join the
    // list -- and neither does the FR expansion, which adds rows.
    w.filter = "Motor".to_string();
    w.dir = 1;
    w.fr_expand = true;
    assert_eq!(
        w.hidden_conds(),
        ["payload", "帧类型", "仅 DBC", "时间范围"],
        "visible controls and row-expansion are not conditions in disguise"
    );
}

/// `CarSpeed>60` is not a CAN-only question. A FlexRay row is filtered by the
/// value its own frame carries -- the same physical number the curve plots and
/// the expanded child row prints -- and a row nothing can decode leaves the
/// table rather than sitting in it unexplained.
#[test]
fn a_flexray_row_is_filtered_by_the_value_its_frame_carries() {
    let arxml = "assets/arxml/PowerTrain.arxml";
    let mut app = App::headless();
    assert!(
        app.load_cluster_description(arxml, Some(0)).is_some(),
        "the bundled description loads"
    );
    // The first signal the description really decodes, with the value an
    // all-zero payload reads as -- the number the row itself would print.
    let (slot, cycle, ab, name, phys, payload) = {
        let db = app.fr_db(0).expect("loaded above");
        let mut found = None;
        for ix in 0..db.frames.len() {
            let Some(f) = db.frame_index(ix) else { continue };
            let bytes = vec![0u8; (f.length as usize).max(1)];
            if let Some(sig) = db.decode_signals(ix, &bytes).into_iter().next() {
                found = Some((
                    f.triggering.slot_id as u16,
                    f.triggering.base_cycle as u8,
                    2u8,
                    sig.name,
                    sig.phys,
                    bytes,
                ));
                break;
            }
        }
        found.expect("the bundled description decodes at least one signal")
    };
    let row = crate::trace::FrRow {
        bus: 0,
        t_us: 1_000,
        ab,
        slot,
        cycle,
        // The payload the value was read out of: a narrower one would drop the
        // signal for running out of bytes, which is a different rule.
        payload,
        header_crc: 0,
        flags: 0,
        name: None,
    };
    let mk = |app: &App, filter: &str| {
        let mut w = app.trace_windows[0].clone();
        w.filter = filter.to_string();
        w.filter_lens()
    };
    let probe = |app: &App, filter: &str| app.trace_fr_match(&mk(app, filter), &row);

    assert!(
        probe(&app, &format!("{name}>{}", phys - 1.0)),
        "the row's own value clears a lower bound"
    );
    assert!(
        !probe(&app, &format!("{name}>{}", phys)),
        "and does not clear itself: `>` is strict"
    );
    assert!(probe(&app, &format!("{name}>={}", phys)));
    assert!(
        probe(&app, &format!("{name}=={phys}")),
        "the compared number is the decoded one, printed or not"
    );
    assert!(
        !probe(&app, "NoSuchSignal>0"),
        "a signal this frame does not carry drops the row rather than passing it"
    );

    // A row on a bus with no description cannot be read at all, so a value
    // condition has no opinion to keep it for.
    let bare = crate::trace::FrRow {
        bus: 7,
        ..row.clone()
    };
    assert!(
        !app.trace_fr_match(&mk(&app, &format!("{name}>-1e18")), &bare),
        "undescribed: no decoding, no match"
    );
    // ...while the same row is still visible under a filter that asks nothing of
    // its values -- the condition, not the missing file, is what hides it.
    assert!(app.trace_fr_match(&mk(&app, ""), &bare), "no condition: it shows");
}

#[test]
fn channels_can_be_added_removed_and_renamed() {
    let mut app = App::headless();
    assert_eq!(app.channels.len(), 2);
    app.channels[0].name = "Powertrain".to_string();
    app.refresh_snapshot();
    assert_eq!(app.channel_name(0), "Powertrain");

    app.add_channel();
    assert_eq!(app.channels.len(), 3);
    assert_eq!(app.channel_name(2), "CAN3");

    app.aggs.insert(
        (1, 0x100, false),
        MessageAgg {
            id: 0x100,
            channel: 1,
            extended: false,
            dir: Direction::Rx,
            count: 1,
            rx: 1,
            tx: 0,
            last_t_us: 0,
            cycle_us: 0.0,
            min_us: 0.0,
            max_us: 0.0,
            jitter_us: 0.0,
            len: 8,
            data: [0; MAX_CAN_FD_LEN],
            flags: FrameFlags::NONE,
        },
    );
    app.trace_windows[0]
        .manual
        .insert(crate::workspace::Pick::Can { ch: 2, id: 0x200 });
    // A hand-picked FlexRay slot in the same window: a cluster index is not a
    // channel index, so removing a CAN channel must leave it alone.
    app.trace_windows[0]
        .manual
        .insert(crate::workspace::Pick::Fr { bus: 2, slot: 71 });
    app.trace_windows[0].scope = SigScope::Bus(2);
    // A FlexRay cluster scope on another window: cluster indices are their own
    // numbering space, so removing a CAN channel must not renumber it -- the
    // shift below would otherwise point the window at a different cable.
    app.msg_windows[0].scope = SigScope::FrBus(1);
    let w = app.trace_windows[0].clone();

    app.remove_channel(0);
    assert_eq!(app.channels.len(), 2);
    assert_eq!(app.channel_name(0), "CAN2", "remaining buses shift down");
    assert!(
        app.aggs.contains_key(&(0, 0x100, false)),
        "agg remapped 1 -> 0"
    );
    assert!(
        app.trace_windows[0]
            .manual
            .contains(&crate::workspace::Pick::Can { ch: 1, id: 0x200 }),
        "filter remapped 2 -> 1"
    );
    assert!(
        app.trace_windows[0]
            .manual
            .contains(&crate::workspace::Pick::Fr { bus: 2, slot: 71 }),
        "the FlexRay pick is not a channel index and did not shift"
    );
    assert_eq!(w.scope, SigScope::Bus(2), "cloned window is untouched");
    assert_eq!(
        app.trace_windows[0].scope,
        SigScope::Bus(1),
        "Bus scope indices shift with the channels"
    );
    assert_eq!(
        app.msg_windows[0].scope,
        SigScope::FrBus(1),
        "a FlexRay cluster scope is not a CAN channel index and does not move"
    );

    while app.channels.len() > 1 {
        app.remove_channel(0);
    }
    assert_eq!(app.channels.len(), 1, "last bus cannot be removed");
}

#[test]
fn replay_position_tracks_playback() {
    let mut app = App::headless();
    let path = std::env::temp_dir().join("roxy_can_seek_test.asc");
    app.record_path_buf = path.to_string_lossy().to_string();
    app.toggle_record();
    for tx in &mut app.tx_list {
        tx.active = true;
        tx.cycle_us = 10_000;
    }
    open_all_gates(&mut app);
    app.start_virtual();
    for _ in 0..12 {
        std::thread::sleep(std::time::Duration::from_millis(11));
        app.update();
    }
    app.stop();
    let file = app.recorder.last_record.clone();
    app.load_log(&file);
    app.replay();
    let (pos0, dur) = app.replay_position().expect("replay has a timeline");
    assert!(dur > 0.0, "timeline covers the whole log");
    assert!(pos0 < 0.01, "playback starts at the beginning");
    // The first poll only anchors the replay clock, so a second
    // cycle is needed before the position actually advances.
    std::thread::sleep(std::time::Duration::from_millis(15));
    app.update();
    std::thread::sleep(std::time::Duration::from_millis(15));
    app.update();
    let (pos1, _) = app.replay_position().unwrap();
    assert!(pos1 > pos0, "position advances while replaying");
    app.stop();
    std::fs::remove_file(&file).ok();
}

/// A log of `n` frames spaced `step_us` apart, so the timeline is exactly
/// `(n-1) * step_us` long and every frame sits on a round second.
fn write_timed_asc(name: &str, n: u32, step_us: u64) -> std::path::PathBuf {
    let path = std::env::temp_dir().join(name);
    {
        let mut w = AscWriter::new(&path.to_string_lossy()).unwrap();
        for i in 0..n {
            let mut f = CanFrame {
                t_us: u64::from(i) * step_us,
                channel: 0,
                id: 0x100,
                extended: false,
                len: 2,
                data: [0; MAX_CAN_FD_LEN],
                dir: Direction::Rx,
                flags: FrameFlags::NONE,
            };
            f.data[0] = 0xAA;
            f.data[1] = 0xBB;
            w.write(&f).unwrap();
        }
        w.finish().unwrap();
    }
    path
}

#[test]
fn play_after_a_finished_run_restarts_from_the_top() {
    // Regression: pressing Play once a replay had run out resumed an
    // exhausted source -- a silent no-op that left the readout stuck at the
    // tail. Play must instead re-open the log and run from zero.
    let mut app = App::headless();
    let path = write_timed_asc("roxy_can_replay_restart.asc", 100, 10_000);
    app.load_log(&path.to_string_lossy());
    // Run the log out the far end without touching Stop: the infinite speed
    // drains it in two laps.
    app.set_replay_speed(f64::INFINITY);
    app.replay();
    app.update();
    app.update();
    assert!(!app.measuring, "the replay finished on its own");
    let (pos_end, dur) = app.replay_position().unwrap();
    assert!(
        pos_end >= dur - 1e-6,
        "playhead parked at the end, got {pos_end} of {dur}"
    );

    app.toggle_play(); // Play, with no Stop in between
    assert!(app.measuring, "Play restarts a finished replay, not a no-op");
    let (pos_top, _) = app.replay_position().unwrap();
    assert!(
        pos_top < pos_end,
        "the playhead returned to the top, got {pos_top}"
    );
    app.stop();
    std::fs::remove_file(&path).ok();
}

#[test]
fn stop_makes_the_next_play_restart_from_zero() {
    let mut app = App::headless();
    let path = write_timed_asc("roxy_can_replay_stop.asc", 100, 10_000);
    app.load_log(&path.to_string_lossy());
    // Park the playhead deep in the log, then Stop explicitly -- the gesture
    // that used to leave a resume-in-place option open.
    app.set_replay_speed(f64::INFINITY);
    app.replay();
    app.update();
    app.update();
    app.stop();
    app.toggle_play();
    let (pos, _) = app.replay_position().unwrap();
    assert!(
        pos < 0.01,
        "Stop is an explicit request to re-open from the beginning, got {pos}"
    );
    app.stop();
    std::fs::remove_file(&path).ok();
}

#[test]
fn the_plot_clock_follows_the_replay_playhead() {
    let mut app = App::headless();
    let path = write_timed_asc("roxy_can_plot_clock.asc", 100, 10_000);
    app.load_log(&path.to_string_lossy());
    app.replay();
    app.update();
    let advanced = app.plot_now_s();
    assert!(
        (advanced - app.replay_position().unwrap().0).abs() < 1e-9,
        "the Graphics axis must ride the playhead, got {advanced}"
    );
    // A faster clock moves the axis with it rather than lagging on wall time.
    app.set_replay_speed(f64::INFINITY);
    app.update();
    let (pos, dur) = app.replay_position().unwrap();
    assert!(
        (app.plot_now_s() - dur).abs() < 1e-9,
        "the axis follows the drained playhead, got {} of {dur}",
        app.plot_now_s()
    );
    assert!(pos >= dur - 1e-6, "setup: the log ran to its end");
    app.stop();
    std::fs::remove_file(&path).ok();
}

#[test]
fn loading_another_log_mid_replay_is_refused() {
    let mut app = App::headless();
    let a = write_timed_asc("roxy_can_guard_a.asc", 50, 10_000);
    let b = write_timed_asc("roxy_can_guard_b.asc", 50, 10_000);
    app.load_log(&a.to_string_lossy());
    app.replay();
    assert!(app.measuring);

    app.load_log(&b.to_string_lossy());
    assert_eq!(
        app.log_path,
        a.to_string_lossy(),
        "the running log must stay selected"
    );
    assert!(
        app.status.contains("stop the replay"),
        "the refusal should say why, got {:?}",
        app.status
    );
    assert!(
        !app.recent_log
            .iter()
            .any(|p| p == &b.to_string_lossy().to_string()),
        "a refused load must not enter the recent list"
    );

    // Stopped, the same selection goes through.
    app.stop();
    app.load_log(&b.to_string_lossy());
    assert_eq!(app.log_path, b.to_string_lossy());
    app.stop();
    std::fs::remove_file(&a).ok();
    std::fs::remove_file(&b).ok();
}

#[test]
fn play_after_choosing_a_new_log_opens_that_log() {
    let mut app = App::headless();
    let a = write_timed_asc("roxy_can_switch_a.asc", 100, 10_000);
    let b = write_timed_asc("roxy_can_switch_b.asc", 20, 10_000);
    app.load_log(&a.to_string_lossy());
    app.replay();
    app.set_replay_speed(8.0);
    // Let the first log run to its natural end, which leaves the run settled
    // but still replay-able -- exactly where the old code could resume the
    // wrong file.
    let (_, dur_a) = app.replay_position().unwrap();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while app.measuring && std::time::Instant::now() < deadline {
        std::thread::sleep(std::time::Duration::from_millis(11));
        app.update();
    }
    assert!(
        !app.measuring,
        "setup: the first log finished on its own, {} s of it unread",
        dur_a
    );

    app.load_log(&b.to_string_lossy());
    app.toggle_play();
    let (_, dur_b) = app.replay_position().expect("the new log has a timeline");
    assert!(
        dur_b < dur_a,
        "Play must open the newly selected log ({dur_b}s) rather than resume \
             the finished one ({dur_a}s)"
    );
    app.stop();
    std::fs::remove_file(&a).ok();
    std::fs::remove_file(&b).ok();
}

fn blank_sub() -> Subscription {
    Subscription {
        latest: 0.0,
        last_raw: 0,
        unit: String::new(),
        label: None,
        type_tag: String::new(),
        min: f64::INFINITY,
        max: f64::NEG_INFINITY,
        avg: 0.0,
        sum: 0.0,
        n: 0,
        last_update_us: 0,
        last_sample_us: 0,
        history: SampleCache::default(),
        published: std::sync::Arc::new(SampleCache::default()),
        history_dirty: false,
        color: 0,
    }
}

#[test]
fn the_head_of_the_curve_survives_past_the_old_point_cap() {
    // 250 s of samples at the real 50 ms interval: beyond the 4000-point
    // cap this replaces, which began popping the head at exactly 200 s and
    // made the left end of a running trace vanish on its own.
    let mut sub = blank_sub();
    for i in 0..5_000u64 {
        sub.push_sample(i * SAMPLE_INTERVAL_US, (i % 97) as f64, SAMPLE_INTERVAL_US);
    }
    assert_eq!(sub.history.len(), 5_000, "250 s fits inside the span");
    assert_eq!(
        sub.history.first().unwrap().0,
        0,
        "the head must still be there mid-run"
    );
}

#[test]
fn eviction_begins_only_past_the_retention_span() {
    let mut sub = blank_sub();
    let n = HISTORY_SPAN_US / SAMPLE_INTERVAL_US + 10;
    for i in 0..n {
        sub.push_sample(i * SAMPLE_INTERVAL_US, 1.0, SAMPLE_INTERVAL_US);
    }
    assert!(sub.history.len() < n as usize, "stale head is dropped");
    let kept = sub.history.len() as u64 * SAMPLE_INTERVAL_US;
    assert!(
        kept <= HISTORY_SPAN_US + SAMPLE_INTERVAL_US,
        "retained span {kept} us exceeds the cap"
    );
    assert!(
        sub.n > sub.history.len() as u64,
        "min/max/avg stay cumulative over the whole run"
    );
}

#[test]
fn retention_backs_the_widest_plot_window() {
    assert!(
        HISTORY_SPAN_US as f64 / 1e6 >= crate::ui::graphics::MAX_TIME_WINDOW_S,
        "the widest window is {} s but history only holds {} s",
        crate::ui::graphics::MAX_TIME_WINDOW_S,
        HISTORY_SPAN_US as f64 / 1e6,
    );
}

/// Records `iters` frames of generator traffic to an ASC, then returns an
/// App with the first sample.dbc signal subscribed and that log loaded but
/// not yet playing. The traffic has to be DBC-decodable for the Graphics
/// history to fill, so a hand-written fixture will not do.
fn app_with_replayable_recording(
    name: &str,
    iters: usize,
) -> (App, crate::observe::SigKey, String) {
    let mut app = App::headless();
    let key = {
        let db = app.channel_dbc(0).expect("sample DBC loaded");
        let id = db.order[0].0;
        SigKey::can(0u8, id, false, db.messages[&(id, false)].signals[0].name.clone())
    };
    app.subscribe(key.clone());
    let out = std::env::temp_dir().join(format!("roxy_can_{name}.asc"));
    app.record_path_buf = out.to_string_lossy().to_string();
    app.toggle_record();
    for tx in &mut app.tx_list {
        tx.active = true;
        tx.cycle_us = 10_000;
    }
    open_all_gates(&mut app);
    app.start_virtual();
    for _ in 0..iters {
        std::thread::sleep(std::time::Duration::from_millis(11));
        app.update();
    }
    app.stop();
    std::fs::remove_file(&out).ok();
    let file = app.recorder.last_record.clone();
    app.load_log(&file);
    (app, key, file)
}

#[test]
fn the_published_cache_view_stays_still_while_the_run_goes_on() {
    let mut c = SampleCache::default();
    c.merge(
        &(0..4u64)
            .map(|i| (i * 10_000, i as f64))
            .collect::<Vec<_>>(),
        10_000,
    );
    let published = c.clone(); // what a snapshot hands the UI

    // More samples arrive and the retention span trims the oldest away;
    // neither may disturb the view the UI is still reading.
    c.merge(
        &(10..12u64)
            .map(|i| (i * 10_000, i as f64))
            .collect::<Vec<_>>(),
        10_000,
    );
    c.trim_oldest(50_000);

    assert_eq!(
        published.len(),
        4,
        "the published view is frozen at its publish instant"
    );
    assert_eq!(published.first().unwrap().0, 0);
    assert_eq!(
        c.first().unwrap().0,
        100_000,
        "the working cache trims independently (50 000 span keeps two points)"
    );
    assert_eq!(c.len(), 2);
}

#[test]
fn merging_across_sealed_chunks_keeps_order_and_gap_rules() {
    let mut c = SampleCache::default();
    let n = crate::observe::SEAL_POINTS * 3 + 5;
    c.merge(
        &(0..n as u64)
            .map(|i| (i * 1_000, i as f64))
            .collect::<Vec<_>>(),
        1_000,
    );
    assert_eq!(c.len(), n);

    // Both candidates sit inside the gap of their neighbours: rejected,
    // and the ascending order across three sealed chunks survives.
    let back: Vec<(u64, f64)> = [3_500u64, 7_500].iter().map(|&t| (t, t as f64)).collect();
    let taken = c.merge(&back, 1_000);
    assert!(taken.is_empty(), "in-gap candidates are rejected");
    assert!(
        c.iter().zip(c.iter().skip(1)).all(|(a, b)| a.0 < b.0),
        "a merge spanning sealed chunks keeps the order"
    );
    assert_eq!(c.at(3_599), Some(3.0), "3_500 was rejected; 3_000 answers");

    // Trimming to a 500 ms span drops the first two sealed chunks whole
    // and trims the third at the horizon: 1_540_000 - 500_000.
    c.trim_oldest(500_000);
    assert_eq!(
        c.first().unwrap().0,
        1_040_000,
        "stale head chunks are dropped wholesale, the head chunk is trimmed in place"
    );
    assert_eq!(c.at(1_100_000), Some(1_100.0));
}

#[test]
fn sample_cache_range_and_lookup_are_inclusive_and_ordered() {
    let mut c = SampleCache::default();
    c.merge(
        &(0..10u64)
            .map(|i| (i * 1_000, i as f64))
            .collect::<Vec<_>>(),
        1_000,
    );
    let win = c.range(2_000, 5_000);
    assert_eq!(
        win.map(|(t, _)| *t).collect::<Vec<_>>(),
        vec![2_000, 3_000, 4_000, 5_000],
        "both ends of the window are included"
    );
    assert_eq!(c.at(4_500), Some(4.0), "last value at or before");
    assert_eq!(
        c.at(0),
        Some(0.0),
        "the first sample resolves on its own edge"
    );
    assert_eq!(c.at(999), Some(0.0), "step-signal semantics hold");
    assert_eq!(
        SampleCache::default().at(999),
        None,
        "an empty cache has no value to report"
    );
}

#[test]
fn sample_cache_trims_by_span_not_by_count() {
    let mut c = SampleCache::default();
    c.merge(
        &(0..30u64).map(|i| (i * 10_000, 1.0)).collect::<Vec<_>>(),
        10_000,
    );
    c.trim_oldest(100_000);
    assert_eq!(
        c.first().unwrap().0,
        190_000,
        "newest is 290 s, so everything from 190 s on survives"
    );
    assert_eq!(c.len(), 11);
}

#[test]
fn a_second_replay_run_samples_from_the_top() {
    let (mut app, key, file) = app_with_replayable_recording("resample", 60);

    // First run: play the log out so the subscription ends up with a
    // sampling baseline near the end of it.
    app.replay();
    app.set_replay_speed(4.0);
    let deadline = std::time::Instant::now() + std::time::Duration::from_millis(250);
    while std::time::Instant::now() < deadline {
        std::thread::sleep(std::time::Duration::from_millis(11));
        app.update();
    }
    let stale_baseline = app.subs.get(&key).unwrap().last_sample_us;
    assert!(
        stale_baseline > 200_000,
        "setup: the first run should have sampled deep into the log, got {stale_baseline} us"
    );

    // Second run. Replaying used to inherit that baseline, and the sampler
    // gate then rejected every frame until the playhead climbed past it --
    // visibly, the start of the curve was simply missing.
    app.replay();
    {
        let sub = app.subs.get(&key).unwrap();
        assert!(sub.history.is_empty(), "a fresh run drops the old trace");
        assert_eq!(
            sub.last_sample_us, 0,
            "a fresh run must not inherit the sampling baseline"
        );
        assert_eq!(sub.n, 0, "a fresh run resets the sample count");
        assert!(
            !sub.min.is_finite() && sub.max == f64::NEG_INFINITY,
            "a fresh run resets min/max instead of keeping the old extremes"
        );
    }
    // The first poll only anchors the replay clock; the second is what
    // moves the playhead past the log's opening frames.
    std::thread::sleep(std::time::Duration::from_millis(20));
    app.update();
    std::thread::sleep(std::time::Duration::from_millis(20));
    app.update();
    let sub = app.subs.get(&key).unwrap();
    assert!(
        !sub.history.is_empty(),
        "sampling must start at the top of the log, not at {stale_baseline} us"
    );
    assert!(
        sub.history.first().unwrap().0 < stale_baseline,
        "the first sample of the new run should precede the previous run's last"
    );
    app.stop();
    std::fs::remove_file(&file).ok();
}

#[test]
fn replay_injection_lands_on_the_log_timeline() {
    let mut app = App::headless();
    let file = write_timed_asc("roxy_can_inject.asc", 3, 100_000);
    app.load_log(&file.to_string_lossy());
    // An id the log does NOT carry: a log-carried id stands down during
    // replay (see the muting test below), and what this test pins is the
    // injection's timing.
    app.tx_list[0].id = 0x123;
    let tx_id = app.tx_list[0].id;
    let tx_ch = app.tx_list[0].channel;
    app.tx_list[0].active = true;
    app.tx_list[0].cycle_us = 40_000;
    app.tx_list[0].data = [0xDE; MAX_CAN_FD_LEN];
    // 角色开关允许发送：条目 node 戳仍是 EngineECU。
    app.set_node_role(tx_ch, "EngineECU", NodeRole::Simulated);
    app.replay();
    // Drive the replay by hand: 50 ms wall steps release the log frames at
    // 0 / 100 / 200 ms. Injection at 40 ms into a quiet stretch must wait
    // for the log's next frame rather than ride the wall clock.
    for step in 1..=7u64 {
        app.tick(step * 50_000);
    }
    let injected: Vec<u64> = app
        .trace
        .iter()
        .filter(|f| f.id == tx_id && matches!(f.dir, Direction::Tx))
        .map(|f| f.t_us)
        .collect();
    assert!(!injected.is_empty(), "the generator injects during replay");
    assert!(
        injected.iter().all(|&t| t <= 200_000),
        "injections ride the log clock, not the wall clock: {injected:?}"
    );
    assert!(
        injected.iter().all(|t| t % 40_000 == 0),
        "injections sit on their declared cycle within the log timeline: {injected:?}"
    );
    let agg = app
        .aggs
        .get(&(tx_ch, tx_id, false))
        .expect("injected frames aggregate");
    assert_eq!(agg.count, injected.len() as u64);
    assert!(
        app.trace
            .iter()
            .filter(|f| f.id == 0x100 && matches!(f.dir, Direction::Rx))
            .count()
            == 3,
        "the log frames themselves survive alongside the injections"
    );
    app.stop();
    std::fs::remove_file(&file).ok();
}

#[test]
fn replay_speed_steps_along_the_ladder() {
    let mut app = App::headless();
    assert_eq!(app.replay_speed, 1.0);
    app.step_replay_speed(1);
    assert_eq!(app.replay_speed, 2.0, "one notch faster");
    app.step_replay_speed(-1);
    app.step_replay_speed(-1);
    assert_eq!(app.replay_speed, 0.5, "two notches slower");
    app.step_replay_speed(-1);
    assert_eq!(app.replay_speed, 0.5, "clamped at the slow end");
    app.step_replay_speed(99);
    assert!(
        app.replay_speed.is_infinite(),
        "the fast end is as-fast-as-possible"
    );
    app.step_replay_speed(-1);
    assert_eq!(app.replay_speed, 4.0, "one notch back from infinity");
}

#[test]
fn starting_clears_the_previous_pause() {
    let mut app = App::headless();
    app.start_virtual();
    app.trace_paused = true;
    app.stop();
    app.start_virtual();
    assert!(!app.trace_paused, "a new start must not stay paused");
    app.update();
    app.stop();
}

#[test]
fn switching_run_mode_stops_a_running_measurement() {
    let mut app = App::headless();
    app.start_virtual();
    assert!(app.measuring);
    app.switch_run_mode(Mode::Replay);
    assert!(!app.measuring, "switching mode stops the run");
    assert!(matches!(app.run_mode, Mode::Replay));
    app.switch_run_mode(Mode::Replay);
    assert!(matches!(app.run_mode, Mode::Replay), "no-op keeps the mode");
}

#[test]
fn recent_lists_dedup_and_cap() {
    let mut app = App::headless();
    for i in 0..10 {
        app.push_recent_dbc(format!("f{i}.dbc"));
    }
    assert_eq!(app.recent_dbc.len(), 8, "recent list is capped");
    assert_eq!(app.recent_dbc[0], "f9.dbc", "newest first");
    app.push_recent_dbc("f3.dbc".to_string());
    assert_eq!(app.recent_dbc[0], "f3.dbc", "reopen moves to the front");
    assert_eq!(
        app.recent_dbc.iter().filter(|p| *p == "f3.dbc").count(),
        1,
        "no duplicates"
    );
}

#[test]
fn dropping_a_dbc_loads_it_into_the_first_bus() {
    let mut app = App::headless();
    // The default bus already carries sample.dbc; a drop **attaches** the
    // new database as an extra instead of replacing the primary.
    app.open_dropped(std::path::Path::new("assets/motbus.dbc"));
    assert_eq!(
        app.channels[0].dbc_paths,
        ["assets/sample.dbc", "assets/motbus.dbc"]
    );
    assert!(
        app.channels[0].dbc.is_some(),
        "dropped DBC is parsed into the first bus"
    );
    assert_eq!(app.recent_dbc[0], "assets/motbus.dbc");

    // Dropping the same file again never duplicates the attach.
    app.open_dropped(std::path::Path::new("assets/motbus.dbc"));
    assert_eq!(
        app.channels[0].dbc_paths.len(),
        2,
        "an already-attached path is not re-added"
    );
}

#[test]
fn jump_to_live_resets_plot_offsets() {
    let mut app = App::headless();
    app.graphics[0].t_offset_s = -42.0;
    app.jump_to_live();
    assert_eq!(app.graphics[0].t_offset_s, 0.0);
}

#[test]
fn reset_restores_the_default_workspace() {
    let mut app = App::headless();
    app.new_trace_window();
    app.push_recent_dbc("keep.dbc".to_string());
    app.start_virtual();
    app.reset_to_defaults();
    assert!(!app.measuring, "reset stops a running measurement");
    assert_eq!(app.trace_windows.len(), 1, "default has one trace window");
    assert!(app.project_path.is_none());
    assert_eq!(app.recent_dbc[0], "keep.dbc", "recents survive the reset");
}

#[test]
fn new_project_starts_completely_empty() {
    let mut app = App::headless();
    app.new_project();
    assert!(
        app.channels
            .iter()
            .all(|c| c.dbc.is_none() && c.dbc_paths.is_empty()),
        "no DBCs on any bus"
    );
    assert!(app.trace_windows.is_empty());
    assert!(app.msg_windows.is_empty());
    assert!(app.stats_windows.is_empty());
    assert!(app.graphics.is_empty());
    assert!(app.data_windows.is_empty());
    assert!(app.tx_list.is_empty());
    assert!(app.project_path.is_none());
    assert!(!app.is_dirty(), "a fresh project has nothing to save");
}

#[test]
fn untouched_workspace_quits_without_prompting() {
    let mut app = App::headless();
    app.request_quit();
    assert!(app.quit, "clean untitled workspace quits silently");
    assert!(app.pending_action.is_none());

    let mut app = App::headless();
    app.new_trace_window();
    app.request_quit();
    assert!(!app.quit, "modified workspace must confirm first");
    assert_eq!(app.pending_action, Some(crate::app::PendingAction::Quit));
}

#[test]
fn autosave_round_trips_the_workspace() {
    let mut app = App::headless();
    let path = std::env::temp_dir().join("roxy_can_autosave.rxproj");
    assert!(app.save_project(Some(path.clone())));
    app.trace_windows[0].filter = "Motor".to_string();
    app.layout_cache = "[Window][Dockspace]\n".to_string();
    app.write_autosave();

    let mut restored = App::headless();
    assert!(restored.load_autosave());
    assert_eq!(restored.project_path.as_deref(), Some(path.as_path()));
    assert_eq!(restored.trace_windows[0].filter, "Motor");
    assert_eq!(
        restored.pending_layout.as_deref(),
        Some("[Window][Dockspace]\n")
    );
    assert!(!restored.is_dirty(), "restored autosave starts clean");
    std::fs::remove_file(&path).ok();
    std::fs::remove_file(crate::config::state_path(crate::config::AUTOSAVE_PATH)).ok();
}

#[test]
fn desktop_switching_restores_window_visibility() {
    let mut app = App::headless();
    assert!(app.trace_windows[0].opened);
    assert!(app.show_network);
    app.add_desktop();
    assert!(!app.trace_windows[0].opened, "a new desktop starts empty");
    assert!(!app.show_network, "a new desktop hides all panels");
    app.switch_desktop(0);
    assert_eq!(app.active_desktop, 0);
    assert!(app.trace_windows[0].opened, "desktop 1 reopens its windows");
    assert!(app.show_network, "desktop 1 restores the panel state");
    app.switch_desktop(1);
    assert!(!app.trace_windows[0].opened, "desktop 2 keeps it closed");
    assert!(!app.show_network);
}

#[test]
fn desktops_round_trip_through_config() {
    let mut app = App::headless();
    app.add_desktop();
    app.switch_desktop(0);
    app.show_bus_stats = true;
    let cfg = Config::from_app(&app, None);
    let mut restored = App::headless();
    cfg.apply(&mut restored);
    assert_eq!(restored.desktops.len(), 2);
    assert_eq!(restored.desktops[0].name, "Desktop 1");
    assert_eq!(restored.desktops[1].name, "Desktop 2");
    assert_eq!(restored.active_desktop, 0);
    assert_eq!(
        restored.desktops[0].open_windows.len(),
        app.desktops[0].open_windows.len()
    );
    assert!(
        restored.show_bus_stats,
        "panel visibility rides the desktop like every other flag"
    );
}

#[test]
fn delete_desktop_keeps_at_least_one() {
    let mut app = App::headless();
    app.delete_desktop(0);
    assert_eq!(app.desktops.len(), 1, "the last desktop cannot be deleted");
    app.add_desktop();
    app.add_desktop();
    assert_eq!(app.active_desktop, 2);
    app.delete_desktop(2);
    assert_eq!(app.desktops.len(), 2);
    assert_eq!(app.active_desktop, 1, "deleting the active one falls back");
    app.delete_desktop(0);
    assert_eq!(app.active_desktop, 0, "indices shift when deleting below");
    assert_eq!(app.desktops.len(), 1);
}

#[test]
fn a_contended_mailbox_keeps_last_frames_snapshot() {
    let mut app = App::headless();
    assert_eq!(app.channel_name(0), "CAN1");
    app.channels[0].name = "Renamed".to_string();
    app.refresh_snapshot();
    assert_eq!(app.channel_name(0), "Renamed");

    // A newer snapshot is published while the reader is mid-read: the
    // try-read bows out (None) instead of waiting on the bus.
    app.channels[0].name = "Renamed again".to_string();
    let newer = std::sync::Arc::new(app.snapshot());
    let mut guard = app.mail.lock().unwrap();
    *guard = newer;
    assert!(
        App::peek_mailbox(&app.mail, &app.snap).is_none(),
        "a held mailbox reads as no-new-snapshot, never as a wait"
    );
    drop(guard);

    // Released, the same read picks the fresh frame up and the regular
    // path lands it in `app.snap`.
    let fresh = App::peek_mailbox(&app.mail, &app.snap)
        .expect("the release makes the newest frame visible");
    assert_eq!(fresh.channels[0].name, "Renamed again");
    app.read_snapshot();
    assert_eq!(app.channel_name(0), "Renamed again");
}

#[test]
fn a_command_status_rides_the_next_snapshot() {
    let mut app = App::headless();
    app.send(crate::bus::BusCommand::StartReplay {
        path: "no/such/log.asc".to_string(),
        speed: 1.0,
    });
    assert!(
        app.status.contains("replay failed"),
        "the drained command's text reached the status line: {:?}",
        app.status
    );
    assert!(
        app.snap.status.is_some(),
        "and it travelled inside the snapshot that carried it"
    );

    // Status is news, not state: a publish without fresh text carries
    // None and the status line keeps the old message.
    app.send(crate::bus::BusCommand::SetEntryActive {
        ch: 9,
        id: 0x999,
        on: true,
    });
    assert!(app.snap.status.is_none(), "routine publishes carry no text");
    assert!(
        app.status.contains("replay failed"),
        "an empty-handed command leaves the old status standing"
    );
}

#[test]
fn new_project_resets_to_single_desktop() {
    let mut app = App::headless();
    app.add_desktop();
    app.rename_desktop(1, "Analysis".to_string());
    app.new_project();
    assert_eq!(app.desktops.len(), 1);
    assert_eq!(app.active_desktop, 0);
    assert_eq!(app.desktops[0].name, "Desktop 1");
}

#[test]
fn project_round_trips_through_an_rxproj_file() {
    let mut app = App::headless();
    app.trace_windows[0].filter = "Motor".to_string();
    let path = std::env::temp_dir().join("roxy_can_test.rxproj");
    assert!(app.save_project(Some(path.clone())), "save writes the file");
    assert_eq!(app.project_path.as_deref(), Some(path.as_path()));

    let mut restored = App::headless();
    restored.open_project_path(&path);
    assert_eq!(restored.project_path.as_deref(), Some(path.as_path()));
    assert_eq!(restored.trace_windows[0].filter, "Motor");
    assert_eq!(restored.channels.len(), app.channels.len());
    assert_eq!(restored.tx_list.len(), app.tx_list.len());
    std::fs::remove_file(&path).ok();
}

#[test]
fn signal_stats_track_min_avg_max() {
    let mut app = App::headless();
    let key = {
        let db = app.channel_dbc(0).expect("sample DBC loaded");
        let id = db.order[0].0;
        SigKey::can(0, id, false, db.messages[&(id, false)].signals[0].name.clone())
    };
    app.subscribe(key.clone());
    for tx in &mut app.tx_list {
        tx.active = true;
        tx.cycle_us = 10_000;
    }
    open_all_gates(&mut app);
    app.start_virtual();
    for _ in 0..8 {
        std::thread::sleep(std::time::Duration::from_millis(11));
        app.update();
    }
    app.stop();
    let sub = app.subs.get(&key).expect("signal subscribed");
    assert!(
        sub.min.is_finite() && sub.max.is_finite(),
        "samples update min/max"
    );
    assert!(sub.min <= sub.avg && sub.avg <= sub.max, "avg within range");
    assert!(!sub.history.is_empty(), "history sampled");
}

#[test]
fn restored_signals_are_resubscribed() {
    let mut app = App::headless();
    let key = {
        let db = app.channel_dbc(0).expect("sample DBC loaded");
        let id = db.order[0].0;
        SigKey::can(0, id, false, db.messages[&(id, false)].signals[0].name.clone())
    };
    app.subscribe(key.clone());
    app.graphics[0].signals.push(GfxSignal {
        key: key.clone(),
        visible: true,
        y_mode: YMode::Auto,
    });
    let path = std::env::temp_dir().join("roxy_can_resub.rxproj");
    assert!(app.save_project(Some(path.clone())));
    let mut restored = App::headless();
    restored.open_project_path(&path);
    assert!(
        restored.subs.contains_key(&key),
        "restored signal is resubscribed so it is not grey"
    );
    std::fs::remove_file(&path).ok();
}

#[test]
fn recording_captures_generator_data_faithfully() {
    let mut app = App::headless();
    app.tx_list[0].active = true;
    app.tx_list[0].cycle_us = 10_000;
    // 角色开关允许发送：0x100 属 EngineECU。
    app.set_node_role(0, "EngineECU", NodeRole::Simulated);
    let mut payload = [0u8; MAX_CAN_FD_LEN];
    payload[..8].copy_from_slice(&[0x11, 0x22, 0x33, 0x44, 0x55, 0x66, 0x77, 0x88]);
    app.tx_list[0].data = payload;
    app.record_path_buf = "target/test_record".to_string();
    app.toggle_record();
    app.start_virtual();
    for _ in 0..8 {
        std::thread::sleep(std::time::Duration::from_millis(11));
        app.update();
    }
    app.stop();
    let path = app.recorder.last_record.clone();
    assert!(!path.is_empty(), "recording produced a file");
    let content = std::fs::read_to_string(&path).expect("record file readable");
    let parsed = crate::log::asc::parse_asc(&content);
    let (id, ch) = (app.tx_list[0].id, app.tx_list[0].channel);
    let hit = parsed
        .iter()
        .find(|f| f.id == id && f.channel == ch)
        .expect("recorded frames parsed back");
    assert_eq!(
        hit.payload(),
        &[0x11u8, 0x22, 0x33, 0x44, 0x55, 0x66, 0x77, 0x88][..],
        "recorded data matches what the generator sent"
    );
    std::fs::remove_file(&path).ok();
}

use crate::spec::Kind;

/// A database covering the three declarations the monitor distinguishes:
/// 100 on a declared 100 ms period, 300 declared event-triggered, and 200
/// with no cycle at all. There is deliberately no `BA_DEF_DEF_` line, so an
/// unannotated message gets no declaration rather than a default one.
const SPEC_DBC: &str = r#"VERSION "roxy-can spec test"

NS_ :

BU_: ECU

BO_ 100 Periodic: 8 ECU
 SG_ S : 0|8@1+ (1,0) [0|0] "" ECU

BO_ 200 Undeclared: 8 ECU
 SG_ S : 0|8@1+ (1,0) [0|0] "" ECU

BO_ 300 EventMsg: 8 ECU
 SG_ S : 0|8@1+ (1,0) [0|0] "" ECU

BA_DEF_ BO_  "GenMsgCycleTime" INT 0 10000;
BA_ "GenMsgCycleTime" BO_ 100 100;
BA_ "GenMsgCycleTime" BO_ 300 0;
"#;

/// A virtual bus with a silent generator, so everything the monitor sees
/// arrived through `receive`.
fn spec_app() -> App {
    let mut app = App::headless();
    app.channels[0].dbc = Some(std::sync::Arc::new(
        crate::dbc::load_dbc_str(SPEC_DBC).unwrap(),
    ));
    app.tx_list.retain(|t| t.channel != 0);
    app.start_virtual();
    app
}

fn frame_at(t_us: u64, id: u32, len: u8, dir: Direction) -> CanFrame {
    CanFrame {
        t_us,
        channel: 0,
        id,
        extended: false,
        len,
        data: [0; MAX_CAN_FD_LEN],
        dir,
        flags: FrameFlags::NONE,
    }
}

/// Runs exactly one measurement step in which `frames` arrive and the
/// simulation clock reads `t_us`. A scripted source is the only way to get
/// received traffic through the real aggregation path without writing a log
/// file, and it keeps the step exact: no wall clock, no sleeping.
fn receive(app: &mut App, t_us: u64, frames: Vec<CanFrame>) {
    struct Scripted(Option<Vec<CanFrame>>);
    impl FrameSource for Scripted {
        fn poll(&mut self, _now_us: u64, out: &mut Vec<CanFrame>) {
            if let Some(v) = self.0.take() {
                out.extend(v);
            }
        }
    }
    app.sim_t_us = t_us;
    app.source = Box::new(Scripted(Some(frames)));
    app.tick(t_us);
}

fn flagged(app: &App, ch: u8, id: u32, kind: Kind) -> bool {
    app.spec.rows.contains_key(&(ch, id, false, kind))
}

fn verdict(app: &App, ch: u8, id: u32, kind: Kind) -> crate::spec::Latch {
    app.spec.rows[&(ch, id, false, kind)]
}

#[test]
fn an_identifier_the_database_lacks_is_reported_unknown() {
    let mut app = spec_app();
    receive(&mut app, 0, vec![frame_at(0, 0x777, 8, Direction::Rx)]);
    assert!(flagged(&app, 0, 0x777, Kind::Unknown));
    receive(
        &mut app,
        10_000,
        vec![frame_at(10_000, 100, 8, Direction::Rx)],
    );
    assert!(
        !flagged(&app, 0, 100, Kind::Unknown),
        "a declared id is not a violation"
    );
    app.stop();
}

#[test]
fn a_bus_with_no_database_reports_nothing_at_all() {
    let mut app = spec_app();
    app.channels[0].dbc = None;
    app.refresh_snapshot();
    assert!(app.channel_dbc(0).is_none(), "test setup: no database");
    receive(&mut app, 0, vec![frame_at(0, 0x777, 3, Direction::Rx)]);
    receive(&mut app, 900_000, vec![]);
    assert!(app.spec.rows.is_empty(), "no database, no opinion");
    app.stop();
}

/// The frame facts are judged only for traffic we did not produce: driving a
/// signal past the base length widens a Tx frame on purpose, and the
/// generator row already offers to restore a hand-tuned period.
#[test]
fn our_own_transmission_is_never_a_cycle_or_dlc_violation() {
    let mut tx = spec_app();
    receive(&mut tx, 0, vec![frame_at(0, 100, 6, Direction::Tx)]);
    receive(
        &mut tx,
        115_000,
        vec![frame_at(115_000, 100, 6, Direction::Tx)],
    );
    assert!(!flagged(&tx, 0, 100, Kind::Dlc), "we chose that length");
    assert!(!flagged(&tx, 0, 100, Kind::Cycle));

    let mut rx = spec_app();
    receive(&mut rx, 0, vec![frame_at(0, 100, 6, Direction::Rx)]);
    receive(
        &mut rx,
        115_000,
        vec![frame_at(115_000, 100, 6, Direction::Rx)],
    );
    assert!(
        flagged(&rx, 0, 100, Kind::Dlc),
        "the same frame received is"
    );
    assert!(flagged(&rx, 0, 100, Kind::Cycle));
    rx.stop();
    tx.stop();
}

#[test]
fn a_frame_shorter_than_the_declared_size_is_a_dlc_mismatch() {
    let mut app = spec_app();
    receive(&mut app, 0, vec![frame_at(0, 100, 6, Direction::Rx)]);
    assert_eq!(
        verdict(&app, 0, 100, Kind::Dlc),
        crate::spec::Latch {
            count: 1,
            first_t_us: 0,
            last_t_us: 0,
            declared: 8.0,
            measured: 6.0,
        }
    );
    app.stop();
}

#[test]
fn timing_is_silent_where_the_database_declares_no_cycle() {
    let mut app = spec_app();
    assert!(
        app.dbc_cycle_us(0, 200).is_none(),
        "test setup: 200 declares no period"
    );
    receive(&mut app, 0, vec![frame_at(0, 200, 8, Direction::Rx)]);
    receive(
        &mut app,
        5_000_000,
        vec![frame_at(5_000_000, 200, 8, Direction::Rx)],
    );
    assert!(
        !flagged(&app, 0, 200, Kind::Cycle),
        "five seconds between two frames promises nothing was broken"
    );
    assert!(!flagged(&app, 0, 200, Kind::Missing));
    app.stop();
}

#[test]
fn an_event_triggered_message_is_never_reported_missing() {
    let mut app = spec_app();
    assert_eq!(
        app.dbc_cycle_us(0, 300),
        Some(0),
        "test setup: a declared 0 means event-triggered"
    );
    receive(&mut app, 0, vec![frame_at(0, 300, 8, Direction::Rx)]);
    receive(&mut app, 5_000_000, vec![]);
    assert!(!flagged(&app, 0, 300, Kind::Missing));
    assert!(!flagged(&app, 0, 300, Kind::Cycle));
    app.stop();
}

/// The report must not become a list of everyone we chose not to simulate:
/// a virtual bus only ever carries the nodes the user switched on.
#[test]
fn a_message_that_never_appeared_is_not_reported_missing() {
    let mut app = spec_app();
    assert_eq!(
        app.dbc_cycle_us(0, 100),
        Some(100_000),
        "test setup: 100 is declared periodic"
    );
    run_sim(&mut app, 20, 50_000);
    assert!(
        app.spec.rows.is_empty(),
        "never seen is not the same as dropped: {:?}",
        app.spec.rows.keys().collect::<Vec<_>>()
    );
    app.stop();
}

#[test]
fn a_message_that_went_silent_beyond_the_grace_is_reported_missing() {
    let mut app = spec_app();
    receive(&mut app, 0, vec![frame_at(0, 100, 8, Direction::Rx)]);
    receive(
        &mut app,
        100_000,
        vec![frame_at(100_000, 100, 8, Direction::Rx)],
    );
    receive(&mut app, 300_000, vec![]);
    assert!(
        !flagged(&app, 0, 100, Kind::Missing),
        "two silent periods is still inside a grace of three"
    );
    receive(&mut app, 420_000, vec![]);
    assert!(flagged(&app, 0, 100, Kind::Missing));
    assert_eq!(verdict(&app, 0, 100, Kind::Missing).measured, 320_000.0);
    app.stop();
}

#[test]
fn the_cycle_check_uses_the_last_interval_not_the_running_average() {
    let mut app = spec_app();
    for i in 0..10u64 {
        let t = i * 100_000;
        receive(&mut app, t, vec![frame_at(t, 100, 8, Direction::Rx)]);
    }
    assert!(!flagged(&app, 0, 100, Kind::Cycle), "ten on-time frames");
    // 115 ms is 15% late, which the aggregate's running average smooths down
    // to 1.5% and would never report.
    receive(
        &mut app,
        1_015_000,
        vec![frame_at(1_015_000, 100, 8, Direction::Rx)],
    );
    assert!(
        flagged(&app, 0, 100, Kind::Cycle),
        "agg.cycle_us reads 1.5% off here; only the raw interval is 15%"
    );
    assert_eq!(verdict(&app, 0, 100, Kind::Cycle).measured, 115_000.0);
    app.stop();
}

#[test]
fn the_tolerance_setting_decides_how_late_is_late() {
    let mut app = spec_app();
    for i in 0..10u64 {
        let t = i * 100_000;
        receive(&mut app, t, vec![frame_at(t, 100, 8, Direction::Rx)]);
    }
    app.spec_tol_pct = 20;
    receive(
        &mut app,
        1_015_000,
        vec![frame_at(1_015_000, 100, 8, Direction::Rx)],
    );
    assert!(
        !flagged(&app, 0, 100, Kind::Cycle),
        "15% late is clean at a 20% tolerance"
    );
    app.spec_tol_pct = 5;
    receive(
        &mut app,
        1_126_000,
        vec![frame_at(1_126_000, 100, 8, Direction::Rx)],
    );
    assert!(
        flagged(&app, 0, 100, Kind::Cycle),
        "the next interval is only 11% late, but the tolerance now says 5"
    );
    app.stop();
}

#[test]
fn the_grace_setting_decides_when_silence_counts_as_dropout() {
    let mut app = spec_app();
    receive(&mut app, 0, vec![frame_at(0, 100, 8, Direction::Rx)]);
    app.spec_grace = 10;
    receive(&mut app, 950_000, vec![]);
    assert!(
        !flagged(&app, 0, 100, Kind::Missing),
        "9.5 periods of silence is inside a grace of ten"
    );
    app.spec_grace = 2;
    receive(&mut app, 960_000, vec![]);
    assert!(
        flagged(&app, 0, 100, Kind::Missing),
        "tightening the grace convicts the same continuing silence"
    );
    app.stop();
}

#[test]
fn one_bad_interval_is_counted_once_and_never_forgotten() {
    let mut app = spec_app();
    receive(&mut app, 0, vec![frame_at(0, 100, 8, Direction::Rx)]);
    receive(
        &mut app,
        200_000,
        vec![frame_at(200_000, 100, 8, Direction::Rx)],
    );
    assert_eq!(verdict(&app, 0, 100, Kind::Cycle).count, 1);
    for i in 1..=5u64 {
        // Continue the declared spacing from 200 ms, so each of these is a
        // clean interval rather than another gap.
        let t = 200_000 + i * 100_000;
        receive(&mut app, t, vec![frame_at(t, 100, 8, Direction::Rx)]);
    }
    assert_eq!(
        verdict(&app, 0, 100, Kind::Cycle).count,
        1,
        "a verdict from min/max would keep convicting"
    );
    app.stop();
}

#[test]
fn the_spec_clear_forgets_the_latches_without_a_new_run() {
    let mut app = spec_app();
    receive(&mut app, 0, vec![frame_at(0, 100, 8, Direction::Rx)]);
    receive(
        &mut app,
        200_000,
        vec![frame_at(200_000, 100, 8, Direction::Rx)],
    );
    assert_eq!(verdict(&app, 0, 100, Kind::Cycle).count, 1);

    app.send(crate::bus::BusCommand::ClearSpec);
    assert!(
        app.snap.spec.rows.is_empty(),
        "clear empties the report in place"
    );
    // The interval memory went with the rows: the next arrival is a first
    // sample again, so a clean frame does not re-convict.
    receive(
        &mut app,
        300_000,
        vec![frame_at(300_000, 100, 8, Direction::Rx)],
    );
    assert!(
        app.snap.spec.rows.is_empty(),
        "a clean interval after clear stays clean"
    );
    app.stop();
}

#[test]
fn an_entry_config_can_pin_the_exact_frame_flags() {
    let mut app = App::headless();
    app.add_tx(0, 0x100);
    // A trace row added to the generator keeps the flags it was seen with,
    // even when they disagree with the fd checkbox's derived default.
    app.send(crate::bus::BusCommand::SetEntryConfig {
        ch: 0,
        id: 0x100,
        active: true,
        cycle_us: 100_000,
        fd: false,
        flags: Some(crate::can::frame::FrameFlags::FD),
        data_text: None,
        srcs: Vec::new(),
    });
    let view = app
        .snap
        .tx
        .iter()
        .find(|t| t.id == 0x100)
        .expect("the entry exists");
    assert!(view.fd, "explicit flags win over the fd-derived ones");
    app.stop();
}

#[test]
fn the_first_sample_of_a_message_is_never_a_cycle_violation() {
    let mut app = spec_app();
    // Arrives five periods late, so the only thing standing between this and
    // a verdict is that nothing preceded it.
    receive(
        &mut app,
        500_000,
        vec![frame_at(500_000, 100, 8, Direction::Rx)],
    );
    assert_eq!(
        app.aggs[&(0, 100, false)].count,
        1,
        "test setup: one frame, no interval yet"
    );
    assert!(!flagged(&app, 0, 100, Kind::Cycle));
    app.stop();
}

#[test]
fn a_step_that_brought_no_new_frame_is_not_a_new_interval() {
    let mut app = spec_app();
    receive(&mut app, 0, vec![frame_at(0, 100, 8, Direction::Rx)]);
    receive(
        &mut app,
        100_000,
        vec![frame_at(100_000, 100, 8, Direction::Rx)],
    );
    assert!(!flagged(&app, 0, 100, Kind::Cycle), "test setup: on time");
    // Nothing arrives for two steps. The aggregate has not moved, so there
    // is no period to measure -- only the silence clock runs here.
    receive(&mut app, 200_000, vec![]);
    receive(&mut app, 300_000, vec![]);
    assert!(!flagged(&app, 0, 100, Kind::Cycle));
    app.stop();
}

#[test]
fn replay_traffic_never_raises_a_missing_violation() {
    let mut app = spec_app();
    app.mode = Mode::Replay;
    receive(&mut app, 0, vec![frame_at(0, 100, 8, Direction::Rx)]);
    receive(&mut app, 5_000_000, vec![]);
    assert!(
        !flagged(&app, 0, 100, Kind::Missing),
        "a log's clock cannot say what is still talking"
    );
    // The frame facts stay judged in replay, because they need no clock.
    receive(
        &mut app,
        5_100_000,
        vec![frame_at(5_100_000, 100, 5, Direction::Rx)],
    );
    assert!(flagged(&app, 0, 100, Kind::Dlc));
    app.stop();
}

#[test]
fn a_paused_clock_does_not_make_a_message_missing() {
    let mut app = spec_app();
    receive(&mut app, 0, vec![frame_at(0, 100, 8, Direction::Rx)]);
    app.trace_paused = true;
    assert!(app.trace_paused, "test setup: paused");
    // The real loop never calls `tick` while paused; driving it anyway
    // shows the verdict is gated on the pause itself, not on a missing step.
    for i in 1..=10u64 {
        receive(&mut app, i * 200_000, vec![]);
    }
    assert!(!flagged(&app, 0, 100, Kind::Missing));
    app.stop();
}

#[test]
fn a_verdict_follows_its_bus_when_an_earlier_bus_is_deleted() {
    let mut app = spec_app();
    // Move the fixture database onto the second bus and leave the first
    // without one, then delete the first.
    app.channels[1].dbc = app.channels[0].dbc.take();
    receive(
        &mut app,
        0,
        vec![CanFrame {
            channel: 1,
            ..frame_at(0, 100, 6, Direction::Rx)
        }],
    );
    assert!(flagged(&app, 1, 100, Kind::Dlc), "test setup: on bus 1");
    app.remove_channel(0);
    assert!(
        flagged(&app, 0, 100, Kind::Dlc),
        "the row must move with the bus, not stay behind at the old index"
    );
    app.stop();
}

#[test]
fn the_monitor_forgets_everything_when_a_new_run_starts() {
    let mut app = spec_app();
    receive(&mut app, 0, vec![frame_at(0, 100, 8, Direction::Rx)]);
    receive(
        &mut app,
        200_000,
        vec![frame_at(200_000, 100, 8, Direction::Rx)],
    );
    assert!(
        flagged(&app, 0, 100, Kind::Cycle),
        "test setup: a verdict worth forgetting"
    );
    app.start_virtual();
    assert!(app.spec.rows.is_empty(), "the report belongs to a run");
    assert_eq!(
        app.spec.previous((0, 100, false)),
        None,
        "and so does the interval memory"
    );
    // Five seconds later on a clock that started over. Against the previous
    // run's interval this would read as one enormous measured period.
    receive(
        &mut app,
        5_000_000,
        vec![frame_at(5_000_000, 100, 8, Direction::Rx)],
    );
    assert!(
        !flagged(&app, 0, 100, Kind::Cycle),
        "a stale interval from the old run would convict this frame"
    );
    app.stop();
}

/// Same mux layout as `MUX_DBC` in the dbc tests, one bus, generator
/// silenced so only the frames built here reach the sampler.
const MUX_SAMPLE_DBC: &str = r#"VERSION "roxy-can mux sampling test"

NS_ :

BU_: ECU

BO_ 400 Muxed: 8 ECU
 SG_ Switch M : 0|8@1+ (1,0) [0|0] "" ECU
 SG_ G1_A m1 : 16|16@1+ (0.1,0) [0|0] "" ECU
 SG_ G2_C m2 : 16|16@1+ (0.5,0) [0|0] "" ECU
"#;

fn mux_app() -> App {
    let mut app = App::headless();
    app.channels[0].dbc = Some(std::sync::Arc::new(
        crate::dbc::load_dbc_str(MUX_SAMPLE_DBC).unwrap(),
    ));
    app.tx_list.retain(|t| t.channel != 0);
    app.start_virtual();
    app
}

fn mux_frame(t_us: u64, switch: u8) -> CanFrame {
    let mut f = frame_at(t_us, 400, 8, Direction::Rx);
    f.data[0] = switch;
    f.data[2] = 100;
    f
}

/// A standard and an extended message sharing one numeric id are two
/// different signals: each subscription samples only its own class's
/// frames, even though the id and the channel are identical.
#[test]
fn twin_class_subscriptions_never_cross_feed() {
    const TWIN_DBC: &str = r#"VERSION "roxy-can twin sub test"

NS_ :

BS_:

BU_: ECU

BO_ 256 Twin: 2 ECU
 SG_ StdSig : 0|16@1+ (0.1,0) [0|0] ""  ECU

BO_ 2147483904 Twin: 2 ECU
 SG_ ExtSig : 0|16@1+ (0.1,0) [0|0] ""  ECU
"#;
    let mut app = App::headless();
    app.channels[0].dbc = Some(std::sync::Arc::new(
        crate::dbc::load_dbc_str(TWIN_DBC).unwrap(),
    ));
    app.tx_list.retain(|t| t.channel != 0);
    app.start_virtual();
    let std_key = SigKey::can(0, 0x100, false, "StdSig");
    let ext_key = SigKey::can(0, 0x100, true, "ExtSig");
    app.subscribe(std_key.clone());
    app.subscribe(ext_key.clone());

    // A standard frame: only the standard subscription sees it...
    receive(
        &mut app,
        10_000,
        vec![frame_at(10_000, 0x100, 2, Direction::Rx)],
    );
    assert!(
        !app.subs[&std_key].history.is_empty(),
        "standard class sampled"
    );
    assert!(
        app.subs[&ext_key].history.is_empty(),
        "extended class got nothing from a standard frame"
    );

    // ...an extended frame: only the extended subscription sees it.
    let mut ext = frame_at(20_000, 0x100, 2, Direction::Rx);
    ext.extended = true;
    ext.data[0] = 200;
    receive(&mut app, 20_000, vec![ext]);
    assert_eq!(
        app.subs[&std_key].latest, 0.0,
        "standard class never saw the extended frame"
    );
    assert!(
        app.subs[&ext_key].latest > 0.0,
        "extended class sampled the extended frame"
    );
    app.stop();
}

#[test]
fn a_signal_of_the_inactive_group_is_not_sampled() {
    let mut app = mux_app();
    let g1 = SigKey::can(0, 400, false, "G1_A");
    let g2 = SigKey::can(0, 400, false, "G2_C");
    app.subscribe(g1.clone());
    app.subscribe(g2.clone());
    assert!(app.subs.contains_key(&g1), "both signals are subscribed");
    assert!(app.subs.contains_key(&g2));

    receive(&mut app, 100_000, vec![mux_frame(100_000, 1)]);
    assert!(
        app.subs[&g2].history.is_empty(),
        "group 2 was not in the frame, so it has no samples"
    );
    assert!(
        !app.subs[&g1].history.is_empty(),
        "group 1 was active and got sampled"
    );
    app.stop();
}

#[test]
fn a_group_signal_gains_samples_once_its_group_is_switched_in() {
    let mut app = mux_app();
    let g2 = SigKey::can(0, 400, false, "G2_C");
    app.subscribe(g2.clone());

    receive(&mut app, 100_000, vec![mux_frame(100_000, 1)]);
    let before = app.subs[&g2].history.len();
    receive(&mut app, 200_000, vec![mux_frame(200_000, 2)]);
    assert_eq!(before, 0, "inactive until the switch changes");
    assert!(
        app.subs[&g2].history.len() > before,
        "once its group is switched in, the signal is sampled again"
    );
    app.stop();
}

#[test]
fn an_insert_marker_trigger_stamps_the_bus_clock() {
    let mut app = quiet_app();
    app.triggers.push(Trigger::new(
        TriggerCond::IdPresent { ch: 0, id: 0x777 },
        TriggerAction::InsertMarker,
    ));
    receive(
        &mut app,
        10_000,
        vec![rx_frame(10_000, 0x100, 8, FrameFlags::NONE)],
    );
    assert!(app.snap.markers.is_empty(), "an unwatched id marks nothing");
    receive(
        &mut app,
        20_000,
        vec![rx_frame(20_000, 0x777, 8, FrameFlags::NONE)],
    );
    assert_eq!(app.snap.markers, [20_000], "the edge stamps the bus clock");
    // A new measurement forgets the markers with the rest of the run.
    app.reset_run();
    app.refresh_snapshot();
    assert!(app.snap.markers.is_empty(), "markers are run-scored state");
}

/// The pre-trigger half of TODO item 8: a trigger-started recording opens
/// with the frames that came before the event, oldest first, and the
/// triggering frame itself lands right after them.
#[test]
fn a_trigger_started_recording_includes_the_pre_buffer() {
    let mut app = quiet_app();
    let base = std::env::temp_dir().join("roxy_can_pre_buffer.asc");
    app.send(crate::bus::BusCommand::SetRecordPath(
        base.to_string_lossy().into_owned(),
    ));
    app.triggers.push(Trigger::new(
        TriggerCond::IdPresent { ch: 0, id: 0x777 },
        TriggerAction::StartRecording,
    ));
    app.recorder.recording = false;

    receive(
        &mut app,
        10_000,
        vec![
            rx_frame(10_000, 0x100, 8, FrameFlags::NONE),
            rx_frame(20_000, 0x200, 8, FrameFlags::NONE),
        ],
    );
    assert!(
        !app.recorder.recording,
        "no watched id yet: the recorder stayed closed"
    );
    receive(
        &mut app,
        30_000,
        vec![rx_frame(30_000, 0x777, 8, FrameFlags::NONE)],
    );
    assert!(app.recorder.recording, "the watched id opened the file");
    app.recorder.close();

    let content = std::fs::read_to_string(&app.recorder.last_record).unwrap();
    let frames = crate::log::asc::parse_asc(&content);
    let ids: Vec<u32> = frames.iter().map(|f| f.id).collect();
    assert_eq!(
        ids,
        [0x100, 0x200, 0x777],
        "pre-context oldest first, then the trigger frame"
    );
    std::fs::remove_file(&app.recorder.last_record).ok();
}

/// The post-trigger roll: a trigger stop keeps recording for a bounded
/// number of frames, so the aftermath of the event stays in the file.
#[test]
fn a_stop_trigger_rolls_on_for_the_post_frames() {
    let mut app = quiet_app();
    let base = std::env::temp_dir().join("roxy_can_post_roll.asc");
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
        vec![rx_frame(10_000, 0x777, 8, FrameFlags::NONE)],
    );
    assert!(
        app.recorder.recording,
        "the stop rolls on instead of closing"
    );

    // Far more frames than the post-roll can hold.
    let fillers: Vec<CanFrame> = (0..60)
        .map(|i| rx_frame(20_000 + i as u64 * 1_000, 0x100, 8, FrameFlags::NONE))
        .collect();
    receive(&mut app, 100_000, fillers);
    assert!(!app.recorder.recording, "the post-roll expired");
    app.recorder.close();

    let text = std::fs::read_to_string(&app.recorder.last_record).unwrap();
    let frames = crate::log::asc::parse_asc(&text);
    assert_eq!(
        frames.len(),
        32, // POST_ROLL_FRAMES: the edge frame plus the roll
        "the file holds exactly the post-roll window"
    );
    assert_eq!(frames[0].id, 0x777, "the event frame is first");
    assert_eq!(frames[0].t_us, 10_000);
}

/// The re-arm half of the appearance-watch story: the ClearTrace action
/// resets the firing condition's own latch, and the manual command
/// resets latches without touching real-level conditions.
#[test]
fn clear_trace_and_rearm_reset_latched_conditions() {
    let mut app = quiet_app();
    app.triggers.push(Trigger::new(
        TriggerCond::IdPresent { ch: 0, id: 0x777 },
        TriggerAction::ClearTrace,
    ));
    receive(
        &mut app,
        10_000,
        vec![rx_frame(10_000, 0x777, 8, FrameFlags::NONE)],
    );
    assert_eq!(app.triggers[0].fired, 1);
    // The action re-armed its own condition: the next occurrence fires
    // again instead of being swallowed by the latch.
    receive(
        &mut app,
        20_000,
        vec![rx_frame(20_000, 0x777, 8, FrameFlags::NONE)],
    );
    assert_eq!(app.triggers[0].fired, 2, "re-armed by its own clear");
}

#[test]
fn the_rearm_command_resets_latches_but_not_real_levels() {
    let mut app = quiet_app();
    // Both actions start recordings: give them temp paths so nothing
    // lands in the repo root.
    let base = std::env::temp_dir().join("roxy_can_rearm_trigger.asc");
    app.send(crate::bus::BusCommand::SetRecordPath(
        base.to_string_lossy().into_owned(),
    ));
    app.triggers.push(Trigger::new(
        TriggerCond::IdPresent { ch: 0, id: 0x777 },
        TriggerAction::StartRecording,
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
    receive(
        &mut app,
        10_000,
        vec![
            rx_frame(10_000, 0x777, 8, FrameFlags::NONE),
            rpm_frame(10_000, 4000.0),
        ],
    );
    assert_eq!(app.triggers[0].fired, 1);
    assert_eq!(app.triggers[1].fired, 1);
    assert!(app.triggers[1].level, "the crossing level is a real level");

    app.send(crate::bus::BusCommand::RearmTriggers);

    // Still above threshold: the crossing must NOT re-fire, while the
    // re-armed presence watch does.
    receive(
        &mut app,
        20_000,
        vec![
            rpm_frame(20_000, 4000.0),
            rx_frame(20_000, 0x777, 8, FrameFlags::NONE),
        ],
    );
    assert_eq!(
        app.triggers[0].fired, 2,
        "presence fires again after re-arm"
    );
    assert_eq!(
        app.triggers[1].fired, 1,
        "a real level is not disturbed by the re-arm"
    );
    assert!(app.triggers[1].level);
}

/// "Switch All Blocks to Simulation" at the bus level: simulate-all 补建
/// 全部条目并把角色开关全部打开（条目保持默认关——发送由用户逐条或经
/// All On 打开）；stop-all 关闭全部开关。
#[test]
fn simulate_all_activates_every_dbc_node() {
    let mut app = App::headless();
    app.simulate_all_nodes(0);
    let db = app.channel_dbc(0).expect("sample DBC loaded");
    let expected: Vec<u32> = db
        .order
        .iter()
        .filter(|&&(_, ext)| !ext)
        .map(|&(id, _)| id)
        .collect();
    for id in &expected {
        let entry = app.tx_list.iter().find(|t| t.channel == 0 && t.id == *id);
        assert!(
            entry.is_some(),
            "node message 0x{id:X} should have an entry"
        );
    }
    // 全部开关开放：逐条目打开后按条目开关发送。
    let bus_entries: Vec<(u8, u32)> = app
        .tx_list
        .iter()
        .filter(|t| t.channel == 0)
        .map(|t| (t.channel, t.id))
        .collect();
    for (ch, id) in &bus_entries {
        app.send(crate::bus::BusCommand::SetEntryActive {
            ch: *ch,
            id: *id,
            on: true,
        });
    }
    let flowing = app
        .tx_list
        .iter()
        .filter(|t| t.channel == 0 && t.active)
        .count();
    assert!(flowing > 0, "entries exist for the whole bus");

    // The reverse sweep closes every gate on the bus (switches preserved).
    app.stop_all_nodes(0);
    app.settle();
    let any_gate = app.snap.tx.iter().any(|t| t.channel == 0 && t.gate_open);
    assert!(!any_gate, "stop-all closes every gate on the bus");
}

/// The State Tracker CSV export mirrors what the bands draw: one row per
/// state segment per visible signal over the window's live span.
#[test]
fn the_state_csv_writes_one_row_per_state_segment() {
    let mut app = App::headless();
    app.tx_list.retain(|t| t.channel != 0);
    app.new_state_window();
    let key = SigKey::can(0, 0x100, false, "EngineSpeed");
    app.subscribe(key.clone());
    app.state_trackers[0]
        .signals
        .push(crate::observe::GfxSignal {
            key: key.clone(),
            visible: true,
            y_mode: YMode::Auto,
        });
    app.start_virtual();
    // 1000 rpm then 8000 rpm: two clearly distinct states.
    receive(&mut app, 10_000, vec![rpm_frame(10_000, 1000.0)]);
    // 70 ms later: beyond the subscription stride, so the second state is
    // actually sampled into the history.
    receive(&mut app, 80_000, vec![rpm_frame(80_000, 8000.0)]);
    // Push the clock past the last frame so the 8000 band has width at
    // export time.
    receive(&mut app, 100_000, vec![]);
    let path = std::env::temp_dir().join("roxy_can_state_export.csv");
    app.export_state_csv(0, &path.to_string_lossy());
    let content = std::fs::read_to_string(&path).unwrap();
    let rows: Vec<&str> = content.lines().skip(1).collect();
    assert!(
        rows.len() >= 2,
        "two rpm states must appear as at least two rows: {content}"
    );
    assert!(content.contains("1000"), "{content}");
    assert!(content.contains("8000"), "{content}");
    std::fs::remove_file(&path).ok();
    app.stop();
}

fn rx_frame(t_us: u64, id: u32, len: u8, flags: FrameFlags) -> CanFrame {
    CanFrame {
        t_us,
        channel: 0,
        id,
        extended: false,
        len,
        data: [0u8; MAX_CAN_FD_LEN],
        dir: Direction::Rx,
        flags,
    }
}

fn ext_rx_frame(t_us: u64, id: u32, len: u8) -> CanFrame {
    CanFrame {
        t_us,
        channel: 0,
        id,
        extended: true,
        len,
        data: [0u8; MAX_CAN_FD_LEN],
        dir: Direction::Rx,
        flags: FrameFlags::NONE,
    }
}

/// One bus carrying **two** DBC files: both decode live traffic, and on
/// a duplicate message id the earlier database wins. The attach list
/// round-trips through the project file.
#[test]
fn one_bus_attaches_several_databases() {
    const DB_A: &str = r#"VERSION "multi A"

NS_ :

BS_:

BU_: ECU

BO_ 256 PrimaryMsg: 2 ECU
 SG_ PSig : 0|16@1+ (0.1,0) [0|0] ""  ECU
"#;
    const DB_B: &str = r#"VERSION "multi B"

NS_ :

BS_:

BU_: ECU

BO_ 256 SecondaryMsg: 3 ECU
 SG_ SSig : 0|24@1+ (1,0) [0|0] ""  ECU

BO_ 257 OnlyInB: 1 ECU
 SG_ BOnly : 0|8@1+ (1,0) [0|0] ""  ECU
"#;
    let a = std::env::temp_dir().join("roxy_can_multi_a.dbc");
    let b = std::env::temp_dir().join("roxy_can_multi_b.dbc");
    std::fs::write(&a, DB_A).unwrap();
    std::fs::write(&b, DB_B).unwrap();

    let mut app = App::headless();
    app.send(crate::bus::BusCommand::LoadDbc {
        ch: 0,
        paths: vec![
            a.to_string_lossy().into_owned(),
            b.to_string_lossy().into_owned(),
        ],
    });
    app.settle();
    let db = app.channel_dbc(0).expect("both databases merged");
    // Three declarations collapse to two keys: the duplicated 256 is one
    // message (first database wins), 257 is the second file's own.
    assert_eq!(db.messages.len(), 2);
    // Duplicate numeric id: the earlier database wins.
    assert_eq!(db.message_name_of((0x100, false)), Some("PrimaryMsg"));
    assert_eq!(db.message_name_of((0x101, false)), Some("OnlyInB"));

    // Both databases decode live traffic on the same bus.
    let psig = SigKey::can(0, 0x100, false, "PSig");
    let bonly = SigKey::can(0, 0x101, false, "BOnly");
    app.subscribe(psig.clone());
    app.subscribe(bonly.clone());
    app.start_virtual();
    receive(
        &mut app,
        10_000,
        vec![
            frame_at(10_000, 0x100, 2, Direction::Rx),
            frame_at(10_000, 0x101, 1, Direction::Rx),
        ],
    );
    assert!(
        !app.subs[&psig].history.is_empty(),
        "the primary database decodes 0x100"
    );
    assert!(
        !app.subs[&bonly].history.is_empty(),
        "the second database decodes 0x101 on the same bus"
    );

    // The attach list survives a project round trip.
    let project = std::env::temp_dir().join("roxy_can_multi.rxproj");
    assert!(app.save_project(Some(project.clone())), "save writes");
    let mut restored = App::headless();
    restored.open_project_path(&project);
    let restored_db = restored.channel_dbc(0).unwrap();
    assert_eq!(restored_db.messages.len(), 2, "both databases re-attach");
    assert_eq!(restored_db.message_name_of((0x101, false)), Some("OnlyInB"));
    std::fs::remove_file(&project).ok();
    app.stop();
}

/// External DBC edits are detected by checksum and reload automatically:
/// the merged table picks up the new message without a manual reload.
#[test]
fn dbc_file_changes_are_detected_and_reloaded() {
    let a = std::env::temp_dir().join("roxy_can_autoreload_a.dbc");
    std::fs::write(
        &a,
        r#"VERSION "auto a"

NS_ :

BS_:

BU_: ECU

BO_ 300 FirstMsg: 1 ECU
 SG_ A1 : 0|8@1+ (1,0) [0|0] ""  ECU
"#,
    )
    .unwrap();

    let mut app = App::headless();
    app.send(crate::bus::BusCommand::LoadDbc {
        ch: 0,
        paths: vec![a.to_string_lossy().into_owned()],
    });
    app.settle();
    let db = app.channel_dbc(0).expect("loaded");
    assert!(
        db.message_name_of((300, false)) == Some("FirstMsg"),
        "BO_ 300 is decimal id 300"
    );

    // External edit: rename the message in the file.
    std::fs::write(
        &a,
        r#"VERSION "auto a"

NS_ :

BS_:

BU_: ECU

BO_ 300 RenamedMsg: 1 ECU
 SG_ A1 : 0|8@1+ (1,0) [0|0] ""  ECU
"#,
    )
    .unwrap();
    assert!(app.maybe_reload_changed_dbcs().is_some());
    app.refresh_snapshot();
    let db = app.channel_dbc(0).expect("still loaded");
    assert_eq!(
        db.message_name_of((300, false)),
        Some("RenamedMsg"),
        "the reload picked up the external edit"
    );
    std::fs::remove_file(&a).ok();
}

/// A DBC reload that moves a message id refreshes the bound nodes'
/// `$` name→id maps in place (with a `[reload]` line in the node log):
/// a wildcard node keeps reading the renamed message off its new id.
#[test]
fn a_dollar_read_follows_a_reload_that_moves_the_message_id() {
    let a = std::env::temp_dir().join("roxy_can_reload_named.dbc");
    let dbc = |id: u32| {
        format!(
            "VERSION \"named\"\n\nNS_ :\n\nBS_:\n\nBU_: EngineECU\n\nBO_ {id} EngineStatus: 8 EngineECU\n SG_ RPM : 0|16@1+ (1,0) [0|8000] \"rpm\" EngineECU\n"
        )
    };
    std::fs::write(&a, dbc(0x100)).unwrap();
    let mut app = App::headless();
    app.open_dbc_for(0, a.to_string_lossy().into_owned());
    app.settle();
    app.send(crate::bus::BusCommand::AddNode {
        name: "watcher".to_string(),
        channel: 0,
        attached: Some((0, "EngineECU".to_string())),
    });
    app.settle();
    let id = app.snap.nodes[0].id;
    app.send(crate::bus::BusCommand::SetNodeSource {
        id,
        source: "on message * { print($EngineStatus::RPM); }".to_string(),
    });
    app.send(crate::bus::BusCommand::SetNodeEnabled { id, on: true });
    app.start_virtual();
    app.settle();

    let frame = |t: u64, fid: u32, rpm: u16| CanFrame {
        t_us: t,
        channel: 0,
        id: fid,
        extended: false,
        len: 2,
        data: {
            let mut d = [0; MAX_CAN_FD_LEN];
            d[..2].copy_from_slice(&rpm.to_le_bytes());
            d
        },
        dir: Direction::Rx,
        flags: FrameFlags::NONE,
    };
    receive(&mut app, 1_000, vec![frame(1_000, 0x100, 1234)]);
    assert!(
        app.snap.nodes[0].log.iter().any(|l| l.starts_with("1234")),
        "the pre-reload read works: {:?}",
        app.snap.nodes[0].log
    );

    std::fs::write(&a, dbc(0x103)).unwrap();
    assert!(app.maybe_reload_changed_dbcs().is_some());
    app.settle();
    receive(&mut app, 2_000, vec![frame(2_000, 0x103, 4321)]);
    let log = &app.snap.nodes[0].log;
    assert!(
        log.iter().any(|l| l.starts_with("4321")),
        "the refreshed map reads the new id: {log:?}"
    );
    assert!(
        log.iter()
            .any(|l| l.contains("[reload]") && l.contains("0x100") && l.contains("0x103")),
        "the reload is explained in the node log: {log:?}"
    );
    std::fs::remove_file(&a).ok();
    app.stop();
}

/// A simulated node whose database gains a message gets the new entry at
/// reload time -- but **inactive**: an external DBC edit must not begin
/// transmitting on its own.
#[test]
fn a_reload_seeds_new_entries_for_simulated_nodes_inactive() {
    let a = std::env::temp_dir().join("roxy_can_role_seed.dbc");
    std::fs::write(
        &a,
        r#"VERSION "role seed"

NS_ :

BS_:

BU_: ECU

BO_ 300 FirstMsg: 1 ECU
 SG_ A1 : 0|8@1+ (1,0) [0|0] ""  ECU
"#,
    )
    .unwrap();

    let mut app = App::headless();
    app.send(crate::bus::BusCommand::LoadDbc {
        ch: 0,
        paths: vec![a.to_string_lossy().into_owned()],
    });
    app.settle();
    app.set_node_role(0, "ECU", NodeRole::Simulated);
    // 新语义：模拟只开放开关，条目默认关——用户显式启用后发送。
    app.send(crate::bus::BusCommand::SetEntryActive {
        ch: 0,
        id: 300,
        on: true,
    });
    app.settle();
    assert_eq!(
        active_ids(&app, 0),
        [300],
        "the enabled entry transmits under the open gate"
    );

    // External edit: the node gains a second message.
    std::fs::write(
        &a,
        r#"VERSION "role seed"

NS_ :

BS_:

BU_: ECU

BO_ 300 FirstMsg: 1 ECU
 SG_ A1 : 0|8@1+ (1,0) [0|0] ""  ECU

BO_ 301 SecondMsg: 1 ECU
 SG_ A2 : 0|8@1+ (1,0) [0|0] ""  ECU
"#,
    )
    .unwrap();
    assert!(app.maybe_reload_changed_dbcs().is_some());
    app.refresh_snapshot();

    let fresh = app
        .tx_list
        .iter()
        .find(|t| t.channel == 0 && t.id == 301)
        .expect("the new message became a generator entry");
    assert!(
        !fresh.active,
        "an external edit must not start traffic by itself"
    );
    assert_eq!(
        active_ids(&app, 0),
        [300],
        "the previously enabled message keeps transmitting"
    );
    std::fs::remove_file(&a).ok();
}

/// A standard and an extended frame sharing one numeric id are two
/// messages as far as every consumer is concerned: two aggregates, two
/// spec watch keys, no verdicts bleeding across the class boundary.
#[test]
fn a_shared_numeric_id_aggregates_as_two_messages() {
    let mut app = App::headless();
    let db = app.channel_dbc(0).expect("sample DBC loaded");
    // Standard 0x100 exists in the sample database; the extended twin
    // deliberately does not, so only the extended class reads Unknown.
    let std_id = db.order.iter().find(|&&(_, ext)| !ext).map(|&(id, _)| id);
    let Some(std_id) = std_id else {
        panic!("sample database has a standard message");
    };
    receive(
        &mut app,
        1_000,
        vec![
            rx_frame(1_000, std_id, 8, FrameFlags::NONE),
            ext_rx_frame(1_000, std_id, 8),
        ],
    );
    assert!(
        app.aggs.contains_key(&(0, std_id, false)),
        "the standard class aggregates"
    );
    assert!(
        app.aggs.contains_key(&(0, std_id, true)),
        "the extended class aggregates separately"
    );
    // The standard id is declared, so no verdict; the extended one is
    // unknown to the database and gets exactly one Unknown row.
    assert!(
        !app.spec
            .rows
            .contains_key(&(0, std_id, false, crate::spec::Kind::Unknown)),
        "the declared standard message stays clean"
    );
    assert!(
        app.spec
            .rows
            .contains_key(&(0, std_id, true, crate::spec::Kind::Unknown)),
        "the undeclared extended twin is judged on its own"
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
    app.channels[0].fd_data_kbps = 500;
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
        app.aggs.contains_key(&(0, 0x100, false)),
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

/// The "+ Signal" button must land on something real: the database's first
/// message and signal, not a blind id the editor cannot decode.
#[test]
fn adding_a_signal_trigger_picks_the_first_database_signal() {
    let mut app = quiet_app();
    app.add_signal_trigger();
    assert_eq!(app.triggers.len(), 1);
    let draft = app
        .trig_draft
        .as_ref()
        .expect("the new row is up for editing");
    assert_eq!(draft.index, 0);
    match &app.triggers[0].cond {
        TriggerCond::SignalCross {
            ch,
            id,
            signal,
            rising,
            ..
        } => {
            assert_eq!(*ch, 0);
            assert_eq!(*id, 0x100, "sample.dbc's first message");
            assert_eq!(signal, "EngineSpeed", "its first signal");
            assert!(*rising);
        }
        other => panic!("expected a signal trigger, got {other:?}"),
    }

    app.add_error_trigger();
    assert_eq!(
        app.trig_draft.as_ref().map(|d| d.index),
        Some(1),
        "the newest row is up for editing"
    );
    if let Some(d) = &mut app.trig_draft {
        d.index = 0;
    }
    app.remove_trigger(1);
    assert_eq!(
        app.trig_draft.as_ref().map(|d| d.index),
        Some(0),
        "deleting another row keeps the editor"
    );
    app.remove_trigger(0);
    assert_eq!(app.trig_draft, None, "nothing left to edit");
    assert_eq!(
        app.trigger_summary(0),
        "",
        "a summary for a missing row is empty, not a panic"
    );
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
    let action = app.triggers[0].action;
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
    let action = app.triggers[0].action;
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

/// The CANoe Graphics behaviour: zooming into a small window must reveal
/// every signal update. A 100 Hz signal watched through a 0.1 s window
/// samples every frame at the tightened stride, where the old fixed 50 ms
/// stride kept only two points per window.
#[test]
fn a_small_graphics_window_pulls_the_sample_stride_down() {
    let mut app = quiet_app();
    let key = SigKey::can(0, 0x100, false, "EngineSpeed");
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

#[test]
fn triggers_round_trip_through_a_project() {
    let mut app = App::headless();
    app.add_signal_trigger();
    if let TriggerCond::SignalCross {
        threshold, rising, ..
    } = &mut app.triggers[0].cond
    {
        *threshold = 3000.0;
        *rising = false;
    }
    app.add_id_trigger();
    app.triggers[1].action = TriggerAction::StopRecording;
    app.triggers[1].enabled = false;
    app.add_error_trigger();
    app.triggers[2].cond = TriggerCond::CycleTimeout { ch: 1, id: 0x200 };
    app.triggers[2].action = TriggerAction::Send { ch: 1, id: 0x300 };

    let path = std::env::temp_dir().join("roxy_can_trig_roundtrip.rxproj");
    app.refresh_snapshot();
    assert!(app.save_project(Some(path.clone())), "save writes the file");

    let mut restored = App::headless();
    restored.open_project_path(&path);
    assert_eq!(restored.triggers.len(), 3, "all three shapes come back");
    match &restored.triggers[0].cond {
        TriggerCond::SignalCross {
            ch,
            id,
            ext,
            signal,
            threshold,
            rising,
        } => {
            assert_eq!(*ch, 0);
            assert_eq!(*id, 0x100);
            assert!(!*ext);
            assert_eq!(signal, "EngineSpeed");
            assert_eq!(*threshold, 3000.0);
            assert!(!*rising);
        }
        other => panic!("expected a signal trigger, got {other:?}"),
    }
    assert!(matches!(
        restored.triggers[1].action,
        TriggerAction::StopRecording
    ));
    assert!(!restored.triggers[1].enabled, "the disabled flag survives");
    assert!(matches!(
        &restored.triggers[2].cond,
        TriggerCond::CycleTimeout { ch: 1, id: 0x200 }
    ));
    assert_eq!(
        restored.triggers[2].action,
        TriggerAction::Send { ch: 1, id: 0x300 },
        "the send target survives as data, not an index"
    );
    assert_eq!(restored.trig_draft, None, "runtime editor state does not");
    std::fs::remove_file(&path).ok();
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
        app.aggs.contains_key(&(0, 0x777, false)),
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
    app.channels[0].dbc = Some(std::sync::Arc::new(
        crate::dbc::load_dbc_str(MIRROR_DBC).unwrap(),
    ));
    app.tx_list.retain(|t| t.channel != 0);
    app.start_virtual();
    // The reaction target, inactive so only the reaction itself emits it;
    // its base payload is all zeroes.
    app.add_tx(0, 0x200);
    let key = SigKey::can(0, 0x200, false, "EngineSpeed");
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
        SigKey::can(0, id, false, db.messages[&(id, false)].signals[0].name.clone())
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
    let key = SigKey::can(0, 0x100, false, "EngineSpeed");
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
fn lock_freezes_the_value_axis_until_the_mode_is_re_entered() {
    let (mut app, key) = gfx_app(YMode::Auto);
    feed_rpm(&mut app, &[(10_000, 0.0), (200_000, 100.0)]);
    assert_eq!(
        crate::ui::graphics::resolve_y_range(&mut app, 0, std::slice::from_ref(&key), 0, 1_000_000),
        (0.0, 100.0),
        "auto fits what the run has shown"
    );

    // Picking Lock captures the range on screen; later, taller values may
    // not move it.
    app.graphics[0].signals[0].y_mode = YMode::Lock;
    assert_eq!(
        crate::ui::graphics::resolve_y_range(&mut app, 0, std::slice::from_ref(&key), 0, 1_000_000),
        (0.0, 100.0),
        "the capture takes the current view"
    );
    feed_rpm(&mut app, &[(300_000, 500.0)]);
    assert_eq!(
        crate::ui::graphics::resolve_y_range(&mut app, 0, std::slice::from_ref(&key), 0, 1_000_000),
        (0.0, 100.0),
        "locked means locked: 500 rpm must not widen the axis"
    );

    // Leaving the mode drops the frozen range -- the list menu does this --
    // so re-entering re-captures afresh.
    app.graphics[0].signals[0].y_mode = YMode::Auto;
    app.graphics[0].y_locks.remove(&format!("{key:?}"));
    feed_rpm(&mut app, &[(400_000, 900.0)]);
    assert_eq!(
        crate::ui::graphics::resolve_y_range(&mut app, 0, std::slice::from_ref(&key), 0, 1_000_000),
        (0.0, 900.0),
        "auto follows again"
    );
    app.stop();
}

#[test]
fn fit_all_only_ever_widens_the_value_axis() {
    let (mut app, key) = gfx_app(YMode::FitAll);
    feed_rpm(&mut app, &[(10_000, 0.0), (200_000, 100.0)]);
    assert_eq!(
        crate::ui::graphics::resolve_y_range(&mut app, 0, std::slice::from_ref(&key), 0, 1_000_000),
        (0.0, 100.0)
    );
    feed_rpm(&mut app, &[(300_000, 250.0)]);
    assert_eq!(
        crate::ui::graphics::resolve_y_range(&mut app, 0, std::slice::from_ref(&key), 0, 1_000_000),
        (0.0, 250.0),
        "a taller peak widens the axis"
    );
    feed_rpm(&mut app, &[(400_000, 30.0)]);
    assert_eq!(
        crate::ui::graphics::resolve_y_range(&mut app, 0, std::slice::from_ref(&key), 0, 1_000_000),
        (0.0, 250.0),
        "quieter values never shrink it back -- everything seen stays visible"
    );
    app.stop();
}

#[test]
fn dbc_mode_scales_by_the_declared_range() {
    let (mut app, key) = gfx_app(YMode::Dbc);
    let declared = app
        .declared_range(&key)
        .expect("sample.dbc declares the range");
    feed_rpm(&mut app, &[(10_000, 0.0), (200_000, 100.0)]);
    assert_eq!(
        crate::ui::graphics::resolve_y_range(&mut app, 0, std::slice::from_ref(&key), 0, 1_000_000),
        declared,
        "the axis is the database's word, not the traffic's"
    );
    // Even traffic beyond the declaration cannot stretch it: the curve
    // clamps at the plot edge instead.
    feed_rpm(&mut app, &[(300_000, 9_000.0)]);
    assert_eq!(
        crate::ui::graphics::resolve_y_range(&mut app, 0, std::slice::from_ref(&key), 0, 1_000_000),
        declared
    );
    app.stop();
}

#[test]
fn each_signal_keeps_its_own_axis_in_the_overlay_union() {
    // Overlay shares one axis, but the policies stay per signal: EngineSpeed
    // locked keeps its span as a floor/ceiling while an Auto neighbour
    // widens the shared span past it.
    let (mut app, key) = gfx_app(YMode::Lock);
    let temp = SigKey::can(0, 0x100, false, "EngineTemp");
    app.subscribe(temp.clone());
    app.graphics[0].signals.push(GfxSignal {
        key: temp.clone(),
        visible: true,
        y_mode: YMode::Auto,
    });
    let keys = vec![key.clone(), temp.clone()];

    // rpm_frame fills EngineSpeed's bytes; byte 2 carries EngineTemp
    // (sample.dbc: raw with a -40 offset).
    let mut f = rpm_frame(10_000, 100.0);
    f.len = 3;
    f.data[2] = 100;
    let mut f2 = rpm_frame(200_000, 50.0);
    f2.len = 3;
    f2.data[2] = 100;
    receive(&mut app, 300_000, vec![f, f2]);
    let pinned = crate::ui::graphics::resolve_y_range(&mut app, 0, &keys, 0, 1_000_000);
    assert_eq!(pinned, (50.0, 100.0), "the locked signal pins the axis");

    let mut f3 = rpm_frame(300_000, 50.0);
    f3.len = 3;
    f3.data[2] = 200;
    receive(&mut app, 400_000, vec![f3]);
    let grown = crate::ui::graphics::resolve_y_range(&mut app, 0, &keys, 0, 1_000_000);
    assert!(
        grown.1 > 100.0,
        "the auto neighbour widens the shared axis: {grown:?}"
    );
    assert_eq!(grown.0, 50.0, "the locked floor survives the union");
    app.stop();
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
    let key = SigKey::can(0, 0x100, false, "EngineSpeed");
    app.state_trackers[0].signals.push(GfxSignal {
        key: key.clone(),
        visible: true,
        y_mode: YMode::Auto,
    });
    app.state_trackers[0].time_window_s = 60.0;
    app.state_trackers[0].rules.insert(
        key.clone(),
        crate::observe::StateRule {
            cuts: vec![1000.0],
            names: vec!["low".to_string(), "high".to_string()],
            colors: vec![Some([0.2, 0.2, 0.8]), None],
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
    assert_eq!(rule.cuts, vec![1000.0]);
    assert_eq!(rule.names, vec!["low", "high"]);
    assert_eq!(rule.colors[0], Some([0.2, 0.2, 0.8]));
    assert_eq!(rule.colors[1], None, "automatic survives the round trip");
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

    let json = serde_json::to_string(&Config::from_app(&app, None)).unwrap();
    assert!(
        json.contains(r#""blocks""#) && json.contains("engine_log"),
        "the declaration is in the file"
    );
    let mut restored = App::headless();
    serde_json::from_str::<Config>(&json)
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

    let key = SigKey::can(0, crate::app::EMITTED_ID_BASE | id as u32, false, "SpeedKmh".to_string());
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
    app.send(crate::bus::BusCommand::DefineSysVar(crate::bus::SysVarDef {
        namespace: "Demo".to_string(),
        name: "Setpoint".to_string(),
        init: 0.0,
        min: Some(0.0),
        max: Some(100.0),
        unit: String::new(),
        comment: String::new(),
    }));
    app.settle();
    app.send(crate::bus::BusCommand::AddTrigger {
        cond: crate::trigger::TriggerCond::SysVar {
            key: "Demo::Setpoint".to_string(),
            threshold: 50.0,
            rising: true,
        },
        action: crate::trigger::TriggerAction::ClearTrace,
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
    assert_eq!(app.snap.triggers[0].fired, 1, "holding high does not refire");

    // Drop below and raise again: a fresh edge.
    set(&mut app, 10.0);
    let t = app.now_us() + 1_000;
    run_to(&mut app, t);
    set(&mut app, 80.0);
    let t = app.now_us() + 1_000;
    run_to(&mut app, t);
    assert_eq!(app.snap.triggers[0].fired, 2, "a second crossing fires again");
    app.stop();
}

/// A Monitor row holds the signal's subscription on its own: removing
/// the same signal from a Data window must not cut the Monitor's feed
/// (and with every watcher gone, the feed is pruned).
#[test]
fn monitor_rows_hold_the_subscription_against_data_removal() {
    let mut app = App::headless();
    app.new_data_window();
    let key = SigKey::can(0, 0x100, false, "EngineStatus");
    app.set_win_signal(PopupTarget::Data(0), key.clone(), true);
    app.set_win_signal(PopupTarget::Monitor, key.clone(), true);
    assert_eq!(app.monitor_rows.len(), 1, "the Monitor row was added");

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

#[test]
fn sysvars_define_clamp_and_reset_on_run_start() {
    let mut app = App::headless();
    app.send(crate::bus::BusCommand::DefineSysVar(crate::bus::SysVarDef {
        namespace: "Demo".to_string(),
        name: "Setpoint".to_string(),
        init: 100.0,
        min: Some(0.0),
        max: Some(500.0),
        unit: "rpm".to_string(),
        comment: "target speed".to_string(),
    }));
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
    app.send(crate::bus::BusCommand::DefineSysVar(crate::bus::SysVarDef {
        namespace: "Demo".to_string(),
        name: "Setpoint".to_string(),
        init: 10.0,
        min: None,
        max: None,
        unit: String::new(),
        comment: String::new(),
    }));
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
    assert_eq!(
        app.snap.sysvars[0].value, 42.0,
        "the script write landed"
    );
    let key = SigKey::can(0, crate::app::EMITTED_ID_BASE | (crate::bus::SYSVAR_STREAM_ID as u32), false, "Setpoint".to_string());
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
        app.snap.write.iter().any(|l| {
            l.kind == crate::bus::WriteKind::Info && l.text.contains("measuring")
        }),
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
        app.snap.write
            .iter()
            .map(|l| l.text.as_str())
            .collect::<Vec<_>>()
    );
}

/// A project whose files moved loads the parts it can and says so -- but the
/// half that failed used to be written to the bar and then overwritten by the
/// "project loaded" line two statements later, so the user never saw it at all.
/// Each problem now stays in the Write ring, and the bar carries the count.
#[test]
fn a_project_load_problem_outlives_the_loaded_line() {
    let asset = "assets/arxml/PowerTrain.arxml";
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
    doc["config"]["fr_buses"][0]["path"] = "assets/no-such-cluster.xml".into();
    std::fs::write(&path, serde_json::to_string(&doc).unwrap()).unwrap();

    let mut restored = App::headless();
    restored.open_project_path(&path);
    restored.settle();
    assert!(
        restored.status.contains("project loaded") && restored.status.contains("1 项未能载入"),
        "the bar says both what worked and how much did not: {}",
        restored.status
    );
    assert!(
        restored.snap.write.iter().any(|l| {
            l.kind == crate::bus::WriteKind::Error && l.text.contains("FlexRay 描述加载失败")
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
        content.lines().all(|l| l.starts_with('[') && l.contains("] ")),
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
    assert!(
        !content.is_empty(),
        "the unfiltered kinds still land"
    );

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
fn an_undefined_sysvar_write_warns_and_is_dropped() {    let mut app = App::headless();
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
    assert!(
        !node.errored,
        "a bad sysvar write does not stop the node"
    );
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
    app.send(crate::bus::BusCommand::DefineSysVar(crate::bus::SysVarDef {
        namespace: "Demo".to_string(),
        name: "Rate".to_string(),
        init: 2.5,
        min: Some(0.0),
        max: Some(10.0),
        unit: "Hz".to_string(),
        comment: String::new(),
    }));
    app.settle();
    let json = serde_json::to_string(&Config::from_app(&app, None)).unwrap();
    assert!(json.contains("\"sysvars\""), "the section is in the file");
    let mut restored = App::headless();
    serde_json::from_str::<Config>(&json)
        .unwrap()
        .apply(&mut restored);
    restored.settle();
    assert_eq!(restored.snap.sysvars.len(), 1, "restored");
    let v = &restored.snap.sysvars[0];
    assert_eq!(v.def.namespace, "Demo");
    assert_eq!(v.def.name, "Rate");
    assert_eq!(v.def.unit, "Hz");
    assert_eq!(v.def.init, 2.5);
    assert_eq!(v.value, 2.5, "value starts at init");
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

    let key = SigKey::can(0, crate::app::EMITTED_ID_BASE | id as u32, false, "Seen".to_string());
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
        SigKey::can(ch, crate::app::EMITTED_ID_BASE | id as u32, false, "X")
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
        // Reading an unseen signal is the easiest runtime error.
        source: "on start { emit_value(\"Ghost\", 1); sig(0x999, \"Nope\"); }".to_string(),
    });
    app.send(crate::bus::BusCommand::SetNodeEnabled { id, on: true });
    app.start_virtual();
    app.settle();
    for t in 1..=5u64 {
        app.advance_clock(t * 1_000);
        app.tick(t * 1_000);
    }

    let node = app.snap.nodes.iter().find(|n| n.id == id).expect("node");
    assert!(node.errored, "the bad sig() tripped the fuse");
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
        app.snap.emitted.iter().any(|(k, _)| matches!(k, SigKey::Can { ch: 1, .. })),
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
        app.snap.emitted.iter().all(|(k, _)| matches!(k, SigKey::Can { ch: 0, .. })),
        "derived streams follow their node's bus"
    );
    app.stop();
}

/// Legacy projects carry scripts with no node binding; the free script
/// is gone from the model. When the bus carries a database, a restored
/// orphan is adopted by the database's first node, so it hangs under a
/// node in the tree and is gated by that node's role like any other.
#[test]
fn an_orphan_script_is_adopted_by_the_first_dbc_node_on_its_bus() {
    let mut app = App::headless();
    // Simulate the restore shape: nodes restored before their adoption
    // pass, still carrying no binding.
    app.send(crate::bus::BusCommand::AddNode {
        name: "orphan".to_string(),
        channel: 0,
        attached: None,
    });
    app.settle();
    let node = app
        .snap
        .nodes
        .iter()
        .find(|n| n.name == "orphan")
        .expect("node");
    assert_eq!(
        node.attached,
        None,
        "fixture precondition: the script restored unbound"
    );

    // A database load sweeps the bus: the orphan gets adopted.
    app.send(crate::bus::BusCommand::LoadDbc {
        ch: 0,
        paths: vec!["assets/sample.dbc".to_string()],
    });
    app.settle();
    let node = app
        .snap
        .nodes
        .iter()
        .find(|n| n.name == "orphan")
        .expect("node");
    assert_eq!(
        node.attached,
        Some((0, "EngineECU".to_string())),
        "the first DBC node gives the orphan a home"
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
    let slots: Vec<(u8, u16)> = app
        .snap
        .fr_aggs
        .iter()
        .map(|a| (a.bus, a.slot))
        .collect();
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
        fr.iter().any(|(b, l)| *b == "FR0" && l.starts_with("slot 13")),
        "bus 0's slot 13: {fr:?}"
    );
    assert!(
        fr.iter().any(|(b, l)| *b == "FR1" && l.starts_with("slot 13")),
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
/// items at once and ImGui stops the window with "2 visible items with
/// conflicting ID". The row therefore carries its own key.
#[test]
fn messages_rows_that_print_one_label_keep_their_own_ids() {
    let mut app = spec_app();
    app.channels[1].dbc = app.channels[0].dbc.clone();
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
    let arxml = "assets/arxml/PowerTrain.arxml";
    let mut app = quiet_app();
    app.tx_list.retain(|t| t.channel != 0);
    let Ok(bytes) = std::fs::read(arxml) else {
        panic!("{arxml} missing -- it is tracked in the repository, so a broken checkout must fail, not skip");
    };
    let db = crate::fr_db::FrDb::parse(&crate::dbc::text_from_bytes(bytes)).expect("parses");
    app.fr_buses.insert(
        0,
        crate::app::FrBusCfg {
            path: arxml.into(),
            db: std::sync::Arc::new(db),
        },
    );
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
    let arxml = "assets/arxml/PowerTrain.arxml";
    let mut app = quiet_app();
    app.tx_list.retain(|t| t.channel != 0);
    let Ok(bytes) = std::fs::read(arxml) else {
        panic!("{arxml} missing -- it is tracked in the repository, so a broken checkout must fail, not skip");
    };
    let db = crate::fr_db::FrDb::parse(&crate::dbc::text_from_bytes(bytes)).expect("parses");
    // A slot the description schedules but does not name any signals for, when
    // the asset has one; otherwise the case below is the only one to check.
    let silent = db
        .slot_signals()
        .into_iter()
        .find(|(_, _, names)| names.is_empty())
        .map(|(slot, _, _)| slot);
    app.fr_buses.insert(
        0,
        crate::app::FrBusCfg {
            path: arxml.into(),
            db: std::sync::Arc::new(db),
        },
    );
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
        Some("（FR0 的描述未在 slot 4095 调度此帧）"),
        "a slot the description resolves no frame for says so"
    );
    assert_eq!(
        note("FR0", 13),
        None,
        "slot 13 is in this description and decodes, so it carries no note at all"
    );
    assert_eq!(
        note("FR1", 13).as_deref(),
        Some("（FR1 未加载集群描述）"),
        "an undescribed cluster says that instead"
    );
    if let Some(slot) = silent {
        assert_eq!(
            note("FR0", slot).as_deref(),
            Some("（该帧不声明信号）"),
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
        written.lock().expect("mock lock").iter().map(|f| f.id).collect()
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
        !app.snap.aggs.iter().any(|a| a.channel == 0 && a.id == 0x555),
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
        app.snap.aggs.iter().any(|a| a.channel == 0 && a.id == 0x556),
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
        .filter(|l| l.text.contains("发送被线路拒绝"))
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
        !app.snap.write.iter().any(|l| l.text.contains("发送被线路拒绝")),
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
        .filter(|l| l.text.contains("发送被线路拒绝"))
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
    let key = SigKey::can(1, 0x200, false, "VehicleSpeed");
    app.set_win_signal(crate::app::PopupTarget::State(0), key.clone(), true);
    assert!(!app.state_trackers[0].signals.is_empty(), "the row landed");
    app.state_trackers[0]
        .color_slots
        .insert(key.clone(), [(7u64, 0usize)].into_iter().collect());

    app.remove_channel(0);
    app.settle();

    let sig = &app.state_trackers[0].signals[0];
    assert!(matches!(sig.key, SigKey::Can { ch: 0, .. }), "the row is a CAN key that shifted down with the bus");
    let shifted = SigKey::can(0, 0x200, false, "VehicleSpeed");
    assert!(
        app.state_trackers[0].color_slots.contains_key(&shifted),
        "the color memory follows the key"
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

#[test]
fn a_script_node_round_trips_through_a_project() {
    let mut app = App::headless();
    app.send(crate::bus::BusCommand::AddNode {
        name: "gen".into(),
        channel: 0,
        attached: Some((0, "EngineECU".to_string())),
    });
    let id = app.snap.nodes[0].id;
    let source = "on timer 100 { print(1); }";
    app.send(crate::bus::BusCommand::SetNodeSource {
        id,
        source: source.to_string(),
    });
    // An unapplied editor is session state and must not leak into the file.
    app.editor_synced.insert(id, "garbage".to_string());
    let path = std::env::temp_dir().join("roxy_can_node.rxproj");
    assert!(app.save_project(Some(path.clone())));

    let mut restored = App::headless();
    restored.open_project_path(&path);
    assert_eq!(restored.snap.nodes.len(), 1);
    let node = &restored.snap.nodes[0];
    assert_eq!(node.name, "gen");
    assert_eq!(node.channel, 0);
    assert_eq!(node.source, source);
    assert!(node.enabled, "enabled defaults ride the round trip");
    // The binding must survive the restore: a lost binding would flag a
    // perfectly bound script as an orphan for adoption.
    assert_eq!(
        node.attached,
        Some((0, "EngineECU".to_string())),
        "the node binding rides the round trip"
    );
    assert!(
        restored.editor_synced.is_empty(),
        "editor sync state is keyed by id and ids are minted fresh"
    );
    std::fs::remove_file(&path).ok();
}

#[test]
fn the_text_gate_fires_on_its_cadence() {
    let mut app = quiet_app();
    app.text_rate_hz = 10;
    // Long since the last refresh: the next frame re-renders text...
    app.last_text_refresh = std::time::Instant::now() - std::time::Duration::from_millis(200);
    app.update();
    assert!(app.text_fresh, "past the period: text re-renders");
    // ...and the frame right after it does not.
    app.update();
    assert!(!app.text_fresh, "within the period: text holds");

    // Rate 0 means follow the frame rate: every frame is a text frame.
    app.text_rate_hz = 0;
    app.update();
    assert!(app.text_fresh, "unthrottled re-renders every frame");
}

#[test]
fn data_values_hold_still_until_the_text_gate_fires() {
    let mut app = quiet_app();
    let key = SigKey::can(0, 0x100, false, "EngineSpeed");
    app.subscribe(key.clone());
    if app.data_windows.is_empty() {
        app.new_data_window();
    }
    app.data_windows[0].signals.push(GfxSignal {
        key: key.clone(),
        visible: true,
        y_mode: YMode::Auto,
    });

    feed_rpm(&mut app, &[(10_000, 100.0)]);
    app.text_fresh = true;
    app.sync_data_text(0);
    assert_eq!(
        app.data_windows[0].text_cache[0][0], "100",
        "first snapshot"
    );

    // New traffic arrives, but the gate has not fired: the drawn text must
    // hold at the last snapshot instead of flickering with every frame.
    feed_rpm(&mut app, &[(200_000, 300.0)]);
    app.text_fresh = false;
    app.sync_data_text(0);
    assert_eq!(
        app.data_windows[0].text_cache[0][0], "100",
        "stale by design between text frames"
    );

    // The gate fires and the snapshot catches up -- while the bar, fed
    // straight from `latest`, never waited.
    app.text_fresh = true;
    app.sync_data_text(0);
    assert_eq!(app.data_windows[0].text_cache[0][0], "300");
    assert_eq!(app.subs[&key].latest, 300.0);
}

#[test]
fn the_text_rate_round_trips_through_a_project() {
    let mut app = App::headless();
    app.text_rate_hz = 5;
    let path = std::env::temp_dir().join("roxy_can_textrate.rxproj");
    assert!(app.save_project(Some(path.clone())));

    let mut restored = App::headless();
    restored.open_project_path(&path);
    assert_eq!(restored.text_rate_hz, 5);
    std::fs::remove_file(&path).ok();
}

#[test]
fn stats_and_message_rows_hold_still_until_the_text_gate_fires() {
    let mut app = quiet_app();
    if app.msg_windows.is_empty() {
        app.new_msg_window();
    }
    if app.stats_windows.is_empty() {
        app.new_stats_window();
    }
    feed_rpm(&mut app, &[(10_000, 100.0), (30_000, 110.0)]);

    app.text_fresh = true;
    app.sync_stats_text(0);
    app.sync_msg_text(0);
    assert_eq!(
        app.stats_windows[0].text_rows[0].count, "2",
        "first snapshot"
    );
    assert_eq!(app.msg_windows[0].text_rows[0].count, "2");
    assert!(
        app.stats_windows[0]
            .text_header
            .starts_with("1 messages, 2 frames"),
        "the header counts rows and frames: {}",
        app.stats_windows[0].text_header
    );

    // More traffic, gate closed: both tables keep drawing the old snapshot.
    feed_rpm(&mut app, &[(50_000, 120.0)]);
    app.text_fresh = false;
    app.sync_stats_text(0);
    app.sync_msg_text(0);
    assert_eq!(
        app.stats_windows[0].text_rows[0].count, "2",
        "stale by design between text frames"
    );
    assert_eq!(app.msg_windows[0].text_rows[0].count, "2");

    // The gate fires and both tables catch up in one step.
    app.text_fresh = true;
    app.sync_stats_text(0);
    app.sync_msg_text(0);
    assert_eq!(app.stats_windows[0].text_rows[0].count, "3");
    assert_eq!(app.msg_windows[0].text_rows[0].count, "3");
    assert_eq!(app.msg_windows[0].text_header, "1 messages");
    app.stop();
}

#[test]
fn the_status_counters_hold_still_until_the_text_gate_fires() {
    let mut app = quiet_app();
    feed_rpm(&mut app, &[(10_000, 100.0)]);

    app.text_fresh = true;
    app.sync_status_text();
    let held = app.status_counters.clone();
    assert!(held.contains("frames:"), "the snapshot was built: {held}");

    feed_rpm(&mut app, &[(30_000, 110.0)]);
    app.text_fresh = false;
    app.sync_status_text();
    assert_eq!(app.status_counters, held, "stale between text frames");

    app.text_fresh = true;
    app.sync_status_text();
    assert_ne!(app.status_counters, held, "the counters caught up");
    app.stop();
}

#[test]
fn graphics_legends_hold_still_until_the_text_gate_fires() {
    let (mut app, _key) = gfx_app(YMode::Auto);
    feed_rpm(&mut app, &[(10_000, 100.0)]);

    app.text_fresh = true;
    app.sync_gfx_legend(0);
    let held = app.graphics[0].legend[0].clone();
    assert!(
        held.starts_with("EngineSpeed = 100"),
        "first snapshot: {held}"
    );

    feed_rpm(&mut app, &[(200_000, 300.0)]);
    app.text_fresh = false;
    app.sync_gfx_legend(0);
    assert_eq!(
        app.graphics[0].legend,
        vec![held],
        "stale between text frames"
    );

    app.text_fresh = true;
    app.sync_gfx_legend(0);
    assert!(
        app.graphics[0].legend[0].starts_with("EngineSpeed = 300"),
        "caught up: {}",
        app.graphics[0].legend[0]
    );
    app.stop();
}

#[test]
fn a_signal_value_condition_filters_the_trace() {
    let mut app = quiet_app();
    // Filter text of the `Signal>value` form is a signal-value condition:
    // only frames whose decoded EngineSpeed passes it match.
    app.trace_windows[0].filter = "EngineSpeed>=200".to_string();
    feed_rpm(&mut app, &[(10_000, 100.0), (30_000, 300.0)]);

    app.text_fresh = true;
    app.sync_trace_rows(0);
    assert_eq!(app.trace_windows[0].rows.len(), 1, "only the >=200 frame");
    assert_eq!(app.trace_windows[0].rows[0].t_us(), 30_000);
    app.stop();
}

#[test]
fn trace_rows_reveal_in_batches_on_the_text_gate() {
    let mut app = quiet_app();
    if app.trace_windows.is_empty() {
        app.new_trace_window();
    }
    feed_rpm(&mut app, &[(10_000, 100.0), (30_000, 110.0)]);

    app.text_fresh = true;
    app.sync_trace_rows(0);
    assert_eq!(app.trace_windows[0].rows.len(), 2, "first reveal");
    assert_eq!(
        app.trace_revealed(&app.trace_windows[0]).count(),
        2,
        "both rows are on screen"
    );

    // More traffic, gate closed: the fresh tail stays hidden while the
    // drawn rows hold still.
    feed_rpm(&mut app, &[(50_000, 120.0)]);
    app.text_fresh = false;
    app.sync_trace_rows(0);
    assert_eq!(app.trace.len(), 3, "the frame did arrive");
    assert_eq!(
        app.trace_revealed(&app.trace_windows[0]).count(),
        2,
        "stale by design between text frames"
    );

    // The gate fires and the row appears in one step.
    app.text_fresh = true;
    app.sync_trace_rows(0);
    assert_eq!(app.trace_revealed(&app.trace_windows[0]).count(), 3);
    assert_eq!(app.trace_windows[0].rows.len(), 3);
    app.stop();
}

/// A window's hand-picked set now speaks both numbering spaces. Picking a
/// FlexRay slot admits that slot's rows and no others, and the two spaces share
/// arbitrary integers -- `(0, 5)` is CAN channel 0's id 5 *and* cluster 0's slot
/// 5 -- so a pick must not read as the other kind.
#[test]
fn a_hand_picked_flexray_slot_admits_only_that_slot() {
    use crate::workspace::Pick;
    let manual: std::collections::HashSet<Pick> =
        [Pick::Fr { bus: 0, slot: 5 }, Pick::Can { ch: 1, id: 5 }]
            .into_iter()
            .collect();
    assert!(App::scope_match_fr(SigScope::Manual, &manual, 0, 5));
    assert!(
        !App::scope_match_fr(SigScope::Manual, &manual, 1, 5),
        "another cluster's slot 5 is a different pick"
    );
    assert!(!App::scope_match_fr(SigScope::Manual, &manual, 0, 6));
    assert!(App::scope_match(SigScope::Manual, &manual, 1, 5));
    assert!(
        !App::scope_match(SigScope::Manual, &manual, 0, 5),
        "the FlexRay pick must not admit CAN channel 0's id 5"
    );
    // The other scopes behave as before.
    assert!(App::scope_match_fr(SigScope::All, &manual, 3, 99));
    assert!(App::scope_match_fr(SigScope::FrBus(0), &manual, 0, 6));
    assert!(
        !App::scope_match_fr(SigScope::Bus(1), &manual, 0, 5),
        "a CAN channel scope still restricts the table to CAN"
    );
    assert!(!App::scope_match(SigScope::FrBus(0), &manual, 0, 5));
}

/// The shape of a Trace window's cache as the table would draw it: the order,
/// which ring each row came from, its address, and for a decoded FlexRay signal
/// row the signal and value text it prints -- one entry per *table* row, so a
/// frame row with three children contributes four. Two caches with the same
/// shape drew the same table.
fn cache_shape(app: &App) -> Vec<(u64, u8, u32, String)> {
    let mut out = Vec::new();
    for r in app.trace_windows[0].rows.iter() {
        match r {
            TraceRow::Can(f) => out.push((f.t_us, 0, f.id, String::new())),
            TraceRow::Fr(f, kids) => {
                out.push((f.t_us, 1, u32::from(f.slot), String::new()));
                for child in 0..*kids {
                    // Resolved the way the window resolves it, so the deferred
                    // decoding is what this compares, not just the count.
                    let (signal, value) = app.fr_child_cell(f, child).unwrap_or_default();
                    out.push((f.t_us, 2, u32::from(f.slot), format!("{signal}={value}")));
                }
            }
        }
    }
    out
}

/// A steady run only prepends to the Trace row cache. Getting that wrong shows
/// up as duplicated or missing rows, so the check is against the thing it
/// replaces: extending the cache batch after batch has to end with exactly the
/// list one walk of the whole ring gives -- here with FlexRay frames expanded
/// into their decoded signal children, the shape where a stale row is easiest
/// to lose.
#[test]
fn the_trace_row_cache_extends_in_place_without_losing_rows() {
    use std::sync::Arc;
    let arxml = "assets/arxml/PowerTrain.arxml";
    let mut app = quiet_app();
    app.tx_list.retain(|t| t.channel != 0);
    let Ok(bytes) = std::fs::read(arxml) else {
        panic!("{arxml} missing -- it is tracked in the repository, so a broken checkout must fail, not skip");
    };
    let db = crate::fr_db::FrDb::parse(&crate::dbc::text_from_bytes(bytes)).expect("parses");
    // Slots whose frame actually declares signals, with the cycle that frame is
    // scheduled in: those rows are the ones that grow children under.
    let drive: Vec<(u16, u8)> = db
        .frames
        .iter()
        .enumerate()
        .filter(|(ix, _)| !db.decode(*ix, &[0u8; 48]).is_empty())
        .map(|(_, f)| (f.triggering.slot_id as u16, f.triggering.base_cycle as u8))
        .take(3)
        .collect();
    assert!(
        !drive.is_empty(),
        "the asset has no static frame with signals to expand"
    );
    app.fr_buses.insert(
        0,
        crate::app::FrBusCfg {
            path: arxml.into(),
            db: Arc::new(db),
        },
    );
    app.push_fr_db_to_core();
    let q = app.hw.attach_fr_mock(0, 5);
    app.start_virtual();
    app.trace_windows[0].fr_expand = true;
    for (round, (slot, cycle)) in drive.iter().enumerate() {
        use crate::hw::vector::flexray::FrFrame;
        q.lock().expect("mock lock").push_back(FrFrame {
            slot: *slot,
            cycle: *cycle,
            // Long enough for the signals the selection above counted: a
            // payload that does not reach a signal decodes to nothing.
            payload: vec![0; 48],
            header_crc: 0,
            flags: 0,
        });
        let t = (round as u64 + 1) * 20_000;
        app.advance_clock(t);
        app.tick(t);
        app.refresh_snapshot();
        app.text_fresh = true;
        app.sync_trace_rows(0);
    }
    let extended = cache_shape(&app);
    assert!(
        !app.trace_windows[0].rows.is_empty(),
        "the cache is not empty, or this proves nothing"
    );
    assert!(
        extended.len() > app.trace_windows[0].rows.len(),
        "the drawn table has more rows than the list has entries, because a \
         FlexRay frame row carries its children: {extended:?}"
    );
    assert!(
        extended.iter().any(|(_, kind, ..)| *kind == 2),
        "FlexRay children are in the cache: {extended:?}"
    );
    assert!(
        extended.windows(2).all(|w| w[0].0 >= w[1].0),
        "newest first: {extended:?}"
    );
    // The same traffic, walked once from scratch.
    app.trace_windows[0].rows_build = None;
    app.text_fresh = true;
    app.sync_trace_rows(0);
    assert_eq!(
        cache_shape(&app),
        extended,
        "batch by batch and all at once have to agree"
    );
    app.stop();
}

/// What must throw the incremental cache away: an edited filter (rows that no
/// longer match have to leave), a column sort (a sorted cache is no longer
/// time-ordered, so there is nothing to prepend to), and a cleared trace (the
/// rows under the build point are gone from the ring). Each case ends with the
/// cache describing the ring as it is now.
#[test]
fn an_edited_filter_or_a_cleared_trace_rebuilds_the_row_cache() {
    let mut app = quiet_app();
    for round in 0..3u64 {
        feed_rpm(&mut app, &[(round * 30_000 + 10_000, 100.0)]);
    }
    app.text_fresh = true;
    app.sync_trace_rows(0);
    assert_eq!(app.trace_windows[0].rows.len(), 3);

    // The filter text narrows, so every row is tested again: the cache shrinks
    // to what matches instead of keeping the old rows plus the new ones.
    app.trace_windows[0].filter = "no-such-frame-name".to_string();
    feed_rpm(&mut app, &[(120_000, 110.0)]);
    app.text_fresh = true;
    app.sync_trace_rows(0);
    assert!(
        app.trace_windows[0].rows.is_empty(),
        "a filter edit rebuilds: {:?}",
        cache_shape(&app)
    );
    // Back to no filter: the whole ring is in again, in one refresh.
    app.trace_windows[0].filter.clear();
    app.text_fresh = true;
    app.sync_trace_rows(0);
    assert_eq!(
        app.trace_windows[0].rows.len(),
        4,
        "and the rebuild finds the whole ring"
    );

    // A sorted cache is no longer time-ordered, so a refresh has to rebuild it
    // rather than prepend: turned oldest-first here, where prepending the new
    // row would leave a list that runs backwards right after the newest row.
    app.trace_windows[0]
        .rows
        .make_contiguous()
        .reverse();
    app.trace_windows[0].rows_sorted = true;
    feed_rpm(&mut app, &[(150_000, 120.0)]);
    app.text_fresh = true;
    app.sync_trace_rows(0);
    let shape = cache_shape(&app);
    assert_eq!(shape.len(), 5);
    assert!(
        shape.windows(2).all(|w| w[0].0 >= w[1].0),
        "back in time order: {shape:?}"
    );
    assert!(
        !app.trace_windows[0].rows_sorted,
        "the flag is consumed by the rebuild"
    );

    // A cleared trace: the ring is empty, so the cache has to be too.
    app.send(crate::bus::BusCommand::ClearTrace);
    app.text_fresh = true;
    app.sync_trace_rows(0);
    assert!(
        app.trace_windows[0].rows.is_empty(),
        "clearing the trace empties the cache: {:?}",
        cache_shape(&app)
    );
    app.stop();
}

/// A one-slot FlexRay description whose frame declares exactly these signals,
/// on a cluster whose static slot lasts `slot_wire_us` microseconds (0 = the
/// description declares no timing at all). Enough to pin how many child rows a
/// frame row owns, which name each one prints, and what the cluster's occupancy
/// is charged.
fn one_slot_db(signals: &[&str], slot_wire_us: f64) -> std::sync::Arc<crate::fr_db::FrDb> {
    use crate::fr_db::{FrChannel, FrClusterParams, FrDb, FrFrameDb, FrPdu, FrSignal, FrTriggering};
    let sig = |name: &str| FrSignal {
        name: name.into(),
        start_bit: 0,
        length_bits: 8,
        big_endian: false,
        signed: false,
        factor: 1.0,
        offset: 0.0,
        min: 0.0,
        max: 255.0,
        unit: String::new(),
        comment: String::new(),
        value_descriptions: vec![],
    };
    FrDb::assemble(
        FrClusterParams {
            // One macrotick of `slot_wire_us`, so the product is the figure the
            // load is charged per arrival.
            static_slot_duration: 1,
            macrotick_duration_us: slot_wire_us,
            ..Default::default()
        },
        vec![],
        vec![FrPdu {
            name: "P".into(),
            length: 1,
            dynamic: false,
            comment: String::new(),
            signals: signals.iter().map(|s| sig(s)).collect(),
        }],
        vec![FrFrameDb {
            name: "F".into(),
            length: 1,
            payload_preamble: false,
            triggering: FrTriggering {
                channel: FrChannel::Both,
                slot_id: 5,
                base_cycle: 0,
                cycle_repetition: 1,
                startup: false,
            },
            pdus: vec![("P".into(), 0u32)],
            comment: String::new(),
        }],
    )
    .into()
}

/// A FlexRay cluster's load is the same kind of number as a CAN channel's: the
/// share of the rolling window the traffic occupied, with a FlexRay frame's
/// occupancy being its static slot (`gstaticSlot` × `gmacrotick`) rather than
/// its length. The description supplies the timing; nothing here is invented.
#[test]
fn a_flexray_cluster_reports_its_static_segment_occupancy() {
    use crate::hw::vector::flexray::FrFrame;
    let mut app = quiet_app();
    app.tx_list.retain(|t| t.channel != 0);
    app.fr_buses.insert(
        0,
        crate::app::FrBusCfg {
            path: "synthetic".into(),
            db: one_slot_db(&["One"], 40.0),
        },
    );
    app.push_fr_db_to_core();
    let q = app.hw.attach_fr_mock(0, 5);
    app.start_virtual();
    for i in 0..25u64 {
        q.lock().expect("mock lock").push_back(FrFrame {
            slot: 5,
            cycle: 0,
            payload: vec![0x2A],
            header_crc: 0,
            flags: 0,
        });
        let t = (i + 1) * 20_000;
        app.advance_clock(t);
        app.tick(t);
        app.refresh_snapshot();
    }
    let load = app.snap.fr_loads.get(&0).expect("bus 0 carried traffic");
    assert_eq!(load.frames, 25);
    assert!(
        (load.load() - 25.0 * 40.0 / 1e6).abs() < 1e-12,
        "25 slots of 40 us inside a one-second window: {}",
        load.load()
    );
    assert_eq!(app.fr_slot_wire_us(0), Some(40.0));
    app.stop();
}

/// A cluster watched with a description that declares no slot timing counts
/// frames and claims no load, which is what keeps the Statistics row an honest
/// "-" instead of a 0 %.
#[test]
fn an_undescribed_flexray_cluster_counts_frames_but_no_occupancy() {
    use crate::hw::vector::flexray::FrFrame;
    let mut app = quiet_app();
    app.tx_list.retain(|t| t.channel != 0);
    app.fr_buses.insert(
        0,
        crate::app::FrBusCfg {
            path: "synthetic".into(),
            db: one_slot_db(&["One"], 0.0),
        },
    );
    app.push_fr_db_to_core();
    let q = app.hw.attach_fr_mock(0, 5);
    app.start_virtual();
    q.lock().expect("mock lock").push_back(FrFrame {
        slot: 5,
        cycle: 0,
        payload: vec![0x2A],
        header_crc: 0,
        flags: 0,
    });
    app.advance_clock(20_000);
    app.tick(20_000);
    app.refresh_snapshot();
    assert_eq!(app.fr_slot_wire_us(0), None, "no timing to divide by");
    let load = app.snap.fr_loads.get(&0).expect("bus 0 carried traffic");
    assert_eq!(load.frames, 1, "the arrival still counts");
    assert_eq!(load.load(), 0.0);
    app.stop();
}

/// A script reads a FlexRay signal the same way it reads a CAN one. End to end
/// on purpose -- a frame off the FlexRay watch, then a node timer that asks for
/// its value -- because the read is worth nothing if the arrival never reaches
/// it: the value has to travel the ingest → per-frame tally → decode → host
/// input path, and the slot number has to mean the frame that actually arrived.
#[test]
fn a_node_script_reads_a_flexray_signal() {
    use crate::hw::vector::flexray::FrFrame;
    let mut app = quiet_app();
    app.tx_list.retain(|t| t.channel != 0);
    app.fr_buses.insert(
        0,
        crate::app::FrBusCfg {
            path: "synthetic".into(),
            db: one_slot_db(&["One"], 40.0),
        },
    );
    app.push_fr_db_to_core();
    // The node lives on a CAN channel and names the cluster by index: a FlexRay
    // bus is not a channel a node could belong to.
    app.send(crate::bus::BusCommand::AddNode {
        name: "reader".to_string(),
        channel: 0,
        attached: None,
    });
    app.settle();
    let id = app.snap.nodes[0].id;
    app.send(crate::bus::BusCommand::SetNodeSource {
        id,
        source: "on timer 100 { emit_value(\"FromFr\", fr_sig(0, 5, \"One\")); }".to_string(),
    });
    app.settle();
    let q = app.hw.attach_fr_mock(0, 5);
    app.start_virtual();
    q.lock().expect("mock lock").push_back(FrFrame {
        slot: 5,
        cycle: 0,
        payload: vec![0x2A],
        header_crc: 0,
        flags: 0,
    });
    app.advance_clock(20_000);
    app.tick(20_000);
    // Past the timer's 100 ms, so the handler runs with the frame already in.
    for t in (30_000..=200_000).step_by(10_000) {
        app.advance_clock(t);
        app.tick(t);
    }
    app.refresh_snapshot();
    let node = app.snap.nodes.iter().find(|n| n.id == id).expect("the node");
    assert!(
        !node.errored,
        "the read did not fail: {:?}",
        node.log
    );
    let key = SigKey::can(0, crate::app::EMITTED_ID_BASE | id as u32, false, "FromFr".to_string());
    let sub = app.subs.get(&key).expect("the derived stream");
    assert_eq!(
        sub.latest, 42.0,
        "0x2A in slot 5's one 8-bit signal, decoded through its frame"
    );
    app.stop();
}

/// A FlexRay arrival wakes a running node's `on fr slot` handler, and the
/// reaction lands on the node's CAN channel in the same step. The read inside
/// the handler is the point of the ordering: `fr_sig` must see the value this
/// very frame carried, so the ingest runs first and the dispatch after -- had
/// the dispatch come first, this script would error on an unseen signal.
#[test]
fn a_flexray_arrival_drives_a_script_reaction() {
    use crate::hw::vector::flexray::FrFrame;
    let mut app = quiet_app();
    app.tx_list.retain(|t| t.channel != 0);
    app.fr_buses.insert(
        0,
        crate::app::FrBusCfg {
            path: "synthetic".into(),
            db: one_slot_db(&["One"], 40.0),
        },
    );
    app.push_fr_db_to_core();
    app.send(crate::bus::BusCommand::AddNode {
        name: "gw".to_string(),
        channel: 0,
        attached: None,
    });
    app.settle();
    let id = app.snap.nodes[0].id;
    app.send(crate::bus::BusCommand::SetNodeSource {
        id,
        source: "on fr slot 5 { send(0x320, fr_sig(0, 5, \"One\")); }".to_string(),
    });
    app.settle();
    let q = app.hw.attach_fr_mock(0, 5);
    app.start_virtual();
    q.lock().expect("mock lock").push_back(FrFrame {
        slot: 5,
        cycle: 3,
        payload: vec![0x2A],
        header_crc: 0,
        flags: 0,
    });
    app.advance_clock(20_000);
    app.tick(20_000);
    app.refresh_snapshot();
    let node = app.snap.nodes.iter().find(|n| n.id == id).expect("the node");
    assert!(
        !node.errored,
        "the handler read the value its own arrival carried: {:?}",
        node.log
    );
    let agg = app
        .aggs
        .get(&(0, 0x320, false))
        .expect("the reaction frame reached the CAN bus");
    assert_eq!(agg.data[0], 42, "the decoded physical value, truncated to a byte");
    assert_eq!(agg.count, 1, "one arrival, one reaction");
    app.stop();
}

/// A script's `fr_send` puts a frame into the session through the same funnel a
/// generator slot uses -- so Trace, the Messages tally and everything else that
/// reads arrivals see it -- and it lands as soon as that step's generator block
/// runs (a FlexRay arrival is drained before it, so the same step; a CAN-driven or
/// timer-driven handler waits for the next one). Either way **one round per
/// step** is the bound a script that answers its own frame runs into, instead of a
/// loop inside a single step.
#[test]
fn a_script_frame_enters_the_session_like_any_arrival() {
    use crate::hw::vector::flexray::FrFrame;
    let mut app = quiet_app();
    app.tx_list.retain(|t| t.channel != 0);
    app.fr_buses.insert(
        0,
        crate::app::FrBusCfg {
            path: "synthetic".into(),
            db: one_slot_db(&["One"], 40.0),
        },
    );
    app.push_fr_db_to_core();
    app.send(crate::bus::BusCommand::AddNode {
        name: "gw".to_string(),
        channel: 0,
        attached: None,
    });
    app.settle();
    let id = app.snap.nodes[0].id;
    // Slot 6 is not in this description at all: with no declared width, the
    // frame goes out exactly as long as the script wrote it.
    app.send(crate::bus::BusCommand::SetNodeSource {
        id,
        source: "on fr slot 5 { fr_send(0, 6, 0x11, 0x22); }".to_string(),
    });
    app.settle();
    let rows = |app: &App| -> Vec<(u8, u16, usize)> {
        (0..app.snap.fr_trace.len())
            .filter_map(|i| app.snap.fr_trace.get(i))
            .map(|r| (r.bus, r.slot, r.payload.len()))
            .collect()
    };
    let q = app.hw.attach_fr_mock(0, 5);
    q.lock().expect("mock lock").push_back(FrFrame {
        slot: 5,
        cycle: 0,
        payload: vec![0x2A],
        header_crc: 0,
        flags: 0,
    });
    app.advance_clock(20_000);
    app.tick(20_000);
    app.refresh_snapshot();
    assert_eq!(
        rows(&app),
        [(0, 5, 1), (0, 6, 2)],
        "the FlexRay watch is drained before the generator block, so the script's \
         frame joins this same step"
    );
    // A second step adds nothing: the row the script sent wakes handlers (slot 6
    // here, which this script does not listen to), and whatever they queue waits
    // for the next step's ingest -- one round per step is the bound a script that
    // answers its own frame runs into, instead of a loop inside one step.
    app.advance_clock(40_000);
    app.tick(40_000);
    app.refresh_snapshot();
    assert_eq!(rows(&app), [(0, 5, 1), (0, 6, 2)], "no cascade");
    let payload = (0..app.snap.fr_trace.len())
        .filter_map(|i| app.snap.fr_trace.get(i))
        .find(|r| r.slot == 6)
        .map(|r| r.payload.clone())
        .expect("the script row");
    assert_eq!(payload, vec![0x11, 0x22], "the bytes as typed");
    assert!(
        app.fr_aggs
            .values()
            .any(|a| a.bus == 0 && a.slot == 6 && a.count == 1),
        "and it is tallied in Messages like any arrival"
    );
    let node = app.snap.nodes.iter().find(|n| n.id == id).expect("the node");
    assert!(!node.errored, "the handler ran clean: {:?}", node.log);
    app.stop();
}

/// A FlexRay slot is as wide as the schedule says, and a script cannot overfill
/// it: a payload longer than the declared width is **dropped whole** with the
/// reason in that node's log, rather than truncated into a frame nobody asked
/// for. Silently sending different bytes than the script wrote is the failure
/// this refuses.
#[test]
fn a_script_frame_wider_than_its_slot_is_dropped_with_the_reason() {
    use crate::hw::vector::flexray::FrFrame;
    let mut app = quiet_app();
    app.tx_list.retain(|t| t.channel != 0);
    app.fr_buses.insert(
        0,
        crate::app::FrBusCfg {
            path: "synthetic".into(),
            db: one_slot_db(&["One"], 40.0),
        },
    );
    app.push_fr_db_to_core();
    app.send(crate::bus::BusCommand::AddNode {
        name: "wide".to_string(),
        channel: 0,
        attached: None,
    });
    app.settle();
    let id = app.snap.nodes[0].id;
    // Slot 5 declares one byte; the handler offers three.
    app.send(crate::bus::BusCommand::SetNodeSource {
        id,
        source: "on fr slot 5 { fr_send(0, 5, 1, 2, 3); }".to_string(),
    });
    app.settle();
    let q = app.hw.attach_fr_mock(0, 5);
    q.lock().expect("mock lock").push_back(FrFrame {
        slot: 5,
        cycle: 0,
        payload: vec![0x2A],
        header_crc: 0,
        flags: 0,
    });
    for t in [20_000u64, 40_000, 60_000] {
        app.advance_clock(t);
        app.tick(t);
    }
    app.refresh_snapshot();
    let lengths = (0..app.snap.fr_trace.len())
        .filter_map(|i| app.snap.fr_trace.get(i))
        .map(|r| r.payload.len())
        .collect::<Vec<_>>();
    assert!(
        !lengths.contains(&3),
        "no three-byte frame was ever admitted: {lengths:?}"
    );
    let node = app.snap.nodes.iter().find(|n| n.id == id).expect("the node");
    assert!(
        node.log
            .iter()
            .any(|l| l.contains("fr_send") && l.contains("声明 1 字节")),
        "the reason is in the node's log: {:?}",
        node.log
    );
    assert!(!node.errored, "a dropped frame is advice, not a fault: {:?}", node.log);
    app.stop();
}

/// A script cannot invent a bus: `fr_send` to a cluster with neither a
/// description nor a watch is dropped with the reason in that node's log, rather
/// than conjuring a Buses row that claims "日志里有这路流量" for traffic nobody
/// received. A slot the description does not schedule is a different thing -- the
/// 路 exists, so the frame goes out with the bytes as typed.
#[test]
fn a_script_frame_for_an_unconfigured_bus_is_refused() {
    use crate::hw::vector::flexray::FrFrame;
    let mut app = quiet_app();
    app.tx_list.retain(|t| t.channel != 0);
    app.fr_buses.insert(
        0,
        crate::app::FrBusCfg {
            path: "synthetic".into(),
            db: one_slot_db(&["One"], 40.0),
        },
    );
    app.push_fr_db_to_core();
    app.send(crate::bus::BusCommand::AddNode {
        name: "typo".to_string(),
        channel: 0,
        attached: None,
    });
    app.settle();
    let id = app.snap.nodes[0].id;
    app.send(crate::bus::BusCommand::SetNodeSource {
        id,
        source: "on fr slot 5 { fr_send(7, 5, 1); fr_send(0, 6, 2); }".to_string(),
    });
    app.settle();
    let q = app.hw.attach_fr_mock(0, 5);
    q.lock().expect("mock lock").push_back(FrFrame {
        slot: 5,
        cycle: 0,
        payload: vec![0x2A],
        header_crc: 0,
        flags: 0,
    });
    for n in 1..=3u64 {
        app.advance_clock(n * 20_000);
        app.tick(n * 20_000);
    }
    app.refresh_snapshot();
    let buses = (0..app.snap.fr_trace.len())
        .filter_map(|i| app.snap.fr_trace.get(i))
        .map(|r| r.bus)
        .collect::<Vec<_>>();
    assert!(!buses.contains(&7), "FR7 was never invented: {buses:?}");
    assert!(buses.contains(&0), "the configured 路 still carries rows");
    assert_eq!(
        app.fr_bus_rows(),
        [0],
        "and the Buses table lists only the 路 that exists"
    );
    let node = app.snap.nodes.iter().find(|n| n.id == id).expect("the node");
    assert!(
        node.log
            .iter()
            .any(|l| l.contains("fr_send") && l.contains("FR7 不是已配置的路")),
        "the typo is reported where the script writes: {:?}",
        node.log
    );
    assert!(!node.errored, "{:?}", node.log);
    app.stop();
}
/// panel: it encodes one signal into the generator entry that owns the slot,
/// through the cluster description -- and `fr_send` to a described slot fills
/// that slot with what the description declares, not with what felt convenient.
/// The write lands in the entry, so an entry that is Off changes what it will
/// carry and still sends nothing.
#[test]
fn a_script_writes_a_flexray_signal_into_its_entry() {
    let arxml = "assets/arxml/PowerTrain.arxml";
    let mut app = quiet_app();
    app.tx_list.retain(|t| t.channel != 0);
    let Ok(bytes) = std::fs::read(arxml) else {
        panic!("{arxml} missing -- it is tracked in the repository, so a broken checkout must fail, not skip");
    };
    let parsed = crate::fr_db::FrDb::parse(&crate::dbc::text_from_bytes(bytes)).expect("parses");
    let (slot, sig, declared_len) = (0..parsed.frames.len())
        .find_map(|ix| {
            parsed.frame_sender(ix)?;
            let f = parsed.frame_index(ix)?;
            let sig = parsed.edit_signals(ix).into_iter().find(|s| s.size >= 4)?;
            Some((f.triggering.slot_id as u16, sig, f.length as usize))
        })
        .expect("the asset binds a described slot to a sending ECU");
    let sig_name = sig.name.clone();
    // Named in raw code words and converted through the signal's own coding, so
    // the value written is exactly the value read back whatever the factor is --
    // and wide enough that the raw number is representable at all.
    let want = crate::decode::to_physical(5, sig.size, sig.signed, sig.factor, sig.offset);
    let db = std::sync::Arc::new(parsed);
    app.fr_buses.insert(
        0,
        crate::app::FrBusCfg {
            path: arxml.into(),
            db: std::sync::Arc::clone(&db),
        },
    );
    app.push_fr_db_to_core();
    app.add_fr_tx(0, slot);
    app.settle();
    let before = app.snap.fr_tx[0].data_text.clone();
    app.send(crate::bus::BusCommand::AddNode {
        name: "ecu".to_string(),
        channel: 0,
        attached: None,
    });
    app.settle();
    let id = app.snap.nodes[0].id;
    app.send(crate::bus::BusCommand::SetNodeSource {
        id,
        source: format!(
            "on timer 10 {{ set_fr_sig(0, {slot}, {sig_name:?}, {want}); fr_send(0, {slot}, \
             0x00); }}"
        ),
    });
    app.settle();
    for n in 1..=6u64 {
        app.advance_clock(n * 10_000);
        app.tick(n * 10_000);
    }
    app.refresh_snapshot();
    let node = app.snap.nodes.iter().find(|n| n.id == id).expect("the node");
    assert!(!node.errored, "the handler ran clean: {:?}", node.log);
    assert!(
        node.log.iter().all(|l| !l.contains("set_fr_sig]")),
        "nothing was refused: {:?}",
        node.log
    );
    assert_ne!(
        app.snap.fr_tx[0].data_text, before,
        "the entry's base payload changed"
    );
    let ix = db.frame_ix_of_slot(slot).expect("described");
    let got = db
        .decode_signals(ix, &app.fr_tx_list[0].data)
        .into_iter()
        .find(|d| d.name == sig_name)
        .expect("the signal is in this frame");
    assert_eq!(got.phys, want, "the value the script wrote reads back");
    // The same script's `fr_send` fills this slot with one byte typed into a
    // frame the description says is `declared_len` wide.
    let row = (0..app.snap.fr_trace.len())
        .filter_map(|i| app.snap.fr_trace.get(i))
        .find(|r| r.slot == slot)
        .expect("the script's frame for this slot");
    assert_eq!(
        row.payload.len(),
        declared_len,
        "the slot keeps the width its schedule declares"
    );
    app.stop();
}

/// A script can only write a slot this tool is going to fill: `set_fr_sig` on a
/// slot with no generator entry, or on a 路 with no cluster description, drops
/// the write and says which fact is missing -- the value it computed is not sent
/// as half a frame, and nothing silently lands in a slot nobody owns.
#[test]
fn a_flexray_signal_write_without_its_entry_is_refused() {
    let mut app = quiet_app();
    app.tx_list.retain(|t| t.channel != 0);
    app.fr_buses.insert(
        0,
        crate::app::FrBusCfg {
            path: "synthetic".into(),
            db: one_slot_db(&["One"], 40.0),
        },
    );
    app.push_fr_db_to_core();
    app.send(crate::bus::BusCommand::AddNode {
        name: "ghost".to_string(),
        channel: 0,
        attached: None,
    });
    app.settle();
    let id = app.snap.nodes[0].id;
    app.send(crate::bus::BusCommand::SetNodeSource {
        id,
        source: "on timer 10 { set_fr_sig(0, 5, \"One\", 7); set_fr_sig(7, 5, \"One\", 7); }"
            .to_string(),
    });
    app.settle();
    for n in 1..=4u64 {
        app.advance_clock(n * 10_000);
        app.tick(n * 10_000);
    }
    app.refresh_snapshot();
    let node = app.snap.nodes.iter().find(|n| n.id == id).expect("the node");
    let lines: Vec<&String> = node.log.iter().filter(|l| l.contains("set_fr_sig]")).collect();
    assert!(
        lines.iter().any(|l| l.contains("FR0 slot 5 没有发送条目")),
        "the slot nobody fills: {lines:?}"
    );
    assert!(
        lines.iter().any(|l| l.contains("FR7 没有集群描述")),
        "the 路 that does not exist: {lines:?}"
    );
    assert!(
        !lines.iter().any(|l| l.contains("不声明信号")),
        "it never got as far as the coding: {lines:?}"
    );
    assert!(!node.errored, "{:?}", node.log);
    assert!(app.fr_tx_list.is_empty(), "no entry was invented either");
    app.stop();
}

/// Replaying a FlexRay log drives the `on fr slot` handlers off the file's own
/// timeline: one run per arrival, in the log's order, with that arrival's slot,
/// cycle and payload as the frame context. This is the shape a user actually
/// tests -- replay a recording and watch a simulated ECU answer it -- and the
/// replay path is a separate call site from the live watch, so it earns its own
/// check.
#[test]
fn replaying_a_flexray_log_drives_the_slot_handlers() {
    let mut app = quiet_app();
    app.tx_list.retain(|t| t.channel != 0);
    app.send(crate::bus::BusCommand::AddNode {
        name: "resp".to_string(),
        channel: 0,
        attached: None,
    });
    app.settle();
    let id = app.snap.nodes[0].id;
    app.send(crate::bus::BusCommand::SetNodeSource {
        id,
        source: "on fr 0 slot 13 { print(frame_id(), fr_cycle(), frame_byte(1)); send(0x320, fr_cycle()); }"
            .to_string(),
    });
    app.settle();
    let src = write_mixed_fr_asc("roxy_can_fr_handler", 4, 13);
    replay_to_the_end(&mut app, &src);
    let node = app.snap.nodes.iter().find(|n| n.id == id).expect("the node");
    assert!(!node.errored, "the handler ran clean: {:?}", node.log);
    assert_eq!(
        node.log,
        ["13 0 0", "13 1 1", "13 2 2", "13 3 3"],
        "one run per arrival, each with its own cycle and payload byte"
    );
    let agg = app
        .aggs
        .get(&(0, 0x320, false))
        .expect("the reactions reached the CAN bus");
    assert_eq!(agg.count, 4, "one CAN frame per FlexRay arrival");
    assert_eq!(agg.data[0], 3, "the last one carried the last cycle");
}

/// The FlexRay half of the interactive generator. An entry fills one slot on
/// the period that slot's own schedule declares, and the frame it puts there
/// goes through the whole receive path -- the tally, the frame count, the
/// payload the views decode -- like a frame that arrived on a watched port.
/// Pinning a signal has to survive that trip: the value written in is the value
/// read back out of the bytes that actually went out.
#[test]
fn a_generated_flexray_slot_fills_the_session_on_its_schedule() {
    let arxml = "assets/arxml/PowerTrain.arxml";
    let mut app = quiet_app();
    app.tx_list.retain(|t| t.channel != 0);
    let db = crate::fr_db::FrDb::parse(&read_fr_asset(arxml)).expect("parses");
    // The first slot the ECU bindings give a sender to *and* that carries a
    // signal: what a generator entry can exist for at all. The period comes from
    // the asset rather than from the code under test, so the assertion below
    // means "as declared" and not "as computed".
    let (slot, ecu, sig, declared_us) = (0..db.frames.len())
        .find_map(|ix| {
            let ecu = db.frame_sender(ix).map(str::to_string)?;
            let f = db.frame_index(ix)?;
            // Wide enough that the raw numbers below are all representable.
            let sig = db
                .edit_signals(ix)
                .into_iter()
                .find(|s| s.size >= 4)?;
            let rep = f.triggering.cycle_repetition.max(1) as u64;
            Some((
                f.triggering.slot_id as u16,
                ecu,
                sig,
                (db.params.cycle_time_ms * rep as f64 * 1_000.0).round() as u64,
            ))
        })
        .expect("the asset binds a slot with a wide enough signal to a sending ECU");
    let sig_name = sig.name.clone();
    // Values named in raw code words and converted with the signal's own
    // encoding, so every one of them is exactly representable whatever the
    // coding's factor, offset, sign or declared range says.
    let phys = |raw: u64| {
        crate::decode::to_physical(raw, sig.size, sig.signed, sig.factor, sig.offset)
    };
    let (lo, hi, want) = (phys(0), phys(10), phys(5));
    let one_lsb = (phys(1) - phys(0)).abs() * 1.5;
    assert!(declared_us > 0, "the cluster declares a cycle time");
    let db = std::sync::Arc::new(db);
    app.fr_buses.insert(
        0,
        crate::app::FrBusCfg {
            path: arxml.into(),
            db: std::sync::Arc::clone(&db),
        },
    );
    app.push_fr_db_to_core();
    app.settle();

    app.add_fr_tx(0, slot);
    app.settle();
    assert_eq!(app.snap.fr_tx.len(), 1, "one entry for the slot");
    assert_eq!(
        app.snap.fr_tx[0].node, ecu,
        "the entry belongs to the ECU the description names as the sender"
    );
    assert_eq!(
        app.snap.fr_tx[0].cycle_us, declared_us,
        "the entry starts on the schedule's own period"
    );
    assert!(!app.snap.fr_tx[0].undescribed, "this slot is described");

    app.start_virtual();
    app.set_fr_tx_active(0, slot, true);
    app.settle();
    let before = app.snap.frame_counter;
    for n in 1..=3u64 {
        app.advance_clock(declared_us * n);
        app.tick(declared_us * n);
    }
    app.refresh_snapshot();
    let agg = app
        .fr_aggs
        .values()
        .find(|a| a.bus == 0 && a.slot == slot)
        .expect("the generated frames are tallied like any arrival");
    assert!(
        agg.count >= 3,
        "three periods passed, the slot filled each time: {}",
        agg.count
    );
    assert_eq!(
        app.snap.frame_counter - before,
        agg.count,
        "a generated frame counts as a frame, not as a special case"
    );
    assert_eq!(
        app.snap.fr_tx[0].sent_text.split_whitespace().count(),
        agg.payload.len(),
        "the row shows the bytes that went out"
    );

    // Pin one signal and the next frame must carry it, read back through the
    // same description the decoder uses.
    app.pin_gen_signal(crate::app::GenRow::Fr(0), &sig_name, want);
    app.settle();
    app.advance_clock(declared_us * 4);
    app.tick(declared_us * 4);
    app.refresh_snapshot();
    let agg = app
        .fr_aggs
        .values()
        .find(|a| a.bus == 0 && a.slot == slot)
        .expect("the slot is still tallied");
    let ix = db.frame_ix_of_slot(slot).expect("described above");
    let got = db
        .decode_signals(ix, &agg.payload)
        .into_iter()
        .find(|d| d.name == sig_name)
        .expect("the pinned signal is in the frame");
    assert!(
        (got.phys - want).abs() <= one_lsb,
        "{sig_name} reads back as {want}, got {} (raw {}), one LSB being {one_lsb}",
        got.phys,
        got.raw
    );

    // And a *driven* signal rides its own waveform: the value in each frame is
    // the source's value at that frame's stamp, laid over the base bytes the pin
    // just wrote. A Step over the slot's period hits both ends of its range, so
    // the set of values read back is exactly {lo, hi} -- never the pinned one.
    let mut src = crate::sim::ValueSrc::new(&sig_name, crate::sim::SrcKind::Step, lo, hi);
    // Four slot periods end to end: sampled once per slot period, the wave
    // would otherwise land on the same phase every frame and read as a
    // constant -- which is the aliasing, not a missing source.
    src.period_us = declared_us * 4;
    app.set_gen_source(crate::app::GenRow::Fr(0), src);
    app.settle();
    let mut seen = Vec::new();
    for n in 5..9u64 {
        app.advance_clock(declared_us * n);
        app.tick(declared_us * n);
        app.refresh_snapshot();
        let agg = app
            .fr_aggs
            .values()
            .find(|a| a.bus == 0 && a.slot == slot)
            .expect("the slot is still tallied");
        let got = db
            .decode_signals(ix, &agg.payload)
            .into_iter()
            .find(|d| d.name == sig_name)
            .expect("the driven signal is in the frame");
        seen.push(got.phys);
    }
    assert!(
        seen.iter().any(|v| (v - lo).abs() <= one_lsb),
        "{sig_name} rode down to its lo: {seen:?}"
    );
    assert!(
        seen.iter().any(|v| (v - hi).abs() <= one_lsb),
        "{sig_name} rode up to its hi: {seen:?}"
    );
    assert!(
        seen.iter().all(|v| (v - want).abs() > one_lsb),
        "the driven value replaced the pinned {want}, not the other way round: {seen:?}"
    );
    app.stop();
}

/// Puts the bundled ARXML on bus 0 the way the Buses window does, and returns
/// the first slot its ECU bindings give a sender to together with that ECU's
/// name -- the only kind of slot a generator entry can be built for.
fn fr_bound_slot(app: &mut App) -> (u16, String) {
    let arxml = "assets/arxml/PowerTrain.arxml";
    let db = crate::fr_db::FrDb::parse(&read_fr_asset(arxml)).expect("parses");
    let found = (0..db.frames.len()).find_map(|ix| {
        let ecu = db.frame_sender(ix).map(str::to_string)?;
        db.edit_signals(ix).into_iter().next()?;
        Some((db.frame_index(ix)?.triggering.slot_id as u16, ecu))
    });
    let (slot, ecu) = found.expect("the asset binds a slot with signals to a sending ECU");
    app.fr_buses.insert(
        0,
        crate::app::FrBusCfg {
            path: arxml.into(),
            db: std::sync::Arc::new(db),
        },
    );
    app.push_fr_db_to_core();
    app.settle();
    (slot, ecu)
}

/// A FlexRay entry whose description went away keeps filling its slot with the
/// bytes it holds and keeps its ECU row: the entry is the user's, and losing the
/// file that named its signals must neither delete the stimulus they built nor
/// hide it behind a tree the document no longer supports. The row says what is
/// missing instead of showing an empty signal list.
#[test]
fn a_flexray_entry_outlives_the_description_it_came_from() {
    let mut app = quiet_app();
    let (slot, ecu) = fr_bound_slot(&mut app);
    app.add_fr_tx(0, slot);
    // The width the description gives this slot, read before anything is typed.
    let wide = app.fr_tx_list[0].len;
    app.set_fr_tx_hex(0, slot, "11 22 33");
    app.set_fr_tx_cycle(0, slot, 10_000);
    app.set_fr_tx_active(0, slot, true);
    app.settle();
    assert_eq!(app.snap.fr_tx.len(), 1, "the entry was built from a slot");
    app.fr_buses.remove(&0);
    app.push_fr_db_to_core();
    app.settle();
    assert!(
        app.snap.fr_tx[0].undescribed,
        "nothing decodes the slot any more, and the row says so"
    );
    assert_eq!(app.snap.fr_tx[0].node, ecu, "and it is still the same row");
    app.start_virtual();
    app.settle();
    for n in 1..=2u64 {
        app.advance_clock(n * 10_000);
        app.tick(n * 10_000);
    }
    app.refresh_snapshot();
    let agg = app
        .fr_aggs
        .values()
        .find(|a| a.bus == 0 && a.slot == slot)
        .expect("the raw bytes still reach the tally");
    // The slot's width came from the description and stays with the entry after
    // it is gone: a FlexRay payload is as wide as the schedule says it is, so
    // typing three bytes sets those three and pads the rest -- it does not
    // shrink the frame the way a CAN DLC would.
    assert_eq!(agg.payload.len(), wide, "the frame keeps the slot's width");
    assert_eq!(
        agg.payload[..3],
        [0x11, 0x22, 0x33],
        "typed bytes lead, zeros pad"
    );
    app.stop();
}

/// A FlexRay payload is as wide as the schedule makes it, so the hex box is
/// bounded by the slot's declared length: three bytes typed into a wider slot
/// fill its head and pad the rest, and more bytes than the slot carries are
/// refused with the number that made the refusal -- silently dropping the tail
/// would send different bytes than the operator typed.
#[test]
fn a_flexray_payload_is_bounded_by_its_slot_width() {
    let mut app = quiet_app();
    let (slot, _) = fr_bound_slot(&mut app);
    app.add_fr_tx(0, slot);
    let wide = app.fr_tx_list[0].len;
    assert!(wide > 3, "the asset needs a slot wider than 3 bytes: {wide}");

    app.set_fr_tx_hex(0, slot, "11 22 33");
    assert_eq!(
        app.fr_tx_list[0].len, wide,
        "typing fewer bytes does not shrink the slot"
    );
    assert_eq!(app.fr_tx_list[0].data[..3], [0x11, 0x22, 0x33]);
    assert!(
        app.fr_tx_list[0].data[3..].iter().all(|&b| b == 0),
        "the rest is padding"
    );

    app.set_fr_tx_hex(0, slot, &vec!["AA"; wide + 1].join(" "));
    assert_eq!(
        app.fr_tx_list[0].data[0], 0x11,
        "the refused edit changed nothing"
    );
    assert!(
        app.status.contains(&format!("{wide} 字节")),
        "the refusal names the width: {}",
        app.status
    );

    app.set_fr_tx_hex(0, slot, "zz 11");
    assert!(
        app.status.contains("hex"),
        "unreadable text gets its own reason: {}",
        app.status
    );
}

/// The FlexRay cycle box counts **cluster cycles**, not milliseconds: a static
/// slot transmits when the schedule says, and 0 -- CAN's "event-triggered" -- is
/// not a thing it can be. `cycles_of` is the same pair of facts read the other
/// way, and it stays silent about a period that is not a whole number of cycles
/// rather than rounding a row onto the grid it is not on.
#[test]
fn the_flexray_cycle_box_counts_cycles_and_refuses_event_fills() {
    use crate::generator::{cycles_of, fr_cycle_draft};
    let ct = 5_000; // a 5 ms cycle grid
    assert_eq!(fr_cycle_draft("2", ct), Ok((2, 10_000)));
    assert_eq!(fr_cycle_draft(" 12 ", ct), Ok((12, 60_000)));
    assert_eq!(fr_cycle_draft("1", ct), Ok((1, 5_000)));
    // CAN's event mode, refused with the reason and the alternative.
    let zero = fr_cycle_draft("0", ct).expect_err("0 is not a period");
    assert!(zero.contains("事件触发"), "{zero}");
    assert!(zero.contains("On"), "{zero}");
    assert!(fr_cycle_draft("", ct).is_err(), "an empty box is not a period");
    assert!(fr_cycle_draft("abc", ct).is_err());
    assert!(fr_cycle_draft("1e6", ct).is_err(), "and neither is scientific");
    assert!(
        fr_cycle_draft("60001", ct).is_err(),
        "the same ceiling the ms box has, in cycles"
    );
    // No cycle time to count in -- an undescribed 路 -- is refused rather than
    // guessed at, because any number here would be an invented grid.
    assert!(fr_cycle_draft("2", 0).is_err());

    assert_eq!(cycles_of(10_000, ct), Some(2));
    assert_eq!(cycles_of(7_500, ct), None, "off the grid: no cycle count");
    assert_eq!(cycles_of(0, ct), None, "never sending is not every 0 cycles");
    assert_eq!(cycles_of(10_000, 0), None, "and no grid means no counting");
}

/// Which node sends which frame is what the FIBEX/ARXML declares, and the
/// generator does not clean up after a document that fails to: a slot whose
/// frame no ECU is bound to gets no entry at all, and the status line names the
/// missing fact. The same for a bus with no description -- no invented owner,
/// no floating row to be edited somewhere off the tree.
#[test]
fn a_flexray_slot_the_description_leaves_unowned_gets_no_entry() {
    let mut app = App::headless();
    app.add_fr_tx(3, 13);
    app.settle();
    assert!(
        app.snap.fr_tx.is_empty(),
        "nothing owns the slot, so nothing was added"
    );
    assert!(app.status.contains("没有集群描述"), "{}", app.status);

    let (bound, _ecu) = fr_bound_slot(&mut app);
    let unowned = {
        let db = app.fr_db(0).expect("installed above");
        (0..db.frames.len())
            .find_map(|ix| {
                if db.frame_sender(ix).is_some() {
                    return None;
                }
                Some(db.frame_index(ix)?.triggering.slot_id as u16)
            })
            .expect("the asset leaves some frames unbound")
    };
    assert_ne!(unowned, bound, "the test needs both kinds of slot");
    app.add_fr_tx(0, unowned);
    app.settle();
    assert!(app.snap.fr_tx.is_empty(), "an unowned slot gets no entry");
    assert!(
        app.status.contains("没有说明") && app.status.contains(&format!("slot {unowned}")),
        "{}",
        app.status
    );
}

/// The same standing-down rule the CAN generator follows while a log is being
/// replayed: if the replayed traffic carries this slot, a second sender of the
/// same signals would mix two values into every curve, count and verdict.
#[test]
fn a_generated_slot_stands_down_while_the_replayed_log_carries_it() {
    let mut app = quiet_app();
    let (slot, _ecu) = fr_bound_slot(&mut app);
    app.add_fr_tx(0, slot);
    app.set_fr_tx_cycle(0, slot, 1_000);
    app.set_fr_tx_active(0, slot, true);
    app.settle();
    let src = write_mixed_fr_asc("roxy_can_fr_gen_mute", 4, slot);
    replay_to_the_end(&mut app, &src);
    assert!(app.snap.fr_tx[0].muted, "the row says why it is quiet");
    // Summed over the slot's occupants: with a description loaded, the tally
    // splits the log's four arrivals by which frame the schedule puts in the
    // slot at each cycle -- which is exactly what a generator adding frames of
    // its own would have inflated.
    let total: u64 = app
        .fr_aggs
        .values()
        .filter(|a| a.bus == 0 && a.slot == slot)
        .map(|a| a.count)
        .sum();
    assert_eq!(
        total, 4,
        "only the log's four frames, not four plus a generator each millisecond"
    );
    std::fs::remove_file(&src).ok();
}

/// The spec monitor's rules have FlexRay counterparts, because the description
/// makes the same kind of promises: which slot a frame occupies, in which cycle
/// phase, how often it repeats and how long it is. Four verdicts from one
/// scripted run -- a payload short of its declared length (Dlc), an arrival off
/// the declared period (Cycle), a frame that then fell silent (Missing), and a
/// slot the schedule does not name at all (Unknown).
#[test]
fn the_spec_monitor_judges_flexray_frames_against_their_schedule() {
    use crate::hw::vector::flexray::FrFrame;
    use crate::spec::Kind;
    let arxml = "assets/arxml/PowerTrain.arxml";
    let mut app = quiet_app();
    app.tx_list.retain(|t| t.channel != 0);
    let Ok(bytes) = std::fs::read(arxml) else {
        panic!("{arxml} missing -- it is tracked in the repository, so a broken checkout must fail, not skip");
    };
    let db = crate::fr_db::FrDb::parse(&crate::dbc::text_from_bytes(bytes)).expect("parses");
    // The declared period comes out of the schedule itself, so the test states
    // the same promise the monitor checks against rather than a number picked
    // to make an assertion pass.
    let (ix, slot, cycle, declared_us) = {
        let (i, f) = db
            .frames
            .iter()
            .enumerate()
            .find(|(_, f)| f.length >= 8)
            .expect("the asset declares a frame at least 8 bytes long");
        let rep = f.triggering.cycle_repetition.max(1);
        (
            i,
            f.triggering.slot_id as u16,
            f.triggering.base_cycle as u8,
            rep as u64 * (db.params.cycle_time_ms * 1e3) as u64,
        )
    };
    assert!(declared_us > 0, "the cluster declares a cycle time");
    app.fr_buses.insert(
        0,
        crate::app::FrBusCfg {
            path: arxml.into(),
            db: db.into(),
        },
    );
    app.push_fr_db_to_core();
    let q = app.hw.attach_fr_mock(0, 5);
    app.start_virtual();
    let arrive = |app: &mut App, slot: u16, payload: Vec<u8>, at_us: u64| {
        q.lock().expect("mock lock").push_back(FrFrame {
            slot,
            cycle,
            payload,
            header_crc: 0,
            flags: 0,
        });
        app.advance_clock(at_us);
        app.tick(at_us);
        app.refresh_snapshot();
    };
    // A short payload, then the same frame five periods later: both a length and
    // a timing violation in two arrivals.
    arrive(&mut app, slot, vec![0u8; 2], 1);
    let t = declared_us * 5;
    arrive(&mut app, slot, vec![0u8; 2], t);
    // Silence past the grace window (the gap since the last arrival, not the
    // time since the run started), then a slot the schedule names no frame for.
    let t = declared_us * (app.spec_grace + 10);
    app.advance_clock(t);
    app.tick(t);
    app.refresh_snapshot();
    arrive(&mut app, 2000, vec![0u8; 2], t + 1);

    let kinds = |app: &App, slot: u16| -> Vec<Kind> {
        app.snap
            .spec
            .fr_rows
            .iter()
            .filter(|(k, _)| k.0 .0 == 0 && k.0 .1 == slot)
            .map(|(k, _)| k.1)
            .collect()
    };
    let judged = kinds(&app, slot);
    for want in [Kind::Dlc, Kind::Cycle, Kind::Missing] {
        assert!(
            judged.contains(&want),
            "{slot} was judged {judged:?}, expected {want:?} too"
        );
    }
    assert!(
        kinds(&app, 2000).contains(&Kind::Unknown),
        "an unscheduled slot is the same finding as an id outside the DBC"
    );
    // The report line names the frame from the same description the verdict came
    // out of, and says which cluster and slot it is on.
    let rows = app.spec_rows();
    let fr = rows
        .iter()
        .find(|r| r.bus == "FR0" && r.addr == format!("slot {slot}"))
        .expect("the FlexRay row is in the report");
    assert_eq!(
        fr.name,
        app.fr_db(0).unwrap().frame_index(ix).unwrap().name,
        "the frame's own name, not an index"
    );
    assert_eq!(
        app.spec_rows()
            .iter()
            .filter(|r| r.bus == "FR0" && r.addr == "slot 2000")
            .map(|r| r.name.as_str())
            .next(),
        Some("not in the schedule"),
        "the unresolved slot says so"
    );
    app.stop();
}

/// The Messages window's Clear resets the FlexRay tallies as well: they are rows
/// of that table now, and leaving them to run up from their old counts made the
/// button lie about half of what it shows.
#[test]
fn clearing_the_message_counters_clears_the_flexray_tallies_too() {
    use crate::hw::vector::flexray::FrFrame;
    let mut app = quiet_app();
    app.tx_list.retain(|t| t.channel != 0);
    app.fr_buses.insert(
        0,
        crate::app::FrBusCfg {
            path: "synthetic".into(),
            db: one_slot_db(&["One"], 40.0),
        },
    );
    app.push_fr_db_to_core();
    let q = app.hw.attach_fr_mock(0, 5);
    app.start_virtual();
    for i in 0..3u64 {
        q.lock().expect("mock lock").push_back(FrFrame {
            slot: 5,
            cycle: 0,
            payload: vec![0x2A],
            header_crc: 0,
            flags: 0,
        });
        let t = (i + 1) * 20_000;
        app.advance_clock(t);
        app.tick(t);
        app.refresh_snapshot();
    }
    app.sync_msg_text(0);
    assert!(
        app.msg_windows[0]
            .text_rows
            .iter()
            .any(|r| r.bus == "FR0" && r.count == "3"),
        "the tally is on screen before the clear: {:?}",
        app.msg_windows[0]
            .text_rows
            .iter()
            .map(|r| (r.bus.clone(), r.label.clone(), r.count.clone()))
            .collect::<Vec<_>>()
    );

    app.send(crate::bus::BusCommand::ClearAggregates);
    app.advance_clock(100_000);
    app.tick(100_000);
    app.refresh_snapshot();
    assert!(
        app.snap.fr_aggs.is_empty(),
        "the FlexRay counters cleared with the CAN ones"
    );
    app.sync_msg_text(0);
    assert!(
        !app.msg_windows[0].text_rows.iter().any(|r| r.bus == "FR0"),
        "and the window shows nothing until a frame arrives again"
    );
    app.stop();
}

/// What must also throw the expanded cache away: the cluster description a
/// child row was indexed against. An expanded FlexRay row stores `(frame,
/// child)` indices into that description, so when the bus gets a different one
/// the old rows mean something else -- here the new frame declares a signal the
/// old one has no row for, and a cache that kept extending would go on showing
/// one child where the walk now yields two.
#[test]
fn a_new_cluster_description_rebuilds_the_expanded_rows() {
    use crate::hw::vector::flexray::FrFrame;
    let mut app = quiet_app();
    app.tx_list.retain(|t| t.channel != 0);
    let describe = |app: &mut App, signals: &[&str]| {
        app.fr_buses.insert(
            0,
            crate::app::FrBusCfg {
                path: "synthetic".into(),
                db: one_slot_db(signals, 40.0),
            },
        );
        app.push_fr_db_to_core();
    };
    describe(&mut app, &["One"]);
    let q = app.hw.attach_fr_mock(0, 5);
    app.start_virtual();
    app.trace_windows[0].fr_expand = true;
    q.lock().expect("mock lock").push_back(FrFrame {
        slot: 5,
        cycle: 0,
        payload: vec![0x2A],
        header_crc: 0,
        flags: 0,
    });
    app.advance_clock(20_000);
    app.tick(20_000);
    app.refresh_snapshot();
    app.text_fresh = true;
    app.sync_trace_rows(0);
    let kids = |app: &App| {
        cache_shape(app)
            .into_iter()
            .filter(|(_, kind, ..)| *kind == 2)
            .map(|(.., text)| text)
            .collect::<Vec<_>>()
    };
    assert_eq!(
        kids(&app),
        vec!["One=42  (2Ah)"],
        "the first description's one child row"
    );

    // The swap, with no new traffic: the filter, the rings and every timestamp
    // still say "extend", so only the description can be what rebuilds.
    describe(&mut app, &["One", "Two"]);
    app.text_fresh = true;
    app.sync_trace_rows(0);
    assert_eq!(
        kids(&app),
        vec!["One=42  (2Ah)", "Two=42  (2Ah)"],
        "rebuilt against the description the bus has now"
    );
    app.stop();
}

/// "仅 DBC" asks one question of every row in a mixed table: does a database
/// account for *this arrival*? A FlexRay frame the cluster description schedules
/// answers yes and stays -- the old reading dropped the entire FlexRay side,
/// which threw away precisely the rows the checkbox exists to keep. What has to
/// go is what no database describes, and there are two distinct kinds of that: a
/// slot this cluster's schedule holds no frame for, and any row on a bus with no
/// description at all.
#[test]
fn dbc_only_keeps_the_flexray_rows_a_description_covers() {
    use crate::aggregate::{FrFrameAgg, FrOccupant};
    let mut app = quiet_app();
    let arxml = "assets/arxml/PowerTrain.arxml";
    let text = read_fr_asset(arxml);
    let db = crate::fr_db::FrDb::parse(&text).expect("the asset parses");
    let (slot, cycle) = {
        let f = &db.frames[0];
        (
            f.triggering.slot_id as u16,
            f.triggering.base_cycle as u8,
        )
    };
    let ix = db
        .frame_ix_at(slot, cycle, 0)
        .expect("the first frame resolves at its own slot and phase");
    app.fr_buses.insert(
        0,
        crate::app::FrBusCfg {
            path: arxml.into(),
            db: std::sync::Arc::new(db),
        },
    );
    let row = |bus: u8, at: u16| crate::trace::FrRow {
        bus,
        t_us: 1_000,
        ab: 0,
        slot: at,
        cycle,
        payload: vec![1],
        header_crc: 0,
        flags: 0,
        name: None,
    };
    let flt = |app: &App, dbc_only: bool| {
        let mut w = app.trace_windows[0].clone();
        w.dbc_only = dbc_only;
        w.filter_lens()
    };
    assert!(
        app.trace_fr_match(&flt(&app, true), &row(0, slot)),
        "the description schedules this frame: the row stays"
    );
    assert!(
        !app.trace_fr_match(&flt(&app, true), &row(0, 4095)),
        "no frame is scheduled in that slot"
    );
    assert!(
        !app.trace_fr_match(&flt(&app, true), &row(1, slot)),
        "FR1 has no description at all"
    );
    assert!(
        app.trace_fr_match(&flt(&app, false), &row(1, slot)),
        "unticked, an undescribed row is table material again"
    );

    // Messages asks the same question, because its CSV must match its table.
    // The tally carries its occupant in the row as well as in the map key --
    // that field is what "described" is read from.
    app.fr_aggs.insert(
        (0, slot, FrOccupant::Frame(ix)),
        FrFrameAgg {
            bus: 0,
            slot,
            count: 3,
            ab: 0,
            occupant: FrOccupant::Frame(ix),
            ..Default::default()
        },
    );
    app.fr_aggs.insert(
        (1, slot, FrOccupant::Unknown),
        FrFrameAgg {
            bus: 1,
            slot,
            count: 3,
            ab: 0,
            occupant: FrOccupant::Unknown,
            ..Default::default()
        },
    );
    app.refresh_snapshot();
    app.msg_windows[0].dbc_only = true;
    app.text_fresh = true;
    app.sync_msg_text(0);
    let buses: Vec<String> = app.msg_windows[0]
        .text_rows
        .iter()
        .map(|r| r.bus.clone())
        .collect();
    assert!(
        buses.iter().any(|b| b == "FR0"),
        "the described cluster's row is still listed: {buses:?}"
    );
    assert!(
        !buses.iter().any(|b| b == "FR1"),
        "the row no description covers is not: {buses:?}"
    );
    app.stop();
}

/// Reads a bundled cluster description the way the product does. The real ARXML
/// export is GBK, so `read_to_string` fails on it -- and a test written as
/// "skip when the read fails" then silently never runs. These assets are
/// committed, so a failure here is a broken test, not a stripped checkout.
fn read_fr_asset(path: &str) -> String {
    let bytes = std::fs::read(path).unwrap_or_else(|e| panic!("{path} is committed: {e}"));
    crate::dbc::text_from_bytes(bytes)
}

/// The trace text filter matches FlexRay frame names from the watch's
/// description database (and slot numbers), instead of hiding FR rows
/// wholesale. Value conditions still exclude them.
#[test]
fn the_trace_text_filter_matches_fr_frame_names() {
    let mut app = quiet_app();
    let text = read_fr_asset("assets/arxml/PowerTrain.arxml");
    let db = crate::fr_db::FrDb::parse(&text).expect("PowerTrain.arxml parses");
    app.fr_buses.insert(
        0,
        crate::app::FrBusCfg {
            path: "assets/arxml/PowerTrain.arxml".into(),
            db: std::sync::Arc::new(db),
        },
    );
    // The database's first frame defines the identity the filter must
    // match: its name and its slot number.
    let first = &app.fr_db(0).expect("bus 0 has a description").frames[0];
    let name = first.name.clone();
    let slot = first.triggering.slot_id;
    let row = crate::trace::FrRow {
        bus: 0,
        t_us: 1_000,
        ab: 0,
        slot: slot as u16,
        cycle: first.triggering.base_cycle as u8,
        payload: vec![1, 2, 3],
        header_crc: 0,
        flags: 0,
        name: None,
    };

    let mk_flt = |app: &App, filter: &str| {
        let mut w = app.trace_windows[0].clone();
        w.filter = filter.to_string();
        w.filter_lens()
    };
    let flt = mk_flt(&app, &name);
    assert!(app.trace_fr_match(&flt, &row), "name match: {name}");
    let flt = mk_flt(&app, &name.to_lowercase());
    assert!(app.trace_fr_match(&flt, &row), "lowercase matches");
    let flt = mk_flt(&app, &format!("slot {slot}"));
    assert!(app.trace_fr_match(&flt, &row), "slot text matches");
    let flt = mk_flt(&app, "\u{7f}no-such-frame\u{7f}");
    assert!(!app.trace_fr_match(&flt, &row), "another name filters it out");
    let flt = mk_flt(&app, "EngineSpeed>=200");
    assert!(
        !app.trace_fr_match(&flt, &row),
        "a signal-value condition is CAN-only"
    );
    app.stop();
}

/// A CANoe ASC names the rows it logs even where no cluster description is
/// loaded, and the Trace table shows that name -- so the text filter has to
/// match the name the row *displays*, not just the description's. The visible
/// slot number counts too: it is what the address column shows.
#[test]
fn the_trace_text_filter_matches_a_log_carried_fr_frame_name() {
    let mut app = quiet_app();
    assert!(
        app.fr_buses.is_empty(),
        "this case is about a row with no description behind it"
    );
    let row = crate::trace::FrRow {
        bus: 0,
        t_us: 1_000,
        ab: 2,
        slot: 47,
        cycle: 3,
        payload: vec![1],
        header_crc: 0,
        flags: 0,
        name: Some("Frame_47_3_1".into()),
    };
    let flt = |app: &App, filter: &str| {
        let mut w = app.trace_windows[0].clone();
        w.filter = filter.to_string();
        w.filter_lens()
    };
    assert!(
        app.trace_fr_match(&flt(&app, "frame_47"), &row),
        "the log's own name matches, case-insensitively"
    );
    assert!(
        app.trace_fr_match(&flt(&app, "47"), &row),
        "so does the slot number the address column shows"
    );
    assert!(
        !app.trace_fr_match(&flt(&app, "Frame_13"), &row),
        "another frame's name still filters it out"
    );
    app.stop();
}

/// Slot numbers repeat across clusters, so a table showing two of them cannot
/// be read as one network. `FlexRay: FR{n}` in the scope combo narrows a
/// Trace/Messages/Statistics window to that cluster -- the same number its Bus
/// column prints -- and CAN rows leave with it. Before this the only way to
/// look at one of a two-cluster recording was the text filter, which cannot
/// name a row that has no name.
#[test]
fn an_analysis_window_can_be_scoped_to_one_flexray_cluster() {
    use crate::aggregate::{FrFrameAgg, FrOccupant};
    let mut app = App::headless();
    for (bus, slot) in [(0u8, 13u16), (0, 24), (1, 13)] {
        app.fr_aggs.insert(
            (bus, slot, FrOccupant::Unknown),
            FrFrameAgg {
                bus,
                slot,
                count: 10,
                // ab = 2 ("unknown"): the Messages label would otherwise print
                // the channel letter after the slot, which is not what this
                // test is about.
                ab: 2,
                ..Default::default()
            },
        );
    }
    app.aggs.insert(
        (0, 0x100, false),
        MessageAgg {
            id: 0x100,
            channel: 0,
            extended: false,
            dir: Direction::Rx,
            count: 5,
            rx: 5,
            tx: 0,
            last_t_us: 0,
            cycle_us: 0.0,
            min_us: 0.0,
            max_us: 0.0,
            jitter_us: 0.0,
            len: 8,
            data: [0; MAX_CAN_FD_LEN],
            flags: FrameFlags::NONE,
        },
    );
    app.refresh_snapshot();
    assert_eq!(app.snap.fr_aggs.len(), 3, "one tally per bus and slot");

    // The combo offers every cluster the windows could show. Nothing is
    // configured here -- the rows alone make both clusters pickable.
    assert_eq!(app.fr_scope_buses(SigScope::All), vec![0, 1]);
    // A choice whose rows have all aged out stays listed, or the combo would
    // drop the entry the window is looking at.
    assert_eq!(app.fr_scope_buses(SigScope::FrBus(3)), vec![0, 1, 3]);

    let fr_row = |bus: u8, slot: u16| crate::trace::FrRow {
        bus,
        t_us: 1_000,
        ab: 0,
        slot,
        cycle: 0,
        payload: vec![0],
        header_crc: 0,
        flags: 0,
        name: None,
    };
    let flt = |app: &App, scope: SigScope| {
        let mut w = app.trace_windows[0].clone();
        w.scope = scope;
        w.filter_lens()
    };
    let (r0, r1) = (fr_row(0, 13), fr_row(1, 13));
    assert!(app.trace_fr_match(&flt(&app, SigScope::All), &r0));
    assert!(
        app.trace_fr_match(&flt(&app, SigScope::FrBus(1)), &r1),
        "the scoped cluster's rows stay"
    );
    assert!(
        !app.trace_fr_match(&flt(&app, SigScope::FrBus(1)), &r0),
        "the other cluster is out, same slot number and all"
    );
    assert!(
        !app.trace_fr_match(&flt(&app, SigScope::Bus(0)), &r0),
        "a CAN channel scope is still CAN-only"
    );
    let mut w = app.trace_windows[0].clone();
    w.scope = SigScope::FrBus(1);
    let can = frame_at(1_000, 0x100, 8, Direction::Rx);
    assert!(
        !app.trace_match(&w, &can),
        "an FR cluster scope keeps CAN out of the table"
    );

    // The Messages table: one cluster's slots, and not the CAN row.
    let shown = |app: &App| -> Vec<(String, String)> {
        app.msg_windows[0]
            .text_rows
            .iter()
            .map(|r| (r.bus.clone(), r.label.clone()))
            .collect()
    };
    app.msg_windows[0].scope = SigScope::FrBus(1);
    app.text_fresh = true;
    app.sync_msg_text(0);
    assert_eq!(shown(&app), vec![("FR1".to_string(), "slot 13".to_string())]);
    app.msg_windows[0].scope = SigScope::FrBus(0);
    app.text_fresh = true;
    app.sync_msg_text(0);
    assert_eq!(
        shown(&app),
        vec![
            ("FR0".to_string(), "slot 13".to_string()),
            ("FR0".to_string(), "slot 24".to_string())
        ],
        "cluster 0 alone, its two slots in slot order"
    );

    // Statistics: the share column is that cluster's own traffic once scoped,
    // not a slice of everything the run carried.
    app.stats_windows[0].scope = SigScope::FrBus(1);
    app.text_fresh = true;
    app.sync_stats_text(0);
    let stats = &app.stats_windows[0].text_rows;
    assert_eq!(stats.len(), 1, "one row for that cluster");
    assert_eq!((stats[0].bus.as_str(), stats[0].share.as_str()), ("FR1", "100.0%"));
    app.stats_windows[0].scope = SigScope::All;
    app.text_fresh = true;
    app.sync_stats_text(0);
    let shares: Vec<(&str, &str)> = app.stats_windows[0]
        .text_rows
        .iter()
        .map(|r| (r.bus.as_str(), r.share.as_str()))
        .collect();
    assert_eq!(shares.len(), 4, "CAN row plus three slots: {shares:?}");
    assert_eq!(
        shares,
        vec![
            ("CAN1", "14.3%"),
            ("FR0", "28.6%"),
            ("FR0", "28.6%"),
            ("FR1", "28.6%")
        ],
        "unscoped, all 35 frames of the run are shared out"
    );

    // And that number is a *name* once the user gives the 路 one: every table
    // that says which bus a row came from follows it, so a renamed cluster does
    // not reappear as an anonymous FR0 two windows down.
    app.set_flexray_name(0, "动力总成");
    app.msg_windows[0].scope = SigScope::FrBus(0);
    app.text_fresh = true;
    app.sync_msg_text(0);
    assert_eq!(
        shown(&app),
        vec![
            ("动力总成".to_string(), "slot 13".to_string()),
            ("动力总成".to_string(), "slot 24".to_string())
        ],
        "the Messages Bus column prints the name"
    );
    app.stats_windows[0].scope = SigScope::All;
    app.text_fresh = true;
    app.sync_stats_text(0);
    assert!(
        app.stats_windows[0]
            .text_rows
            .iter()
            .filter(|r| r.bus == "动力总成")
            .count()
            == 2,
        "both of cluster 0's slots, and no other row: {:?}",
        app.stats_windows[0]
            .text_rows
            .iter()
            .map(|r| (r.bus.clone(), r.label.clone()))
            .collect::<Vec<_>>()
    );
    app.stop();
}

/// R2 装配层校验：脚本发送了不属于自己节点的报文 → 节点日志出 warning。
/// 脚本绑定 EngineECU，但 send 的是 0x200（ChassisECU 的 VehicleState）。
#[test]
fn a_send_to_another_nodes_message_logs_a_warning() {
    let mut app = App::headless();
    app.send(crate::bus::BusCommand::AddNode {
        name: "gateway".to_string(),
        channel: 0,
        attached: Some((0, "EngineECU".to_string())),
    });
    app.settle();
    let id = app.snap.nodes[0].id;
    app.send(crate::bus::BusCommand::SetNodeSource {
        id,
        source: "on timer 100 { send(0x200, 1); }".to_string(),
    });
    app.send(crate::bus::BusCommand::SetNodeEnabled { id, on: true });
    app.start_virtual();
    app.settle();
    // Give the node a moment to produce the warning.
    std::thread::sleep(std::time::Duration::from_millis(50));
    app.update();
    let node = app.snap.nodes.iter().find(|n| n.id == id).expect("node");
    assert!(
        node.log.iter().any(|l| l.contains("0x200") && l.contains("ChassisECU")),
        "log should warn about the foreign message: {:?}",
        node.log
    );
    app.stop();
}

#[test]
fn a_rewound_or_restarted_run_reveals_its_rows_at_once() {
    let mut app = quiet_app();
    if app.trace_windows.is_empty() {
        app.new_trace_window();
    }
    feed_rpm(&mut app, &[(10_000, 100.0), (30_000, 110.0)]);
    app.text_fresh = true;
    app.sync_trace_rows(0);
    assert_eq!(app.trace_windows[0].shown_t_us, 30_000);

    // A backward seek re-stamps frames below the watermark; those must not
    // wait for the gate -- only the fresh tail is batched.
    feed_rpm(&mut app, &[(5_000, 40.0)]);
    app.text_fresh = false;
    app.sync_trace_rows(0);
    assert_eq!(
        app.trace_revealed(&app.trace_windows[0]).count(),
        3,
        "the older-stamped row shows immediately"
    );
    app.stop();
}

#[test]
fn a_log_id_stays_silent_during_replay_and_returns_in_simulation() {
    // The log carries 0x100 at 100 ms. An active generator twin at 10 ms
    // must hold its breath for the run -- replaying a recording of this
    // same simulation used to interleave two senders of one signal -- while
    // an id the log lacks keeps injecting, so "stir a few frames in"
    // survives.
    let path = write_timed_asc("roxy_can_mute_twin.asc", 20, 100_000);
    let mut app = App::headless();
    // The sample config pre-populates a generator entry for the log's 0x100;
    // add one id the log lacks as the stirring control.
    // 角色开关允许发送：EngineECU 模拟（否则开关拦截发送）。
    app.set_node_role(0, "EngineECU", NodeRole::Simulated);
    let twin = app
        .tx_list
        .iter()
        .position(|t| t.channel == 0 && t.id == 0x100)
        .expect("sample config carries a 0x100 entry");
    {
        let tx = &mut app.tx_list[twin];
        tx.cycle_us = 10_000;
        tx.active = true;
    }
    app.add_tx(0, 0x123);
    {
        let tx = app.tx_list.last_mut().unwrap();
        tx.cycle_us = 10_000;
        tx.active = true;
    }
    app.load_log(&path.to_string_lossy());
    app.replay();
    assert!(matches!(app.mode, Mode::Replay), "the replay is running");
    assert!(
        app.replay_ids.contains(&(0u8, 0x100u32)),
        "the scan saw the log's id"
    );

    // ~2.4 s of replay clock covers the whole 1.9 s log.
    let mut now = 0u64;
    for _ in 0..240 {
        now += 10_000;
        app.tick(now);
    }

    assert_eq!(
        slots_of(&app, 0x100).len(),
        20,
        "exactly the log's own 0x100 frames, none injected"
    );
    let stirred = slots_of(&app, 0x123);
    assert!(
        stirred.len() > 100,
        "an id the log lacks still injects freely, got {}",
        stirred.len()
    );

    // Back on the simulated bus the twin transmits again from a clean start.
    app.stop();
    app.start_virtual();
    run_sim(&mut app, 30, 10_000);
    assert!(
        slots_of(&app, 0x100).len() >= 30,
        "the twin speaks again once the replay is over"
    );
    std::fs::remove_file(&path).ok();
}

#[test]
fn re_activating_a_generator_starts_from_now_not_from_history() {
    // The old activation path re-zeroed next_t_us, so the catch-up loop
    // re-emitted the whole off period with wave values at those stale
    // stamps -- Graphics read the refilled gap as a continuous wave.
    let mut app = quiet_app();
    app.add_tx(0, 0x300);
    let i = app.tx_list.len() - 1;
    app.tx_list[i].cycle_us = 10_000;
    app.set_tx_active(i, true);
    run_sim(&mut app, 100, 10_000); // transmits until t = 1 s

    // Off for two seconds of clock -- time moves, no frames go out.
    app.set_tx_active(i, false);
    let mut now = 1_000_000u64;
    while now < 2_990_000 {
        now += 10_000;
        app.sim_t_us = now;
        app.tick(now);
    }
    let before = slots_of(&app, 0x300).len();
    assert_eq!(
        before, 101,
        "one batch on the first tick, one per tick after"
    );

    // On again: anchored at the clock, the silent second stays silent.
    app.set_tx_active(i, true); // schedule starts at t = 2.99 s
    let end = now + 100 * 10_000;
    while now < end {
        now += 10_000;
        app.sim_t_us = now;
        app.tick(now);
    }
    let slots = slots_of(&app, 0x300);
    assert_eq!(slots.len(), 202, "the off period contributes no frames");
    assert!(
        slots.iter().all(|&t| t <= 1_000_000 || t >= 2_990_000),
        "no frame may be dated inside the off period"
    );
    app.stop();
}

#[test]
fn the_trace_ring_wraps_and_keeps_the_newest_limit_frames() {
    let mut app = App::headless();
    app.start_virtual();
    let extra = 100u64;
    let total: u64 = (crate::app::TRACE_LIMIT + extra as usize) as u64;
    let frames: Vec<CanFrame> = (0..total)
        .map(|i| frame_at(i * 1_000, 0x100, 2, Direction::Rx))
        .collect();
    receive(&mut app, total * 1_000, frames);
    assert_eq!(
        app.trace.len(),
        crate::app::TRACE_LIMIT,
        "the ring is capped"
    );
    assert_eq!(
        app.trace.iter().next().unwrap().t_us,
        extra * 1_000,
        "the frames beyond the cap are dropped from the front"
    );
    assert_eq!(app.trace.back().unwrap().t_us, (total - 1) * 1_000);
    // Chunked storage must not break reverse iteration of the view.
    let newest_two: Vec<u64> = app
        .snap
        .trace
        .iter()
        .rev()
        .take(2)
        .map(|f| f.t_us)
        .collect();
    assert_eq!(
        newest_two,
        vec![(total - 1) * 1_000, (total - 2) * 1_000],
        "the view's newest frames sit at the back, sealed chunks notwithstanding"
    );
}

#[test]
fn a_published_trace_view_stays_frozen_while_the_ring_moves_on() {
    let mut app = App::headless();
    app.start_virtual();
    receive(&mut app, 1_000, vec![frame_at(0, 0x100, 2, Direction::Rx)]);
    let view = app.snap.trace.clone();
    assert_eq!(view.len(), 1);

    // The run goes on: the ring grows, the published view does not --
    // the UI keeps reading a stable instant while the bus advances.
    receive(
        &mut app,
        2_000,
        vec![frame_at(1_000, 0x100, 2, Direction::Rx)],
    );
    assert_eq!(view.len(), 1, "the published view is frozen at its instant");
    assert_eq!(app.trace.len(), 2);
    assert_eq!(app.snap.trace.len(), 2, "the next snapshot sees the growth");
}

/// Loading a second cluster's description is not the same act as opening a
/// second port: a recording can hold two clusters and the second one needs its
/// own description before its frames can be named, decoded, subscribed or
/// exported -- with no hardware in the loop. A file that cannot be read is
/// reported and takes no bus index with it.
#[test]
fn a_second_cluster_description_loads_without_any_hardware() {
    let first = "assets/arxml/PowerTrain.arxml";
    let second = "assets/fibex/PowerTrain_v2.xml";
    if !std::path::Path::new(first).exists() || !std::path::Path::new(second).exists() {
        println!("assets absent -- skipped");
        return;
    }
    let mut app = quiet_app();
    assert_eq!(app.load_cluster_description(first, None), Some(0));
    assert_eq!(app.load_cluster_description(second, None), Some(1));
    assert_eq!(
        app.load_cluster_description("assets/no-such-cluster.xml", None),
        None,
        "a file that cannot be read is refused"
    );
    assert!(app.status.contains("读取失败"), "{}", app.status);
    assert_eq!(
        app.fr_buses.keys().copied().collect::<Vec<_>>(),
        [0, 1],
        "each description got its own bus"
    );
    assert_eq!(
        app.fr_dbs.keys().copied().collect::<Vec<_>>(),
        [0, 1],
        "and the core was told about both, so a replay decodes per cluster"
    );
    app.stop();
}

/// Which FlexRay 路 a description file describes is the user's call: a
/// recording numbers its clusters 0/1 and names nothing, and a 路 given the
/// wrong schedule keeps working -- it just names and decodes every frame of
/// one network against the other network's slots. That is the one failure mode
/// worse than having no names at all, so the file lands where it is pointed
/// rather than on "the first 路 without one", in whatever order the files
/// happened to be picked. A 路 with a port open refuses the swap, because its
/// watch was configured from the description already there.
#[test]
fn a_description_lands_on_the_bus_it_is_pointed_at() {
    let first = "assets/arxml/PowerTrain.arxml";
    let second = "assets/fibex/PowerTrain_v2.xml";
    if !std::path::Path::new(first).exists() || !std::path::Path::new(second).exists() {
        println!("assets absent -- skipped");
        return;
    }
    let mut app = quiet_app();
    let sig = |app: &App, bus: u8| -> (String, usize) {
        let db = app.fr_db(bus).expect("that 路 has a description");
        (db.params.name.clone(), db.frames.len())
    };
    assert_eq!(app.load_cluster_description(first, Some(1)), Some(1));
    assert_eq!(app.load_cluster_description(second, Some(0)), Some(0));
    let (on_first, on_second) = (sig(&app, 1), sig(&app, 0));
    assert_ne!(on_first, on_second, "the two assets are told apart");
    // Replacing is how a misplaced file gets moved off a 路.
    assert_eq!(app.load_cluster_description(first, Some(0)), Some(0));
    assert_eq!(sig(&app, 0), on_first, "bus 0 now carries the file it was given");
    assert_eq!(
        app.fr_dbs.keys().copied().collect::<Vec<_>>(),
        [0, 1],
        "the core still has one description per 路"
    );
    assert_eq!(sig(&app, 1), on_first, "bus 1 already had this one");
    // A watched 路 refuses, and keeps what its port runs from.
    let _q = app.hw.attach_fr_mock(1, 5);
    app.refresh_snapshot();
    assert_eq!(app.load_cluster_description(second, Some(1)), None);
    assert!(
        app.status.contains(crate::channel::DETACH_LABEL),
        "the refusal names the button that can do it: {}",
        app.status
    );
    assert_eq!(sig(&app, 1), on_first, "the live 路 kept its file");
    // The rows the Buses table lists: the 路 that exist as state -- the two
    // configured ones. No placeholder for "the next free index"; a bus nobody
    // added is not a bus, and "+ Add FlexRay" is what adds one.
    assert_eq!(app.fr_bus_rows(), [0, 1]);
    assert_eq!(
        app.next_flexray_bus(),
        Some(2),
        "the add button would place the next description on the first empty index"
    );
    // A 路 that exists only as traffic in the log being replayed is a row: its
    // frames cannot be named until a description lands on it, and that is what
    // its `加载描述…` button is for. Listed in bus order, not appended.
    app.fr_loads.insert(4, crate::load::FrLoad::default());
    // The rollups are only re-published when they changed; a hand-inserted load
    // has to say so, which is what a step does on every arrival.
    app.loads_dirty = true;
    app.refresh_snapshot();
    assert_eq!(app.fr_bus_rows(), [0, 1, 4]);
    app.stop();
}

/// A 路 that has a description but no port is attached with the file it already
/// holds (no second picker); `断开` closes that port and leaves the 路 alone; and
/// removal takes the whole 路, port included -- which a watched 路 allows
/// precisely because closing the port is part of what removal means.
#[test]
fn a_flexray_bus_can_be_detached_and_removed() {
    let arxml = "assets/arxml/PowerTrain.arxml";
    if !std::path::Path::new(arxml).exists() {
        panic!("{arxml} missing -- it is tracked in the repository, so a broken checkout must fail, not skip");
    }
    let mut app = quiet_app();
    assert_eq!(app.load_cluster_description(arxml, None), Some(0));

    // Attaching uses the loaded description: what comes back is the missing
    // driver, not a missing file.
    app.attach_fr_watch_on(0, 5);
    assert!(app.status.contains("FR0"), "{}", app.status);
    assert!(
        !app.status.contains("没有集群描述"),
        "the description was there to use: {}",
        app.status
    );
    // A bus with nothing loaded cannot be attached.
    app.attach_fr_watch_on(3, 5);
    assert!(app.status.contains("没有集群描述"), "{}", app.status);

    // 断开 closes the port and keeps the 路: it is still the cluster its
    // description describes, so nothing pointed at it has to move.
    app.hw.attach_fr_mock(0, 5);
    app.refresh_snapshot();
    app.detach_flexray_watch(0);
    assert!(app.snap.fr_watches.is_empty(), "the port closed");
    assert!(app.fr_buses.contains_key(&0), "the 路 stayed");
    assert!(
        app.fr_dbs.contains_key(&0),
        "and so did the core's copy of its description"
    );

    // Removal is the whole 路, no prior 断开 needed.
    app.hw.attach_fr_mock(0, 5);
    app.refresh_snapshot();
    app.remove_fr_bus(0);
    assert!(app.fr_buses.is_empty(), "the frontend forgot it");
    assert!(app.fr_dbs.is_empty(), "and told the core");
    assert!(app.snap.fr_watches.is_empty(), "the port went with it");
    assert!(app.status.contains("FR0"), "the bar names it: {}", app.status);
    app.stop();
}

/// Deleting a FlexRay 路 takes everything keyed on its cluster index with it --
/// the frames it received, their tallies, its subscriptions, curves, hand-picked
/// rows, send entries and rules -- while the surviving 路 keeps both its number
/// and its data. The number is the point: a cluster index is an identity, not a
/// position in a list. A BLF stamps each row with its own `clusterNo`, a script
/// types `fr_sig(1, ..)` and the record filter types `FR1:5` with it, so
/// shifting the survivors down would silently re-aim every one of those at the
/// wrong network -- and the gap is not a problem, since the next 路 added takes
/// the lowest free index anyway.
#[test]
fn removing_a_flexray_bus_takes_everything_keyed_on_it() {
    use crate::hw::vector::flexray::FrFrame;
    use crate::observe::GfxSignal;
    use crate::workspace::{Pick, SigScope};
    let arxml = "assets/arxml/PowerTrain.arxml";
    let mut app = quiet_app();
    let (slot, _) = fr_bound_slot(&mut app);
    let db = std::sync::Arc::clone(&app.fr_buses[&0].db);
    app.fr_buses.insert(
        1,
        crate::app::FrBusCfg {
            path: arxml.into(),
            db,
        },
    );
    app.push_fr_db_to_core();
    let q0 = app.hw.attach_fr_mock(0, 5);
    let q1 = app.hw.attach_fr_mock(1, 6);
    // One reference of each kind, on each 路.
    for bus in [0u8, 1] {
        let key = SigKey::Fr {
            bus,
            slot,
            name: "CarSpeed".to_string(),
        };
        app.subscribe(key.clone());
        app.graphics[0].signals.push(GfxSignal {
            key,
            visible: true,
            y_mode: YMode::Auto,
        });
        app.trace_windows[0].manual.insert(Pick::Fr { bus, slot });
        app.add_fr_tx(bus, slot);
    }
    app.msg_windows[0].scope = SigScope::FrBus(0);
    app.stats_windows[0].scope = SigScope::FrBus(1);
    // A rule on each 路: removal has one to take away and one to leave alone.
    // `add_fr_trigger` aims at `fr_default_slot`, which reads the first 路 --
    // the one being deleted.
    app.add_fr_trigger();
    app.send(crate::bus::BusCommand::AddTrigger {
        cond: TriggerCond::FrSignalCross {
            bus: 1,
            slot,
            signal: "CarSpeed".to_string(),
            threshold: 0.0,
            rising: true,
        },
        action: TriggerAction::StartRecording,
    });
    let rules_on = |app: &App, bus: u8| -> usize {
        app.snap
            .triggers
            .iter()
            .filter(|t| t.cond.fr_bus() == Some(bus))
            .count()
    };
    assert_eq!(rules_on(&app, 0), 1, "the new FlexRay rule sits on FR0");
    assert_eq!(rules_on(&app, 1), 1, "and the other on FR1");
    app.start_virtual();
    for q in [&q0, &q1] {
        q.lock().expect("mock lock").push_back(FrFrame {
            slot,
            cycle: 0,
            payload: vec![0; 8],
            header_crc: 0,
            flags: 0,
        });
    }
    app.advance_clock(1_000);
    app.tick(1_000);
    app.refresh_snapshot();
    let rows = |app: &App| -> Vec<u8> {
        (0..app.snap.fr_trace.len())
            .filter_map(|i| app.snap.fr_trace.get(i))
            .map(|r| r.bus)
            .collect()
    };
    assert_eq!(rows(&app), [0, 1], "both 路 carried traffic");

    // A spec verdict and its interval memory on each 路, so the report has one
    // row to lose and one to keep. Planted after the last step, on a slot no
    // traffic fills: the monitor rewrites both tables from the arrivals on every
    // step, so anything planted before the run is the monitor's data, not the
    // test's.
    let phantom = slot + 1;
    for bus in [0u8, 1] {
        app.spec.record_fr(
            (bus, phantom, None),
            crate::spec::Kind::Cycle,
            1_000,
            5.0,
            9.0,
        );
        app.spec.note_fr((bus, phantom, 0), 1_000);
    }
    // The port each row's picker last chose, mirroring the mock watches.
    app.fr_pick.insert(0, 5);
    app.fr_pick.insert(1, 6);

    app.remove_fr_bus(0);
    app.refresh_snapshot();

    // The survivor kept its index, on both sides.
    assert_eq!(app.fr_buses.keys().copied().collect::<Vec<_>>(), [1]);
    assert_eq!(app.fr_dbs.keys().copied().collect::<Vec<_>>(), [1]);
    assert_eq!(
        app.fr_bus_rows(),
        [1],
        "the Buses table has one 路 left, and it is still FR1"
    );
    assert_eq!(
        app.fr_pick.get(&1).copied(),
        Some(6),
        "FR1's picker still names the port it chose"
    );
    assert!(
        !app.fr_pick.contains_key(&0),
        "the deleted 路 leaves no port choice for whoever takes the index later"
    );
    // Its traffic stayed, the deleted 路's went.
    assert_eq!(rows(&app), [1], "FR0's rows are out of the ring");
    assert_eq!(
        app.fr_aggs.values().filter(|a| a.bus == 0).count(),
        0,
        "and its tallies with them"
    );
    assert!(
        app.fr_aggs.values().any(|a| a.bus == 1 && a.count >= 1),
        "FR1 is still tallied: {:?}",
        app.fr_aggs.values().map(|a| (a.bus, a.count)).collect::<Vec<_>>()
    );
    // No subscription, curve or hand-picked row names the deleted cluster.
    assert!(
        !app.subs.keys().any(|k| matches!(k, SigKey::Fr { bus: 0, .. })),
        "FR0's subscription is gone"
    );
    assert!(app.subs.contains_key(&SigKey::Fr {
        bus: 1,
        slot,
        name: "CarSpeed".to_string()
    }));
    assert_eq!(
        app.graphics[0]
            .signals
            .iter()
            .map(|s| match &s.key {
                SigKey::Fr { bus, .. } => *bus,
                SigKey::Can { .. } => 255,
            })
            .collect::<Vec<_>>(),
        [1],
        "only FR1's curve is left"
    );
    assert_eq!(
        app.trace_windows[0]
            .manual
            .iter()
            .copied()
            .collect::<Vec<_>>(),
        [Pick::Fr { bus: 1, slot }],
        "and only FR1's picked row"
    );
    // Its send entries and its rule.
    assert_eq!(
        app.snap.fr_tx.iter().map(|t| t.bus).collect::<Vec<_>>(),
        [1],
        "FR0's generator entry went with it"
    );
    // The spec report loses the deleted 路's verdicts and keeps the survivor's.
    assert!(
        app.spec.fr_rows.keys().all(|(s, _)| s.0 != 0),
        "no verdict about a cluster that is gone"
    );
    assert_eq!(
        app.spec
            .fr_rows
            .keys()
            .filter(|(s, _)| s.0 == 1 && s.1 == phantom)
            .count(),
        1,
        "FR1's row is still reported"
    );
    assert_eq!(
        app.spec.fr_previous((0, phantom, 0)),
        None,
        "and FR0's interval memory with it"
    );
    assert_eq!(app.spec.fr_previous((1, phantom, 0)), Some(1_000));
    assert_eq!(
        rules_on(&app, 0),
        0,
        "a rule about a cluster that is gone can never fire"
    );
    assert_eq!(rules_on(&app, 1), 1, "FR1's rule is untouched");
    // The window pointed at the deleted cluster falls back to the widest view;
    // the one pointed at the survivor is nobody's business.
    assert_eq!(app.msg_windows[0].scope, SigScope::All);
    assert_eq!(app.stats_windows[0].scope, SigScope::FrBus(1));
    // The gap is what the next added 路 uses.
    assert_eq!(app.next_flexray_bus(), Some(0));
    assert!(app.status.contains("FR0"), "{}", app.status);
    app.stop();
}
