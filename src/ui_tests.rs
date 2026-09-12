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
use imgui::Context;
use std::sync::Mutex;

/// imgui's current context is process-global, so tests that each build
/// a context must not run concurrently.
static UI_LOCK: Mutex<()> = Mutex::new(());

const FONT: &[u8] = include_bytes!("../fonts/Inconsolata-Regular.ttf");

/// An imgui context wired like the app's (docking on, no ini file) but
/// with no platform and no renderer behind it.
fn harness() -> Context {
    let mut context = Context::create();
    context.set_ini_filename(None);
    context.io_mut().config_flags |= imgui::ConfigFlags::DOCKING_ENABLE;
    context.io_mut().display_size = [1280.0, 800.0];
    context.io_mut().delta_time = 1.0 / 60.0;
    context.fonts().add_font(&[imgui::FontSource::TtfData {
        data: FONT,
        size_pixels: 13.0,
        config: Some(imgui::FontConfig {
            oversample_h: 1,
            pixel_snap_h: true,
            size_pixels: 13.0,
            ..Default::default()
        }),
    }]);
    // The renderer normally triggers the atlas build in its NewFrame;
    // with no renderer, build it here or imgui asserts on frame().
    let _atlas = context.fonts().build_rgba32_texture();
    context
}

/// Runs `n` full frames of the real UI render path. `ctx.render()`
/// ends the frame (the renderer normally does this in the app).
fn frames(app: &mut App, ctx: &mut Context, n: usize) {
    for _ in 0..n {
        let ui = ctx.frame();
        crate::ui::render(app, &ui);
        ctx.render();
    }
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

/// The script editor is the most custom drawing in the app (gutter,
/// space dots, glyph repaint, cursor-tracked overlay): run it with
/// source that exercises every highlight class, over several frames so
/// the class cache and the widget callbacks are both on.
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
        "/* block\n",
        "   comment */\n",
    );
    app.send(crate::bus::BusCommand::SetNodeSource {
        id,
        source: source.to_string(),
    });
    // The editor shows the draft, not the applied source: seed both.
    app.node_src_draft.insert(id, source.to_string());
    app.open_script_editor(id);
    app.settle();
    frames(&mut app, &mut ctx, 5);
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
