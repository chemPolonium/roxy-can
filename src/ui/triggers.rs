use crate::app::App;
use crate::trigger::{TriggerAction, TriggerCond};
use crate::ui::help::popup_is_open;
use imgui::{Condition, Key, StyleVar, TableColumnFlags, TableColumnSetup, TableFlags, Ui};

/// Draft state of the trigger editor popup: which row it edits plus the
/// not-yet-applied shape of the condition and action. Nothing reaches the
/// bus until Apply, so a half-edited rule never runs, and the row being
/// edited no longer has to stay selected underneath the growing list.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct TrigDraft {
    pub index: usize,
    pub cond: TriggerCond,
    pub action: TriggerAction,
    pub id_buf: String,
}

impl TrigDraft {
    pub(crate) fn new(index: usize, cond: TriggerCond, action: TriggerAction) -> Self {
        let id_buf = match &cond {
            TriggerCond::SignalCross { id, .. }
            | TriggerCond::IdPresent { id, .. }
            | TriggerCond::CycleTimeout { id, .. } => format!("{id:X}"),
            TriggerCond::ErrorFrame { .. } => String::new(),
        };
        TrigDraft {
            index,
            cond,
            action,
            id_buf,
        }
    }

    fn for_index(app: &App, i: usize) -> Option<Self> {
        let t = app.snap.triggers.get(i)?;
        Some(Self::new(i, t.cond.clone(), t.action))
    }
}

/// Trigger management: the armed condition list plus a popup editor. The
/// evaluation itself lives in `trigger.rs`; this window only shapes
/// `App.triggers`.
pub fn render(app: &mut App, ui: &Ui) {
    if !app.show_triggers {
        return;
    }
    let io = ui.io();
    let mut open = app.show_triggers;
    // The table cannot compress below its own columns; a floor keeps the
    // drag handle from folding everything into unreadability.
    let min = ui.push_style_var(StyleVar::WindowMinSize([460.0, 200.0]));
    ui.window("Triggers")
        .opened(&mut open)
        .position(
            [io.display_size[0] * 0.3, io.display_size[1] * 0.3],
            Condition::FirstUseEver,
        )
        .size([560.0, 320.0], Condition::FirstUseEver)
        .build(|| content(app, ui));
    min.pop();
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
    ui.same_line();
    if ui.button("+ Timeout") {
        app.add_timeout_trigger();
    }
    ui.separator();

    let flags = TableFlags::BORDERS_INNER
        | TableFlags::ROW_BG
        | TableFlags::RESIZABLE
        | TableFlags::NO_BORDERS_IN_BODY
        | TableFlags::SIZING_STRETCH_PROP;
    let mut remove: Option<usize> = None;
    let n = app.snap.triggers.len();
    {
        let Some(_table) = ui.begin_table_with_flags("trig_table", 6, flags) else {
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
            init_width_or_weight: 42.0,
            ..TableColumnSetup::new("Edit")
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
            // The checkbox in the next column gives the row its height;
            // without the baseline alignment the plain text would float at
            // the top of the taller row.
            ui.align_text_to_frame_padding();
            ui.text(summary);
            ui.table_next_column();
            let mut on = app.snap.triggers[i].enabled;
            if ui.checkbox(format!("##trigon{i}"), &mut on) {
                app.send(crate::bus::BusCommand::SetTriggerEnabled { index: i, on });
            }
            ui.table_next_column();
            let action_text = match app.snap.triggers[i].action {
                TriggerAction::StartRecording => "start rec".to_string(),
                TriggerAction::StopRecording => "stop rec".to_string(),
                TriggerAction::Send { id, .. } => format!("send 0x{id:X}"),
            };
            ui.align_text_to_frame_padding();
            ui.text(action_text);
            ui.table_next_column();
            ui.align_text_to_frame_padding();
            ui.text(format!("{}", app.snap.triggers[i].fired));
            ui.table_next_column();
            if ui.small_button(format!("edit##triged{i}")) {
                app.trig_draft = TrigDraft::for_index(app, i);
            }
            ui.table_next_column();
            if ui.small_button(format!("x##trigrm{i}")) {
                remove = Some(i);
            }
        }
    }
    if let Some(i) = remove {
        app.remove_trigger(i);
    }

    editor_modal(app, ui);
}

