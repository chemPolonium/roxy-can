use crate::app::App;
use imgui::{Condition, TableColumnFlags, TableColumnSetup, TableFlags, Ui};

/// Bus management: rename buses, load a DBC per bus, add/remove buses.
pub fn render(app: &mut App, ui: &Ui) {
    if !app.show_buses {
        return;
    }
    let io = ui.io();
    let mut open = app.show_buses;
    ui.window("Buses")
        .opened(&mut open)
        .position(
            [io.display_size[0] * 0.38, io.display_size[1] * 0.25],
            Condition::FirstUseEver,
        )
        .size([480.0, 240.0], Condition::FirstUseEver)
        .build(|| content(app, ui));
    app.show_buses = open;
}

fn file_name(p: &str) -> String {
    std::path::Path::new(p)
        .file_name()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| p.to_string())
}

/// Enumerates the installed Kvaser channels once per session; later calls
/// reuse the cached answer (including "driver unavailable").
fn ensure_kvaser_list(
    app: &mut App,
) -> Result<Vec<crate::hw::kvaser::ChannelInfo>, String> {
    let cached = app.kvaser_channels.clone();
    cached.unwrap_or_else(|| {
        let fresh = crate::hw::kvaser::enumerate();
        app.kvaser_channels = Some(fresh.clone());
        fresh
    })
}

