use crate::app::App;
use crate::trigger::TriggerCond;
use imgui::{Condition, TableColumnFlags, TableColumnSetup, TableFlags, Ui};

/// Trigger management: the armed condition list plus the editor for the
/// selected trigger. The evaluation itself lives in `trigger.rs`; this
/// window only shapes `App.triggers`.
pub fn render(app: &mut App, ui: &Ui) {
    if !app.show_triggers {
        return;
    }
    let io = ui.io();
    let mut open = app.show_triggers;
    ui.window("Triggers")
        .opened(&mut open)
        .position(
            [io.display_size[0] * 0.3, io.display_size[1] * 0.3],
            Condition::FirstUseEver,
        )
        .size([560.0, 320.0], Condition::FirstUseEver)
        .build(|| content(app, ui));
    app.show_triggers = open;
}

fn parse_hex(s: &str) -> Option<u32> {
    let t = s.trim().trim_start_matches("0x").trim_start_matches("0X");
    u32::from_str_radix(t, 16).ok()
}

fn content(app: &mut App, ui: &Ui) {
    if ui.button("+ Signal") {
        app.add_signal_trigger();
    }
    ui.same_line();
    if ui.button("+ ID") {
        app.add_id_trigger();
    }
    ui.same_line();
    if ui.button("+ Error frames") {
        app.add_error_trigger();
    }
    ui.separator();

    let flags = TableFlags::BORDERS_INNER
        | TableFlags::ROW_BG
        | TableFlags::RESIZABLE
        | TableFlags::NO_BORDERS_IN_BODY
        | TableFlags::SIZING_STRETCH_PROP;
    let mut remove: Option<usize> = None;
    let n = app.triggers.len();
    {
        let Some(_table) = ui.begin_table_with_flags("trig_table", 5, flags) else {
            return;
        };
        ui.table_setup_column_with(TableColumnSetup {
            flags: TableColumnFlags::WIDTH_STRETCH,
            init_width_or_weight: 1.0,
            ..TableColumnSetup::new("Condition")
        });
        ui.table_setup_column_with(TableColumnSetup {
            flags: TableColumnFlags::WIDTH_FIXED,
            init_width_or_weight: 26.0,
            ..TableColumnSetup::new("On")
        });
        ui.table_setup_column_with(TableColumnSetup {
            flags: TableColumnFlags::WIDTH_FIXED,
            init_width_or_weight: 90.0,
            ..TableColumnSetup::new("Action")
        });
        ui.table_setup_column_with(TableColumnSetup {
            flags: TableColumnFlags::WIDTH_FIXED,
            init_width_or_weight: 44.0,
            ..TableColumnSetup::new("Fired")
        });
        ui.table_setup_column_with(TableColumnSetup {
            flags: TableColumnFlags::WIDTH_FIXED,
            init_width_or_weight: 18.0,
            ..TableColumnSetup::new("")
        });
        ui.table_headers_row();

        for i in 0..n {
            ui.table_next_row();
            if !ui.table_next_column() {
                continue;
            }
            let summary = app.trigger_summary(i);
            if ui
                .selectable_config(format!("{summary}##trigsel{i}"))
                .build()
            {
                app.trigger_sel = Some(i);
            }
            ui.table_next_column();
            let mut on = app.triggers[i].enabled;
            if ui.checkbox(format!("##trigon{i}"), &mut on) {
                app.triggers[i].enabled = on;
            }
            ui.table_next_column();
            ui.text(match app.triggers[i].action {
                crate::trigger::TriggerAction::StartRecording => "start rec",
                crate::trigger::TriggerAction::StopRecording => "stop rec",
            });
            ui.table_next_column();
            ui.text(format!("{}", app.triggers[i].fired));
            ui.table_next_column();
            if ui.small_button(format!("x##trigrm{i}")) {
                remove = Some(i);
            }
        }
    }
    if let Some(i) = remove {
        app.remove_trigger(i);
    }

    editor(app, ui);
}

