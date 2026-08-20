use crate::app::{App, PopupTarget};
use crate::ui::idfilter::scope_combo;
use crate::ui::siglist::ListKind;
use imgui::{Condition, StyleColor, TableColumnFlags, TableColumnSetup, TableFlags, Ui};

fn tip(ui: &Ui, text: &str) {
    if ui.is_item_hovered() {
        ui.tooltip_text(text);
    }
}

/// Square open/close toggle: down-arrow when the window is visible,
/// right-arrow when hidden. `###` keeps the widget ID stable across the
/// glyph change so ImGui does not lose button state.
fn open_button(ui: &Ui, id: &str, opened: &mut bool) {
    let label = if *opened { "\u{25BC}" } else { "\u{25B6}" };
    let _color = ui.push_style_color(
        StyleColor::Button,
        if *opened {
            [0.20, 0.42, 0.62, 1.0]
        } else {
            [0.16, 0.16, 0.20, 1.0]
        },
    );
    if ui.button_with_size(format!("{label}###open_{id}"), [22.0, 22.0]) {
        *opened = !*opened;
    }
    if ui.is_item_hovered() {
        ui.tooltip_text(if *opened {
            "Hide this window"
        } else {
            "Show this window"
        });
    }
}

/// Single-table overview of every observer (Trace, Messages, Statistics,
/// Graphics, Data): open state, Signals scope, and per-observer export.
/// Replaces CANoe's graph-based Measurement Setup with a flat list.
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
            .flags(imgui::WindowFlags::NO_SAVED_SETTINGS)
            .build(|| content(app, ui));
    }
    app.show_measurement = open;
}

fn content(app: &mut App, ui: &Ui) {
    if ui.small_button("+ Trace") {
        app.new_trace_window();
    }
    tip(ui, "Add a Trace observer (frame list)");
    ui.same_line();
    if ui.small_button("+ Messages") {
        app.new_msg_window();
    }
    tip(ui, "Add a Messages observer (aggregated by ID)");
    ui.same_line();
    if ui.small_button("+ Statistics") {
        app.new_stats_window();
    }
    tip(ui, "Add a Statistics observer");
    ui.same_line();
    if ui.small_button("+ Graphics") {
        app.new_graphics_window();
    }
    tip(ui, "Add a Graphics observer (signal plots)");
    ui.same_line();
    if ui.small_button("+ Data") {
        app.new_data_window();
    }
    tip(ui, "Add a Data observer (signal values)");
    ui.separator();

    let flags = TableFlags::BORDERS_INNER
        | TableFlags::ROW_BG
        | TableFlags::RESIZABLE
        | TableFlags::SCROLL_Y
        | TableFlags::SIZING_STRETCH_PROP;
    let mut rm_trace: Option<usize> = None;
    let mut rm_msgs: Option<usize> = None;
    let mut rm_stats: Option<usize> = None;
    let mut rm_graphics: Option<usize> = None;
    let mut rm_data: Option<usize> = None;
    {
        let Some(_table) = ui.begin_table_with_flags("meas_table", 6, flags) else {
            return;
        };
        ui.table_setup_column_with(TableColumnSetup {
            flags: TableColumnFlags::WIDTH_FIXED,
            init_width_or_weight: 40.0,
            ..TableColumnSetup::new("Open")
        });
        ui.table_setup_column_with(TableColumnSetup {
            flags: TableColumnFlags::WIDTH_FIXED,
            init_width_or_weight: 72.0,
            ..TableColumnSetup::new("Type")
        });
        ui.table_setup_column_with(TableColumnSetup {
            flags: TableColumnFlags::WIDTH_STRETCH,
            init_width_or_weight: 1.0,
            ..TableColumnSetup::new("Name")
        });
        ui.table_setup_column_with(TableColumnSetup {
            flags: TableColumnFlags::WIDTH_STRETCH,
            init_width_or_weight: 1.6,
            ..TableColumnSetup::new("Filter")
        });
        ui.table_setup_column_with(TableColumnSetup {
            flags: TableColumnFlags::WIDTH_FIXED,
            init_width_or_weight: 52.0,
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
}

/// Keeps the Filter Selection popup pointing at the right window after an
/// observer row is deleted. `which`: 0 = Trace, 1 = Messages, 2 = Statistics,
/// 3 = Graphics, 4 = Data.
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
        _ => Some(t),
    }
}

fn trace_row(app: &mut App, ui: &Ui, i: usize, rm: &mut Option<usize>) {
    ui.table_next_row();
    if !ui.table_next_column() {
        return;
    }
    open_button(ui, &format!("t{i}"), &mut app.trace_windows[i].opened);
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
    tip(ui, "Export filtered frames as ASC");
    ui.table_next_column();
    if ui.small_button(format!("x##t{i}")) {
        *rm = Some(i);
    }
    tip(ui, "Remove this observer");
}

fn messages_row(app: &mut App, ui: &Ui, i: usize, rm: &mut Option<usize>) {
    ui.table_next_row();
    if !ui.table_next_column() {
        return;
    }
    open_button(ui, &format!("m{i}"), &mut app.msg_windows[i].opened);
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
    tip(ui, "Export the visible aggregation rows as CSV");
    ui.table_next_column();
    if ui.small_button(format!("x##m{i}")) {
        *rm = Some(i);
    }
    tip(ui, "Remove this observer");
}

fn stats_row(app: &mut App, ui: &Ui, i: usize, rm: &mut Option<usize>) {
    ui.table_next_row();
    if !ui.table_next_column() {
        return;
    }
    open_button(ui, &format!("s{i}"), &mut app.stats_windows[i].opened);
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
    tip(ui, "Export per-message statistics as CSV");
    ui.table_next_column();
    if ui.small_button(format!("x##s{i}")) {
        *rm = Some(i);
    }
    tip(ui, "Remove this observer");
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
    };
    if ui.small_button(format!("…##selsig{prefix}{i}")) {
        app.popup_target = Some(match kind {
            ListKind::Graphics(i) => PopupTarget::Graphics(i),
            ListKind::Data(i) => PopupTarget::Data(i),
        });
        app.show_id_filter = true;
    }
    tip(ui, "Select signals for this window");
    ui.same_line();
    ui.text(format!("{vis}/{total}"));
}

fn graphics_row(app: &mut App, ui: &Ui, i: usize, rm: &mut Option<usize>) {
    ui.table_next_row();
    if !ui.table_next_column() {
        return;
    }
    open_button(ui, &format!("g{i}"), &mut app.graphics[i].opened);
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
    tip(ui, "Export the plotted signal history as CSV");
    ui.table_next_column();
    if ui.small_button(format!("x##g{i}")) {
        *rm = Some(i);
    }
    tip(ui, "Remove this observer");
}

fn data_row(app: &mut App, ui: &Ui, i: usize, rm: &mut Option<usize>) {
    ui.table_next_row();
    if !ui.table_next_column() {
        return;
    }
    open_button(ui, &format!("d{i}"), &mut app.data_windows[i].opened);
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
    tip(ui, "Export the latest signal values as CSV");
    ui.table_next_column();
    if ui.small_button(format!("x##d{i}")) {
        *rm = Some(i);
    }
    tip(ui, "Remove this observer");
}
