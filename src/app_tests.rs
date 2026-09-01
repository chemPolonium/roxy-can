use super::*;
// The tests rely on the parent's imports through `use super::*`; the
// ones app.rs itself no longer needs are imported directly here.
use crate::can::frame::{FrameFlags, MAX_CAN_FD_LEN};
use crate::config::Config;
use crate::log::AscWriter;
use crate::sim::ValueSrc;
use crate::trigger::{Trigger, TriggerAction, TriggerCond};

#[test]
fn record_survives_start_and_writes_frames() {
    let mut app = App::new();
    let path = std::env::temp_dir().join("roxy_can_record_test.asc");
    app.recorder.record_path = path.to_string_lossy().to_string();
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

#[test]
fn replay_after_recorded_simulation_creates_no_second_file() {
    let mut app = App::new();
    let path = std::env::temp_dir().join("roxy_can_replay_rec_test.asc");
    app.recorder.record_path = path.to_string_lossy().to_string();
    app.toggle_record();
    for tx in &mut app.tx_list {
        tx.active = true;
        tx.cycle_us = 10_000;
    }
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

#[test]
fn loading_log_does_not_start_replay() {
    let mut app = App::new();
    let path = std::env::temp_dir().join("roxy_can_load_asc_test.asc");
    app.recorder.record_path = path.to_string_lossy().to_string();
    app.toggle_record();
    for tx in &mut app.tx_list {
        tx.active = true;
        tx.cycle_us = 10_000;
    }
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
    let mut app = App::new();
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
    let mut app = App::new();
    app.open_dropped(std::path::Path::new("/tmp/does-not-exist.mf4"));
    assert!(
        app.status.contains("unsupported format: MF4"),
        "MF4 should surface a clear reason, got {:?}",
        app.status
    );
}

#[test]
fn aggregates_frames_per_message_id() {
    let mut app = App::new();
    let tx = app
        .tx_list
        .iter_mut()
        .find(|t| t.id == 0x100)
        .expect("EngineStatus pre-populated in generator");
    tx.active = true;
    tx.cycle_us = 10_000;
    app.start_virtual();
    for _ in 0..8 {
        std::thread::sleep(std::time::Duration::from_millis(12));
        app.update();
    }
    let agg = app
        .aggs
        .get(&(0, 0x100))
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
    let mut app = App::new();
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
    let agg = app.aggs.get(&(0, 0x777)).expect("aggregate");
    assert_eq!((agg.min_us, agg.max_us), (20_000.0, 20_000.0));
    app.stop();
}

#[test]
fn a_tick_exactly_on_a_slot_does_not_drop_a_cycle() {
    let mut app = App::new();
    app.add_tx(0, 0x779);
    let tx = app.tx_list.last_mut().unwrap();
    tx.cycle_us = 20_000;
    tx.active = true;
    app.start_virtual();
    // Ticks land precisely on slot boundaries.
    run_sim(&mut app, 6, 20_000);
    assert_eq!(
        slots_of(&app, 0x779),
        vec![0, 20_000, 40_000, 60_000, 80_000, 100_000],
        "one frame per cycle, none skipped at the boundary"
    );
    app.stop();
}

#[test]
fn a_stalled_ui_drops_cycles_instead_of_bursting() {
    let mut app = App::new();
    app.add_tx(0, 0x778);
    let tx = app.tx_list.last_mut().unwrap();
    tx.cycle_us = 20_000;
    tx.active = true;
    app.start_virtual();
    app.sim_t_us = 0;
    app.tick(0);
    // Twelve cycles' worth of stall must produce one frame, and the
    // schedule must end up past the stall rather than queued behind it.
    app.sim_t_us = 250_000;
    app.tick(250_000);
    assert_eq!(slots_of(&app, 0x778), vec![0, 20_000], "one frame per tick");
    assert_eq!(
        app.tx_list.last().unwrap().next_t_us,
        260_000,
        "the backlog was skipped, not deferred"
    );
    app.stop();
}

#[test]
fn a_pause_freezes_the_simulation_clock_and_its_phase() {
    let mut app = App::new();
    app.start_virtual();
    app.update();
    let before = app.sim_t_us;
    // Age the wall clock by 400 ms the way a real pause would, then check
    // the simulation clock neither moves during the pause nor absorbs the
    // paused span afterwards.
    app.trace_paused = true;
    app.t0 = Instant::now() - std::time::Duration::from_millis(400);
    app.update();
    assert_eq!(app.sim_t_us, before, "a paused clock must not advance");
    app.trace_paused = false;
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
    let mut app = App::new();
    app.channels[0].dbc = Some(crate::dbc::load_dbc_str(WIDE_DBC).expect("wide dbc parses"));
    app.add_tx(0, 0x300);
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
    let app = App::new();
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
    let mut app = App::new();
    app.channels[0].dbc = Some(crate::dbc::load_dbc_str(CYCLE_TEST_DBC).unwrap());
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
    let mut app = App::new();
    app.channels[0].sim_nodes.push("EngineECU".to_string());
    app.channels[1].sim_nodes.push("ABS".to_string());
    app.remove_channel(0);
    assert_eq!(
        app.channels[0].sim_nodes,
        ["ABS"],
        "the survivor keeps its own nodes instead of inheriting the deleted bus's"
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

fn entry_of(app: &App, ch: u8, id: u32) -> &TxMsg {
    app.tx_list
        .iter()
        .find(|t| t.channel == ch && t.id == id)
        .expect("entry exists")
}

#[test]
fn ticking_a_node_activates_only_its_own_messages() {
    let mut app = App::new();
    app.set_node_sim(1, "ABS", true);
    // assets/motbus.dbc:31,35,54 -- ABS owns these three, nobody else.
    assert_eq!(active_ids(&app, 1), [199, 200, 201]);
    assert!(active_ids(&app, 0).is_empty(), "the other bus untouched");
    assert!(
        app.tx_list
            .iter()
            .filter(|t| t.channel == 1 && t.active)
            .all(|t| t.next_t_us == 0),
        "a ticked node starts on the next tick, not one period later"
    );
    assert!(app.is_node_simulated(1, "ABS"));
    assert!(!app.is_node_simulated(1, "GearBox"), "not a side effect");
}

#[test]
fn ticking_a_node_creates_the_entries_it_lacks() {
    let mut app = App::new();
    app.tx_list.clear();
    app.set_node_sim(1, "ABS", true);
    assert_eq!(
        active_ids(&app, 1),
        [199, 200, 201],
        "the generator refills from the DBC"
    );
    assert_eq!(app.tx_list.len(), 3, "and only this node's messages");
    assert_eq!(entry_of(&app, 1, 201).name, "ABSdata");
    assert_eq!(entry_of(&app, 1, 201).cycle_us, 50_000);
}

#[test]
fn ticking_a_node_never_overwrites_a_tuned_cycle() {
    let mut app = App::new();
    let tuned = entry_of(&app, 1, 201).cycle_us;
    assert_eq!(tuned, 50_000, "what the DBC declares");
    let i = app
        .tx_list
        .iter()
        .position(|t| t.channel == 1 && t.id == 201)
        .unwrap();
    app.tx_list[i].cycle_us = 250_000;
    app.set_node_sim(1, "ABS", true);
    assert_eq!(
        app.tx_list[i].cycle_us, 250_000,
        "a period someone dialed in outlives the click"
    );
    assert!(app.tx_list[i].active, "but the entry is switched on");
}

#[test]
fn unticking_a_node_keeps_its_entries_and_their_stimulus() {
    let mut app = App::new();
    app.set_node_sim(1, "ABS", true);
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

    app.set_node_sim(1, "ABS", false);
    assert!(active_ids(&app, 1).is_empty(), "stopped sending");
    assert_eq!(app.tx_list.len(), before, "entries survive");
    assert_eq!(app.tx_list[i].srcs.len(), 1, "with the waveform attached");
    assert_eq!(app.tx_list[i].cycle_us, 50_000, "and the declared period");

    app.set_node_sim(1, "ABS", true);
    assert_eq!(
        app.tx_list[i].srcs.len(),
        1,
        "ticking it back on does not rebuild the entry"
    );
    assert!(app.tx_list[i].active);
}

/// Loading a different database does not rebuild the generator, so the
/// only thing left to go by is the node stamped on each entry.
#[test]
fn unticking_still_silences_a_node_after_its_dbc_is_gone() {
    let mut app = App::new();
    app.set_node_sim(1, "ABS", true);
    assert_eq!(active_ids(&app, 1).len(), 3);
    app.channels[1].dbc = None;
    app.set_node_sim(1, "ABS", false);
    assert!(
        active_ids(&app, 1).is_empty(),
        "unchecking must work even with nothing to look up"
    );
    assert!(!app.is_node_simulated(1, "ABS"));
}

#[test]
fn a_receive_only_node_can_still_be_ticked() {
    let mut app = App::new();
    app.set_node_sim(1, "DashBoard", true);
    assert!(active_ids(&app, 1).is_empty(), "it has no messages to send");
    assert!(
        app.is_node_simulated(1, "DashBoard"),
        "the intent is remembered anyway"
    );
}

/// The restore chip compares a row against this, so it has to stay the
/// database's own opinion even after the row disagrees with it.
#[test]
fn the_declared_cycle_survives_a_hand_tuned_row() {
    let mut app = App::new();
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

    app.channels[0].dbc = Some(crate::dbc::load_dbc_str(CYCLE_TEST_DBC).unwrap());
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
    let mut app = App::new();
    app.channels[0].dbc = Some(crate::dbc::load_dbc_str(NO_OWNER_DBC).unwrap());
    app.tx_list.retain(|t| t.channel != 0);
    app.add_tx(0, 4096);
    assert_eq!(entry_of(&app, 0, 4096).node, "", "unassigned");

    app.set_node_sim(0, "", true);
    assert!(
        app.channels[0].sim_nodes.is_empty(),
        "not even recorded as a tick"
    );
    assert!(
        active_ids(&app, 0).is_empty(),
        "an empty name must not adopt every message without an owner"
    );
}

#[test]
fn tx_generator_emits_frames() {
    let mut app = App::new();
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
        app.aggs.contains_key(&(0, 0x777)),
        "generator frames aggregate"
    );
    app.stop();
}

#[test]
fn export_trace_writes_parseable_asc() {
    let mut app = App::new();
    for tx in &mut app.tx_list {
        tx.active = true;
        tx.cycle_us = 10_000;
    }
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

/// The report carries its own premises -- which databases, the tolerance
/// and grace in effect, how many messages declared a period -- plus every
/// latched row. Without the header, the same table means something else
/// at a different threshold setting and cannot be re-checked.
#[test]
fn the_spec_report_export_includes_its_premises_and_rows() {
    let mut app = App::new();
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
            .contains_key(&(0, 0x777, crate::spec::Kind::Unknown))
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
    let mut app = App::new();
    assert_eq!(app.channels.len(), 2);
    for (ch, c) in app.channels.iter().enumerate() {
        assert!(c.dbc.is_some(), "CAN{} should load its DBC", ch + 1);
    }
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
    let a = app.aggs.get(&(0, 0x100)).expect("CAN1 aggregate");
    let b = app.aggs.get(&(1, 0xC8)).expect("CAN2 aggregate");
    assert!(a.count >= 3, "CAN1 frames: {}", a.count);
    assert!(b.count >= 3, "CAN2 frames: {}", b.count);
    assert!(app.trace.iter().any(|f| f.channel == 1 && f.id == 0xC8));
    app.stop();
}

#[test]
fn csv_exports_match_window_state() {
    let mut app = App::new();
    let db = app.channels[0].dbc.as_ref().expect("sample DBC loaded");
    let id = db.order[0];
    let sig = db.messages[&id].signals[0].name.clone();
    let key = (0u8, id, sig);
    app.subscribe(key.clone());
    app.graphics[0].signals.push(GfxSignal {
        key: key.clone(),
        visible: true,
    });
    app.data_windows[0].signals.push(GfxSignal {
        key: key.clone(),
        visible: true,
    });
    for tx in &mut app.tx_list {
        tx.active = true;
        tx.cycle_us = 10_000;
    }
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
    assert!(g.contains(&key.2), "graphics history names the signal");

    let data = dir.join("roxy_data_test.csv");
    app.export_data_csv(0, &data.to_string_lossy());
    let d = std::fs::read_to_string(&data).unwrap();
    assert!(d.contains(&key.2), "data snapshot names the signal");

    for p in [&stats, &msgs, &gfx, &data] {
        std::fs::remove_file(p).ok();
    }
    app.stop();
}

#[test]
fn trace_filter_matches_by_name_id_and_direction() {
    let app = App::new();
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
    w.manual.insert((0, 0x320));
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

#[test]
fn channels_can_be_added_removed_and_renamed() {
    let mut app = App::new();
    assert_eq!(app.channels.len(), 2);
    app.channels[0].name = "Powertrain".to_string();
    assert_eq!(app.channel_name(0), "Powertrain");

    app.add_channel();
    assert_eq!(app.channels.len(), 3);
    assert_eq!(app.channel_name(2), "CAN3");

    app.aggs.insert(
        (1, 0x100),
        MessageAgg {
            id: 0x100,
            channel: 1,
            extended: false,
            dir: Direction::Rx,
            count: 1,
            last_t_us: 0,
            cycle_us: 0.0,
            min_us: 0.0,
            max_us: 0.0,
            len: 8,
            data: [0; MAX_CAN_FD_LEN],
            flags: FrameFlags::NONE,
        },
    );
    app.trace_windows[0].manual.insert((2, 0x200));
    app.trace_windows[0].scope = SigScope::Bus(2);
    let w = app.trace_windows[0].clone();

    app.remove_channel(0);
    assert_eq!(app.channels.len(), 2);
    assert_eq!(app.channel_name(0), "CAN2", "remaining buses shift down");
    assert!(app.aggs.contains_key(&(0, 0x100)), "agg remapped 1 -> 0");
    assert!(
        app.trace_windows[0].manual.contains(&(1, 0x200)),
        "filter remapped 2 -> 1"
    );
    assert_eq!(w.scope, SigScope::Bus(2), "cloned window is untouched");
    assert_eq!(
        app.trace_windows[0].scope,
        SigScope::Bus(1),
        "Bus scope indices shift with the channels"
    );

    while app.channels.len() > 1 {
        app.remove_channel(0);
    }
    assert_eq!(app.channels.len(), 1, "last bus cannot be removed");
}

#[test]
fn replay_position_tracks_playback() {
    let mut app = App::new();
    let path = std::env::temp_dir().join("roxy_can_seek_test.asc");
    app.recorder.record_path = path.to_string_lossy().to_string();
    app.toggle_record();
    for tx in &mut app.tx_list {
        tx.active = true;
        tx.cycle_us = 10_000;
    }
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
fn seek_replay_moves_the_playhead() {
    let mut app = App::new();
    let path = write_timed_asc("roxy_can_scrub.asc", 100, 10_000);
    app.load_log(&path.to_string_lossy());
    app.replay();
    assert!(app.measuring);

    app.seek_replay_seconds(0.5);
    let (pos, dur) = app.replay_position().expect("replay has a timeline");
    assert!(
        (pos - 0.5).abs() < 1e-6,
        "playhead should land on the 0.5 s frame, got {pos}"
    );
    assert!(dur > 0.9, "timeline covers the log, got {dur}");

    // The first update after a seek only re-anchors the clock, so exactly
    // the landing frame is emitted -- a scrub must not dump the prefix.
    app.update();
    assert_eq!(app.trace.len(), 1, "no flood of skipped frames");
    assert_eq!(app.trace.back().unwrap().t_us, 500_000);

    app.seek_replay_seconds(0.1);
    app.update();
    assert_eq!(app.trace.len(), 2, "seeking backwards replays earlier rows");
    assert_eq!(app.trace.back().unwrap().t_us, 100_000);
    app.stop();
    std::fs::remove_file(&path).ok();
}

#[test]
fn play_after_a_scrub_resumes_in_place() {
    let mut app = App::new();
    let path = write_timed_asc("roxy_can_scrub_resume.asc", 100, 10_000);
    app.load_log(&path.to_string_lossy());
    app.replay();

    // Run the log out the far end without touching Stop.
    let (_, dur) = app.replay_position().unwrap();
    app.seek_replay_seconds(dur);
    app.update();
    app.update();
    assert!(!app.measuring, "the replay finished on its own");

    app.seek_replay_seconds(0.3);
    assert!(
        app.replay_position().is_some(),
        "the timeline must survive the end of the log so the scrub bar stays usable"
    );
    app.toggle_play();
    assert!(app.measuring, "Play resumes a finished, scrubbed replay");
    let (pos, _) = app.replay_position().unwrap();
    assert!(
        (pos - 0.3).abs() < 1e-6,
        "must continue from the scrubbed position, got {pos}"
    );
    app.stop();
    std::fs::remove_file(&path).ok();
}

#[test]
fn stop_makes_the_next_play_restart_from_zero() {
    let mut app = App::new();
    let path = write_timed_asc("roxy_can_scrub_stop.asc", 100, 10_000);
    app.load_log(&path.to_string_lossy());
    app.replay();
    app.seek_replay_seconds(0.5);
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
    let mut app = App::new();
    let path = write_timed_asc("roxy_can_plot_clock.asc", 100, 10_000);
    app.load_log(&path.to_string_lossy());
    app.replay();
    app.seek_replay_seconds(0.5);
    assert!(
        (app.plot_now_s() - 0.5).abs() < 1e-6,
        "the Graphics axis must track the scrub bar, got {}",
        app.plot_now_s()
    );
    app.seek_replay_seconds(0.2);
    assert!(
        (app.plot_now_s() - 0.2).abs() < 1e-6,
        "and track a rewind, got {}",
        app.plot_now_s()
    );
    app.stop();
    std::fs::remove_file(&path).ok();
}

#[test]
fn loading_another_log_mid_replay_is_refused() {
    let mut app = App::new();
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

    // Stopped, the same selection goes through and demands a fresh open.
    app.stop();
    app.load_log(&b.to_string_lossy());
    assert_eq!(app.log_path, b.to_string_lossy());
    assert!(
        app.replay_reset_pending,
        "a newly selected log must not be resumed over"
    );
    app.stop();
    std::fs::remove_file(&a).ok();
    std::fs::remove_file(&b).ok();
}

#[test]
fn play_after_choosing_a_new_log_opens_that_log() {
    let mut app = App::new();
    let a = write_timed_asc("roxy_can_switch_a.asc", 100, 10_000);
    let b = write_timed_asc("roxy_can_switch_b.asc", 20, 10_000);
    app.load_log(&a.to_string_lossy());
    app.replay();
    // Let the first log run to its natural end, which leaves the source
    // parked but replay-able -- exactly where the old code could resume the
    // wrong file.
    let (_, dur_a) = app.replay_position().unwrap();
    app.seek_replay_seconds(dur_a);
    app.update();
    app.update();
    assert!(!app.measuring, "setup: the first log finished on its own");

    app.load_log(&b.to_string_lossy());
    app.play();
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
        sub.push_sample(i * SAMPLE_INTERVAL_US, (i % 97) as f64);
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
        sub.push_sample(i * SAMPLE_INTERVAL_US, 1.0);
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
fn app_with_replayable_recording(name: &str, iters: usize) -> (App, (u8, u32, String), String) {
    let mut app = App::new();
    let key = {
        let db = app.channel_dbc(0).expect("sample DBC loaded");
        let id = db.order[0];
        (0u8, id, db.messages[&id].signals[0].name.clone())
    };
    app.subscribe(key.clone());
    let out = std::env::temp_dir().join(format!("roxy_can_{name}.asc"));
    app.recorder.record_path = out.to_string_lossy().to_string();
    app.toggle_record();
    for tx in &mut app.tx_list {
        tx.active = true;
        tx.cycle_us = 10_000;
    }
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
fn a_backward_scrub_rewinds_signal_state() {
    let (mut app, key, file) = app_with_replayable_recording("scrub_history", 60);
    app.replay();
    // Let the clock actually run so sampling fills history across the log;
    // a forward seek cannot do it, since seeking discards the prefix.
    app.set_replay_speed(4.0);
    let deadline = std::time::Instant::now() + std::time::Duration::from_millis(250);
    while std::time::Instant::now() < deadline {
        std::thread::sleep(std::time::Duration::from_millis(11));
        app.update();
    }
    let filled = app.subs.get(&key).expect("subscribed");
    assert!(
        filled.history.len() > 3,
        "expected samples across the log, got {}",
        filled.history.len()
    );
    let (_, dur) = app.replay_position().unwrap();

    app.seek_replay_seconds(dur / 3.0);
    let landed = app.replay_position().unwrap().0;
    let sub = app.subs.get(&key).unwrap();
    assert!(
        sub.history.iter().any(|(t, _)| *t as f64 / 1e6 > landed),
        "the cache keeps samples ahead of a rewound playhead; the window \
             slice hides them, and deleting them was what blanked the curve"
    );
    assert!(
        sub.history
            .iter()
            .zip(sub.history.iter().skip(1))
            .all(|(a, b)| a.0 <= b.0),
        "the cache must stay ascending for the binary search in value_at"
    );
    let after_rewind = sub.history.len();

    // Replaying across ground the cache already holds must not inject
    // near-duplicates: the sampler's own baseline was pulled back to the
    // rewind point, so only the cache's spacing rule keeps it honest.
    app.play();
    assert!(app.measuring, "Play resumes the rewound replay");
    app.set_replay_speed(4.0);
    for _ in 0..5 {
        std::thread::sleep(std::time::Duration::from_millis(11));
        app.update();
    }
    let (pos_now, _) = app.replay_position().unwrap();
    assert!(
        pos_now * 1e6 > landed,
        "the playhead should advance past the rewind point, got {pos_now}"
    );
    let sub = app.subs.get(&key).unwrap();
    assert_eq!(
        sub.history.len(),
        after_rewind,
        "replaying cached ground must add nothing, not even near-duplicates"
    );
    assert!(
        sub.history
            .iter()
            .zip(sub.history.iter().skip(1))
            .all(|(a, b)| a.0 <= b.0),
        "re-sampled history must remain ascending"
    );
    app.stop();
    std::fs::remove_file(&file).ok();
}

#[test]
fn sample_cache_stays_ascending_when_filled_from_either_end() {
    let mut c = SampleCache::default();
    // Streaming fills the later stretch first, then a backfill lands behind
    // it; the buffer must still read ascending for the plot and value_at.
    c.merge(
        &(100..110u64)
            .map(|i| (i * 1_000, i as f64))
            .collect::<Vec<_>>(),
        1_000,
    );
    c.merge(
        &(0..10).map(|i| (i * 1_000, i as f64)).collect::<Vec<_>>(),
        1_000,
    );
    assert_eq!(c.len(), 20);
    assert!(
        c.iter().zip(c.iter().skip(1)).all(|(a, b)| a.0 <= b.0),
        "merge behind existing points must keep the buffer sorted"
    );
    assert_eq!(c.first().unwrap().0, 0);
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
        win.iter().map(|(t, _)| *t).collect::<Vec<_>>(),
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
fn overlapping_backfills_do_not_pile_up_near_duplicates() {
    let (mut app, key, file) = app_with_replayable_recording("dupstride", 60);
    app.replay();
    // Two requests overlapping by most of their span -- exactly what
    // happens on consecutive frames as the playhead advances.
    app.ensure_samples_in(100_000, 400_000);
    app.ensure_samples_in(110_000, 410_000);
    let sub = app.subs.get(&key).unwrap();
    let mut tight = 0usize;
    let mut prev: Option<u64> = None;
    for &(t, _) in sub.history.iter() {
        if let Some(p) = prev
            && t.saturating_sub(p) < SAMPLE_INTERVAL_US
        {
            tight += 1;
        }
        prev = Some(t);
    }
    assert_eq!(
        tight, 0,
        "{} samples landed within one stride of a neighbour; the polyline \
             then zig-zags between them and reads as a thick band",
        tight
    );
    app.stop();
    std::fs::remove_file(&file).ok();
}

#[test]
fn a_rewind_does_not_record_a_zero_cycle_time() {
    let (mut app, _key, file) = app_with_replayable_recording("cycle_rebase", 60);
    app.replay();
    app.set_replay_speed(4.0);
    let deadline = std::time::Instant::now() + std::time::Duration::from_millis(250);
    while std::time::Instant::now() < deadline {
        std::thread::sleep(std::time::Duration::from_millis(11));
        app.update();
    }
    let (_, dur) = app.replay_position().unwrap();
    assert!(!app.aggs.is_empty(), "setup: messages should be aggregated");

    // Walk back over ground already seen: every replayed frame is a
    // backwards timestamp for its message, which used to be folded in as a
    // zero-length cycle.
    app.seek_replay_seconds(dur / 3.0);
    app.play();
    for _ in 0..10 {
        std::thread::sleep(std::time::Duration::from_millis(11));
        app.update();
    }
    for agg in app.aggs.values() {
        if agg.count > 1 {
            assert!(
                agg.min_us > 0.0,
                "message {:#05X} reports a {} us minimum cycle after a rewind",
                agg.id,
                agg.min_us
            );
        }
    }
    app.stop();
    std::fs::remove_file(&file).ok();
}

#[test]
fn a_plot_window_decodes_without_waiting_for_playback() {
    let (mut app, key, file) = app_with_replayable_recording("backfill", 60);
    app.replay();
    let (pos0, dur) = app.replay_position().unwrap();
    assert!(
        dur > 0.4,
        "setup: the log should span the window under test, got {dur} s"
    );

    {
        // Ask for a stretch the playback cursor has never walked through.
        // Under the old streaming-only design this window was simply empty.
        app.ensure_samples_in(200_000, 400_000);
        let sub = app.subs.get(&key).unwrap();
        let win = sub.history.range(200_000, 400_000);
        assert!(
            win.len() > 3,
            "the window must decode on demand, got {} points",
            win.len()
        );
        assert!(
            win.iter()
                .zip(win.iter().skip(1))
                .all(|(a, b)| b.0 - a.0 >= SAMPLE_INTERVAL_US),
            "a backfill must honour the sampling stride"
        );
        assert!(
            win.first().unwrap().0 >= 200_000 && win.last().unwrap().0 <= 400_000,
            "returned points must lie inside the request"
        );
    }
    let (pos1, _) = app.replay_position().unwrap();
    assert_eq!(pos1, pos0, "a backfill must not move the playhead");
    app.stop();
    std::fs::remove_file(&file).ok();
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
    let mut app = App::new();
    let file = write_timed_asc("roxy_can_inject.asc", 3, 100_000);
    app.load_log(&file.to_string_lossy());
    let tx_id = app.tx_list[0].id;
    let tx_ch = app.tx_list[0].channel;
    app.tx_list[0].active = true;
    app.tx_list[0].cycle_us = 40_000;
    app.tx_list[0].data = [0xDE; MAX_CAN_FD_LEN];
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
        .get(&(tx_ch, tx_id))
        .expect("injected frames aggregate");
    // The default generator entry shares the log's id 0x100, so the one
    // aggregate row folds both traffics together -- exactly what a real
    // bus with a responder on the same id looks like.
    assert_eq!(agg.count, (injected.len() + 3) as u64);
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
    let mut app = App::new();
    assert_eq!(app.replay_speed, 1.0);
    app.step_replay_speed(1);
    assert_eq!(app.replay_speed, 2.0, "one notch faster");
    app.step_replay_speed(-1);
    app.step_replay_speed(-1);
    assert_eq!(app.replay_speed, 0.5, "two notches slower");
    app.step_replay_speed(-1);
    assert_eq!(app.replay_speed, 0.5, "clamped at the slow end");
    app.step_replay_speed(99);
    assert_eq!(app.replay_speed, 4.0, "clamped at the fast end");
}

#[test]
fn starting_clears_the_previous_pause() {
    let mut app = App::new();
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
    let mut app = App::new();
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
    let mut app = App::new();
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
    let mut app = App::new();
    app.open_dropped(std::path::Path::new("assets/motbus.dbc"));
    assert_eq!(app.channels[0].dbc_path, "assets/motbus.dbc");
    assert!(
        app.channels[0].dbc.is_some(),
        "dropped DBC is parsed into the first bus"
    );
    assert_eq!(app.recent_dbc[0], "assets/motbus.dbc");
}

#[test]
fn jump_to_live_resets_plot_offsets() {
    let mut app = App::new();
    app.graphics[0].t_offset_s = -42.0;
    app.jump_to_live();
    assert_eq!(app.graphics[0].t_offset_s, 0.0);
}

#[test]
fn reset_restores_the_default_workspace() {
    let mut app = App::new();
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
    let mut app = App::new();
    app.new_project();
    assert!(
        app.channels
            .iter()
            .all(|c| c.dbc.is_none() && c.dbc_path.is_empty()),
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
    let mut app = App::new();
    app.request_quit();
    assert!(app.quit, "clean untitled workspace quits silently");
    assert!(app.pending_action.is_none());

    let mut app = App::new();
    app.new_trace_window();
    app.request_quit();
    assert!(!app.quit, "modified workspace must confirm first");
    assert_eq!(app.pending_action, Some(crate::app::PendingAction::Quit));
}

#[test]
fn autosave_round_trips_the_workspace() {
    let mut app = App::new();
    let path = std::env::temp_dir().join("roxy_can_autosave.rxproj");
    assert!(app.save_project(Some(path.clone())));
    app.trace_windows[0].filter = "Motor".to_string();
    app.layout_cache = "[Window][Dockspace]\n".to_string();
    app.write_autosave();

    let mut restored = App::new();
    assert!(restored.load_autosave());
    assert_eq!(restored.project_path.as_deref(), Some(path.as_path()));
    assert_eq!(restored.trace_windows[0].filter, "Motor");
    assert_eq!(
        restored.pending_layout.as_deref(),
        Some("[Window][Dockspace]\n")
    );
    assert!(!restored.is_dirty(), "restored autosave starts clean");
    std::fs::remove_file(&path).ok();
    std::fs::remove_file(crate::config::AUTOSAVE_PATH).ok();
}

#[test]
fn desktop_switching_restores_window_visibility() {
    let mut app = App::new();
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
    let mut app = App::new();
    app.add_desktop();
    app.switch_desktop(0);
    let cfg = Config::from_app(&app, None);
    let mut restored = App::new();
    cfg.apply(&mut restored);
    assert_eq!(restored.desktops.len(), 2);
    assert_eq!(restored.desktops[0].name, "Desktop 1");
    assert_eq!(restored.desktops[1].name, "Desktop 2");
    assert_eq!(restored.active_desktop, 0);
    assert_eq!(
        restored.desktops[0].open_windows.len(),
        app.desktops[0].open_windows.len()
    );
}

#[test]
fn delete_desktop_keeps_at_least_one() {
    let mut app = App::new();
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
fn new_project_resets_to_single_desktop() {
    let mut app = App::new();
    app.add_desktop();
    app.rename_desktop(1, "Analysis".to_string());
    app.new_project();
    assert_eq!(app.desktops.len(), 1);
    assert_eq!(app.active_desktop, 0);
    assert_eq!(app.desktops[0].name, "Desktop 1");
}

#[test]
fn project_round_trips_through_an_rxproj_file() {
    let mut app = App::new();
    app.trace_windows[0].filter = "Motor".to_string();
    let path = std::env::temp_dir().join("roxy_can_test.rxproj");
    assert!(app.save_project(Some(path.clone())), "save writes the file");
    assert_eq!(app.project_path.as_deref(), Some(path.as_path()));

    let mut restored = App::new();
    restored.open_project_path(&path);
    assert_eq!(restored.project_path.as_deref(), Some(path.as_path()));
    assert_eq!(restored.trace_windows[0].filter, "Motor");
    assert_eq!(restored.channels.len(), app.channels.len());
    assert_eq!(restored.tx_list.len(), app.tx_list.len());
    std::fs::remove_file(&path).ok();
}

#[test]
fn signal_stats_track_min_avg_max() {
    let mut app = App::new();
    let key = {
        let db = app.channel_dbc(0).expect("sample DBC loaded");
        let id = db.order[0];
        (0u8, id, db.messages[&id].signals[0].name.clone())
    };
    app.subscribe(key.clone());
    for tx in &mut app.tx_list {
        tx.active = true;
        tx.cycle_us = 10_000;
    }
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
    let mut app = App::new();
    let key = {
        let db = app.channel_dbc(0).expect("sample DBC loaded");
        let id = db.order[0];
        (0u8, id, db.messages[&id].signals[0].name.clone())
    };
    app.subscribe(key.clone());
    app.graphics[0].signals.push(GfxSignal {
        key: key.clone(),
        visible: true,
    });
    let path = std::env::temp_dir().join("roxy_can_resub.rxproj");
    assert!(app.save_project(Some(path.clone())));
    let mut restored = App::new();
    restored.open_project_path(&path);
    assert!(
        restored.subs.contains_key(&key),
        "restored signal is resubscribed so it is not grey"
    );
    std::fs::remove_file(&path).ok();
}

#[test]
fn recording_captures_generator_data_faithfully() {
    let mut app = App::new();
    app.tx_list[0].active = true;
    app.tx_list[0].cycle_us = 10_000;
    let mut payload = [0u8; MAX_CAN_FD_LEN];
    payload[..8].copy_from_slice(&[0x11, 0x22, 0x33, 0x44, 0x55, 0x66, 0x77, 0x88]);
    app.tx_list[0].data = payload;
    app.recorder.record_path = "target/test_record".to_string();
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

#[test]
fn set_bus_tx_toggles_a_whole_bus() {
    let mut app = App::new();
    assert!(app.tx_list.iter().all(|t| !t.active));
    app.set_bus_tx(0, true);
    assert!(
        app.tx_list
            .iter()
            .filter(|t| t.channel == 0)
            .all(|t| t.active),
        "bus 0 fully enabled"
    );
    assert!(
        app.tx_list
            .iter()
            .filter(|t| t.channel == 1)
            .all(|t| !t.active),
        "other buses untouched"
    );
    app.set_bus_tx(0, false);
    assert!(app.tx_list.iter().all(|t| !t.active));
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
    let mut app = App::new();
    app.channels[0].dbc = Some(crate::dbc::load_dbc_str(SPEC_DBC).unwrap());
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
    app.spec.rows.contains_key(&(ch, id, kind))
}

fn verdict(app: &App, ch: u8, id: u32, kind: Kind) -> crate::spec::Latch {
    app.spec.rows[&(ch, id, kind)]
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
        app.aggs[&(0, 100)].count,
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
        app.spec.previous((0, 100)),
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
    let mut app = App::new();
    app.channels[0].dbc = Some(crate::dbc::load_dbc_str(MUX_SAMPLE_DBC).unwrap());
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

#[test]
fn a_signal_of_the_inactive_group_is_not_sampled() {
    let mut app = mux_app();
    let g1 = (0u8, 400u32, "G1_A".to_string());
    let g2 = (0u8, 400u32, "G2_C".to_string());
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
    let g2 = (0u8, 400u32, "G2_C".to_string());
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

fn quiet_app() -> App {
    let mut app = App::new();
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
    app.reset_time();
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
    app.recorder.record_path = base.to_string_lossy().to_string();
    app.triggers.push(Trigger::new(
        TriggerCond::SignalCross {
            ch: 0,
            id: 0x100,
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

    // The recording opened on the crossing frame, so the very frame that
    // fired the trigger is in the file.
    app.recorder.close();
    let text = std::fs::read_to_string(&app.recorder.last_record).unwrap();
    let times: Vec<u64> = crate::log::asc::parse_asc(&text)
        .iter()
        .map(|f| f.t_us)
        .collect();
    assert_eq!(
        times,
        vec![20_000, 30_000, 40_000, 50_000],
        "capture starts at the firing frame"
    );
    std::fs::remove_file(&app.recorder.last_record).ok();
}

#[test]
fn an_id_present_trigger_latches_and_can_stop_a_recording() {
    let mut app = quiet_app();
    let base = std::env::temp_dir().join("roxy_can_trigger_id.asc");
    app.recorder.record_path = base.to_string_lossy().to_string();
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
    assert!(
        !app.recorder.recording,
        "the watched id stops the recording"
    );
    assert_eq!(app.triggers[0].fired, 1);

    receive(
        &mut app,
        30_000,
        vec![rx_frame(30_000, 0x777, 8, FrameFlags::NONE)],
    );
    assert_eq!(app.triggers[0].fired, 1, "presence latches for the run");
    app.recorder.close();
    std::fs::remove_file(&app.recorder.last_record).ok();
}

#[test]
fn an_error_frame_trigger_latches_once_per_run() {
    let mut app = quiet_app();
    let base = std::env::temp_dir().join("roxy_can_trigger_err.asc");
    app.recorder.record_path = base.to_string_lossy().to_string();
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
