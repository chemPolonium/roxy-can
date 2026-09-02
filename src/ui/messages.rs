use crate::app::{App, PopupTarget, SigScope};
use crate::ui::flags_color;
use crate::ui::idfilter::scope_combo;
use imgui::{Condition, TableColumnFlags, TableColumnSetup, TableFlags, TreeNodeFlags, Ui};

pub fn render(app: &mut App, ui: &Ui) {
    let io = ui.io();
    let n = app.msg_windows.len();
    for i in 0..n {
        if !app.msg_windows[i].opened {
            continue;
        }
        let mut open = true;
        let raw = app.msg_windows[i].name.clone();
        let title = if raw.trim().is_empty() {
            format!("Messages {}", i + 1)
        } else {
            raw
        };
        let off = i as f32 * 30.0;
        if app.focus_title.as_deref() == Some(title.as_str()) {
            unsafe { imgui::sys::igSetNextWindowFocus() };
            app.focus_title = None;
        }
        ui.window(format!("{title}###msgs{i}"))
            .opened(&mut open)
            .position(
                [
                    io.display_size[0] * 0.25 + off,
                    io.display_size[1] * 0.45 + off,
                ],
                Condition::FirstUseEver,
            )
            .size([700.0, io.display_size[1] * 0.42], Condition::FirstUseEver)
            .build(|| window_content(app, ui, i));
        app.msg_windows[i].opened = open;
    }
}

fn window_content(app: &mut App, ui: &Ui, i: usize) {
    let new_scope = scope_combo(
        app,
        ui,
        &format!("##mscope{i}"),
        app.msg_windows[i].scope,
        PopupTarget::Messages(i),
    );
    app.msg_windows[i].scope = new_scope;
    ui.same_line();
    ui.set_next_item_width(150.0);
    ui.input_text(format!("##mfilter{i}"), &mut app.msg_windows[i].filter)
        .build();
    ui.same_line();
    ui.checkbox(
        format!("DBC only##mbc{i}"),
        &mut app.msg_windows[i].dbc_only,
    );
    ui.same_line();
    if ui.small_button(format!("Clear##mf{i}")) {
        let w = &mut app.msg_windows[i];
        w.filter.clear();
        w.dbc_only = false;
        w.scope = SigScope::All;
    }
    ui.same_line();
    if ui.small_button(format!("Export##mx{i}")) {
        app.export_messages_dialog(i);
    }

    // Throttled like every number readout: the rows are snapshots from the
    // text gate, so Count and Cycle hold still long enough to read.
    app.sync_msg_text(i);

    ui.same_line();
    ui.text(&app.msg_windows[i].text_header);
    ui.separator();

    // NO_BORDERS_IN_BODY restricts column-resize dragging to the header row.
    let tbl_flags = TableFlags::BORDERS_INNER
        | TableFlags::ROW_BG
        | TableFlags::RESIZABLE
        | TableFlags::NO_BORDERS_IN_BODY
        | TableFlags::SCROLL_Y
        | TableFlags::SIZING_STRETCH_PROP
        | TableFlags::NO_CLIP;
    let Some(_table) = ui.begin_table_with_flags(format!("msg_table{i}"), 7, tbl_flags) else {
        return;
    };
    ui.table_setup_column_with(TableColumnSetup {
        flags: TableColumnFlags::WIDTH_STRETCH,
        init_width_or_weight: 2.0,
        ..TableColumnSetup::new("Message")
    });
    ui.table_setup_column_with(TableColumnSetup {
        flags: TableColumnFlags::WIDTH_FIXED,
        init_width_or_weight: 60.0,
        ..TableColumnSetup::new("Bus")
    });
    ui.table_setup_column_with(TableColumnSetup {
        flags: TableColumnFlags::WIDTH_FIXED,
        init_width_or_weight: 34.0,
        ..TableColumnSetup::new("Dir")
    });
    ui.table_setup_column_with(TableColumnSetup {
        flags: TableColumnFlags::WIDTH_FIXED,
        init_width_or_weight: 55.0,
        ..TableColumnSetup::new("Count")
    });
    ui.table_setup_column_with(TableColumnSetup {
        flags: TableColumnFlags::WIDTH_FIXED,
        init_width_or_weight: 72.0,
        ..TableColumnSetup::new("Cycle (ms)")
    });
    ui.table_setup_column_with(TableColumnSetup {
        flags: TableColumnFlags::WIDTH_FIXED,
        init_width_or_weight: 44.0,
        ..TableColumnSetup::new("Flags")
    });
    ui.table_setup_column_with(TableColumnSetup {
        flags: TableColumnFlags::WIDTH_STRETCH,
        init_width_or_weight: 2.0,
        ..TableColumnSetup::new("Data")
    });
    ui.table_headers_row();

    for row in &app.msg_windows[i].text_rows {
        ui.table_next_row();
        if !ui.table_next_column() {
            continue;
        }
        let token = ui
            .tree_node_config(row.label.clone())
            .flags(TreeNodeFlags::SPAN_FULL_WIDTH)
            .push();

        ui.table_next_column();
        ui.text(&row.bus);
        ui.table_next_column();
        ui.text(row.dir);
        ui.table_next_column();
        ui.text(&row.count);
        ui.table_next_column();
        ui.text(&row.cycle);
        ui.table_next_column();
        ui.text_colored(flags_color(row.flags), row.flags.tag());
        ui.table_next_column();
        ui.text(&row.data);

        if token.is_some() {
            if row.signals.is_empty() {
                ui.table_next_row();
                ui.table_next_column();
                ui.text("   (not in DBC)");
            } else {
                for (name, value) in &row.signals {
                    ui.table_next_row();
                    ui.table_next_column();
                    ui.text(format!("   {name}"));
                    ui.table_set_column_index(1);
                    ui.text(value);
                }
            }
        }
    }
}
