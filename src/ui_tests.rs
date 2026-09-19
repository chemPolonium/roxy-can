//! Headless UI smoke harness: imgui produces full logical frames with no
//! renderer attached, so every window's *draw path* -- the one layer
//! plain unit tests cannot reach -- executes here. A panic in any draw
//! function fails the test instead of killing a running app (the
//! 2026-09-13 script-editor crash shipped exactly this way: all logic
//! tests were green, the draw path had never run).
//!
//! The harness is deliberately minimal: one font, a fixed display size,
//! a handful of frames per scenario. Widgets that need real clicks
//! (dialogs' Apply buttons) are driven by arming their draft state
//! directly, the way a user's first frame would see it.

use crate::app::{App, TraceRow};
use crate::observe::SigKey;
use crate::can::frame::CanFrame;
use dear_imgui_rs::{ConfigFlags, Context, FontConfig, FontSource};
use std::sync::Mutex;

/// imgui's current context is process-global, so tests that each build
/// a context must not run concurrently.
static UI_LOCK: Mutex<()> = Mutex::new(());

const FONT: &[u8] = include_bytes!("../fonts/Inconsolata-Regular.ttf");

/// An imgui context wired like the app's (docking on, no ini file) but
/// with no platform and no renderer behind it.
fn harness() -> Context {
    let mut context = Context::create();
    context.set_ini_filename(None::<String>).unwrap();
    let mut flags = context.io().config_flags();
    flags.insert(ConfigFlags::DOCKING_ENABLE);
    context.io_mut().set_config_flags(flags);
    // Windows never move from their content area -- the observer signal
    // lists drag-reorder from inside their windows.
    context
        .io_mut()
        .set_config_windows_move_from_title_bar_only(true);
    context.io_mut().set_display_size([1280.0, 800.0]);
    context.io_mut().set_delta_time(1.0 / 60.0);
    // # Safety: embedded font bytes are a complete TTF.
    context.font_atlas().add_font(&[unsafe {
        FontSource::ttf_data_with_size(FONT, 13.0)
            .with_config(FontConfig::new().pixel_snap_h(true).oversample_h(1))
    }]);
    // The renderer normally triggers the atlas build in its NewFrame;
    // with no renderer, claim the legacy atlas and build it here or
    // frame() asserts on a missing texture. The claim lives to the end
    // of the harness scope.
    context
        .font_atlas()
        .try_claim_legacy_renderer()
        .expect("legacy font atlas")
        .build();
    context
}

/// Runs `n` full frames of the real UI render path. `ctx.render_legacy()`
/// ends the frame (the renderer normally does this in the app).
fn frames(app: &mut App, ctx: &mut Context, n: usize) {
    for _ in 0..n {
        // Script editors bind to the context and are created by main
        // outside the frame; the harness does that queue's work here.
        if !app.pending_editors.is_empty() {
            let wanted: Vec<u64> = std::mem::take(&mut app.pending_editors);
            for id in wanted {
                let mut editor = dear_imgui_cte::TextEditor::create(ctx);
                crate::ui::script_editor::configure_new_editor(&mut editor);
                app.editors.entry(id).or_insert(editor);
            }
        }
        let ui = ctx.frame();
        crate::ui::render(app, ui);
        let _ = ctx.render_legacy();
    }
}

/// Puts a cluster description on one FlexRay bus, the way the Buses window's
/// picker does. False when the asset is missing or does not parse, so a test
/// can skip rather than draw an empty table.
fn load_fr_db(app: &mut App, bus: u8, path: &str) -> bool {
    let Ok(bytes) = std::fs::read(path) else {
        return false;
    };
    let text = crate::dbc::text_from_bytes(bytes);
    match crate::fr_db::FrDb::parse(&text) {
        Ok(db) => {
            app.fr_buses.insert(
                bus,
                crate::app::FrBusCfg {
                    path: path.to_string(),
                    db: std::sync::Arc::new(db),
                },
            );
            true
        }
        Err(_) => false,
    }
}

