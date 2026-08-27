use std::cmp::Ordering;
use std::sync::Mutex;

use crate::app::{App, PopupTarget, SigScope, TOOLBAR_H};
use crate::can::frame::{CanFrame, Direction};
use crate::ui::flags_color;
use crate::ui::idfilter::scope_combo;
use imgui::{
    Condition, TableBgTarget, TableColumnFlags, TableColumnSetup, TableFlags, TableSortDirection,
    Ui,
};

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
            .build(|| window_content(app, ui, i));
        app.trace_windows[i].opened = open;
    }
}

fn fmt_id(f: &CanFrame) -> String {
    if f.extended {
        format!("{:08X}x", f.id)
    } else {
        format!("{:03X}", f.id)
    }
}

fn fmt_data(f: &CanFrame) -> String {
    f.payload()
        .iter()
        .map(|b| format!("{b:02X} "))
        .collect()
}

/// One trace row as plain text, used for "Copy row".
fn fmt_row(app: &App, f: &CanFrame) -> String {
    let tag = f.flags.tag();
    let flags = if tag.is_empty() { "-" } else { tag };
    format!(
        "{:.6}  {}  {}  {}  {}  {}  {}  {}",
        f.t_us as f64 / 1e6,
        app.channel_name(f.channel),
        fmt_id(f),
        app.message_name(f.channel, f.id).unwrap_or("-"),
        f.len,
        flags,
        fmt_data(f).trim_end(),
        match f.dir {
            Direction::Rx => "Rx",
            Direction::Tx => "Tx",
        }
    )
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
    ui.separator();

    // NO_BORDERS_IN_BODY restricts column-resize dragging to the header row.
    let tbl_flags = TableFlags::BORDERS_INNER
        | TableFlags::ROW_BG
        | TableFlags::RESIZABLE
        | TableFlags::NO_BORDERS_IN_BODY
        | TableFlags::SCROLL_Y
        | TableFlags::SORTABLE
        | TableFlags::SORT_TRISTATE
        | TableFlags::SIZING_STRETCH_PROP;
    let Some(_table) = ui.begin_table_with_flags(format!("trace_table{i}"), 8, tbl_flags) else {
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
        init_width_or_weight: 36.0,
        ..TableColumnSetup::new("Len")
    });
    ui.table_setup_column_with(TableColumnSetup {
        flags: TableColumnFlags::WIDTH_FIXED,
        init_width_or_weight: 44.0,
        ..TableColumnSetup::new("Flags")
    });
    // A full 64-byte FD payload needs room; let it stretch with the window.
    ui.table_setup_column_with(TableColumnSetup {
        flags: TableColumnFlags::WIDTH_STRETCH,
        init_width_or_weight: 1.4,
        ..TableColumnSetup::new("Data")
    });
    ui.table_setup_column_with(TableColumnSetup {
        flags: TableColumnFlags::WIDTH_FIXED,
        init_width_or_weight: 34.0,
        ..TableColumnSetup::new("Dir")
    });
    ui.table_headers_row();

    // Frozen while rendering so row drawing and the right-click popup can
    // share one copy of the window settings.
    let w = app.trace_windows[i].clone();

    // Newest first (the default order); sorted when a column header is
    // clicked, back to default on the third click (tri-state).
    let mut rows: Vec<CanFrame> = app
        .trace
        .iter()
        .rev()
        .filter(|f| app.trace_match(&w, f))
        .take(MAX_VISIBLE)
        .copied()
        .collect();

    // imgui-rs builds the specs slice unconditionally; with no active sort
    // the pointer is NULL, so only read it when SpecsCount > 0.
    let specs_active = unsafe {
        let raw = imgui::sys::igTableGetSortSpecs();
        !raw.is_null() && (*raw).SpecsCount > 0
    };
    if specs_active {
        if let Some(mut specs) = ui.table_sort_specs_mut() {
            let spec = specs.specs().iter().next();
            if let Some(s) = spec {
                let col = s.column_idx();
                let asc = s.sort_direction() == Some(TableSortDirection::Ascending);
                rows.sort_by(|a, b| sort_frame(app, col, a, b, asc));
            }
            specs.set_sorted();
        }
    }

    let mut shown = 0usize;
    for f in &rows {
        if shown >= MAX_VISIBLE {
            break;
        }
        shown += 1;
        let mut hovered = false;
        ui.table_next_row();
        if f.is_error() {
            ui.table_set_bg_color(TableBgTarget::ROW_BG1, [0.55, 0.12, 0.12, 0.35]);
        } else if f.is_remote() {
            ui.table_set_bg_color(TableBgTarget::ROW_BG1, [0.35, 0.22, 0.55, 0.25]);
        }
        if !ui.table_next_column() {
            continue;
        }
        ui.text(format!("{:.6}", f.t_us as f64 / 1e6));
        hovered |= ui.is_item_hovered();
        ui.table_next_column();
        ui.text(app.channel_name(f.channel));
        hovered |= ui.is_item_hovered();
        ui.table_next_column();
        ui.text(fmt_id(f));
        hovered |= ui.is_item_hovered();
        ui.table_next_column();
        match app.message_name(f.channel, f.id) {
            Some(name) => ui.text(name),
            None => ui.text("-"),
        }
        hovered |= ui.is_item_hovered();
        ui.table_next_column();
        ui.text(format!("{}", f.len));
        hovered |= ui.is_item_hovered();
        ui.table_next_column();
        ui.text_colored(flags_color(f.flags), f.flags.tag());
        hovered |= ui.is_item_hovered();
        ui.table_next_column();
        ui.text(fmt_data(f));
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
                    "{}  {}  {name}",
                    app.channel_name(f.channel),
                    fmt_id(&f)
                ));
                ui.separator();
                if ui.menu_item(format!("Filter this ID ({})", fmt_id(&f))) {
                    app.trace_windows[i].filter = format!("{:03X}", f.id);
                }
                if ui.menu_item("Clear filter") {
                    let w = &mut app.trace_windows[i];
                    w.filter.clear();
                    w.dir = 0;
                    w.dbc_only = false;
                    w.scope = SigScope::All;
                }
                let addable = !f.is_error() && !f.is_remote();
                if ui
                    .menu_item_config("Add to Interactive Generator")
                    .enabled(addable)
                    .build()
                {
                    let was_len = app.tx_list.len();
                    app.add_tx(f.channel, f.id);
                    if app.tx_list.len() > was_len {
                        let known = app
                            .channel_dbc(f.channel)
                            .is_some_and(|db| db.messages.contains_key(&f.id));
                        if let Some(t) = app.tx_list.last_mut() {
                            t.flags = f.flags;
                            if !known {
                                t.len = f.len;
                                t.data = f.data;
                                t.data_text = f.data[..f.len as usize]
                                    .iter()
                                    .map(|b| format!("{b:02X}"))
                                    .collect::<Vec<_>>()
                                    .join(" ");
                            }
                        }
                    }
                }
                ui.separator();
                if ui.menu_item("Copy row") {
                    ui.set_clipboard_text(fmt_row(app, &f));
                }
                if ui.menu_item("Copy ID") {
                    ui.set_clipboard_text(fmt_id(&f));
                }
            }
        }
    }
}

fn sort_frame(app: &App, col: usize, a: &CanFrame, b: &CanFrame, asc: bool) -> Ordering {
    let ord = match col {
        0 => a.t_us.cmp(&b.t_us),
        1 => a.channel.cmp(&b.channel),
        2 => a.id.cmp(&b.id),
        3 => {
            let na = app.message_name(a.channel, a.id).unwrap_or("");
            let nb = app.message_name(b.channel, b.id).unwrap_or("");
            na.cmp(nb)
        }
        4 => a.len.cmp(&b.len),
        5 => (a.is_fd(), a.esi(), a.brs()).cmp(&(b.is_fd(), b.esi(), b.brs())),
        6 => a.payload().cmp(b.payload()),
        _ => a.dir.cmp(&b.dir),
    };
    if asc { ord } else { ord.reverse() }
}
