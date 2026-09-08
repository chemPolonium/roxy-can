//! The network entity table: every actor on the bus -- DBC-declared nodes
//! and script nodes alike -- as one flat list with a kind badge, an inline
//! state control, and a way into the entity's own UI. No topology drawing,
//! no kind folders: this table IS the network's representation.

use crate::app::{App, EntityKind, NodeRole};
use imgui::{Condition, TableColumnFlags, TableColumnSetup, TableFlags, Ui};

pub fn render(app: &mut App, ui: &Ui) {
    if !app.show_entities {
        return;
    }
    let io = ui.io();
    let mut open = app.show_entities;
    ui.window("Entities")
        .opened(&mut open)
        .position(
            [io.display_size[0] * 0.58, io.display_size[1] * 0.3],
            Condition::FirstUseEver,
        )
        .size([460.0, 360.0], Condition::FirstUseEver)
        .build(|| content(app, ui));
    app.show_entities = open;
}

fn content(app: &mut App, ui: &Ui) {
    let rows = app.entity_rows();
    ui.text_disabled("点击一行打开该实体的界面；右键行内菜单可插入或删除");
    ui.separator();

    if rows.is_empty() {
        ui.text("no entities: attach a DBC or add a script node");
        return;
    }

    let flags = TableFlags::BORDERS_INNER
        | TableFlags::ROW_BG
        | TableFlags::RESIZABLE
        | TableFlags::NO_BORDERS_IN_BODY
        | TableFlags::SCROLL_Y
        | TableFlags::SIZING_STRETCH_PROP;
    let Some(_t) = ui.begin_table_with_flags("entity_table", 4, flags) else {
        return;
    };
    ui.table_setup_column_with(TableColumnSetup {
        flags: TableColumnFlags::WIDTH_FIXED,
        init_width_or_weight: 52.0,
        ..TableColumnSetup::new("类型")
    });
    ui.table_setup_column_with(TableColumnSetup {
        flags: TableColumnFlags::WIDTH_STRETCH,
        init_width_or_weight: 1.4,
        ..TableColumnSetup::new("名称")
    });
    ui.table_setup_column_with(TableColumnSetup {
        flags: TableColumnFlags::WIDTH_STRETCH,
        init_width_or_weight: 0.8,
        ..TableColumnSetup::new("总线")
    });
    ui.table_setup_column_with(TableColumnSetup {
        flags: TableColumnFlags::WIDTH_FIXED,
        init_width_or_weight: 96.0,
        ..TableColumnSetup::new("状态")
    });
    ui.table_headers_row();

    let bus_names: Vec<String> = (0..app.snap.channel_count)
        .map(|ch| app.channel_name(ch as u8))
        .collect();
    for (i, row) in rows.iter().enumerate() {
        ui.table_next_row();

        if !ui.table_next_column() {
            continue;
        }
        match row.kind {
            EntityKind::Dbc => ui.text_colored([0.95, 0.70, 0.20, 1.0], "DBC"),
            EntityKind::Script => ui.text_colored([0.35, 0.85, 1.0, 1.0], "脚本"),
        }

        ui.table_next_column();
        // The name carries the row interaction: double-click opens the
        // entity's own UI, right-click its context menu. The id token
        // scopes every widget of the cell, so sibling rows never collide.
        {
            let _id = ui.push_id_usize(i);
            if ui.selectable_config(row.name.clone()).build() {
                open_entity_ui(app, row);
            }
            if ui.is_item_hovered() && ui.is_mouse_clicked(imgui::MouseButton::Right) {
                ui.open_popup("##entity-menu");
            }
            ui.popup("##entity-menu", || {
                match row.kind {
                    EntityKind::Dbc => {
                        ui.text_disabled("来自 DBC 声明，不可插入或删除");
                    }
                    EntityKind::Script => {
                        if ui.menu_item("删除节点") {
                            if let Some(id) = row.script_id {
                                app.send(crate::bus::BusCommand::RemoveNode { id });
                            }
                        }
                    }
                }
                ui.separator();
                if ui.menu_item("插入脚本节点") {
                    let name = format!("Node {}", app.snap.nodes.len() + 1);
                    app.send(crate::bus::BusCommand::AddNode {
                        name,
                        channel: row.channel,
                    });
                }
                ui.text_disabled("回放块（待实现）");
            });
        }

        ui.table_next_column();
        ui.text(
            bus_names
                .get(row.channel as usize)
                .cloned()
                .unwrap_or_default(),
        );

        ui.table_next_column();
        {
            let _id = ui.push_id_usize(10_000 + i);
            match row.kind {
                EntityKind::Dbc => {
                    // Same role combo as the Nodes-window card and the
                    // Network view: one declaration, three editors.
                    let mut role_idx = NodeRole::ALL
                        .iter()
                        .position(|r| Some(*r) == row.role)
                        .unwrap_or(0);
                    let labels: Vec<&str> = NodeRole::ALL.iter().map(|r| r.label()).collect();
                    if ui.combo_simple_string("##erole", &mut role_idx, &labels) {
                        app.send(crate::bus::BusCommand::SetNodeRole {
                            ch: row.channel,
                            node: row.name.clone(),
                            role: NodeRole::ALL[role_idx],
                        });
                    }
                }
                EntityKind::Script => {
                    let mut enabled = row.script_enabled.unwrap_or(false);
                    if ui.checkbox("##erun", &mut enabled) {
                        if let Some(id) = row.script_id {
                            app.send(crate::bus::BusCommand::SetNodeEnabled { id, on: enabled });
                        }
                    }
                }
            }
        }
    }
}

/// Double-click opens the entity's own UI: a script node lives in the
/// Nodes window, a DBC node's details live in the Network view with the
/// node selected.
fn open_entity_ui(app: &mut App, row: &crate::app::EntityRow) {
    match row.kind {
        EntityKind::Script => app.show_nodes = true,
        EntityKind::Dbc => {
            app.show_network = true;
            if let Some(idx) = row.network_select {
                app.net_selected = idx;
            }
        }
    }
}
