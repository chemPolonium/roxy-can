//! ScriptNode tests: dispatch, timers, checks and FlexRay/Ethernet reads.

use super::*;
use std::collections::HashMap;

fn node(source: &str) -> ScriptNode {
    let mut n = ScriptNode::new(1, "n".into(), 0);
    n.source = source.to_string();
    n
}

/// A FlexRay cluster with one frame per `(slot, signal)` pair, so two
/// pairs can declare the same signal in different slots.
fn fr_dbc(frames: &[(u16, &str)]) -> std::sync::Arc<crate::fr_db::FrDb> {
    use crate::fr_db::{
        FrChannel, FrClusterParams, FrDb, FrFrameDb, FrPdu, FrSignal, FrTriggering,
    };
    let mut pdus = Vec::new();
    let mut defs = Vec::new();
    for (i, (slot, name)) in frames.iter().enumerate() {
        let pdu = format!("P{i}");
        pdus.push(FrPdu {
            name: pdu.clone(),
            length: 8,
            dynamic: false,
            comment: String::new(),
            signals: vec![FrSignal {
                name: (*name).into(),
                length_bits: 8,
                factor: 1.0,
                max: 255.0,
                ..Default::default()
            }],
        });
        defs.push(FrFrameDb {
            name: format!("F{i}"),
            length: 8,
            payload_preamble: false,
            triggering: FrTriggering {
                channel: FrChannel::Both,
                slot_id: u32::from(*slot),
                base_cycle: 0,
                cycle_repetition: 1,
                startup: false,
            },
            pdus: vec![(pdu, 0u32)],
            comment: String::new(),
        });
    }
    std::sync::Arc::new(FrDb::assemble(
        FrClusterParams {
            name: "PowerTrain".into(),
            ..Default::default()
        },
        vec![],
        pdus,
        defs,
    ))
}

/// Two-node mini database: EngineECU transmits 0x100 + 0x101, GearBox
/// transmits 0x200. RPM is range-limited to 0..8000.
fn engine_dbc() -> std::sync::Arc<crate::dbc::SymbolTable> {
    std::sync::Arc::new(
        crate::dbc::load_dbc_str(
            r#"
VERSION ""

BS_:

BU_: EngineECU GearBox

BO_ 256 EngineStatus: 8 EngineECU
 SG_ RPM : 0|16@1+ (1,0) [0|8000] "rpm" GearBox

BO_ 257 EngineCmd: 8 EngineECU
 SG_ Target : 0|8@1+ (1,0) [0|100] "%" GearBox

BO_ 512 GearInfo: 8 GearBox
 SG_ Ratio : 0|8@1+ (1,0) [0|255] "" EngineECU
"#,
        )
        .expect("test dbc parses"),
    )
}

/// Panel buttons run a named event handler, not a user function, so the
/// VM budget, outbox and log stay on the normal node path.
#[test]
fn panel_actions_dispatch_by_name_and_queue_their_sends() {
    let mut n = node(
        r#"
                on panel_action("Go") {
                    print("panel go");
                    send(0x123, 7);
                }
            "#,
    );
    n.start(None);
    assert_eq!(n.panel_actions(), ["Go"]);
    let (ran, out) = n.dispatch_panel_action("Go", None, &HostInput::default());
    assert_eq!(ran, 1);
    assert_eq!(out, vec![(0x123, false, vec![7], 0)]);
    assert!(
        n.take_new_lines().iter().any(|line| line == "panel go"),
        "handler log reaches the node output"
    );
    let (ran, out) = n.dispatch_panel_action("Missing", None, &HostInput::default());
    assert_eq!(ran, 0);
    assert!(out.is_empty());
}