/// The editor for the selected trigger. The condition is cloned out,
/// edited against local widgets (the DBC picker needs `&self` while the
/// row is being shaped) and written back once.
fn editor(app: &mut App, ui: &Ui) {
    let Some(sel) = app.trigger_sel.filter(|s| *s < app.triggers.len()) else {
        return;
    };
    ui.separator();
    let mut cond = app.triggers[sel].cond.clone();
    let mut action = app.triggers[sel].action;
    let mut changed = false;
    let cond_bus = cond.bus();

    let bus_names: Vec<String> = app.channels.iter().map(|c| c.name.clone()).collect();
    let bus_refs: Vec<&str> = bus_names.iter().map(|s| s.as_str()).collect();
    let mut bus = (cond.bus() as usize).min(bus_refs.len() - 1);
    ui.set_next_item_width(90.0);
    if ui.combo_simple_string("##trigbus", &mut bus, &bus_refs) {
        set_bus(&mut cond, bus as u8);
        changed = true;
    }

    match &mut cond {
        TriggerCond::SignalCross {
            id,
            signal,
            threshold,
            rising,
            ..
        } => {
            sync_id_buf(app, sel, *id);
            if id_field(app, ui, id) {
                changed = true;
                // The old signal may not exist on the new message; keep
                // it (evaluation just reports nothing) until picked.
                let _ = signal;
            }
            ui.same_line();
            let names = app.signal_names(cond_bus, *id);
            if names.is_empty() {
                ui.set_next_item_width(150.0);
                let mut s = signal.clone();
                if ui.input_text("##trigsignal", &mut s).build() {
                    *signal = s;
                    changed = true;
                }
            } else {
                let refs: Vec<&str> = names.iter().map(|s| s.as_str()).collect();
                let mut pick = refs.iter().position(|n| n == &signal.as_str()).unwrap_or(0);
                ui.set_next_item_width(150.0);
                if ui.combo_simple_string("##trigsignal", &mut pick, &refs) {
                    *signal = names[pick].clone();
                    changed = true;
                }
            }
            ui.same_line();
            ui.set_next_item_width(110.0);
            let mut th = *threshold as f32;
            if ui.input_float("##trigth", &mut th).build() {
                *threshold = th as f64;
                changed = true;
            }
            ui.same_line();
            let mut dir = *rising as usize;
            if ui.combo_simple_string("##trigdir", &mut dir, &["rising", "falling"]) {
                *rising = dir == 0;
                changed = true;
            }
        }
        TriggerCond::IdPresent { id, .. } => {
            sync_id_buf(app, sel, *id);
            if id_field(app, ui, id) {
                changed = true;
            }
        }
        TriggerCond::ErrorFrame { .. } => {
            ui.text("any error frame on the bus");
        }
    }

    let mut act = match action {
        crate::trigger::TriggerAction::StartRecording => 0,
        crate::trigger::TriggerAction::StopRecording => 1,
    };
    ui.set_next_item_width(150.0);
    if ui.combo_simple_string(
        "##trigaction",
        &mut act,
        &["Start recording", "Stop recording"],
    ) {
        action = match act {
            0 => crate::trigger::TriggerAction::StartRecording,
            _ => crate::trigger::TriggerAction::StopRecording,
        };
        changed = true;
    }

    if changed {
        app.triggers[sel].cond = cond;
        app.triggers[sel].action = action;
        // The level belongs to the old condition; a reshaped trigger
        // starts from a clean edge.
        app.triggers[sel].level = false;
    }
}

fn set_bus(cond: &mut TriggerCond, ch: u8) {
    match cond {
        TriggerCond::SignalCross { ch: c, .. }
        | TriggerCond::IdPresent { ch: c, .. }
        | TriggerCond::ErrorFrame { ch: c } => *c = ch,
    }
}

/// Keeps the hex edit buffer on the selected trigger's id; switching
/// selection resets it from the model instead of fighting the typist.
fn sync_id_buf(app: &mut App, sel: usize, id: u32) {
    if app.trig_edit_sel != Some(sel) {
        app.trig_id_buf = format!("{id:X}");
        app.trig_edit_sel = Some(sel);
    }
}

fn id_field(app: &mut App, ui: &Ui, id: &mut u32) -> bool {
    ui.set_next_item_width(70.0);
    let mut changed = false;
    if ui.input_text("##trigid", &mut app.trig_id_buf).build()
        && let Some(v) = parse_hex(&app.trig_id_buf)
    {
        *id = v;
        changed = true;
    }
    changed
}