/// Regression driver for the observer signal drag-reorder: a fixed
/// window draws the shared siglist, then synthesized mouse events
/// (press on the row-leading grip, drag, release) must flip the two
/// rows -- while the same gesture on the visibility checkbox must NOT
/// (the checkbox only toggles; the grip is the handle). Panics with the
/// working offset so failures name the exact geometry.
#[test]
fn siglist_drag_reorders_a_row() {
    use crate::app::PopupTarget;
    let _ui_lock = UI_LOCK.lock().unwrap();
    let mut ctx = harness();
    let mut app = App::headless();
    app.new_graphics_window();
    let keys = [
        SigKey::can(0, 0x100, false, "Alpha"),
        SigKey::can(0, 0x101, false, "Beta"),
    ];
    for key in &keys {
        app.set_win_signal(PopupTarget::Graphics(0), key.clone(), true);
    }
    assert_eq!(app.graphics[0].signals.len(), 2);

    let order = |app: &App| -> Vec<String> {
        app.graphics[0].signals.iter().map(|s| s.key.name().to_string()).collect()
    };
    let before = order(&app);

    // One long frame pass per candidate hover position: press, drag
    // 40 px down, release. Any candidate that lands on a row's grip
    // must flip the two rows.
    let mut drag_from = |app: &mut App, probe_x: f32, probe_y: f32| -> bool {
        for _ in 0..2 {
            let ui = ctx.frame();
            ui.set_window_pos([10.0, 10.0]);
            ui.window("siglist probe")
                .size([320.0, 400.0], dear_imgui_rs::Condition::Always)
                .build(|| crate::ui::siglist::draw(app, ui, crate::ui::siglist::ListKind::Graphics(0)));
            let _ = ctx.render_legacy();
        }
        ctx.io_mut().add_mouse_pos_event([probe_x, probe_y]);
        ctx.io_mut()
            .add_mouse_button_event(dear_imgui_rs::MouseButton::Left, true);
        for _ in 0..2 {
            let ui = ctx.frame();
            ui.set_window_pos([10.0, 10.0]);
            ui.window("siglist probe")
                .size([320.0, 400.0], dear_imgui_rs::Condition::Always)
                .build(|| crate::ui::siglist::draw(app, ui, crate::ui::siglist::ListKind::Graphics(0)));
            let _ = ctx.render_legacy();
        }
        ctx.io_mut().add_mouse_pos_event([probe_x, probe_y + 40.0]);
        {
            let ui = ctx.frame();
            ui.set_window_pos([10.0, 10.0]);
            ui.window("siglist probe")
                .size([320.0, 400.0], dear_imgui_rs::Condition::Always)
                .build(|| crate::ui::siglist::draw(app, ui, crate::ui::siglist::ListKind::Graphics(0)));
            let _ = ctx.render_legacy();
        }
        ctx.io_mut()
            .add_mouse_button_event(dear_imgui_rs::MouseButton::Left, false);
        {
            let ui = ctx.frame();
            ui.set_window_pos([10.0, 10.0]);
            ui.window("siglist probe")
                .size([320.0, 400.0], dear_imgui_rs::Condition::Always)
                .build(|| crate::ui::siglist::draw(app, ui, crate::ui::siglist::ListKind::Graphics(0)));
            let _ = ctx.render_legacy();
        }
        order(app) != before
    };

    let mut working: Option<(f32, f32)> = None;
    let mut last_lines: Vec<String> = Vec::new();
    'probe: for probe_y in (100..140).step_by(2) {
        for probe_x in [70.0, 74.0, 78.0] {
            if drag_from(&mut app, probe_x, probe_y as f32) {
                working = Some((probe_x, probe_y as f32));
                break 'probe;
            }
        }
        last_lines = app
            .graphics[0]
            .signals
            .iter()
            .map(|s| s.key.name().to_string())
            .collect();
    }
    assert!(
        working.is_some(),
        "no hover offset started a reorder; signals still {last_lines:?}"
    );
    let (gx, gy) = working.unwrap();
    println!(
        "siglist drag works from grip hover ({gx},{gy}) (order {:?} -> {:?})",
        before,
        order(&app)
    );

    // The gesture that used to drag rows -- pressing the checkbox --
    // must now only toggle: same probe grid over the checkbox column,
    // order must survive every attempt. The positive phase above left
    // the pair flipped; restore the original order first.
    app.graphics[0].signals.clear();
    for key in &keys {
        app.set_win_signal(PopupTarget::Graphics(0), key.clone(), true);
    }
    assert_eq!(order(&app), before);
    for probe_y in (100..140).step_by(2) {
        drag_from(&mut app, 90.0, probe_y as f32);
        assert_eq!(
            order(&app),
            before,
            "the checkbox column must not start a reorder (y={probe_y})"
        );
    }
    // Sanity: x=90 really is the checkbox column -- a plain click there
    // toggles visibility (the drag probe above releases 40 px below the
    // press, which no checkbox counts as its own click).
    let mut clicked = false;
    for probe_y in (100..140).step_by(2) {
        let sigs = &app.graphics[0].signals;
        let visible_before: Vec<bool> = sigs.iter().map(|s| s.visible).collect();
        for _ in 0..2 {
            let ui = ctx.frame();
            ui.set_window_pos([10.0, 10.0]);
            ui.window("siglist probe")
                .size([320.0, 400.0], dear_imgui_rs::Condition::Always)
                .build(|| crate::ui::siglist::draw(&mut app, ui, crate::ui::siglist::ListKind::Graphics(0)));
            let _ = ctx.render_legacy();
        }
        ctx.io_mut().add_mouse_pos_event([90.0, probe_y as f32]);
        ctx.io_mut()
            .add_mouse_button_event(dear_imgui_rs::MouseButton::Left, true);
        for _ in 0..2 {
            let ui = ctx.frame();
            ui.set_window_pos([10.0, 10.0]);
            ui.window("siglist probe")
                .size([320.0, 400.0], dear_imgui_rs::Condition::Always)
                .build(|| crate::ui::siglist::draw(&mut app, ui, crate::ui::siglist::ListKind::Graphics(0)));
            let _ = ctx.render_legacy();
        }
        ctx.io_mut()
            .add_mouse_button_event(dear_imgui_rs::MouseButton::Left, false);
        for _ in 0..2 {
            let ui = ctx.frame();
            ui.set_window_pos([10.0, 10.0]);
            ui.window("siglist probe")
                .size([320.0, 400.0], dear_imgui_rs::Condition::Always)
                .build(|| crate::ui::siglist::draw(&mut app, ui, crate::ui::siglist::ListKind::Graphics(0)));
            let _ = ctx.render_legacy();
        }
        let sigs = &app.graphics[0].signals;
        if sigs.iter().map(|s| s.visible).collect::<Vec<bool>>() != visible_before {
            clicked = true;
            break;
        }
    }
    assert!(
        clicked,
        "no plain click at x=90 toggled a checkbox -- the negative assertion proves nothing"
    );
}

