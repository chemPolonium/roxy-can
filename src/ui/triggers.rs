use crate::app::App;
use crate::trigger::{TriggerAction, TriggerCond};
use crate::ui::help::popup_is_open;
use dear_imgui_rs::{Condition, Key, NumericFormat, StyleVar, TableColumnFlags, TableFlags, Ui};

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
            [io.display_size()[0] * 0.3, io.display_size()[1] * 0.3],
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
    ui.same_line();
    if ui.button("Re-arm latched") {
        app.send(crate::bus::BusCommand::RearmTriggers);
    }
    if ui.is_item_hovered() {
        ui.tooltip_text("重置出现类条件的锁存（ID 出现 / 错误帧）：下一个对应事件再次触发");
    }
    // Trigger-recording context: pre-trigger frames, post-roll frames,
    // and the marker cap. Each accepted edit is its own command, like the
    // bitrate inputs in the Buses window.
    ui.same_line();
    ui.align_text_to_frame_padding();
    ui.text_disabled("预触发");
    ui.same_line();
    ui.set_next_item_width(64.0);
    let mut pre = app.limits.pre_frames as i32;
    if ui.input_int_config("##prelim").step(64).build(&mut pre) {
        app.limits.pre_frames = pre.max(0) as usize;
        app.set_run_limits(app.limits.pre_frames, app.limits.post_frames, app.limits.marker_cap);
    }
    if ui.is_item_hovered() {
        ui.tooltip_text("触发启动的录制：文件以事件前这么多帧开头");
    }
    ui.same_line();
    ui.align_text_to_frame_padding();
    ui.text_disabled("post");
    ui.same_line();
    ui.set_next_item_width(64.0);
    let mut post = app.limits.post_frames as i32;
    if ui.input_int_config("##postlim").step(8).build(&mut post) {
        app.limits.post_frames = post.max(0) as u32;
        app.set_run_limits(app.limits.pre_frames, app.limits.post_frames, app.limits.marker_cap);
    }
    if ui.is_item_hovered() {
        ui.tooltip_text("触发停止的录制：边沿后再滚这么多帧才闭文件");
    }
    ui.same_line();
    ui.align_text_to_frame_padding();
    ui.text_disabled("标记上限");
    ui.same_line();
    ui.set_next_item_width(64.0);
    let mut cap = app.limits.marker_cap as i32;
    if ui.input_int_config("##marklim").step(16).build(&mut cap) {
        app.limits.marker_cap = cap.max(8) as usize;
        app.set_run_limits(app.limits.pre_frames, app.limits.post_frames, app.limits.marker_cap);
    }
    if ui.is_item_hovered() {
        ui.tooltip_text("Graphics 标记竖线最多保留这么多条");
    }
    ui.separator();

    let flags = TableFlags::BORDERS_INNER
        | TableFlags::ROW_BG
        | TableFlags::RESIZABLE
        | TableFlags::NO_BORDERS_IN_BODY;
    let table_opts = dear_imgui_rs::TableOptions::from(flags)
        .sizing_policy(dear_imgui_rs::TableSizingPolicy::StretchProp);
    let mut remove: Option<usize> = None;
    let n = app.snap.triggers.len();
    {
        let Some(_table) = ui.begin_table_with_flags("trig_table", 6, table_opts) else {
            return;
        };
        ui.table_setup_column_stretch_weight("Condition", TableColumnFlags::NONE, 1.0);
        for (label, w) in [("On", 26.0), ("Action", 90.0), ("Fired", 44.0), ("Edit", 42.0), ("", 18.0)] {
            ui.table_setup_column_fixed_width(label, TableColumnFlags::NONE, w);
        }
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
                TriggerAction::ClearTrace => "clear trace".to_string(),
                TriggerAction::InsertMarker => "marker".to_string(),
            };
            ui.align_text_to_frame_padding();
            ui.text(action_text);
            ui.table_next_column();
            ui.align_text_to_frame_padding();
            ui.text(format!("{}", app.snap.triggers[i].fired));
            ui.table_next_column();
            if ui.button(format!("edit##triged{i}")) {
                app.trig_draft = TrigDraft::for_index(app, i);
            }
            ui.table_next_column();
            if ui.button(format!("x##trigrm{i}")) {
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
    ui.modal_popup_with_opened(ID, &mut open, || {
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
            TableFlags::BORDERS_INNER_V,
        ) else {
            return;
        };
        ui.table_setup_column_fixed_width("", TableColumnFlags::NONE, 84.0);
        ui.table_setup_column_stretch_weight("", TableColumnFlags::NONE, 1.0);

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
                    let th_fmt = NumericFormat::new("%g").expect("static format");
                    if ui
                        .input_float_config("##trigth")
                        .display_format(th_fmt)
                        .build(&mut th)
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
            TriggerAction::ClearTrace => 3,
            TriggerAction::InsertMarker => 4,
        };
        row(ui, "Action", |ui| {
            let mut act = act;
            ui.set_next_item_width(-1.0);
            if ui.combo_simple_string(
                "##trigaction",
                &mut act,
                &[
                    "Start recording",
                    "Stop recording",
                    "Send generator entry",
                    "Clear trace",
                    "Insert marker",
                ],
            ) {
                draft.action = match act {
                    0 => TriggerAction::StartRecording,
                    1 => TriggerAction::StopRecording,
                    3 => TriggerAction::ClearTrace,
                    4 => TriggerAction::InsertMarker,
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
        app.trig_draft = None;
    } else if dismissed || !open {
        app.trig_draft = None;
    } else {
        // Write the edited draft back: the widgets type into this
        // frame's clone, and without this the next frame would restart
        // from the stale copy -- typed text lost, Apply a no-op.
        app.trig_draft = Some(draft);
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
