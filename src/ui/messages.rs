use crate::app::{App, PopupTarget};
use crate::ui::flags_color;
use crate::ui::idfilter::scope_combo;
use dear_imgui_rs::{
    Condition, TableColumnFlags, TableFlags, TableOptions, TableSizingPolicy, Ui,
};

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
        let focus = app.focus_title.as_deref() == Some(title.as_str());
        if focus {
            app.focus_title = None;
        }
        let mut window = ui
            .window(format!("{title}###msgs{i}"))
            .opened(&mut open)
            .position(
                [
                    io.display_size()[0] * 0.25 + off,
                    io.display_size()[1] * 0.45 + off,
                ],
                Condition::FirstUseEver,
            )
            .size([700.0, io.display_size()[1] * 0.42], Condition::FirstUseEver);
        if focus {
            window = window.focused(true);
        }
        window.build(|| window_content(app, ui, i));
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
    // Clear resets the per-message counters this window reads; the
    // filter controls keep their settings.
    if ui.button(format!("Clear##mf{i}")) {
        app.send(crate::bus::BusCommand::ClearAggregates);
    }
    ui.same_line();
    if ui.button(format!("Export##mx{i}")) {
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
        | TableFlags::NO_CLIP;
    let opts = TableOptions::from(tbl_flags).sizing_policy(TableSizingPolicy::StretchProp);
    let Some(_table) = ui.begin_table_with_flags(format!("msg_table{i}"), 7, opts) else {
        return;
    };
    ui.table_setup_column_stretch_weight("Message", TableColumnFlags::NONE, 1.0);
    for (label, w) in [
        ("Bus", 60.0),
        ("Dir", 34.0),
        ("Count", 55.0),
        ("Cycle (ms)", 72.0),
        ("Flags", 44.0),
    ] {
        ui.table_setup_column_fixed_width(label, TableColumnFlags::NONE, w);
    }
    ui.table_setup_column_stretch_weight("Data", TableColumnFlags::NONE, 1.0);
    ui.table_setup_scroll_freeze(0, 1);
    ui.table_headers_row();

    for row in &app.msg_windows[i].text_rows {
        ui.table_next_row();
        if !ui.table_next_column() {
            continue;
        }
        let token = ui
            .tree_node_config(row.label.clone())
            .span_full_width(true)
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
                // FlexRay rows are not DBC-declared by definition; the
                // empty case there means the watch has no description
                // database to decode with.
                if row.label.starts_with("FR slot") {
                    ui.text("   （无描述文件，无法解码信号）");
                } else {
                    ui.text("   (not in DBC)");
                }
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