/// Live probe against the machine's real vxlapi driver: enumeration must
/// succeed (or report no hardware) without panicking on layout mismatch.
#[test]
fn vector_enumerate_hits_the_real_driver() {
    match crate::hw::vector::enumerate() {
        Ok(channels) => {
            println!("vector channels: {}", channels.len());
            for c in &channels {
                println!(
                    "  ch{}: {} [can:{} fr:{}]",
                    c.index, c.name, c.can, c.flexray
                );
            }
        }
        Err(e) => println!("vector enumerate: {e}"),
    }
}

/// The FlexRay slice's first deliverable: a FlexRay-capable channel
/// (VN7640 port) is listed by `enumerate_flexray` and excluded from the
/// CAN attach dropdown. On a CAN-only machine both halves are trivially
/// consistent -- the interesting run happens with the VN7640 attached.
#[test]
fn flexray_channels_are_separate_from_can_channels() {
    let all = crate::hw::vector::enumerate();
    let fr = crate::hw::vector::enumerate_flexray();
    match (all, fr) {
        (Ok(all), Ok(fr)) => {
            let fr_indexes: Vec<i32> = fr.iter().map(|c| c.index).collect();
            for c in &all {
                assert!(
                    !fr_indexes.contains(&c.index) || c.flexray,
                    "channel {} is listed as FlexRay but not CAN-attachable",
                    c.index
                );
            }
            let attachable: Vec<i32> = crate::hw::enumerate_all()
                .unwrap_or_default()
                .iter()
                .filter(|a| a.driver == crate::hw::HwDriver::Vector)
                .map(|a| a.index)
                .collect();
            for c in &all {
                if c.flexray && !c.can {
                    assert!(
                        !attachable.contains(&c.index),
                        "FlexRay channel {} leaked into the CAN attach list",
                        c.index
                    );
                }
            }
            println!(
                "vector: {} channel(s), {} flexray-capable",
                all.len(),
                fr.len()
            );
        }
        (Err(e), _) | (_, Err(e)) => println!("vector enumerate: {e}"),
    }
}

