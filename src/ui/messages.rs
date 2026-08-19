use crate::app::{App, MessageAgg};
use crate::can::frame::{CanFrame, Direction};
use imgui::{Condition, TableColumnFlags, TableColumnSetup, TableFlags, TreeNodeFlags, Ui};

pub fn render(app: &mut App, ui: &Ui) {
    let io = ui.io();
    let mut open = app.show_messages;
    if open {
        ui.window("Messages")
            .opened(&mut open)
            .position(
                [io.display_size[0] * 0.25, io.display_size[1] * 0.45],
                Condition::FirstUseEver,
            )
            .size(
                [700.0, io.display_size[1] * 0.42],
                Condition::FirstUseEver,
            )
            .flags(imgui::WindowFlags::NO_SAVED_SETTINGS)
            .build(|| window_content(app, ui));
    }
    app.show_messages = open;
}

fn window_content(app: &mut App, ui: &Ui) {
    ui.set_next_item_width(200.0);
    ui.input_text("Filter", &mut app.msg_filter).build();
    ui.same_line();
    ui.checkbox("DBC only", &mut app.dbc_only);
    ui.same_line();
    if ui.button("Clear") {
        app.aggs.clear();
    }

    let filter = app.msg_filter.trim().to_lowercase();
    let dbc_only = app.dbc_only;
    let mut rows: Vec<MessageAgg> = app
        .aggs
        .values()
        .copied()
        .filter(|a| {
            let name = app
                .dbc
                .as_ref()
                .and_then(|db| db.message_name(a.id))
                .unwrap_or("-");
            if dbc_only && name == "-" {
                return false;
            }
            if filter.is_empty() {
                return true;
            }
            let id_str = format!("{:x}", a.id);
            name.to_lowercase().contains(&filter) || id_str.contains(&filter)
        })
        .collect();
    rows.sort_by_key(|a| a.id);

    ui.same_line();
    ui.text(format!("{} messages", rows.len()));
    ui.separator();

    let tbl_flags = TableFlags::BORDERS_INNER
        | TableFlags::ROW_BG
        | TableFlags::RESIZABLE
        | TableFlags::SCROLL_Y
        | TableFlags::SIZING_STRETCH_PROP
        | TableFlags::NO_CLIP;
    let Some(_table) = ui.begin_table_with_flags("msg_table", 6, tbl_flags) else {
        return;
    };
    ui.table_setup_column_with(TableColumnSetup {
        flags: TableColumnFlags::WIDTH_STRETCH,
        init_width_or_weight: 2.0,
        ..TableColumnSetup::new("Message")
    });
    ui.table_setup_column_with(TableColumnSetup {
        flags: TableColumnFlags::WIDTH_FIXED,
        init_width_or_weight: 30.0,
        ..TableColumnSetup::new("Ch")
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
        init_width_or_weight: 70.0,
        ..TableColumnSetup::new("Cycle (ms)")
    });
    ui.table_setup_column_with(TableColumnSetup {
        flags: TableColumnFlags::WIDTH_STRETCH,
        init_width_or_weight: 1.6,
        ..TableColumnSetup::new("Data")
    });
    ui.table_headers_row();

    for agg in &rows {
        let name = app
            .dbc
            .as_ref()
            .and_then(|db| db.message_name(agg.id))
            .unwrap_or("-");
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
        ui.text(format!("{}", agg.channel + 1));
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
                .dbc
                .as_ref()
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
