use crate::app::{App, PopupTarget};
use crate::ui::idfilter::{scope_combo, target_name};
use crate::ui::siglist::ListKind;
use imgui::{Condition, TableColumnFlags, TableColumnSetup, TableFlags, Ui};

/// "Go to" button: opens the window if hidden and always brings it to the
/// front. There is no close action — windows close via their own title
/// bar. `###` keeps the widget ID stable.
fn goto_button(ui: &Ui, id: &str) -> bool {
    ui.button(format!("->###goto_{id}"))
}

/// Single-table overview of every observer (Trace, Messages, Statistics,
/// Graphics, Data): open state, Signals scope, and per-observer export.
/// A flat list rather than a node graph: there is no measurement topology to
/// wire here, only a table of what exists.
pub fn render(app: &mut App, ui: &Ui) {
    let io = ui.io();
    let mut open = app.show_measurement;
    if open {
        ui.window("Measurement Setup")
            .opened(&mut open)
            .position(
                [io.display_size[0] * 0.04, io.display_size[1] * 0.30],
                Condition::FirstUseEver,
            )
            .size([620.0, 320.0], Condition::FirstUseEver)
            .build(|| content(app, ui));
    }
    app.show_measurement = open;
}

fn content(app: &mut App, ui: &Ui) {
    if ui.small_button("+ Trace") {
        app.new_trace_window();
    }
    ui.same_line();
    if ui.small_button("+ Messages") {
        app.new_msg_window();
    }
    ui.same_line();
    if ui.small_button("+ Statistics") {
        app.new_stats_window();
    }
    ui.same_line();
    if ui.small_button("+ Graphics") {
        app.new_graphics_window();
    }
    ui.same_line();
    if ui.small_button("+ Data") {
        app.new_data_window();
    }
    ui.same_line();
    if ui.small_button("+ State Tracker") {
        app.new_state_window();
    }
    ui.separator();

    // NO_BORDERS_IN_BODY restricts column-resize dragging to the header row.
    let flags = TableFlags::BORDERS_INNER
        | TableFlags::ROW_BG
        | TableFlags::RESIZABLE
        | TableFlags::NO_BORDERS_IN_BODY
        | TableFlags::SCROLL_Y
        | TableFlags::SIZING_STRETCH_PROP;
    let mut rm_trace: Option<usize> = None;
    let mut rm_msgs: Option<usize> = None;
    let mut rm_stats: Option<usize> = None;
    let mut rm_graphics: Option<usize> = None;
    let mut rm_data: Option<usize> = None;
    let mut rm_state: Option<usize> = None;
    {
        let Some(_table) = ui.begin_table_with_flags("meas_table", 6, flags) else {
            return;
        };
        // default-size "->" button
        ui.table_setup_column_with(TableColumnSetup {
            flags: TableColumnFlags::WIDTH_FIXED,
            init_width_or_weight: 32.0,
            ..TableColumnSetup::new("Open")
        });
        // longest type label is "State Tracker"
        ui.table_setup_column_with(TableColumnSetup {
            flags: TableColumnFlags::WIDTH_FIXED,
            init_width_or_weight: 96.0,
            ..TableColumnSetup::new("Type")
        });
        ui.table_setup_column_with(TableColumnSetup {
            flags: TableColumnFlags::WIDTH_STRETCH,
            init_width_or_weight: 1.0,
            ..TableColumnSetup::new("Name")
        });
        ui.table_setup_column_with(TableColumnSetup {
            flags: TableColumnFlags::WIDTH_FIXED,
            init_width_or_weight: 160.0,
            ..TableColumnSetup::new("Filter")
        });
        ui.table_setup_column_with(TableColumnSetup {
            flags: TableColumnFlags::WIDTH_FIXED,
            init_width_or_weight: 44.0,
            ..TableColumnSetup::new("Save")
        });
        ui.table_setup_column_with(TableColumnSetup {
            flags: TableColumnFlags::WIDTH_FIXED,
            init_width_or_weight: 26.0,
            ..TableColumnSetup::new("")
        });
        ui.table_headers_row();

        let n = app.trace_windows.len();
        for i in 0..n {
            trace_row(app, ui, i, &mut rm_trace);
        }
        let n = app.msg_windows.len();
        for i in 0..n {
            messages_row(app, ui, i, &mut rm_msgs);
        }
        let n = app.stats_windows.len();
        for i in 0..n {
            stats_row(app, ui, i, &mut rm_stats);
        }
        for i in 0..app.graphics.len() {
            graphics_row(app, ui, i, &mut rm_graphics);
        }
        for i in 0..app.data_windows.len() {
            data_row(app, ui, i, &mut rm_data);
        }
        for i in 0..app.state_trackers.len() {
            state_row(app, ui, i, &mut rm_state);
        }
    }

    if let Some(i) = rm_trace {
        app.trace_windows.remove(i);
        app.popup_target = app.popup_target.and_then(|t| popup_after_remove(t, i, 0));
    }
    if let Some(i) = rm_msgs {
        app.msg_windows.remove(i);
        app.popup_target = app.popup_target.and_then(|t| popup_after_remove(t, i, 1));
    }
    if let Some(i) = rm_stats {
        app.stats_windows.remove(i);
        app.popup_target = app.popup_target.and_then(|t| popup_after_remove(t, i, 2));
    }
    if let Some(i) = rm_graphics {
        let keys: Vec<_> = app.graphics[i]
            .signals
            .iter()
            .map(|s| s.key.clone())
            .collect();
        app.graphics.remove(i);
        for k in keys {
            app.prune_signal(&k);
        }
        app.popup_target = app.popup_target.and_then(|t| popup_after_remove(t, i, 3));
    }
    if let Some(i) = rm_data {
        let keys: Vec<_> = app.data_windows[i]
            .signals
            .iter()
            .map(|s| s.key.clone())
            .collect();
        app.data_windows.remove(i);
        for k in keys {
            app.prune_signal(&k);
        }
        app.popup_target = app.popup_target.and_then(|t| popup_after_remove(t, i, 4));
    }
    if let Some(i) = rm_state {
        let keys: Vec<_> = app.state_trackers[i]
            .signals
            .iter()
            .map(|s| s.key.clone())
            .collect();
        app.state_trackers.remove(i);
        for k in keys {
            app.prune_signal(&k);
        }
        app.popup_target = app.popup_target.and_then(|t| popup_after_remove(t, i, 5));
    }
}