/// Isolates the LoadLibrary step of the Vector binding.
#[test]
fn load_library_probe() {
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn LoadLibraryW(name: *const u16) -> *mut core::ffi::c_void;
        fn GetLastError() -> u32;
    }
    let name: Vec<u16> = "vxlapi64.dll\0".encode_utf16().collect();
    let module = unsafe { LoadLibraryW(name.as_ptr()) };
    println!(
        "LoadLibraryW(vxlapi64.dll) = {:?}  err={}",
        module,
        unsafe { GetLastError() }
    );
}

/// Every panel and one of every observer window draw together -- the
/// "open everything" layout a curious user ends up with.
#[test]
fn every_window_draws_without_panicking() {
    let _ui_lock = UI_LOCK.lock().unwrap();
    let mut ctx = harness();
    let mut app = App::headless();
    app.new_trace_window();
    app.new_msg_window();
    app.new_stats_window();
    app.new_graphics_window();
    app.new_data_window();
    app.new_state_window();
    app.show_buses = true;
    app.show_triggers = true;
    app.show_bus_stats = true;
    app.show_tx = true;
    app.show_network = true;
    app.show_spec = true;
    app.show_measurement = true;
    app.show_sysvars = true;
    app.show_write = true;
    // Give the observers something to draw: a defined system variable
    // (its own manager row and observable stream) and a generator row.
    app.send(crate::bus::BusCommand::DefineSysVar(crate::bus::SysVarDef {
        namespace: "Demo".to_string(),
        name: "Speed".to_string(),
        init: 10.0,
        min: None,
        max: None,
        unit: String::new(),
        comment: String::new(),
    }));
    app.settle();
    frames(&mut app, &mut ctx, 5);
}

/// The script editor hosts the CTE text widget plus the fact sidebar:
/// run it with source that compiles and with source that fails, over
/// several frames so the fact cache and the marker refresh both run.
#[test]
fn script_editor_draws_with_highlighting() {
    let _ui_lock = UI_LOCK.lock().unwrap();
    let mut ctx = harness();
    let mut app = App::headless();
    app.send(crate::bus::BusCommand::AddNode {
        name: "s".to_string(),
        channel: 0,
        attached: None,
    });
    app.settle();
    let id = app.snap.nodes[0].id;
    let source = concat!(
        "on start {\n",
        "    // 行注释 comment\n",
        "    let b = bytes(8);\n",
        "    set_sig(b, 0x100, \"RPM\", 3000);\n",
        "    emit_value(\"X\", 1.5 + 2);\n",
        "}\n",
    );
    app.send(crate::bus::BusCommand::SetNodeSource {
        id,
        source: source.to_string(),
    });
    app.open_script_editor(id);
    app.settle();
    frames(&mut app, &mut ctx, 5);

    // The broken draft: the editor re-seeds from the model and the
    // error marker lands on the offending line.
    app.send(crate::bus::BusCommand::SetNodeSource {
        id,
        source: "on start {\n    send();\n}".to_string(),
    });
    app.settle();
    frames(&mut app, &mut ctx, 3);
}