/// The editor popup for one trigger. Drafted like the send-cycle modal:
/// the widgets shape a local copy, Apply crosses it to the bus in one
/// command, Cancel or Escape throws it away. Nothing here writes while it
/// edits, and the list position of the row is irrelevant.
fn editor_modal(app: &mut App, ui: &Ui) {
    const ID: &str = "Edit trigger##trigmodal";
    let Some(mut draft) = app.trig_draft.clone() else {
        return;
    };
    if !popup_is_open(ui, ID) {
        ui.open_popup(ID);
    }
    let mut open = true;
    let mut dismissed = false;
    let mut confirmed = false;
    let min = ui.push_style_var(StyleVar::WindowMinSize([380.0, 0.0]));
    ui.modal_popup_config(ID).opened(&mut open).build(|| {
        let kind = match draft.cond {
            TriggerCond::SignalCross { .. } => "signal cross",
            TriggerCond::IdPresent { .. } => "id present",
            TriggerCond::CycleTimeout { .. } => "cycle timeout",
            TriggerCond::ErrorFrame { .. } => "error frames",
        };
        ui.text(format!(
            "trigger {} of {} -- {kind}",
            draft.index + 1,
            app.snap.triggers.len()
        ));
        ui.separator();
        if ui.is_window_appearing() {
            ui.set_keyboard_focus_here();
        }
        let Some(_grid) = ui.begin_table_with_flags(
            "##trigedit",
            2,
            TableFlags::BORDERS_INNER_V | TableFlags::SIZING_STRETCH_PROP,
        ) else {
            return;
        };
        ui.table_setup_column_with(TableColumnSetup {
            flags: TableColumnFlags::WIDTH_FIXED,
            init_width_or_weight: 84.0,
            ..TableColumnSetup::new("")
        });
        ui.table_setup_column_with(TableColumnSetup {
            flags: TableColumnFlags::WIDTH_STRETCH,
            init_width_or_weight: 1.0,
            ..TableColumnSetup::new("")
        });

        row(ui, "Bus", |ui| {
            let names: Vec<String> = app.snap.channels.iter().map(|c| c.name.clone()).collect();
            let refs: Vec<&str> = names.iter().map(|s| s.as_str()).collect();
            let mut bus = (draft.cond.bus() as usize).min(refs.len().saturating_sub(1));
            ui.set_next_item_width(-1.0);
            if ui.combo_simple_string("##trigbus", &mut bus, &refs) {
                set_bus(&mut draft.cond, bus as u8);
            }
        });
        let cond_bus = draft.cond.bus();
        match &mut draft.cond {
            TriggerCond::SignalCross {
                id,
                signal,
                threshold,
                rising,
                ..
            } => {
                row(ui, "Message", |ui| {
                    id_field(ui, &mut draft.id_buf, id);
                });
                row(ui, "Signal", |ui| {
                    let names = app.signal_names(cond_bus, *id);
                    ui.set_next_item_width(-1.0);
                    if names.is_empty() {
                        // No database for the id: keep the name editable by
                        // hand, evaluation just reports nothing until it
                        // names something decodable.
                        let mut s = signal.clone();
                        if ui.input_text("##trigsignal", &mut s).build() {
                            *signal = s;
                        }
                    } else {
                        let refs: Vec<&str> = names.iter().map(|s| s.as_str()).collect();
                        let mut pick = refs.iter().position(|n| n == &signal.as_str()).unwrap_or(0);
                        if ui.combo_simple_string("##trigsignal", &mut pick, &refs) {
                            *signal = names[pick].clone();
                        }
                    }
                });
                row(ui, "Threshold", |ui| {
                    let mut th = *threshold as f32;
                    if ui
                        .input_float("##trigth", &mut th)
                        .display_format("%g")
                        .build()
                    {
                        *threshold = th as f64;
                    }
                });
                row(ui, "Direction", |ui| {
                    let mut dir = *rising as usize;
                    if ui.combo_simple_string("##trigdir", &mut dir, &["rising", "falling"]) {
                        *rising = dir == 0;
                    }
                });
            }
            TriggerCond::IdPresent { id, .. } => {
                row(ui, "Message", |ui| {
                    id_field(ui, &mut draft.id_buf, id);
                });
            }
            TriggerCond::CycleTimeout { id, .. } => {
                row(ui, "Message", |ui| {
                    id_field(ui, &mut draft.id_buf, id);
                });
                row(ui, "Note", |ui| {
                    ui.text("fires once each time it goes silent");
                });
            }
            TriggerCond::ErrorFrame { .. } => {
                row(ui, "Watches", |ui| {
                    ui.text("any error frame on the bus");
                });
            }
        }

        let act = match draft.action {
            TriggerAction::StartRecording => 0,
            TriggerAction::StopRecording => 1,
            TriggerAction::Send { .. } => 2,
        };
        row(ui, "Action", |ui| {
            let mut act = act;
            ui.set_next_item_width(-1.0);
            if ui.combo_simple_string(
                "##trigaction",
                &mut act,
                &["Start recording", "Stop recording", "Send generator entry"],
            ) {
                draft.action = match act {
                    0 => TriggerAction::StartRecording,
                    1 => TriggerAction::StopRecording,
                    // Coming back to Send keeps whatever target was last
                    // set; a fresh Send starts from the first entry.
                    _ => match draft.action {
                        TriggerAction::Send { ch, id } => TriggerAction::Send { ch, id },
                        _ => match app.snap.tx.first() {
                            Some(t) => TriggerAction::Send {
                                ch: t.channel,
                                id: t.id,
                            },
                            None => TriggerAction::Send { ch: 0, id: 0x100 },
                        },
                    },
                };
            }
        });
        // The entry picker only means something while the action is Send.
        if matches!(draft.action, TriggerAction::Send { .. }) && !app.snap.tx.is_empty() {
            row(ui, "Entry", |ui| {
                let names: Vec<String> = app
                    .snap
                    .tx
                    .iter()
                    .map(|t| format!("{} 0x{:03X} {}", app.channel_name(t.channel), t.id, t.name))
                    .collect();
                let refs: Vec<&str> = names.iter().map(|s| s.as_str()).collect();
                let mut pick = app
                    .snap
                    .tx
                    .iter()
                    .position(|t| {
                        matches!(draft.action, TriggerAction::Send { ch, id }
                            if t.channel == ch && t.id == id)
                    })
                    .unwrap_or(0);
                ui.set_next_item_width(-1.0);
                if ui.combo_simple_string("##trigsend", &mut pick, &refs) {
                    let t = &app.snap.tx[pick];
                    draft.action = TriggerAction::Send {
                        ch: t.channel,
                        id: t.id,
                    };
                }
            });
        }
        ui.separator();
        if ui.is_key_pressed(Key::Escape) {
            dismissed = true;
        }
        if ui.button_with_size("Apply", [90.0, 0.0]) {
            confirmed = true;
            ui.close_current_popup();
        }
        ui.same_line();
        if ui.button_with_size("Cancel", [90.0, 0.0]) {
            dismissed = true;
        }
    });
    min.pop();
    if confirmed {
        app.send(crate::bus::BusCommand::EditTrigger {
            index: draft.index,
            cond: draft.cond,
            action: draft.action,
        });
    }
    if confirmed || dismissed || !open {
        app.trig_draft = None;
    }
}

/// One label/widget line of the editor grid.
fn row(ui: &Ui, label: &str, body: impl FnOnce(&Ui)) {
    ui.table_next_row();
    ui.table_next_column();
    ui.text(label);
    ui.table_next_column();
    body(ui);
}

fn set_bus(cond: &mut TriggerCond, ch: u8) {
    match cond {
        TriggerCond::SignalCross { ch: c, .. }
        | TriggerCond::IdPresent { ch: c, .. }
        | TriggerCond::CycleTimeout { ch: c, .. }
        | TriggerCond::ErrorFrame { ch: c } => *c = ch,
    }
}

fn id_field(ui: &Ui, buf: &mut String, id: &mut u32) -> bool {
    ui.set_next_item_width(-1.0);
    let mut changed = false;
    if ui.input_text("##trigid", buf).build()
        && let Some(v) = parse_hex(buf)
    {
        *id = v;
        changed = true;
    }
    changed
}