/// Keeps the Filter Selection popup pointing at the right window after an
/// observer row is deleted. `which`: 0 = Trace, 1 = Messages, 2 = Statistics,
/// 3 = Graphics, 4 = Data, 5 = State Tracker.
fn popup_after_remove(t: PopupTarget, removed: usize, which: u8) -> Option<PopupTarget> {
    match t {
        PopupTarget::Trace(i) if which == 0 => match i.cmp(&removed) {
            std::cmp::Ordering::Equal => None,
            std::cmp::Ordering::Greater => Some(PopupTarget::Trace(i - 1)),
            std::cmp::Ordering::Less => Some(t),
        },
        PopupTarget::Messages(i) if which == 1 => match i.cmp(&removed) {
            std::cmp::Ordering::Equal => None,
            std::cmp::Ordering::Greater => Some(PopupTarget::Messages(i - 1)),
            std::cmp::Ordering::Less => Some(t),
        },
        PopupTarget::Stats(i) if which == 2 => match i.cmp(&removed) {
            std::cmp::Ordering::Equal => None,
            std::cmp::Ordering::Greater => Some(PopupTarget::Stats(i - 1)),
            std::cmp::Ordering::Less => Some(t),
        },
        PopupTarget::Graphics(i) if which == 3 => match i.cmp(&removed) {
            std::cmp::Ordering::Equal => None,
            std::cmp::Ordering::Greater => Some(PopupTarget::Graphics(i - 1)),
            std::cmp::Ordering::Less => Some(t),
        },
        PopupTarget::Data(i) if which == 4 => match i.cmp(&removed) {
            std::cmp::Ordering::Equal => None,
            std::cmp::Ordering::Greater => Some(PopupTarget::Data(i - 1)),
            std::cmp::Ordering::Less => Some(t),
        },
        PopupTarget::State(i) if which == 5 => match i.cmp(&removed) {
            std::cmp::Ordering::Equal => None,
            std::cmp::Ordering::Greater => Some(PopupTarget::State(i - 1)),
            std::cmp::Ordering::Less => Some(t),
        },
        _ => Some(t),
    }
}