/// The Trace table draws the merged CAN + FlexRay row kinds -- FR rows
/// carry the frame name from the display database and the right-click
/// copy popup compiles. The producer (the FR watch drain) is
/// hardware-bound, so the merged cache is injected directly; with the
/// text gate frozen the cache survives the frames.
#[test]
fn trace_draws_merged_flexray_rows_without_panicking() {
    let _ui_lock = UI_LOCK.lock().unwrap();
    let mut ctx = harness();
    let mut app = App::headless();
    app.new_trace_window();
    // The real description, when present; the draw path is the same for
    // any database, and for none.
    load_fr_db(&mut app, 0, "assets/arxml/PowerTrain.arxml");
    app.settle();
    app.text_fresh = false;
    app.trace_windows[0].rows = vec![
        TraceRow::Can(CanFrame {
            t_us: 4_000,
            channel: 0,
            id: 0x100,
            extended: false,
            len: 2,
            data: {
                let mut d = [0u8; crate::can::frame::MAX_CAN_FD_LEN];
                d[..2].copy_from_slice(&[0xAB, 0xCD]);
                d
            },
            dir: crate::can::frame::Direction::Rx,
            flags: crate::can::frame::FrameFlags::NONE,
        }),
        TraceRow::Fr(crate::trace::FrRow {
            bus: 0,
            t_us: 5_000,
            ab: 1,
            slot: 3,
            cycle: 4,
            payload: vec![1, 2, 3],
            header_crc: 0xBEEF,
            flags: 0,
            name: Some("ChassisStatus".to_string()),
        }),
        TraceRow::FrSig {
            t_us: 5_000,
            slot: 3,
            signal: "WheelSpeedFL".to_string(),
            value: "3.5 km/h  (7h)".to_string(),
        },
    ];
    frames(&mut app, &mut ctx, 5);
}

/// The signal-selection popup renders its FlexRay section -- a checkbox per
/// decoded FlexRay signal -- against a real cluster description without
/// panicking. This is the ImGui path an observer uses to add a FlexRay
/// signal; the interaction can't be driven headless, but the draw must not
/// break on the synthetic FlexRay keys.
#[test]
fn flexray_signal_picker_draws_without_panicking() {
    use crate::app::PopupTarget;
    let _ui_lock = UI_LOCK.lock().unwrap();
    let mut ctx = harness();
    let mut app = App::headless();
    app.new_graphics_window();
    if load_fr_db(&mut app, 0, "assets/arxml/PowerTrain.arxml") {
        assert!(
            app.fr_db(0)
                .expect("the bus has a description")
                .slot_signals()
                .iter()
                .any(|(_, _, s)| !s.is_empty()),
            "the bundled FlexRay description should expose signals to pick"
        );
    }
    // A second cluster on a second bus: the same slot number there names a
    // different signal, and the section has to show both.
    load_fr_db(&mut app, 1, "assets/fibex/PowerTrain_v2.xml");
    assert_eq!(app.fr_buses.len(), 2, "both descriptions loaded");
    app.settle();
    app.popup_target = Some(PopupTarget::Graphics(0));
    app.show_id_filter = true;
    frames(&mut app, &mut ctx, 3);
}

/// The Buses window's FlexRay section is a list now: two watched clusters,
/// each with its own row, its own parked marker and its own detach, above the
/// combo that offers only the channels still free. Drawing it is the point --
/// a view shaped like the old single `Option<FrWatch>` panics right where a
/// user looks for their second bus, and the schedule table below has to carry
/// two clusters' frames in two different rows.
#[test]
fn the_buses_window_draws_two_flexray_watches() {
    let _ui_lock = UI_LOCK.lock().unwrap();
    let mut ctx = harness();
    let mut app = App::headless();
    app.show_buses = true;
    app.hw.attach_fr_mock(0, 5);
    app.hw.attach_fr_mock(1, 6);
    load_fr_db(&mut app, 0, "assets/arxml/PowerTrain.arxml");
    load_fr_db(&mut app, 1, "assets/fibex/PowerTrain_v2.xml");
    // A third bus with a description and no port at all: the list has to show
    // it, because loading a description is state the user can create and undo.
    load_fr_db(&mut app, 2, "assets/fibex/PowerTrain2_v2.xml");
    app.refresh_snapshot();
    assert_eq!(
        app.snap
            .fr_watches
            .iter()
            .map(|w| (w.bus, w.channel_index))
            .collect::<Vec<_>>(),
        vec![(0, 5), (1, 6)],
        "both watches are listed, in bus order"
    );
    assert_eq!(
        app.fr_buses.keys().copied().collect::<Vec<_>>(),
        [0, 1, 2],
        "and one of the three buses is described-only"
    );
    frames(&mut app, &mut ctx, 3);
}