fn content(app: &mut App, ui: &Ui) {
    if ui.small_button("+ Add bus") {
        app.add_channel();
    }
    ui.same_line();
    ui.text(format!("{} bus(es)", app.snap.channel_count));
    ui.same_line();
    // One global re-enumeration for the whole window: the channel list is
    // machine-wide, not per bus.
    if ui.small_button("刷新通道##hwref") {
        app.kvaser_channels = None;
    }
    if ui.is_item_hovered() {
        ui.tooltip_text("重新枚举本机 Kvaser 通道");
    }
    ui.separator();

    // NO_BORDERS_IN_BODY restricts column-resize dragging to the header row.
    let flags = TableFlags::BORDERS_INNER
        | TableFlags::ROW_BG
        | TableFlags::RESIZABLE
        | TableFlags::NO_BORDERS_IN_BODY
        | TableFlags::SCROLL_Y
        | TableFlags::SIZING_STRETCH_PROP;
    let mut remove: Option<usize> = None;
    {
        let Some(_table) = ui.begin_table_with_flags("bus_table", 5, flags) else {
            return;
        };
        ui.table_setup_column_with(TableColumnSetup {
            flags: TableColumnFlags::WIDTH_STRETCH,
            init_width_or_weight: 1.0,
            ..TableColumnSetup::new("Name")
        });
        ui.table_setup_column_with(TableColumnSetup {
            flags: TableColumnFlags::WIDTH_STRETCH,
            init_width_or_weight: 1.6,
            ..TableColumnSetup::new("DBC")
        });
        ui.table_setup_column_with(TableColumnSetup {
            flags: TableColumnFlags::WIDTH_FIXED,
            init_width_or_weight: 150.0,
            ..TableColumnSetup::new("kbit/s (arb / FD data)")
        });
        ui.table_setup_column_with(TableColumnSetup {
            flags: TableColumnFlags::WIDTH_FIXED,
            init_width_or_weight: 140.0,
            ..TableColumnSetup::new("硬件")
        });
        ui.table_setup_column_with(TableColumnSetup {
            flags: TableColumnFlags::WIDTH_FIXED,
            init_width_or_weight: 26.0,
            ..TableColumnSetup::new("")
        });
        ui.table_headers_row();

        // The rows render from the snapshot; edits are frontend drafts
        // that commit as commands.
        let views: Vec<(String, Vec<String>, u32, u32)> = app
            .snap
            .channels
            .iter()
            .map(|c| {
                (
                    c.name.clone(),
                    c.dbc_paths.clone(),
                    c.bitrate_kbps,
                    c.fd_data_kbps,
                )
            })
            .collect();
        for (i, (name, paths, arb_kbps, data_kbps)) in views.into_iter().enumerate() {
            ui.table_next_row();
            if !ui.table_next_column() {
                continue;
            }
            ui.set_next_item_width(-1.0);
            // The rename draft lives in `bus_name_edit` while the box has
            // focus; the bus sees the new name when the edit commits.
            let editing = matches!(&app.bus_name_edit, Some((r, _)) if *r == i);
            let mut name_buf = match &app.bus_name_edit {
                Some((r, s)) if *r == i => s.clone(),
                _ => name,
            };
            ui.input_text(format!("##busname{i}"), &mut name_buf)
                .build();
            if ui.is_item_active() {
                app.bus_name_edit = Some((i, name_buf.clone()));
            }
            if ui.is_item_deactivated_after_edit() {
                app.bus_name_edit = None;
                app.send(crate::bus::BusCommand::SetChannelConfig {
                    ch: i as u8,
                    name: Some(name_buf),
                    dbc_path: None,
                    bitrate_kbps: None,
                    fd_data_kbps: None,
                    node_roles: None,
                });
            } else if editing && !ui.is_item_active() {
                app.bus_name_edit = None;
            }
            ui.table_next_column();
            // No baseline alignment: the small button is text-height, so
            // label and button sit level on their own, and aligning would
            // push the `same_line` button a step below the row.
            let path = paths.first().cloned().unwrap_or_default();
            let extras: Vec<String> = paths.iter().skip(1).cloned().collect();
            ui.text(if path.trim().is_empty() {
                "(none)".to_string()
            } else {
                file_name(&path)
            });
            ui.same_line();
            if ui.small_button(format!("Open...##busdbc{i}")) {
                app.pick_dbc_for(i);
            }
            // Extra attached databases: one row each with a detach button.
            for (e, extra) in extras.iter().enumerate() {
                ui.text(file_name(extra));
                ui.same_line();
                if ui.small_button(format!("x##busdbx{i}_{e}")) {
                    app.detach_dbc_extra(i, e);
                }
            }
            ui.same_line();
            if ui.small_button(format!("+##busdbadd{i}")) {
                app.attach_dbc_dialog(i);
            }
            if ui.is_item_hovered() {
                ui.tooltip_text("附加更多 DBC 文件到该总线");
            }
            ui.table_next_column();
            // The load view divides wire bits by these; there is no hardware
            // behind the simulation, so the values are declarations about the
            // bus being analysed, not device settings. Each field is a plain
            // "type the number" box: the draft lives in App while the field
            // has focus, the parsed value commits when the edit ends, and an
            // unparsable text simply reverts to the model.
            let mut arb = match &app.bus_arb_edit {
                Some((r, s)) if *r == i => s.clone(),
                _ => arb_kbps.to_string(),
            };
            ui.set_next_item_width(70.0);
            if ui.input_text(format!("##busarb{i}"), &mut arb).build()
                || ui.is_item_active()
            {
                app.bus_arb_edit = Some((i, arb.clone()));
            }
            if ui.is_item_deactivated_after_edit() {
                if let Ok(v) = arb.trim().parse::<u32>() {
                    app.send(crate::bus::BusCommand::SetChannelConfig {
                        ch: i as u8,
                        name: None,
                        dbc_path: None,
                        bitrate_kbps: Some(v.max(1)),
                        fd_data_kbps: None,
                        node_roles: None,
                    });
                }
                app.bus_arb_edit = None;
            }
            if ui.is_item_hovered() {
                ui.tooltip_text("仲裁比特率 kbit/s，直接输入数字");
            }
            ui.same_line();
            let mut data = match &app.bus_data_edit {
                Some((r, s)) if *r == i => s.clone(),
                _ => data_kbps.to_string(),
            };
            ui.set_next_item_width(70.0);
            if ui.input_text(format!("##busdata{i}"), &mut data).build()
                || ui.is_item_active()
            {
                app.bus_data_edit = Some((i, data.clone()));
            }
            if ui.is_item_deactivated_after_edit() {
                if let Ok(v) = data.trim().parse::<u32>() {
                    app.send(crate::bus::BusCommand::SetChannelConfig {
                        ch: i as u8,
                        name: None,
                        dbc_path: None,
                        bitrate_kbps: None,
                        fd_data_kbps: Some(v.max(1)),
                        node_roles: None,
                    });
                }
                app.bus_data_edit = None;
            }
            if ui.is_item_hovered() {
                ui.tooltip_text("CAN FD 数据段比特率 kbit/s，直接输入数字");
            }
            ui.table_next_column();
            // Hardware attachment: one adapter per bus, enumerated from
            // the installed driver on first need.
            let attached = app
                .snap
                .hw
                .iter()
                .find(|h| h.bus as usize == i)
                .map(|h| (h.adapter, h.kbps, h.can_tx, h.fd));
            match attached {
                Some((adapter, kbps, can_tx, fd)) => {
                    // 两行布局：上行状态、下行解挂按钮——任何列宽下都完整
                    // 可见可点（单行塞不下时按钮会被单元格裁掉）。
                    ui.text(if can_tx {
                        format!("ch{adapter} {kbps}k{}", if fd { " FD" } else { "" })
                    } else {
                        format!("ch{adapter} {kbps}k 只收{}", if fd { " FD" } else { "" })
                    });
                    if ui.small_button(format!("解挂##hwdet{i}")) {
                        app.detach_hardware(i as u8);
                    }
                    if ui.is_item_hovered() {
                        // FD 数据段配在总线上但通道没带上 FD：降级原因
                        // 常驻提示（状态行早已被后续事件冲掉）。
                        ui.tooltip_text(if can_tx {
                            if fd {
                                "挂接中（收发，FD 数据段参数已应用）"
                            } else if data_kbps > 0 {
                                "挂接中（收发）。总线配了 FD 数据段波特率，但通道未带 FD——预设不匹配或硬件不支持，FD 帧上不了硬件。"
                            } else {
                                "挂接中（收发）"
                            }
                        } else {
                            "挂接中（只收：通道初始化访问被其他程序占用）"
                        });
                    }
                }
                None => {
                    let channels = ensure_kvaser_list(app).clone();
                    match channels {
                        Ok(channels) if !channels.is_empty() => {
                            let labels: Vec<String> = channels
                                .iter()
                                .map(|c| format!("ch{}: {}", c.index, c.name))
                                .collect();
                            let refs: Vec<&str> = labels.iter().map(|s| s.as_str()).collect();
                            ui.set_next_item_width(120.0);
                            let mut pick = 0;
                            if ui.combo_simple_string(format!("##hw{i}"), &mut pick, &refs) {
                                let info = &channels[pick];
                                app.set_hardware_channel(i as u8, info.index, arb_kbps, Some(data_kbps));
                            }
                            if ui.is_item_hovered() {
                                ui.tooltip_text("挂接 Kvaser 适配器：收到的帧进总线，节点可经它发车");
                            }
                        }
                        Ok(_) => ui.text_disabled("无通道"),
                        Err(e) => ui.text_disabled(e),
                    }
                }
            }
            ui.table_next_column();
            if ui.small_button(format!("x##busrm{i}")) {
                remove = Some(i);
            }
        }
    }
    if let Some(i) = remove {
        app.remove_channel(i);
    }
}
