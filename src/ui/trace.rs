use std::sync::Mutex;

use crate::app::{App, PopupTarget, SigScope, TOOLBAR_H};
use crate::can::frame::{CanFrame, Direction};
use crate::ui::idfilter::scope_combo;
use imgui::{Condition, TableColumnFlags, TableColumnSetup, TableFlags, Ui};

const MAX_VISIBLE: usize = 1_000;

/// Window index and frame targeted by the row context menu; must survive
/// across frames while the popup is open.
static CTX: Mutex<Option<(usize, CanFrame)>> = Mutex::new(None);

pub fn render(app: &mut App, ui: &Ui) {
    let io = ui.io();
    let n = app.trace_windows.len();
    for i in 0..n {
        if !app.trace_windows[i].opened {
            continue;
        }
        let mut open = true;
        let raw = app.trace_windows[i].name.clone();
        let title = if raw.trim().is_empty() {
            format!("Trace {}", i + 1)
        } else {
            raw
        };
        let off = i as f32 * 30.0;
        if app.focus_title.as_deref() == Some(title.as_str()) {
            unsafe { imgui::sys::igSetNextWindowFocus() };
            app.focus_title = None;
        }
        ui.window(format!("{title}###trace{i}"))
            .opened(&mut open)
            .position([off, TOOLBAR_H + off], Condition::FirstUseEver)
            .size(
                [
                    io.display_size[0] * 0.55,
                    io.display_size[1] * 0.5 - TOOLBAR_H,
                ],
                Condition::FirstUseEver,
            )
            .flags(imgui::WindowFlags::NO_SAVED_SETTINGS)
            .build(|| window_content(app, ui, i));
        app.trace_windows[i].opened = open;
    }
}