/// The armed editor popups -- a system variable and three trigger kinds
/// (signal cross, FlexRay frame with and without a description) -- render from
/// their draft state across frames (the frame-persistence path the stale-draft
/// bug lived in).
#[test]
fn editor_popups_draw_without_panicking() {
    let _ui_lock = UI_LOCK.lock().unwrap();
    let mut ctx = harness();
    let mut app = App::headless();
    app.sysvar_draft = Some(crate::ui::sysvars::SysVarDraft::for_add());
    app.settle();
    frames(&mut app, &mut ctx, 3);
    app.sysvar_draft = None;

    app.add_signal_trigger();
    app.settle();
    app.trig_draft = app
        .snap
        .triggers
        .first()
        .map(|t| crate::ui::triggers::TrigDraft::new(0, t.cond.clone(), t.action));
    frames(&mut app, &mut ctx, 3);
    app.trig_draft = None;

    // The FlexRay editor, twice: once with nothing loaded (the cluster combo
    // has to fall back to the rule's own index instead of an empty list), then
    // with a description on bus 0 -- which is also what lets `add_fr_trigger`
    // default the slot to one the schedule really has.
    app.add_fr_trigger();
    frames(&mut app, &mut ctx, 3);
    load_fr_db(&mut app, 0, "assets/arxml/PowerTrain.arxml");
    app.add_fr_trigger();
    app.settle();
    assert_eq!(
        app.snap.triggers.last().map(|t| t.cond.fr_bus()),
        Some(Some(0)),
        "the new rule points at the cluster with a description"
    );
    frames(&mut app, &mut ctx, 3);
    // ...and its signal crossing sibling, whose picker lists the signals that
    // slot declares. `push_trigger` already opens the editor on the new row.
    app.add_fr_signal_trigger();
    app.settle();
    assert!(
        matches!(
            app.snap.triggers.last().map(|t| &t.cond),
            Some(crate::trigger::TriggerCond::FrSignalCross { .. })
        ),
        "the row is the crossing kind"
    );
    assert!(app.trig_draft.is_some(), "and its editor is open");
    frames(&mut app, &mut ctx, 3);
}

/// A plot window must not starve the replay it is plotting. The real
/// FlexRay capture (60k rows, ~50 s) runs behind an open Graphics window with
/// one FlexRay curve selected; the clock has to keep pace with the wall clock
/// and no single lap may balloon. This is the headless shape of the report
/// that switching to a Graphics desktop froze playback.
#[test]
fn a_plot_window_never_starves_the_replay() {
    use crate::app::PopupTarget;
    use std::time::{Duration, Instant};
    let _ui_lock = UI_LOCK.lock().unwrap();
    if !std::path::Path::new("assets/fibex/Logging.blf").exists() {
        println!("assets/fibex/Logging.blf not present -- skipped");
        return;
    }
    let mut ctx = harness();
    // The real app: the core runs on its own thread, which is the shape
    // the stall was reported in.
    let mut app = App::new();
    app.load_log("assets/fibex/Logging.blf");
    let bytes = std::fs::read("assets/arxml/PowerTrain.arxml").expect("arxml");
    let text = crate::dbc::text_from_bytes(bytes);
    let db = std::sync::Arc::new(crate::fr_db::FrDb::parse(&text).expect("parses"));
    let (slot, _, sigs) = db
        .slot_signals()
        .into_iter()
        .find(|(_, frame, _)| frame.contains("13"))
        .expect("slot 13 exposes signals");
    app.fr_buses.insert(
        0,
        crate::app::FrBusCfg {
            path: "assets/arxml/PowerTrain.arxml".into(),
            db,
        },
    );
    app.push_fr_db_to_core();
    app.new_graphics_window();
    app.set_win_signal(
        PopupTarget::Graphics(0),
        crate::app::fr_signal_key(0, slot, &sigs[0]),
        true,
    );
    app.replay();

    let start = Instant::now();
    let mut worst_lap = Duration::ZERO;
    let mut laps = 0usize;
    while start.elapsed() < Duration::from_millis(1500) {
        std::thread::sleep(Duration::from_millis(5));
        let t = Instant::now();
        app.update();
        {
            let ui = ctx.frame();
            crate::ui::render(&mut app, ui);
            let _ = ctx.render_legacy();
        }
        worst_lap = worst_lap.max(t.elapsed());
        laps += 1;
    }
    let wall = start.elapsed().as_secs_f64();
    let (pos, dur) = app.replay_position().expect("a replay timeline");
    assert!(
        app.snap.measuring,
        "the run is still measuring after {wall:.2} s of plotting"
    );
    assert!(
        pos >= wall * 0.25,
        "the log clock kept pace: {pos:.2} s of log in {wall:.2} s of wall \
         ({dur:.1} s log total, {laps} laps, worst lap {worst_lap:?})"
    );
    assert!(
        worst_lap < Duration::from_secs(2),
        "one lap took {worst_lap:?} with a plot window open ({laps} laps, pos={pos:.2})"
    );
    app.stop();
}

