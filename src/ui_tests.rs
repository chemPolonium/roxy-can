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
                let _ = editor.set_language(Some(dear_imgui_cte::Language::Lua));
                editor.set_show_line_numbers(true);
                editor.set_show_whitespaces(true);
                app.editors.entry(id).or_insert(editor);
            }
        }
        let ui = ctx.frame();
        crate::ui::render(app, &ui);
        let _ = ctx.render_legacy();
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