#[test]
fn panel_action_arguments_reach_the_handler() {
    let mut n = node(
        r#"
                on panel_action("Set", v) {
                    print(v);
                }
            "#,
    );
    n.start(None);
    let (ran, _) = n.dispatch_panel_action("Set", Some(42.0), &HostInput::default());
    assert_eq!(ran, 1);
    assert!(
        n.take_new_lines().iter().any(|line| line == "42.0"),
        "the control's value is the handler parameter"
    );

    let mut n = node(r#"on panel_action("Set", v) { print(defined(v)); }"#);
    n.start(None);
    let (ran, _) = n.dispatch_panel_action("Set", None, &HostInput::default());
    assert_eq!(ran, 1);
    assert!(
        n.take_new_lines().iter().any(|line| line == "false"),
        "an action without a value leaves the parameter nil"
    );
}

/// R2 reverse check: DBC traffic the bound node declares but the
/// script never sends surfaces as an info line; what the script does
/// send is not flagged.
#[test]
fn reverse_check_reports_dbc_traffic_the_script_never_sends() {
    let mut n = node("on start { send(0x100); }");
    n.attached = Some((0, "EngineECU".to_string()));
    n.start(Some(engine_dbc()));
    let logs = n.log_snapshot();
    assert!(
        logs.iter()
            .any(|l| l.contains("0x101") && l.contains("never sends them")),
        "EngineCmd (0x101) is named: {logs:?}"
    );
    assert!(
        !logs
            .iter()
            .any(|l| l.contains("never sends") && l.contains("0x100")),
        "the implemented EngineStatus is not flagged"
    );
    // Foreign DBC traffic (GearBox's 0x200) is nobody's business here.
    assert!(
        !logs
            .iter()
            .any(|l| l.contains("never sends") && l.contains("0x200")),
        "another node's messages are not this script's job"
    );
}

/// A wildcard script is a forwarder or recorder by declaration: the
/// reverse check does not apply to it.
#[test]
fn reverse_check_skips_wildcard_scripts() {
    let mut n = node("on message * { }");
    n.attached = Some((0, "EngineECU".to_string()));
    n.start(Some(engine_dbc()));
    assert!(
        !n.log_snapshot()
            .iter()
            .any(|l| l.contains("never sends them")),
        "a wildcard forwarder is exempt"
    );
}

/// R2 wildcard classification: a `on message *` script is named a
/// forwarder with the send-set rule, and a wildcard handler that
/// transmits gets the self-excitation hint -- guarded (the static
/// analysis cannot see the guard) at info level, not check.
#[test]
fn wildcard_scripts_are_classified_and_warned_about_self_excitation() {
    let mut n = node(
        r#"
                on message * {
                    if (frame_id() != 0x700 && frame_dlc() > 0) {
                        send(0x700);
                    }
                }
            "#,
    );
    n.attached = Some((0, "EngineECU".to_string()));
    n.start(Some(engine_dbc()));
    let logs = n.log_snapshot();
    assert!(
        logs.iter()
            .any(|l| l.contains("forwarder") && l.contains("received set")),
        "the forwarder classification line is there: {logs:?}"
    );
    assert!(
        logs.iter()
            .any(|l| l.contains("feedback-loop") && l.starts_with("[info]")),
        "the self-excitation hint is info-level: {logs:?}"
    );

    // A wildcard recorder that never sends skips the hint.
    let mut n = node("on message * { }");
    n.attached = Some((0, "EngineECU".to_string()));
    n.start(Some(engine_dbc()));
    assert!(
        !n.log_snapshot().iter().any(|l| l.contains("feedback-loop")),
        "no send, no loop hint"
    );
}

/// The frame_id pure forward is caught too -- the send is opaque but
/// still attributed to the wildcard handler.
#[test]
fn wildcard_frame_id_forward_triggers_the_hint() {
    let mut n = node("on message * { send(frame_id()); }");
    n.attached = Some((0, "EngineECU".to_string()));
    n.start(Some(engine_dbc()));
    assert!(
        n.log_snapshot().iter().any(|l| l.contains("feedback-loop")),
        "an unguarded pure forward is the textbook loop: {logs:?}",
        logs = n.log_snapshot()
    );
}

/// Literal `set_sig` values are range-checked against the database;
/// in-range writes stay silent.
#[test]
fn set_sig_literal_outside_dbc_range_is_reported() {
    let mut n = node(
        r#"
                on start {
                    let b = bytes(8);
                    set_sig(b, 0x100, "RPM", 9999);
                    set_sig(b, 0x100, "RPM", 3000);
                }
            "#,
    );
    n.attached = Some((0, "EngineECU".to_string()));
    n.start(Some(engine_dbc()));
    let logs = n.log_snapshot();
    assert!(
        logs.iter()
            .any(|l| l.contains("9999") && l.contains("outside")),
        "the out-of-range literal is named: {logs:?}"
    );
    assert!(
        !logs.iter().any(|l| l.contains("3000")),
        "the in-range literal stays silent"
    );
}

/// The start check reports literal `sys_get` / `sys_set` keys that
/// are not in the defined set the bus pushed in.
#[test]
fn undefined_sysvar_references_are_reported_at_start() {
    let mut n = node(
        r#"
                on start {
                    let v = sys_get("Demo::Speed");
                    sys_set("Demo::Setpoint", v + 1);
                }
            "#,
    );
    n.sysvar_keys = vec!["Demo::Speed".to_string()];
    n.start(None);
    let logs = n.log_snapshot();
    assert!(
        logs.iter().any(|l| l.contains("Demo::Setpoint")),
        "the undefined write is reported: {logs:?}"
    );
    assert!(
        !logs.iter().any(|l| l.contains("Demo::Speed")),
        "the defined read is silent"
    );
}

/// A `sys_set` in a handler reaches `take_sys_sets`; `sys_get` reads
/// the values the bus published into the host input.
#[test]
fn sys_set_from_a_handler_reaches_the_drain() {
    let mut n = node(
        r#"
                on message 0x100 {
                    sys_set("Demo::Setpoint", sys_get("Demo::Speed") * 2);
                }
            "#,
    );
    n.sysvar_keys = vec!["Demo::Speed".to_string(), "Demo::Setpoint".to_string()];
    n.start(None);
    let mut hit = HostInput::default();
    hit.sysvars.insert("Demo::Speed".to_string(), 21.0);
    n.dispatch_frame(0, 0x100, false, false, &[], &hit);
    assert_eq!(
        n.take_sys_sets(),
        vec![("Demo::Setpoint".to_string(), 42.0)]
    );
}

#[test]
fn start_message_and_timers_drive_the_log_and_outbox() {
    let mut n = node(
        r#"
                let ticks = 0;
                on start { print("hello"); }
                on message 0x100 { send(0x200, 1, 2); }
                on timer 100 {
                    ticks = ticks + 1;
                    print("tick", ticks);
                    send(0x300);
                }
            "#,
    );
    n.start(None);
    assert!(n.running());
    // on start already printed.
    assert_eq!(n.log_snapshot(), ["hello"]);

    // A watched frame queues the reaction payload.
    let out = n.dispatch_frame(0, 0x100, false, false, &[], &HostInput::default());
    assert_eq!(out, vec![(0x200, false, vec![1, 2], 0)]);

    // Timers arm lazily one period out (armed at 50 ms -> first due
    // at 150 ms), then fire and resync.
    assert!(n.run_timers(50_000, &HostInput::default()).is_empty());
    assert!(n.run_timers(120_000, &HostInput::default()).is_empty());
    let out = n.run_timers(160_000, &HostInput::default());
    assert_eq!(out.len(), 1);
    assert_eq!(out[0].0, 0x300);
    assert!(n.log_snapshot().last().unwrap().starts_with("tick 1"));

    // An error in a handler stops the node until restart.
    let mut bad = node("on message 0x100 { print(1 / 0); }");
    bad.start(None);
    bad.dispatch_frame(0, 0x100, false, false, &[], &HostInput::default());
    assert!(bad.errored());
    let issue = bad.issue().expect("runtime error reaches the editor");
    assert_eq!(issue.line, 1);
    assert!(issue.message.contains("division"), "{:?}", issue.message);
    assert!(
        bad.dispatch_frame(0, 0x100, false, false, &[], &HostInput::default())
            .is_empty(),
        "errored nodes stay quiet"
    );
    bad.start(None);
    assert!(bad.running(), "a restart clears the error");
    assert!(bad.issue().is_none(), "a restart clears the diagnostic");
}

#[test]
fn runtime_errors_in_start_handlers_keep_their_line() {
    let mut n = node("on start {\n    print(1 / 0);\n}");
    n.start(None);
    assert!(n.errored());
    let issue = n.issue().expect("start error reaches the editor");
    assert_eq!(issue.line, 2);
    assert!(issue.message.contains("division"), "{:?}", issue.message);
}

#[test]
fn compile_errors_land_in_the_log_and_the_editor_diagnostic() {
    let mut n = node("on start { print(; }");
    n.start(None);
    assert!(!n.running());
    assert!(n.log_snapshot()[0].contains("[compile]"));
    let issue = n.issue().expect("compile error reaches the editor");
    assert_eq!(issue.line, 1);
    assert!(issue.col.is_some(), "the token column is kept");
    assert!(!issue.message.is_empty());
    n.set_source("on start { }".into(), None, None, None, None, false);
    assert!(n.issue().is_none(), "an edit clears the stale diagnostic");
}

/// Literal `sig` / `set_sig` references are checked against the
/// channel database at Apply: a typo'd name is a logged warning, not
/// a runtime surprise. With no database there is nothing to check.
#[test]
fn unknown_signal_references_are_reported_at_apply() {
    let dbc = Some(std::sync::Arc::new(
        crate::dbc::load_dbc_str(
            r#"VERSION "check"

NS_ :

BS_:

BU_: ECU

BO_ 256 Real: 2 ECU
 SG_ RealSig : 0|16@1+ (0.1,0) [0|0] ""  ECU
"#,
        )
        .unwrap(),
    ));
    let mut n = node(
        r#"
                on start {
                    let v = sig(0x100, "RealSig");
                    let w = sig(0x100, "TypoSig");
                }
            "#,
    );
    n.start(dbc.clone());
    let log = n.log_snapshot();
    let check_lines: Vec<&String> = log.iter().filter(|l| l.starts_with("[check]")).collect();
    assert_eq!(
        check_lines.len(),
        1,
        "exactly the typo is flagged: {check_lines:?}"
    );
    assert!(check_lines[0].contains("TypoSig"));

    // Without a database there is no vocabulary to check against —
    // and no false warnings either.
    let mut n2 = node("on start { let v = sig(0x100, \"Whatever\"); }");
    n2.start(None);
    let log2 = n2.log_snapshot();
    assert!(
        log2.iter().all(|l| !l.starts_with("[check]")),
        "no db: no static check warnings: {log2:?}"
    );
}

#[test]
fn stop_timer_halts_a_periodic_handler_after_its_runs() {
    let mut n = node(
        r#"
                let n = 0;
                on timer 100 {
                    n = n + 1;
                    print("tick", n);
                    if (n == 2) { stop_timer(); }
                }
            "#,
    );
    n.start(None);
    n.run_timers(50_000, &HostInput::default()); // arms: due 150_000
    n.run_timers(160_000, &HostInput::default()); // tick 1, arms 310_000
    n.run_timers(320_000, &HostInput::default()); // tick 2, stops
    n.run_timers(1_000_000, &HostInput::default()); // stopped: silence
    eprintln!("DEBUG LOG: {:?}", n.log_snapshot());
    let ticks = n
        .log_snapshot()
        .iter()
        .filter(|l| l.starts_with("tick "))
        .count();
    assert_eq!(ticks, 2, "stop_timer must halt the periodic handler");
}

#[test]
fn set_period_reschedules_the_running_timer() {
    let mut n = node(
        r#"
                let ticks = 0;
                on timer 100 {
                    ticks = ticks + 1;
                    if (ticks == 1) { set_period(50); }
                    print("t", ticks);
                }
            "#,
    );
    n.start(None);
    n.run_timers(50_000, &HostInput::default()); // arms: due 150_000
    n.run_timers(150_000, &HostInput::default()); // tick 1, period -> 50ms
    n.run_timers(199_999, &HostInput::default()); // not yet
    n.run_timers(200_000, &HostInput::default()); // tick 2
    let ticks = n
        .log_snapshot()
        .iter()
        .filter(|l| l.starts_with("t "))
        .count();
    assert_eq!(ticks, 2);
}

/// A periodic slot re-anchors each beat on the schedule, not on the
/// (late) fire: wake jitter must not drift every beat after it.
#[test]
fn periodic_beats_stay_on_schedule_after_a_late_wake() {
    let mut n = node(
        r#"
                let ticks = 0;
                on timer 100 { ticks = ticks + 1; print("t", ticks); }
            "#,
    );
    n.start(None);
    n.run_timers(50_000, &HostInput::default()); // arms: due 150_000
    n.run_timers(160_000, &HostInput::default()); // late wake: tick 1 at 160
    n.run_timers(249_999, &HostInput::default()); // not yet
    n.run_timers(250_000, &HostInput::default()); // beat 2 at 250, not 260
    let ticks = n
        .log_snapshot()
        .iter()
        .filter(|l| l.starts_with("t "))
        .count();
    assert_eq!(ticks, 2, "beats 150+100=250, unaffected by the late wake");
}

#[test]
fn channel_mismatch_is_ignored() {
    let mut n = node("on message 0x100 { send(0x200); }");
    n.start(None);
    assert!(
        n.dispatch_frame(1, 0x100, false, false, &[], &HostInput::default())
            .is_empty(),
        "other channel"
    );
    assert_eq!(
        n.dispatch_frame(0, 0x100, false, false, &[], &HostInput::default())
            .len(),
        1
    );
}

/// Frame classes are told apart: a bare id never matches across the
/// standard/extended boundary, in either direction.
#[test]
fn extended_frames_need_extended_handlers() {
    let mut n = node(
        r#"
                on message 0x100 { print("std"); }
                on message 0x1C3D1E5 { print("ext"); }
            "#,
    );
    n.start(None);
    // Standard 0x100 fires the standard handler only.
    assert_eq!(
        n.dispatch_frame(0, 0x100, false, false, &[], &HostInput::default())
            .len(),
        0
    );
    assert_eq!(std_count(&n), 1);
    // Extended 0x100 fires nothing: no handler names an extended
    // frame with that id.
    n.dispatch_frame(0, 0x100, true, false, &[], &HostInput::default());
    assert_eq!(std_count(&n), 1);
    // Extended 0x1C3D1E5 fires the extended handler only.
    n.dispatch_frame(0, 0x1C3D1E5, true, false, &[], &HostInput::default());
    assert_eq!(std_count(&n), 1);
    assert_eq!(ext_count(&n), 1);
    // The same id arriving as standard fires nothing.
    n.dispatch_frame(0, 0x1C3D1E5, false, false, &[], &HostInput::default());
    assert_eq!(ext_count(&n), 1);

    fn std_count(n: &ScriptNode) -> usize {
        n.log_snapshot().iter().filter(|l| *l == "std").count()
    }
    fn ext_count(n: &ScriptNode) -> usize {
        n.log_snapshot().iter().filter(|l| *l == "ext").count()
    }
}

/// Extended frames whose numeric id sits inside the standard range are
/// reachable through the explicit `on extended message` form and
/// `send_ext`, where plain `on message` / `send` cannot go.
#[test]
fn extended_syntax_addresses_small_numeric_extended_ids() {
    let mut n = node(
        r#"
            let count = 0;
            on message 0x50 { print("std"); }
            on extended message 0x50 { count = count + 1; print("ext", count); }
            on message 0x60 { send_ext(0x60, 7); }
        "#,
    );
    n.start(None);
    // Standard 0x50: the plain handler; extended 0x50: the explicit one.
    n.dispatch_frame(0, 0x50, false, false, &[], &HostInput::default());
    let log = n.log_snapshot();
    assert_eq!(log[0], "std");
    n.dispatch_frame(0, 0x50, true, false, &[], &HostInput::default());
    let log = n.log_snapshot();
    assert_eq!(
        log[1], "ext 1",
        "the extended form catches what plain cannot"
    );

    // send_ext always travels extended, even below 0x700.
    let out = n.dispatch_frame(0, 0x60, false, false, &[], &HostInput::default());
    assert_eq!(out, vec![(0x60, true, vec![7], 0)]);
}

/// A buffer payload past 8 bytes travels as CAN FD with a valid FD
/// length; the VM refuses anything past the 64-byte ceiling.
#[test]
fn long_buffers_travel_as_fd_frames() {
    let mut n = node(
        r#"
                on message 0x100 {
                    let fd = bytes(12);
                    fd[11] = 0xEE;
                    send(0x300, fd);
                }
            "#,
    );
    n.start(None);
    let out = n.dispatch_frame(0, 0x100, false, false, &[], &HostInput::default());
    assert_eq!(out.len(), 1);
    assert_eq!(out[0].0, 0x300);
    // A 12-byte payload needs CAN FD but not an extended id: those
    // are two independent dimensions. The core snaps the length to
    // a valid FD DLC and sets the FD flag.
    assert!(!out[0].1);
    assert_eq!(out[0].2.len(), 12);

    let mut too_long = node("let big = bytes(65); send(0x300, big);");
    too_long.start(None);
    too_long.dispatch_frame(0, 0x100, false, false, &[], &HostInput::default());
    assert!(
        too_long.errored(),
        "past 64 bytes the send must fail the handler"
    );
}

/// `send_fd` forces FD even for an 8-byte payload, where length alone
/// would mean a classic frame. The third argument selects BRS.
#[test]
fn send_fd_forces_fd_and_can_switch_brs() {
    let mut n = node(
        r#"
                on message 0x100 {
                    let reply = bytes(8);
                    reply[0] = 0xAA;
                    send_fd(0x300, reply);
                    send_fd(0x301, reply, true);
                    send_fd_ext(0x100, reply);
                }
            "#,
    );
    n.start(None);
    let out = n.dispatch_frame(0, 0x100, false, false, &[], &HostInput::default());
    assert_eq!(out.len(), 3);
    assert_eq!(out[0].0, 0x300);
    assert_eq!(out[0].2.len(), 8);
    assert_eq!(out[0].2[0], 0xAA);
    assert_eq!(out[0].3, crate::script::CAN_SEND_FD);
    assert!(!out[0].1);
    assert_eq!(
        out[1].3,
        crate::script::CAN_SEND_FD | crate::script::CAN_SEND_BRS
    );
    assert_eq!(out[2].3, crate::script::CAN_SEND_FD);
    assert!(out[2].1, "send_fd_ext forces an extended id");

    let mut too_long = node("let big = bytes(65); send_fd(0x300, big);");
    too_long.start(None);
    too_long.dispatch_frame(0, 0x100, false, false, &[], &HostInput::default());
    assert!(too_long.errored(), "past 64 bytes the send must fail");

    let mut not_a_buffer = node("on message 0x100 { send_fd(0x300, 1); }");
    not_a_buffer.start(None);
    not_a_buffer.dispatch_frame(0, 0x100, false, false, &[], &HostInput::default());
    assert!(
        not_a_buffer.errored(),
        "send_fd takes a bytes() buffer, not positional bytes"
    );
}

/// `on message *` sees every frame on the channel; specific handlers
/// still fire for their own ids, in declaration order.
#[test]
fn a_wildcard_handler_sees_every_frame() {
    let mut n = node(
        r#"
                let hits = 0;
                on message * {
                    hits = hits + 1;
                    print("any", frame_byte(0), frame_id());
                }
                on message 0x55 { print("specific"); }
            "#,
    );
    n.start(None);
    let hit = HostInput {
        now_s: 0.01,
        ..HostInput::default()
    };
    n.dispatch_frame(0, 0x100, false, false, &[7], &hit);
    n.dispatch_frame(0, 0x55, false, false, &[9], &hit);
    n.dispatch_frame(0, 0x1ABCDEF, true, false, &[11], &hit);
    let log = n.log_snapshot();
    assert_eq!(log[0], "any 7 256", "the wildcard saw 0x100");
    // For 0x55 both handlers match and run in declaration order:
    // the wildcard was declared first.
    assert_eq!(log[1], "any 9 85");
    assert_eq!(log[2], "specific");
    assert_eq!(
        log[3], "any 11 28036591",
        "the wildcard saw the extended frame"
    );
    assert_eq!(hits_of(&n), 3);

    fn hits_of(n: &ScriptNode) -> usize {
        n.log_snapshot()
            .iter()
            .filter(|l| l.starts_with("any "))
            .count()
    }
}

/// Error frames belong to `on errorFrame` alone: wildcard and
/// id handlers never see them.
#[test]
fn an_error_frame_reaches_only_the_error_handler() {
    let mut n = node(
        r#"
                let errors = 0;
                on message * {
                    print("any", frame_dlc());
                }
                on errorFrame {
                    errors = errors + 1;
                    print("err", errors);
                }
            "#,
    );
    n.start(None);
    // An error frame carries no payload and no regular handler runs.
    n.dispatch_frame(0, 0x1, false, true, &[], &HostInput::default());
    let log = n.log_snapshot();
    assert_eq!(log, ["err 1"], "only the error handler ran: {log:?}");
    // Regular traffic keeps flowing through the wildcard.
    n.dispatch_frame(0, 0x100, false, false, &[1], &HostInput::default());
    let log = n.log_snapshot();
    assert_eq!(log.last().map(|l| l.as_str()), Some("any 1"));
    assert_eq!(log.len(), 2, "no error handler fire on a data frame");
}

/// The delayed-response shape: a frame arrives, the node answers one
/// fixed delay later, exactly once per arming.
#[test]
fn a_set_timer_one_shot_fires_once_from_an_event() {
    let mut n = node(
        r#"
                on message 0x100 { set_timer("resp", 100); }
                on timer "resp" { send(0x200, 1); }
            "#,
    );
    n.start(None);
    let hit = HostInput {
        now_s: 0.05,
        ..HostInput::default()
    };
    assert!(
        n.dispatch_frame(0, 0x100, false, false, &[], &hit)
            .is_empty(),
        "arming"
    );
    assert!(n.run_timers(100_000, &hit).is_empty(), "not yet due");
    let out = n.run_timers(150_000, &hit);
    assert_eq!(out.len(), 1, "one shot, one frame");
    assert_eq!(out[0].0, 0x200);
    assert!(
        n.run_timers(400_000, &hit).is_empty(),
        "a one-shot never repeats"
    );
}

#[test]
fn a_one_shot_can_re_arm_itself() {
    let mut n = node(
        r#"
                let fired = 0;
                on start { set_timer("beat", 100); }
                on timer "beat" {
                    fired = fired + 1;
                    print("beat", fired);
                    if (fired < 3) { set_timer("beat", 100); }
                }
            "#,
    );
    n.start(None);
    let t0 = HostInput::default();
    // The `on start` arming lands on the first tick's clock.
    n.run_timers(0, &t0);
    n.run_timers(100_000, &t0); // beat 1, re-arms
    n.run_timers(200_000, &t0); // beat 2, re-arms
    n.run_timers(300_000, &t0); // beat 3, no re-arm
    n.run_timers(500_000, &t0);
    let beats = n
        .log_snapshot()
        .iter()
        .filter(|l| l.starts_with("beat "))
        .count();
    assert_eq!(beats, 3, "log: {:?}", n.log_snapshot());
}

#[test]
fn cancel_timer_disarms_a_one_shot() {
    let mut n = node(
        r#"
                on start { set_timer("beat", 100); }
                on message 0x1 { cancel_timer("beat"); }
                on timer "beat" { print("beat"); }
            "#,
    );
    n.start(None);
    let t0 = HostInput::default();
    n.run_timers(0, &t0);
    let hit = HostInput {
        now_s: 0.01,
        ..HostInput::default()
    };
    n.dispatch_frame(0, 0x1, false, false, &[], &hit);
    assert!(
        n.run_timers(500_000, &t0).is_empty(),
        "a cancelled one-shot never fires"
    );
    assert!(n.log_snapshot().iter().all(|l| !l.contains("beat")));
}

#[test]
fn set_timer_without_a_handler_logs_a_warning() {
    let mut n = node(r#"on start { set_timer("nope", 50); }"#);
    n.start(None);
    n.run_timers(0, &HostInput::default());
    assert!(
        n.log_snapshot()
            .iter()
            .any(|l| l.contains("nope") && l.contains("no such handler")),
        "log: {:?}",
        n.log_snapshot()
    );
}

#[test]
fn now_and_sig_read_the_published_host_input() {
    let mut n = node(
        r#"
                on message 0x100 {
                    print(now());
                    print(sig(0x100, "RPM"));
                }
            "#,
    );
    n.start(None);
    let input = HostInput {
        now_s: 1.5,
        signals: [((0x100, "RPM".to_string()), 2400.0)].into_iter().collect(),
        fr_signals: HashMap::new(),
        eth_signals: HashMap::new(),
        sysvars: HashMap::new(),
        derived_signals: HashMap::new(),
    };
    n.dispatch_frame(0, 0x100, false, false, &[], &input);
    assert_eq!(n.log_snapshot(), ["1.5", "2400.0"]);

    // An unseen signal reads nil; defined() is the explicit guard.
    let mut n2 = node(
        r#"
                on message 0x100 {
                    let v = sig(0x100, "Nope");
                    print(v, defined(v));
                }
            "#,
    );
    n2.start(None);
    n2.dispatch_frame(0, 0x100, false, false, &[], &HostInput::default());
    assert_eq!(n2.log_snapshot(), ["nil false"]);
    assert!(!n2.errored(), "an unseen read must not park the node");
}

#[test]
fn unseen_signal_arithmetic_is_an_explicit_error() {
    let mut n = node("on start {\n    print(sig(0x100, \"RPM\") + 1);\n}");
    n.start(None);
    assert!(n.errored());
    let issue = n.issue().expect("runtime error reaches the editor");
    assert_eq!(issue.line, 2);
    assert!(issue.message.contains("nil"), "{:?}", issue.message);
}

/// `fr_sig(cluster, slot, "Name")` reads the FlexRay side of the same
/// published input: the cluster index (the number the tables print as
/// `FR{n}`) and the slot pick the value, so two clusters carrying the same
/// slot number stay two separate reads. Like `sig`, an unseen signal is an
/// error rather than a silent zero.
#[test]
fn fr_sig_reads_the_flexray_side_of_the_published_input() {
    let mut n = node(
        r#"
                on message 0x100 {
                    print(fr_sig(0, 5, "One"));
                    print(fr_sig(1, 5, "One"));
                }
            "#,
    );
    n.start(None);
    let input = HostInput {
        now_s: 0.0,
        signals: HashMap::new(),
        fr_signals: [
            ((0, 5, "One".to_string()), 42.0),
            ((1, 5, "One".to_string()), 7.0),
        ]
        .into_iter()
        .collect(),
        eth_signals: HashMap::new(),
        sysvars: HashMap::new(),
        derived_signals: HashMap::new(),
    };
    n.dispatch_frame(0, 0x100, false, false, &[], &input);
    assert_eq!(
        n.log_snapshot(),
        ["42.0", "7.0"],
        "the cluster index selects which bus the slot number means"
    );

    let mut n2 = node(
        r#"
                on message 0x100 {
                    let v = fr_sig(0, 9, "Never");
                    print(v, defined(v));
                }
            "#,
    );
    n2.start(None);
    n2.dispatch_frame(0, 0x100, false, false, &[], &input);
    assert_eq!(n2.log_snapshot(), ["nil false"]);
    assert!(!n2.errored(), "an unseen FR slot reads nil");
}

/// A named read resolves against the node channel's database at start,
/// then reads the published value.
#[test]
fn a_dotted_read_resolves_the_message_name() {
    let mut n = node(
        r#"
                on message 0x100 {
                    print(EngineStatus.RPM);
                }
            "#,
    );
    n.start(Some(engine_dbc()));
    let input = HostInput {
        now_s: 0.0,
        signals: [((0x100, "RPM".to_string()), 1234.0)].into_iter().collect(),
        fr_signals: HashMap::new(),
        eth_signals: HashMap::new(),
        sysvars: HashMap::new(),
        derived_signals: HashMap::new(),
    };
    n.dispatch_frame(0, 0x100, false, false, &[], &input);
    assert_eq!(n.log_snapshot(), ["1234.0"]);
}

/// Dotted FlexRay reads resolve the signal name to its slot once, at
/// start, and then read the published per-slot value. A name that is
/// missing or lives in several slots is a start check, and the runtime
/// call names the explicit `fr_sig` form instead of guessing.
#[test]
fn a_dotted_flexray_read_resolves_the_signal_to_its_slot() {
    let mut dbs = std::collections::BTreeMap::new();
    dbs.insert(0, fr_dbc(&[(13, "CarSpeed")]));
    let mut n = node("on message 0x100 { print(FR0.CarSpeed); }");
    n.start_with(None, Some(&dbs), None, None);
    assert!(!n.errored(), "{:?}", n.log_snapshot());
    let input = HostInput {
        fr_signals: [((0, 13, "CarSpeed".to_string()), 88.0)]
            .into_iter()
            .collect(),
        ..HostInput::default()
    };
    n.dispatch_frame(0, 0x100, false, false, &[], &input);
    assert_eq!(n.log_snapshot(), ["88.0"]);

    let mut typo = node("on message 0x100 { print(FR0.CarSped); }");
    typo.start_with(None, Some(&dbs), None, None);
    assert!(
        typo.log_snapshot()
            .iter()
            .any(|l| l.contains("[check]") && l.contains("CarSped")),
        "{:?}",
        typo.log_snapshot()
    );

    let mut ambiguous_dbs = std::collections::BTreeMap::new();
    ambiguous_dbs.insert(0, fr_dbc(&[(13, "CarSpeed"), (14, "CarSpeed")]));
    let mut ambiguous = node("on start { print(FR0.CarSpeed); }");
    ambiguous.start_with(None, Some(&ambiguous_dbs), None, None);
    assert!(
        ambiguous
            .log_snapshot()
            .iter()
            .any(|l| l.contains("several FR0 slots") && l.contains("fr_sig(0,")),
        "{:?}",
        ambiguous.log_snapshot()
    );
}

/// `this` is the triggering frame: the bus it arrived on, the id (CAN)
/// or slot (FlexRay), its payload, and a signal read that uses that
/// frame's own layout. `node.bus` is the script's own CAN channel.
#[test]
fn this_reads_the_can_triggering_frame() {
    let mut n = node(
        r#"
                on message * {
                    print(this.bus, this.id, this.len, this[0],
                          this.signal("RPM"), node.bus);
                }
            "#,
    );
    n.start(None);
    let input = HostInput {
        signals: [((0x100, "RPM".to_string()), 1234.0)].into_iter().collect(),
        ..HostInput::default()
    };
    n.dispatch_frame(0, 0x100, false, false, &[7, 9], &input);
    assert_eq!(
        n.log_snapshot(),
        ["0 256 2 7 1234.0 0"],
        "bus, id, length, byte, signal, own bus"
    );

    let mut fr = node(
        r#"
                on fr slot 13 {
                    print(this.bus, this.slot, this.len, this.cycle,
                          this.signal("CarSpeed"));
                }
            "#,
    );
    fr.start(None);
    let input = HostInput {
        fr_signals: [((1, 13, "CarSpeed".to_string()), 42.0)]
            .into_iter()
            .collect(),
        ..HostInput::default()
    };
    fr.dispatch_fr_row(1, 13, 5, &[0x2A], &input);
    assert_eq!(fr.log_snapshot(), ["1 13 1 5 42.0"]);
}

/// `node.bus` names the script's own channel even outside an event.
#[test]
fn node_bus_is_the_script_own_channel() {
    let mut n = ScriptNode::new(1, "n".into(), 2);
    n.source = "on start { print(node.bus); }".to_string();
    n.start(None);
    assert_eq!(n.log_snapshot(), ["2"]);
}

/// Frame-only fields fail fast: outside a frame handler there is no
/// trigger to describe, and `this.id` must not silently report a slot
/// when the trigger was FlexRay.
#[test]
fn this_is_rejected_without_a_matching_frame_context() {
    let mut start = node("on start { print(this.bus); }");
    start.start(None);
    assert!(start.errored(), "no frame handler was running");
    assert!(
        start
            .log_snapshot()
            .iter()
            .any(|l| l.contains("frame handler")),
        "{:?}",
        start.log_snapshot()
    );

    let mut wrong = node("on message 0x100 { print(this.slot); }");
    wrong.start(None);
    wrong.dispatch_frame(0, 0x100, false, false, &[1], &HostInput::default());
    assert!(wrong.errored(), "a CAN trigger has no slot");
    assert!(
        wrong.log_snapshot().iter().any(|l| l.contains("this.slot")),
        "{:?}",
        wrong.log_snapshot()
    );
}

/// Naming the cluster instead of its index: the name comes from the
/// cluster description, so the read survives renumbering and is easier
/// to review. Unknown or duplicated names are start checks.
#[test]
fn a_named_cluster_read_resolves_through_the_description() {
    let mut dbs = std::collections::BTreeMap::new();
    dbs.insert(0, fr_dbc(&[(13, "CarSpeed")]));
    let mut n = node("on message 0x100 { print(FR.PowerTrain.CarSpeed); }");
    n.start_with(None, Some(&dbs), None, None);
    assert!(
        !n.log_snapshot().iter().any(|l| l.contains("[check]")),
        "{:?}",
        n.log_snapshot()
    );
    let input = HostInput {
        fr_signals: [((0, 13, "CarSpeed".to_string()), 88.0)]
            .into_iter()
            .collect(),
        ..HostInput::default()
    };
    n.dispatch_frame(0, 0x100, false, false, &[], &input);
    assert_eq!(n.log_snapshot(), ["88.0"]);

    // Bracket spelling handles a signal name with a space.
    let mut spaced = std::collections::BTreeMap::new();
    spaced.insert(0, fr_dbc(&[(13, "Car Speed")]));
    let mut n2 = node("on message 0x100 { print(FR[\"PowerTrain\"][\"Car Speed\"]); }");
    n2.start_with(None, Some(&spaced), None, None);
    let input = HostInput {
        fr_signals: [((0, 13, "Car Speed".to_string()), 7.0)]
            .into_iter()
            .collect(),
        ..HostInput::default()
    };
    n2.dispatch_frame(0, 0x100, false, false, &[], &input);
    assert_eq!(n2.log_snapshot(), ["7.0"]);

    let mut unknown = node("on start { print(FR.Nope.CarSpeed); }");
    unknown.start_with(None, Some(&dbs), None, None);
    assert!(
        unknown
            .log_snapshot()
            .iter()
            .any(|l| l.contains("[check]") && l.contains("Nope")),
        "{:?}",
        unknown.log_snapshot()
    );

    let mut ambiguous_dbs = std::collections::BTreeMap::new();
    ambiguous_dbs.insert(0, fr_dbc(&[(13, "CarSpeed")]));
    ambiguous_dbs.insert(1, fr_dbc(&[(14, "CarSpeed")]));
    let mut ambiguous = node("on start { print(FR.PowerTrain.CarSpeed); }");
    ambiguous.start_with(None, Some(&ambiguous_dbs), None, None);
    assert!(
        ambiguous
            .log_snapshot()
            .iter()
            .any(|l| l.contains("[check]") && l.contains("ambiguous")),
        "{:?}",
        ambiguous.log_snapshot()
    );
}

/// Replacing the cluster description re-points a named read in place,
/// with a `[reload]` line so a moved slot is explainable.
#[test]
fn a_named_cluster_read_follows_a_reloaded_description() {
    let mut before = std::collections::BTreeMap::new();
    before.insert(0, fr_dbc(&[(13, "CarSpeed")]));
    let mut n = node("on message 0x100 { print(FR.PowerTrain.CarSpeed); }");
    n.start_with(None, Some(&before), None, None);
    let mut after = std::collections::BTreeMap::new();
    after.insert(0, fr_dbc(&[(20, "CarSpeed")]));
    n.refresh_fr_named(&after);
    assert!(
        n.log_snapshot()
            .iter()
            .any(|l| l.contains("[reload]") && l.contains("slot 20")),
        "{:?}",
        n.log_snapshot()
    );
    let input = HostInput {
        fr_signals: [((0, 20, "CarSpeed".to_string()), 5.0)]
            .into_iter()
            .collect(),
        ..HostInput::default()
    };
    n.dispatch_frame(0, 0x100, false, false, &[], &input);
    assert_eq!(n.log_snapshot().last().map(String::as_str), Some("5.0"));
}

/// The Ethernet named form uses the ARXML cluster name and verifies the
/// frame and signal on the bus that name resolves to.
#[test]
fn a_named_ethernet_read_checks_the_arxml_names() {
    use crate::eth::db::{EthCluster, EthDb, EthFrameDef, EthPdu, EthSignal};
    let db = EthDb {
        clusters: vec![EthCluster {
            name: "Body".into(),
            vlans: vec![100],
        }],
        frames: vec![EthFrameDef {
            name: "FrameA".into(),
            cluster: "Body".into(),
            length: 8,
            pdus: vec![EthPdu {
                name: "P".into(),
                length: 8,
                start_bit: 0,
                signals: vec![EthSignal {
                    name: "Speed".into(),
                    size: 8,
                    factor: 1.0,
                    ..Default::default()
                }],
            }],
            ..Default::default()
        }],
        ..Default::default()
    };
    let mut dbs = std::collections::BTreeMap::new();
    dbs.insert(0, std::sync::Arc::new(db));

    let mut n = node("on message 0x100 { print(ETH.Body.FrameA.Speed); }");
    n.start_with(None, None, Some(&dbs), None);
    assert!(
        !n.log_snapshot().iter().any(|l| l.contains("[check]")),
        "{:?}",
        n.log_snapshot()
    );
    let input = HostInput {
        eth_signals: [((0, "FrameA".to_string(), "Speed".to_string()), 7.5)]
            .into_iter()
            .collect(),
        ..HostInput::default()
    };
    n.dispatch_frame(0, 0x100, false, false, &[], &input);
    assert_eq!(n.log_snapshot(), ["7.5"]);

    for (src, want) in [
        ("on start { print(ETH.Nope.FrameA.Speed); }", "Nope"),
        ("on start { print(ETH.Body.FrameB.Speed); }", "FrameB"),
        ("on start { print(ETH.Body.FrameA.Torque); }", "Torque"),
    ] {
        let mut bad = node(src);
        bad.start_with(None, None, Some(&dbs), None);
        assert!(
            bad.log_snapshot()
                .iter()
                .any(|l| l.contains("[check]") && l.contains(want)),
            "{src}: {:?}",
            bad.log_snapshot()
        );
    }
}

/// A dotted Ethernet read verifies the frame and signal against the ARXML
/// at start; the value itself arrives through the published table.
#[test]
fn a_dotted_ethernet_read_checks_the_arxml_names() {
    use crate::eth::db::{EthDb, EthFrameDef, EthPdu, EthSignal};
    let frame = EthFrameDef {
        name: "FrameA".into(),
        length: 8,
        pdus: vec![EthPdu {
            name: "P".into(),
            length: 8,
            start_bit: 0,
            signals: vec![EthSignal {
                name: "Speed".into(),
                size: 8,
                factor: 1.0,
                ..Default::default()
            }],
        }],
        ..Default::default()
    };
    let mut dbs = std::collections::BTreeMap::new();
    dbs.insert(
        0,
        std::sync::Arc::new(EthDb {
            frames: vec![frame],
            ..Default::default()
        }),
    );

    let mut n = node("on message 0x100 { print(ETH0.FrameA.Speed); }");
    n.start_with(None, None, Some(&dbs), None);
    assert!(!n.errored(), "{:?}", n.log_snapshot());
    assert!(
        !n.log_snapshot().iter().any(|l| l.contains("[check]")),
        "{:?}",
        n.log_snapshot()
    );
    let input = HostInput {
        eth_signals: [((0, "FrameA".to_string(), "Speed".to_string()), 7.5)]
            .into_iter()
            .collect(),
        ..HostInput::default()
    };
    n.dispatch_frame(0, 0x100, false, false, &[], &input);
    assert_eq!(n.log_snapshot(), ["7.5"]);

    for (src, want) in [
        ("on start { print(ETH0.FrameB.Speed); }", "FrameB"),
        ("on start { print(ETH0.FrameA.Torque); }", "Torque"),
    ] {
        let mut bad = node(src);
        bad.start_with(None, None, Some(&dbs), None);
        assert!(
            bad.log_snapshot()
                .iter()
                .any(|l| l.contains("[check]") && l.contains(want)),
            "{src}: {:?}",
            bad.log_snapshot()
        );
    }
}

/// A named handler resolves its DBC message at start: it wakes on that
/// id only, and `frame_id()` forwards the resolved id.
#[test]
fn a_named_handler_resolves_its_dbc_message() {
    let mut n = node(
        r#"
                on message EngineStatus {
                    print(frame_byte(0));
                    send(frame_id(), 9);
                }
            "#,
    );
    n.start(Some(engine_dbc()));
    let out = n.dispatch_frame(0, 0x100, false, false, &[7], &HostInput::default());
    assert_eq!(n.log_snapshot(), ["7"]);
    assert_eq!(out, vec![(0x100, false, vec![9], 0)]);
    assert!(
        n.dispatch_frame(0, 0x101, false, false, &[8], &HostInput::default())
            .is_empty(),
        "another id does not wake the named handler"
    );
    assert_eq!(n.log_snapshot(), ["7"]);
}

/// An unknown handler name is an assembly error; the handler never fires.
#[test]
fn a_named_handler_is_checked_against_the_dbc() {
    let mut n = node("on message Ghost { print(1); }");
    n.start(Some(engine_dbc()));
    assert!(
        n.log_snapshot()
            .iter()
            .any(|line| line.contains("Ghost") && line.contains("not found")),
        "the unknown message is named: {:?}",
        n.log_snapshot()
    );
}

/// Wrong message / signal names in named reads (dot or bracket) are
/// assembly errors, named at start as `[check]`.
#[test]
fn named_reads_are_checked_against_the_dbc() {
    let mut n = node("on start { print(EngineStatus.Nope); }");
    n.attached = Some((0, "EngineECU".to_string()));
    n.start(Some(engine_dbc()));
    assert!(
        n.log_snapshot().iter().any(|l| l.contains("Nope")),
        "the unknown signal is named: {:?}",
        n.log_snapshot()
    );

    let mut n = node("on start { print(Ghost.RPM); }");
    n.attached = Some((0, "EngineECU".to_string()));
    n.start(Some(engine_dbc()));
    assert!(
        n.log_snapshot().iter().any(|l| l.contains("Ghost")),
        "the unknown message is named: {:?}",
        n.log_snapshot()
    );

    let mut n = node("on start { print(EngineStatus[\"Nope\"]); }");
    n.attached = Some((0, "EngineECU".to_string()));
    n.start(Some(engine_dbc()));
    assert!(
        n.log_snapshot().iter().any(|l| l.contains("Nope")),
        "the bracket form gets the same assembly check: {:?}",
        n.log_snapshot()
    );
}

/// `on fr slot` wakes on a FlexRay arrival, and the frame context inside it
/// *is* that arrival: `frame_id()` the slot, `frame_dlc()`/`frame_byte()`
/// the payload, `fr_cycle()` the cycle. The reaction leaves on the node's
/// CAN channel -- the gateway shape, and the only direction we can send.
#[test]
fn a_flexray_arrival_wakes_its_slot_handler() {
    let mut n = node(
        r#"
                on fr slot 13 {
                    print(frame_id(), fr_cycle(), frame_dlc(), frame_byte(0));
                    send(0x300, frame_byte(0));
                }
            "#,
    );
    n.start(None);
    assert!(
        n.waits_on_flexray(),
        "the declaration alone is what the bus gates on"
    );
    let out = n.dispatch_fr_row(0, 13, 4, &[0x2A, 0x00], &HostInput::default());
    assert_eq!(n.log_snapshot(), ["13 4 2 42"]);
    assert_eq!(out.len(), 1, "the handler's send queued");
    assert_eq!(out[0].0, 0x300);
    assert_eq!(out[0].2[0], 0x2A, "the payload byte came through");
    // Another slot's arrival does not wake it.
    assert!(
        n.dispatch_fr_row(0, 14, 4, &[], &HostInput::default())
            .is_empty(),
        "slot 14 is not slot 13"
    );
    // A node that never declared one is not even asked.
    let mut idle = node("on start { }");
    idle.start(None);
    assert!(!idle.waits_on_flexray());
    // ...and an errored node stops listening, like every other event.
    let mut bad = node("on fr slot 13 { print(1 / 0); }");
    bad.start(None);
    bad.dispatch_fr_row(0, 13, 0, &[], &HostInput::default());
    assert!(bad.errored());
    assert!(!bad.waits_on_flexray(), "a dead node stays quiet");
}

/// The cluster is part of the address, and a slot number is not a CAN id:
/// `on fr 0 slot 13` stays quiet for FR1's slot 13, the any-cluster form
/// answers both, and `on message 0xD` -- the same number in the other
/// numbering space -- answers neither.
#[test]
fn a_flexray_handler_can_name_its_cluster() {
    let mut n = node(
        r#"
                on fr 0 slot 13 { print("fr0"); }
                on fr slot 20 { print("any"); }
                on message 0xD { print("can13"); }
            "#,
    );
    n.start(None);
    n.dispatch_fr_row(1, 13, 0, &[], &HostInput::default());
    assert_eq!(
        n.log_snapshot(),
        Vec::<String>::new(),
        "FR1's slot 13 is not FR0's"
    );
    n.dispatch_fr_row(0, 13, 0, &[], &HostInput::default());
    n.dispatch_fr_row(1, 20, 0, &[], &HostInput::default());
    n.dispatch_fr_row(0, 20, 0, &[], &HostInput::default());
    assert_eq!(
        n.log_snapshot(),
        ["fr0", "any", "any"],
        "the named cluster once, the any-cluster form on both buses"
    );
}

/// `fr_cycle()` is the arrival's own cycle, and 0 outside a FlexRay event:
/// a CAN frame has no cycle to hand over, and a silent zero there must not
/// look like cycle 0 of a real one.
#[test]
fn fr_cycle_reads_the_arrival_and_nothing_else() {
    let mut n = node(
        r#"
                on message 0x100 { print("can", fr_cycle()); }
                on fr slot 5 { print("fr", fr_cycle()); }
            "#,
    );
    n.start(None);
    n.dispatch_frame(0, 0x100, false, false, &[], &HostInput::default());
    n.dispatch_fr_row(0, 5, 3, &[], &HostInput::default());
    assert_eq!(
        n.log_snapshot(),
        ["can 0", "fr 3"],
        "the cycle belongs to the FlexRay arrival"
    );
}

/// A handler failure inside an `on fr slot` run stops the node, exactly as
/// on the CAN side -- one bad loop must not spam the log every macrotick.
#[test]
fn a_failing_flexray_handler_parks_the_node() {
    let mut n = node(
        r#"
                let hits = 0;
                on fr slot 5 { hits = hits + 1; print(1 / 0); }
            "#,
    );
    n.start(None);
    n.dispatch_fr_row(0, 5, 0, &[], &HostInput::default());
    let log = n.log_snapshot();
    assert!(
        log.last().is_some_and(|l| l.starts_with("[error]")),
        "a failing FR handler parks the node: {log:?}"
    );
    assert!(n.errored());
    assert!(
        n.dispatch_fr_row(0, 5, 0, &[], &HostInput::default())
            .is_empty(),
        "a parked node answers no further arrival"
    );
}
/// `sys_set` queues exactly the key and value; the old `@sysvar::`
/// spelling is gone.
#[test]
fn sys_set_queues_the_key_and_value() {
    let mut n = node(
        r#"
                on message 0x100 {
                    sys_set("Demo::Setpoint", 42);
                }
            "#,
    );
    n.start(None);
    n.dispatch_frame(0, 0x100, false, false, &[], &HostInput::default());
    let sets = n.take_sys_sets();
    assert_eq!(sets, vec![("Demo::Setpoint".to_string(), 42.0)]);
}

#[test]
fn extended_ids_send_flagged_extended() {
    // The outbox carries the raw id plus the "force extended" bit
    // (`send_ext` sets it); the core ORs it with the id-size rule
    // when building the frame.
    let mut n = node("on message 0x100 { send(0x18FF10, 1); }");
    n.start(None);
    let out = n.dispatch_frame(0, 0x100, false, false, &[], &HostInput::default());
    assert_eq!(out, vec![(0x18FF10, false, vec![1], 0)]);
}

const SIG_DBC: &str = r#"VERSION "roxy-can node sig test"

NS_ :

BU_: ECU

BO_ 512 Status: 8 ECU
 SG_ RPM : 0|16@1+ (0.25,0) [0|0] "" ECU
"#;

#[test]
fn set_sig_without_a_can_model_is_refused() {
    let mut n = node(
        r#"
                let buf = bytes(8);
                on start { set_sig(buf, 0x200, "RPM", 1000); }
            "#,
    );
    n.start(None);
    assert!(
        n.log_snapshot()
            .iter()
            .any(|line| line.contains("no CAN model")),
        "a channel without a model cannot encode: {:?}",
        n.log_snapshot()
    );
}

/// The same builtins through the model codec, which is what the core
/// passes once a channel has plans.
#[test]
fn set_sig_and_get_sig_use_the_model_codec() {
    use crate::model::{ClusterId, ClusterSource, ElementIds, PlanCache, import_symbol_table};
    let db = std::sync::Arc::new(crate::dbc::load_dbc_str(SIG_DBC).unwrap());
    let mut ids = ElementIds::new();
    let outcome = import_symbol_table(
        &db,
        "CAN1",
        ClusterId(1),
        &ClusterSource::Dbc {
            path: String::new(),
        },
        &mut ids,
        500,
        2000,
    );
    let (plans, _) = PlanCache::build(&outcome.comm, 1);
    let codec = (
        std::sync::Arc::new(outcome.comm),
        std::sync::Arc::new(plans),
        ClusterId(1),
    );
    let mut n = node(
        r#"
                let buf = bytes(8);
                on start {
                    set_sig(buf, 0x200, "RPM", 1000);
                    print(get_sig(buf, 0x200, "RPM"));
                }
                on message 0x1 { send(0x200, buf); }
            "#,
    );
    n.start_with(None, None, None, Some(codec));
    assert!(n.running(), "log: {:?}", n.log_snapshot());
    assert_eq!(n.log_snapshot(), ["1000.0"]);
    let out = n.dispatch_frame(0, 0x1, false, false, &[], &HostInput::default());
    assert_eq!(
        out,
        vec![(0x200, false, vec![0xA0, 0x0F, 0, 0, 0, 0, 0, 0], 0)]
    );
}

/// A catalog frame send joins the numeric send-set facts used by the
/// ownership and reverse-coverage checks.
#[test]
fn frame_sends_join_the_static_send_set() {
    let db = crate::dbc::load_dbc_str(crate::sample_data::SAMPLE_DBC).unwrap();
    let mut script = crate::script::compile(
        r#"
            on timer 10 {
                let f = frame(CAN1.EngineStatus);
                f.EngineSpeed = 1000;
                send_frame(f);
            }
            "#,
    )
    .unwrap();
    resolve_frame_send_refs(&mut script, &db);
    assert!(
        script
            .send_refs
            .iter()
            .any(|(_, id, ext)| (*id, *ext) == (0x100, false)),
        "{:?}",
        script.send_refs
    );
}

/// Constant writes outside a signal's declared range are reported at
/// assembly, while in-range constants stay silent.
#[test]
fn constant_frame_writes_warn_outside_the_declared_range() {
    let db = crate::dbc::load_dbc_str(crate::sample_data::SAMPLE_DBC).unwrap();
    let script = crate::script::compile(
        r#"
            on timer 10 {
                let f = frame(CAN1.EngineStatus);
                f.EngineSpeed = 1000;
                f.EngineTemp = -50;
                send_frame(f);
            }
            "#,
    )
    .unwrap();
    let lines = frame_checks(&script, Some(&db), None, None);
    assert_eq!(lines.len(), 1, "{lines:?}");
    assert!(
        lines[0].contains("EngineTemp") && lines[0].contains("outside"),
        "{lines:?}"
    );
}