fn trace_row(app: &mut App, ui: &Ui, i: usize, rm: &mut Option<usize>) {
    ui.table_next_row();
    if !ui.table_next_column() {
        return;
    }
    if goto_button(ui, &format!("t{i}")) {
        app.trace_windows[i].opened = true;
        app.focus_title = Some(target_name(app, PopupTarget::Trace(i)));
    }
    ui.table_next_column();
    ui.text("Trace");
    ui.table_next_column();
    ui.set_next_item_width(-1.0);
    ui.input_text(format!("##name_t{i}"), &mut app.trace_windows[i].name)
        .build();
    ui.table_next_column();
    let s = scope_combo(
        app,
        ui,
        &format!("##mscope_t{i}"),
        app.trace_windows[i].scope,
        PopupTarget::Trace(i),
    );
    app.trace_windows[i].scope = s;
    ui.table_next_column();
    if ui.small_button(format!("ASC##save_t{i}")) {
        app.export_trace_dialog(i);
    }
    ui.table_next_column();
    if ui.small_button(format!("x##t{i}")) {
        *rm = Some(i);
    }
}

fn messages_row(app: &mut App, ui: &Ui, i: usize, rm: &mut Option<usize>) {
    ui.table_next_row();
    if !ui.table_next_column() {
        return;
    }
    if goto_button(ui, &format!("m{i}")) {
        app.msg_windows[i].opened = true;
        app.focus_title = Some(target_name(app, PopupTarget::Messages(i)));
    }
    ui.table_next_column();
    ui.text("Messages");
    ui.table_next_column();
    ui.set_next_item_width(-1.0);
    ui.input_text(format!("##name_m{i}"), &mut app.msg_windows[i].name)
        .build();
    ui.table_next_column();
    let s = scope_combo(
        app,
        ui,
        &format!("##mscope_m{i}"),
        app.msg_windows[i].scope,
        PopupTarget::Messages(i),
    );
    app.msg_windows[i].scope = s;
    ui.table_next_column();
    if ui.small_button(format!("CSV##save_m{i}")) {
        app.export_messages_dialog(i);
    }
    ui.table_next_column();
    if ui.small_button(format!("x##m{i}")) {
        *rm = Some(i);
    }
}

fn stats_row(app: &mut App, ui: &Ui, i: usize, rm: &mut Option<usize>) {
    ui.table_next_row();
    if !ui.table_next_column() {
        return;
    }
    if goto_button(ui, &format!("s{i}")) {
        app.stats_windows[i].opened = true;
        app.focus_title = Some(target_name(app, PopupTarget::Stats(i)));
    }
    ui.table_next_column();
    ui.text("Statistics");
    ui.table_next_column();
    ui.set_next_item_width(-1.0);
    ui.input_text(format!("##name_s{i}"), &mut app.stats_windows[i].name)
        .build();
    ui.table_next_column();
    let s = scope_combo(
        app,
        ui,
        &format!("##mscope_s{i}"),
        app.stats_windows[i].scope,
        PopupTarget::Stats(i),
    );
    app.stats_windows[i].scope = s;
    ui.table_next_column();
    if ui.small_button(format!("CSV##save_s{i}")) {
        app.export_stats_dialog(i);
    }
    ui.table_next_column();
    if ui.small_button(format!("x##s{i}")) {
        *rm = Some(i);
    }
}