fn window_content(app: &mut App, ui: &Ui, i: usize) {
    ui.text(format!(
        "{} frames (showing newest {})",
        app.trace.len(),
        MAX_VISIBLE.min(app.trace.len())
    ));
    let new_scope = scope_combo(
        app,
        ui,
        &format!("##tscope{i}"),
        app.trace_windows[i].scope,
        PopupTarget::Trace(i),
    );
    app.trace_windows[i].scope = new_scope;
    ui.same_line();
    ui.set_next_item_width(120.0);
    ui.input_text(format!("##tfilter{i}"), &mut app.trace_windows[i].filter)
        .build();
    ui.same_line();
    ui.set_next_item_width(60.0);
    ui.combo_simple_string(
        format!("##tdir{i}"),
        &mut app.trace_windows[i].dir,
        &["All", "Rx", "Tx"],
    );
    ui.same_line();
    ui.checkbox(
        format!("DBC only##tdbc{i}"),
        &mut app.trace_windows[i].dbc_only,
    );
    ui.same_line();
    if ui.small_button(format!("Clear##tf{i}")) {
        let w = &mut app.trace_windows[i];
        w.filter.clear();
        w.dir = 0;
        w.dbc_only = false;
        w.scope = SigScope::All;
    }
    ui.same_line();
    if ui.small_button(format!("Export##tx{i}")) {
        app.export_trace_dialog(i);
    }
    if ui.is_item_hovered() {
        ui.tooltip_text("Export this trace as ASC");
    }
    ui.separator();

    // NO_BORDERS_IN_BODY restricts column-resize dragging to the header row.
    let tbl_flags = TableFlags::BORDERS_INNER
        | TableFlags::ROW_BG
        | TableFlags::RESIZABLE
        | TableFlags::NO_BORDERS_IN_BODY
        | TableFlags::SCROLL_Y
        | TableFlags::SIZING_STRETCH_PROP;
    let Some(_table) = ui.begin_table_with_flags(format!("trace_table{i}"), 7, tbl_flags) else {
        return;
    };
    // "{:.6}" timestamp: up to ~10 chars
    ui.table_setup_column_with(TableColumnSetup {
        flags: TableColumnFlags::WIDTH_FIXED,
        init_width_or_weight: 76.0,
        ..TableColumnSetup::new("Time")
    });
    ui.table_setup_column_with(TableColumnSetup {
        flags: TableColumnFlags::WIDTH_FIXED,
        init_width_or_weight: 60.0,
        ..TableColumnSetup::new("Bus")
    });
    // extended IDs render as "1FFFFFFFx" (9 chars)
    ui.table_setup_column_with(TableColumnSetup {
        flags: TableColumnFlags::WIDTH_FIXED,
        init_width_or_weight: 68.0,
        ..TableColumnSetup::new("ID")
    });
    ui.table_setup_column_with(TableColumnSetup {
        flags: TableColumnFlags::WIDTH_STRETCH,
        init_width_or_weight: 1.0,
        ..TableColumnSetup::new("Name")
    });
    ui.table_setup_column_with(TableColumnSetup {
        flags: TableColumnFlags::WIDTH_FIXED,
        init_width_or_weight: 32.0,
        ..TableColumnSetup::new("DLC")
    });
    ui.table_setup_column_with(TableColumnSetup {
        flags: TableColumnFlags::WIDTH_STRETCH,
        init_width_or_weight: 1.6,
        ..TableColumnSetup::new("Data")
    });
    ui.table_setup_column_with(TableColumnSetup {
        flags: TableColumnFlags::WIDTH_FIXED,
        init_width_or_weight: 34.0,
        ..TableColumnSetup::new("Dir")
    });
    ui.table_headers_row();

    let w = app.trace_windows[i].clone();
    let mut shown = 0usize;
    for f in app.trace.iter().rev() {
        if !app.trace_match(&w, f) {
            continue;
        }
        if shown >= MAX_VISIBLE {
            break;
        }
        shown += 1;
        let mut hovered = false;
        ui.table_next_row();
        if !ui.table_next_column() {
            continue;
        }
        ui.text(format!("{:.6}", f.t_us as f64 / 1e6));
        hovered |= ui.is_item_hovered();
        ui.table_next_column();
        ui.text(app.channel_name(f.channel));
        hovered |= ui.is_item_hovered();
        ui.table_next_column();
        if f.extended {
            ui.text(format!("{:08X}x", f.id));
        } else {
            ui.text(format!("{:03X}", f.id));
        }
        hovered |= ui.is_item_hovered();
        ui.table_next_column();
        match app.message_name(f.channel, f.id) {
            Some(name) => ui.text(name),
            None => ui.text("-"),
        }
        hovered |= ui.is_item_hovered();
        ui.table_next_column();
        ui.text(format!("{}", f.dlc));
        hovered |= ui.is_item_hovered();
        ui.table_next_column();
        let data_str: String = f.data[..f.dlc.min(8) as usize]
            .iter()
            .map(|b| format!("{b:02X} "))
            .collect();
        ui.text(data_str);
        hovered |= ui.is_item_hovered();
        ui.table_next_column();
        match f.dir {
            Direction::Rx => ui.text_colored([0.6, 0.65, 0.7, 1.0], "Rx"),
            Direction::Tx => ui.text_colored([1.0, 0.65, 0.2, 1.0], "Tx"),
        }
        hovered |= ui.is_item_hovered();
        if hovered && ui.is_mouse_released(imgui::MouseButton::Right) {
            *CTX.lock().unwrap() = Some((i, *f));
            ui.open_popup(format!("trace_row_ctx{i}"));
        }
    }
    if let Some(_p) = ui.begin_popup(format!("trace_row_ctx{i}")) {
        if let Some((pi, f)) = *CTX.lock().unwrap() {
            if pi == i {
                let name = app.message_name(f.channel, f.id).unwrap_or("-");
                ui.text(format!(
                    "{}  {:03X}  {name}",
                    app.channel_name(f.channel),
                    f.id
                ));
                ui.separator();
                if ui.menu_item(format!("Filter this ID ({:03X})", f.id)) {
                    app.trace_windows[i].filter = format!("{:03X}", f.id);
                }
                if ui.menu_item("Clear filter") {
                    let w = &mut app.trace_windows[i];
                    w.filter.clear();
                    w.dir = 0;
                    w.dbc_only = false;
                    w.scope = SigScope::All;
                }
                if ui.menu_item("Add to Interactive Generator") {
                    app.add_tx(f.channel, f.id);
                }
            }
        }
    }
}
