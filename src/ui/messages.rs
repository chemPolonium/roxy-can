use crate::app::{App, MessageAgg, PopupTarget, SigScope};
use crate::can::frame::{CanFrame, Direction};
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

    let w = app.msg_windows[i].clone();
    let filter = w.filter.trim().to_lowercase();
    let mut rows: Vec<MessageAgg> = app
        .aggs
        .values()
        .copied()
        .filter(|a| {
            if !App::scope_match(w.scope, &w.manual, a.channel, a.id) {
                return false;
            }
            let name = app.message_name(a.channel, a.id).unwrap_or("-");
            if w.dbc_only && name == "-" {
                return false;
            }
            if filter.is_empty() {
                return true;
            }
            let id_str = format!("{:x}", a.id);
            name.to_lowercase().contains(&filter) || id_str.contains(&filter)
        })
        .collect();
    rows.sort_by_key(|a| (a.channel, a.id));

    ui.same_line();
    ui.text(format!("{} messages", rows.len()));
    ui.separator();

    // NO_BORDERS_IN_BODY restricts column-resize dragging to the header row.
    let tbl_flags = TableFlags::BORDERS_INNER
        | TableFlags::ROW_BG
        | TableFlags::RESIZABLE
        | TableFlags::NO_BORDERS_IN_BODY
        | TableFlags::SCROLL_Y
        | TableFlags::SIZING_STRETCH_PROP
        | TableFlags::NO_CLIP;
    let Some(_table) = ui.begin_table_with_flags(format!("msg_table{i}"), 6, tbl_flags) else {
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
        init_width_or_weight: 165.0,
        ..TableColumnSetup::new("Data")
    });
    ui.table_headers_row();

    for agg in &rows {
        let name = app.message_name(agg.channel, agg.id).unwrap_or("-");
        ui.table_next_row();
        if !ui.table_next_column() {
            continue;
        }
        let id_str = if agg.extended {
            format!("{:08X}x", agg.id)
        } else {
            format!("{:03X}", agg.id)
        };
        let token = ui
            .tree_node_config(format!("{id_str}  {name}"))
            .flags(TreeNodeFlags::SPAN_FULL_WIDTH)
            .push();

        ui.table_next_column();
        ui.text(app.channel_name(agg.channel));
        ui.table_next_column();
        ui.text(match agg.dir {
            Direction::Rx => "Rx",
            Direction::Tx => "Tx",
        });
        ui.table_next_column();
        ui.text(format!("{}", agg.count));
        ui.table_next_column();
        if agg.count > 1 {
            ui.text(format!("{:.1}", agg.cycle_us / 1000.0));
        } else {
            ui.text("-");
        }
        ui.table_next_column();
        let data_str: String = agg.data[..agg.dlc.min(8) as usize]
            .iter()
            .map(|b| format!("{b:02X} "))
            .collect();
        ui.text(data_str);

        if token.is_some() {
            let frame = CanFrame {
                t_us: agg.last_t_us,
                channel: agg.channel,
                id: agg.id,
                extended: agg.extended,
                dlc: agg.dlc,
                data: agg.data,
                dir: agg.dir,
            };
            let sigs = app
                .channel_dbc(agg.channel)
                .map(|db| db.decode_signals(&frame))
                .unwrap_or_default();
            if sigs.is_empty() {
                ui.table_next_row();
                ui.table_next_column();
                ui.text("   (not in DBC)");
            } else {
                for (sig_name, phys, unit) in &sigs {
                    ui.table_next_row();
                    ui.table_next_column();
                    ui.text(format!("   {sig_name}"));
                    ui.table_set_column_index(1);
                    ui.text(format!("{phys:.3} {unit}"));
                }
            }
        }
    }
}
