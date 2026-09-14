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

use crate::app::App;
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
            let wanted: Vec<u64> = app.pending_editors.drain(..).collect();
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
        (0u8, 0x100u32, false, "Alpha".to_string()),
        (0, 0x101, false, "Beta".to_string()),
    ];
    for key in &keys {
        app.set_win_signal(PopupTarget::Graphics(0), key.clone(), true);
    }
    assert_eq!(app.graphics[0].signals.len(), 2);

    let order = |app: &App| -> Vec<String> {
        app.graphics[0].signals.iter().map(|s| s.key.3.clone()).collect()
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
            .map(|s| s.key.3.clone())
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

/// The two armed editor popups render from their draft state across
/// frames (the frame-persistence path the stale-draft bug lived in).
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
    if let Some(t) = app.trig_draft.clone() {
        let _ = t;
    }
    app.trig_draft = app
        .snap
        .triggers
        .first()
        .map(|t| crate::ui::triggers::TrigDraft::new(0, t.cond.clone(), t.action));
    frames(&mut app, &mut ctx, 3);
}
