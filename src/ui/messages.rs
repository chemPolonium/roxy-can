use crate::app::{App, PopupTarget};
use crate::ui::flags_color;
use crate::ui::idfilter::scope_combo;
use dear_imgui_rs::{
    Condition, TableColumnFlags, TableFlags, TableOptions, TableSizingPolicy, Ui,
};

/// The Messages table's columns, and the one an expanded row puts its value in.
/// A table cell is clipped to its own column, so the long text belongs in the
/// wide last one ("Data") rather than the narrow column beside the name.
const COLUMNS: usize = 7;
const VALUE_COL: usize = COLUMNS - 1;

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
    // No *table-level* NO_CLIP here: this table freezes its header row, and a
    // frozen-row table needs its per-cell clip rects -- with clipping switched
    // off wholesale the body rows keep the first cell's rect and every column
    // after "Message" vanishes (that was 0616fdc). The per-column flag on "Bus"
    // below is a different mechanism: the table's own clip stays in force and
    // only the narrowing of that column's merged draw channel is skipped, so its
    // text can run right without escaping the table.
    let tbl_flags = TableFlags::BORDERS_INNER
        | TableFlags::ROW_BG
        | TableFlags::RESIZABLE
        | TableFlags::NO_BORDERS_IN_BODY
        | TableFlags::SCROLL_Y;
    let opts = TableOptions::from(tbl_flags).sizing_policy(TableSizingPolicy::StretchProp);
    let Some(_table) = ui.begin_table_with_flags(format!("msg_table{i}"), COLUMNS, opts) else {
        return;
    };
    // An expanded row reads `signal name` in "Message" and the value in the last
    // column, "Data" -- the wide one. Not in the column next to the name: a cell
    // is clipped to its own column rect (`TableBeginCell` sets that rect unless
    // the *table* skips clipping, and the table cannot -- see above), and "Bus"
    // is 60 px, which cut "6600 rpm [u16]" down to "6600 rpm [". A decoded
    // FlexRay value ("2.54 Mpa (正常)  (1Fh)", "2 (完成)  (2h)") needs the room,
    // and on a child row every column after "Message" is empty anyway. This is
    // also how the Trace window's FlexRay sub-rows place their values: same
    // shape in both tables, and the values of a list of signals line up.
    //
    // "Message" stays clipped on purpose: when both the name and the value were
    // drawn in that one cell, at a fixed offset for the value, a long name
    // printed straight through its reading ("...OrderStat" under "(1h)").
    // Clipped, an over-long name stops at the column edge -- widen the column by
    // dragging its header, which is what RESIZABLE is for.
    ui.table_setup_column_stretch_weight("Message", TableColumnFlags::NONE, 2.0);
    ui.table_setup_column(
        "Bus",
        TableColumnFlags::NONE,
        Some(dear_imgui_rs::TableColumnWidth::fixed(60.0)),
    );
    for (label, w) in [
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
        // The row's own ID, not its label: a table does not seed item IDs per
        // cell, and the same message on two buses prints the same label.
        let _row_id = ui.push_id(row.id_key.as_str());
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
                // The sync pass worked out *why* there is nothing here; the
                // window only prints it. It goes in the value's column because
                // the sentence is longer than any cell before it.
                if ui.table_set_column_index(VALUE_COL) {
                    ui.text(row.empty_note.as_deref().unwrap_or("(no signals)"));
                }
            } else {
                for (name, value) in &row.signals {
                    ui.table_next_row();
                    ui.table_next_column();
                    ui.text(format!("   {name}"));
                    if ui.table_set_column_index(VALUE_COL) {
                        ui.text(value);
                    }
                }
            }
        }
    }
}