/// Replaying a recording this tool wrote must keep pace, with a plot window
/// open and one curve of each bus. Such a file states no duration in its
/// header, so the reported position used to be capped by a lagging estimate of
/// it -- which read as the replay freezing at a fraction of a second while the
/// bus thread carried on. The header now carries the traffic span and the
/// finite playhead is never capped.
#[test]
fn a_replay_of_our_own_recording_keeps_pace_behind_a_plot_window() {
    use crate::app::PopupTarget;
    use crate::can::frame::{CanFrame, Direction, FrameFlags, MAX_CAN_FD_LEN};
    use crate::log::blf::BlfWriter;
    use crate::trace::FrRow;
    use std::time::{Duration, Instant};
    let _ui_lock = UI_LOCK.lock().unwrap();
    let path =
        std::env::temp_dir().join(format!("roxy_can_pace_{}.blf", std::process::id()));
    {
        let mut w = BlfWriter::create(&path.to_string_lossy()).expect("create");
        for i in 0..10_000u64 {
            if i % 10 == 0 {
                w.write(&CanFrame {
                    t_us: i * 1_000,
                    channel: 0,
                    id: 0x100,
                    extended: false,
                    len: 8,
                    data: [1; MAX_CAN_FD_LEN],
                    dir: Direction::Rx,
                    flags: FrameFlags::NONE,
                });
            }
            w.write_fr(&FrRow {
                bus: 0,
                t_us: i * 1_000,
                ab: 0,
                slot: 13,
                cycle: (i % 16) as u8,
                payload: vec![0x00, (i % 200) as u8, 0, 0, 0, 0, 0, 0],
                header_crc: 0,
                flags: 0,
                name: None,
            });
        }
        w.finish().expect("finish");
    }
    let mut ctx = harness();
    let mut app = App::new();
    app.send(crate::bus::BusCommand::LoadDbc {
        ch: 0,
        paths: vec!["assets/sample.dbc".to_string()],
    });
    let text =
        crate::dbc::text_from_bytes(std::fs::read("assets/arxml/PowerTrain.arxml").expect("arxml"));
    let db = std::sync::Arc::new(crate::fr_db::FrDb::parse(&text).expect("parses"));
    let (slot, _, sigs) = db
        .slot_signals()
        .into_iter()
        .find(|(_, frame, _)| frame.contains("13"))
        .expect("slot 13 exposes signals");
    app.fr_buses.insert(
        0,
        crate::app::FrBusCfg {
            path: "assets/arxml/PowerTrain.arxml".into(),
            db,
        },
    );
    app.push_fr_db_to_core();
    app.new_graphics_window();
    app.set_win_signal(
        PopupTarget::Graphics(0),
        crate::observe::SigKey::can(0, 0x100, false, "EngineSpeed"),
        true,
    );
    app.set_win_signal(
        PopupTarget::Graphics(0),
        crate::app::fr_signal_key(0, slot, &sigs[0]),
        true,
    );
    app.load_log(&path.to_string_lossy());
    app.replay();

    let start = Instant::now();
    while start.elapsed() < Duration::from_millis(1200) {
        std::thread::sleep(Duration::from_millis(5));
        app.update();
        {
            let ui = ctx.frame();
            crate::ui::render(&mut app, ui);
            let _ = ctx.render_legacy();
        }
    }
    let wall = start.elapsed().as_secs_f64();
    let (pos, dur) = app.replay_position().expect("a replay timeline");
    app.stop();
    std::fs::remove_file(&path).ok();
    assert!(
        dur > 9.0,
        "the file states its 10 s span: {dur:.2} s -- a recording with no \
         header duration leaves the progress bar guessing"
    );
    assert!(
        pos >= wall * 0.5,
        "the log clock reached {pos:.2} s in {wall:.2} s of wall time of a \
         {dur:.1} s recording"
    );
}