/// Filter cell of a Graphics/Data row: a "…" button opening the Signal
/// Selection popup and the visible/total signal count.
fn signal_cell(app: &mut App, ui: &Ui, kind: ListKind) {
    let (i, prefix, vis, total) = match kind {
        ListKind::Graphics(i) => {
            let w = &app.graphics[i];
            (
                i,
                "g",
                w.signals.iter().filter(|s| s.visible).count(),
                w.signals.len(),
            )
        }
        ListKind::Data(i) => {
            let w = &app.data_windows[i];
            (
                i,
                "d",
                w.signals.iter().filter(|s| s.visible).count(),
                w.signals.len(),
            )
        }
        ListKind::State(i) => {
            let w = &app.state_trackers[i];
            (
                i,
                "st",
                w.signals.iter().filter(|s| s.visible).count(),
                w.signals.len(),
            )
        }
    };
    if ui.small_button(format!("…##selsig{prefix}{i}")) {
        app.popup_target = Some(match kind {
            ListKind::Graphics(i) => PopupTarget::Graphics(i),
            ListKind::Data(i) => PopupTarget::Data(i),
            ListKind::State(i) => PopupTarget::State(i),
        });
        app.show_id_filter = true;
    }
    ui.same_line();
    ui.text(format!("{vis}/{total}"));
}

fn graphics_row(app: &mut App, ui: &Ui, i: usize, rm: &mut Option<usize>) {
    ui.table_next_row();
    if !ui.table_next_column() {
        return;
    }
    if goto_button(ui, &format!("g{i}")) {
        app.graphics[i].opened = true;
        app.focus_title = Some(target_name(app, PopupTarget::Graphics(i)));
    }
    ui.table_next_column();
    ui.text("Graphics");
    ui.table_next_column();
    ui.set_next_item_width(-1.0);
    ui.input_text(format!("##name_g{i}"), &mut app.graphics[i].name)
        .build();
    ui.table_next_column();
    signal_cell(app, ui, ListKind::Graphics(i));
    ui.table_next_column();
    if ui.small_button(format!("CSV##save_g{i}")) {
        app.export_graphics_dialog(i);
    }
    ui.table_next_column();
    if ui.small_button(format!("x##g{i}")) {
        *rm = Some(i);
    }
}

fn data_row(app: &mut App, ui: &Ui, i: usize, rm: &mut Option<usize>) {
    ui.table_next_row();
    if !ui.table_next_column() {
        return;
    }
    if goto_button(ui, &format!("d{i}")) {
        app.data_windows[i].opened = true;
        app.focus_title = Some(target_name(app, PopupTarget::Data(i)));
    }
    ui.table_next_column();
    ui.text("Data");
    ui.table_next_column();
    ui.set_next_item_width(-1.0);
    ui.input_text(format!("##name_d{i}"), &mut app.data_windows[i].name)
        .build();
    ui.table_next_column();
    signal_cell(app, ui, ListKind::Data(i));
    ui.table_next_column();
    if ui.small_button(format!("CSV##save_d{i}")) {
        app.export_data_dialog(i);
    }
    ui.table_next_column();
    if ui.small_button(format!("x##d{i}")) {
        *rm = Some(i);
    }
}

/// The State Tracker row mirrors Data: signal-level selection, no CSV
/// export yet (the band view has nothing tabular to write).
fn state_row(app: &mut App, ui: &Ui, i: usize, rm: &mut Option<usize>) {
    ui.table_next_row();
    if !ui.table_next_column() {
        return;
    }
    if goto_button(ui, &format!("st{i}")) {
        app.state_trackers[i].opened = true;
        app.focus_title = Some(target_name(app, PopupTarget::State(i)));
    }
    ui.table_next_column();
    ui.text("State Tracker");
    ui.table_next_column();
    ui.set_next_item_width(-1.0);
    ui.input_text(format!("##name_st{i}"), &mut app.state_trackers[i].name)
        .build();
    ui.table_next_column();
    signal_cell(app, ui, ListKind::State(i));
    ui.table_next_column();
    ui.table_next_column();
    if ui.small_button(format!("x##st{i}")) {
        *rm = Some(i);
    }
}
