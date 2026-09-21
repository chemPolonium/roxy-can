use crate::app::App;
use dear_imgui_rs::{Condition, TableColumnFlags, TableFlags, Ui};

/// The arbitration bit rates a CAN bus is wired for in practice. A pick list,
/// not a typed number: a wrong bit rate does not fail, it silently mis-decodes
/// every frame on the wire, so the choices are the standard ones -- plus the
/// value a project already carries, which stays listed so opening an old
/// project never "fixes" a rate the user chose on purpose.
const CAN_ARB_KBPS: &[u32] = &[10, 50, 100, 125, 250, 500, 800, 1000];

/// CAN FD data-phase rates. `0` means the bus has no data phase, which is the
/// common case for a classic-CAN project and has to be reachable, not merely
/// the absence of a number.
const CAN_DATA_KBPS: &[u32] = &[0, 1000, 2000, 4000, 5000, 8000];

/// One bit-rate pick list: the standard values with `current` folded in, and
/// the labels to show them by.
fn rate_choices(standards: &[u32], current: u32, zero_label: &str) -> (Vec<u32>, Vec<String>) {
    let mut vals: Vec<u32> = standards.to_vec();
    if !vals.contains(&current) {
        vals.push(current);
        vals.sort_unstable();
    }
    let labels = vals
        .iter()
        .map(|v| {
            if *v == 0 && !zero_label.is_empty() {
                zero_label.to_string()
            } else {
                v.to_string()
            }
        })
        .collect();
    (vals, labels)
}

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
            [io.display_size()[0] * 0.38, io.display_size()[1] * 0.25],
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

/// What a bus's loaded description says its cluster is called, plus how many
/// frames it schedules. The one piece of identity a description file carries
/// that a recording does not -- with two networks in one log it is how the
/// user checks he put the right file on the right 路.
fn fr_cluster_tag(app: &App, bus: u8) -> String {
    match app.fr_db(bus) {
        Some(db) if !db.params.name.is_empty() => {
            format!(" · cluster {}（{} 帧）", db.params.name, db.frames.len())
        }
        _ => String::new(),
    }
}

/// Enumerates the hardware channels of every supported driver once per
/// session; later calls reuse the cached answer (including "driver
/// unavailable").
fn ensure_hw_list(app: &mut App) -> Result<Vec<crate::hw::AnyChannelInfo>, String> {
    let cached = app.hw_channels.clone();
    cached.unwrap_or_else(|| {
        let fresh = crate::hw::enumerate_all();
        app.hw_channels = Some(fresh.clone());
        fresh
    })
}

/// Enumerates the Vector channels once per session, on the same
/// cache-or-probe pattern as the CAN list. Portal devices (VN7640...)
/// often do not report the FlexRay capability bit at all, so the FR
/// section offers every channel -- the attach attempt is the real test,
/// and its failure lands in the status line.
fn ensure_fr_list(app: &mut App) -> Result<Vec<crate::hw::vector::ChannelInfo>, String> {
    let cached = app.fr_channels.clone();
    cached.unwrap_or_else(|| {
        let fresh = crate::hw::vector::enumerate();
        app.fr_channels = Some(fresh.clone());
        fresh
    })
}

fn content(app: &mut App, ui: &Ui) {
    if ui.button("+ Add bus") {
        app.add_channel();
    }
    ui.same_line();
    ui.text(format!("{} bus(es)", app.snap.channel_count));
    ui.same_line();
    // One global re-enumeration for the whole window: the channel list is
    // machine-wide, not per bus.
    if ui.button("刷新通道##hwref") {
        app.hw_channels = None;
        app.fr_channels = None;
    }
    if ui.is_item_hovered() {
        ui.tooltip_text("重新枚举本机硬件通道（Kvaser / Vector）");
    }
    ui.separator();

    // NO_BORDERS_IN_BODY restricts column-resize dragging to the header row.
    let flags = TableFlags::RESIZABLE;
    let opts = dear_imgui_rs::TableOptions::from(flags)
        .sizing_policy(dear_imgui_rs::TableSizingPolicy::StretchProp);
    let mut remove: Option<usize> = None;
    {
        let Some(_table) = ui.begin_table_with_flags("bus_table", 5, opts) else {
            return;
        };
        ui.table_setup_column_stretch_weight("Name", TableColumnFlags::NONE, 1.0);
        ui.table_setup_column_stretch_weight("DBC", TableColumnFlags::NONE, 1.6);
        ui.table_setup_column_fixed_width("kbit/s (arb / FD data)", TableColumnFlags::NONE, 190.0);
        ui.table_setup_column_fixed_width("硬件", TableColumnFlags::NONE, 140.0);
        ui.table_setup_column_fixed_width("", TableColumnFlags::NONE, 26.0);
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
            ui.input_text(format!("##busname{i}"), &mut name_buf).build();
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
            let path = paths.first().cloned().unwrap_or_default();
            let extras: Vec<String> = paths.iter().skip(1).cloned().collect();
            ui.text(if path.trim().is_empty() {
                "(none)".to_string()
            } else {
                file_name(&path)
            });
            // Same two lines as the FlexRay rows below: the value first, the
            // actions underneath, so a long file name can never push a button
            // out of the cell. No baseline alignment either -- the small
            // buttons are text-height, so they sit level on their own line.
            if ui.button(format!("Open...##busdbc{i}")) {
                app.pick_dbc_for(i);
            }
            if ui.is_item_hovered() {
                ui.tooltip_text("换这条总线的主 DBC 文件（解码与查名以它为先）");
            }
            ui.same_line();
            if ui.button(format!("+##busdbadd{i}")) {
                app.attach_dbc_dialog(i);
            }
            if ui.is_item_hovered() {
                ui.tooltip_text("附加更多 DBC 文件到该总线");
            }
            // Extra attached databases: one line each with a detach button.
            for (e, extra) in extras.iter().enumerate() {
                ui.text_disabled(file_name(extra));
                ui.same_line();
                if ui.button(format!("x##busdbx{i}_{e}")) {
                    app.detach_dbc_extra(i, e);
                }
            }
            ui.table_next_column();
            // The load view divides wire bits by these; there is no hardware
            // behind the simulation, so the values are declarations about the
            // bus being analysed, not device settings. Picks, not typed
            // numbers: a wrong bit rate never complains, it just mis-decodes
            // the wire, so the list is the standard set (plus whatever this
            // project already carries, which stays selectable).
            let (arb_vals, arb_labels) = rate_choices(CAN_ARB_KBPS, arb_kbps, "");
            let arb_refs: Vec<&str> = arb_labels.iter().map(|s| s.as_str()).collect();
            let arb_at = arb_vals.iter().position(|v| *v == arb_kbps).unwrap_or(0);
            let mut arb_pick = arb_at;
            ui.set_next_item_width(76.0);
            if ui.combo_simple_string(format!("##busarb{i}"), &mut arb_pick, &arb_refs) {
                app.send(crate::bus::BusCommand::SetChannelConfig {
                    ch: i as u8,
                    name: None,
                    dbc_path: None,
                    bitrate_kbps: Some(arb_vals[arb_pick.min(arb_vals.len() - 1)].max(1)),
                    fd_data_kbps: None,
                    node_roles: None,
                });
            }
            if ui.is_item_hovered() {
                ui.tooltip_text("仲裁段比特率 kbit/s（选一个，选中即下发）");
            }
            ui.same_line();
            let (data_vals, data_labels) = rate_choices(CAN_DATA_KBPS, data_kbps, "0（无 FD 段）");
            let data_refs: Vec<&str> = data_labels.iter().map(|s| s.as_str()).collect();
            let data_at = data_vals.iter().position(|v| *v == data_kbps).unwrap_or(0);
            let mut data_pick = data_at;
            ui.set_next_item_width(96.0);
            if ui.combo_simple_string(format!("##busdata{i}"), &mut data_pick, &data_refs) {
                app.send(crate::bus::BusCommand::SetChannelConfig {
                    ch: i as u8,
                    name: None,
                    dbc_path: None,
                    bitrate_kbps: None,
                    fd_data_kbps: Some(data_vals[data_pick.min(data_vals.len() - 1)]),
                    node_roles: None,
                });
            }
            if ui.is_item_hovered() {
                ui.tooltip_text(
                    "CAN FD 数据段比特率 kbit/s。0 = 这条总线没有 FD 数据段（经典 CAN 就选它）。",
                );
            }
            ui.table_next_column();
            // Hardware attachment: one adapter per bus, enumerated from
            // the installed driver on first need.
            // Hardware attachment: one adapter per bus, enumerated from
            // the installed driver on first need.
            let attached = app
                .snap
                .hw
                .iter()
                .find(|h| h.bus as usize == i)
                .map(|h| (h.driver, h.adapter, h.kbps, h.can_tx, h.fd));
            match attached {
                Some((driver, adapter, kbps, can_tx, fd)) => {
                    // 两行布局：上行状态、下行解挂按钮——任何列宽下都完整
                    // 可见可点（单行塞不下时按钮会被单元格裁掉）。
                    ui.text(format!(
                        "[{}] ch{adapter} {kbps}k{}",
                        driver.tag(),
                        if fd { " FD" } else { "" }
                    ));
                    // Simulated 模式下硬件挂着但不上线——列内写明，免得
                    // 用户以为帧上不了线是适配器坏了。
                    if !app.snap.real_bus {
                        ui.same_line();
                        ui.text_colored([1.0, 0.8, 0.4, 1.0], "已下线");
                        if ui.is_item_hovered() {
                            ui.tooltip_text(
                                "总线模式为 Simulated：硬件保留配置但不收不发；顶部切到 Real bus 上线",
                            );
                        }
                    }
                    if ui.button(format!("解挂##hwdet{i}")) {
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
                    let channels = ensure_hw_list(app).clone();
                    match channels {
                        Ok(channels) if !channels.is_empty() => {
                            let labels: Vec<String> = channels
                                .iter()
                                .map(|c| format!("[{}] ch{}: {}", c.driver.tag(), c.index, c.name))
                                .collect();
                            let refs: Vec<&str> = labels.iter().map(|s| s.as_str()).collect();
                            ui.set_next_item_width(110.0);
                            let mut pick = 0usize;
                            if ui.combo_simple_string(format!("##hw{i}"), &mut pick, &refs) {
                                let info = &channels[pick];
                                app.set_hardware_channel(
                                    i as u8,
                                    info.driver,
                                    info.index,
                                    arb_kbps,
                                    Some(data_kbps),
                                );
                            }
                            if ui.is_item_hovered() {
                                ui.tooltip_text(
                                    "挂接适配器：收到的帧进总线，节点可经它发车（[K] Kvaser / [V] Vector）",
                                );
                            }
                        }
                        Ok(_) => ui.text_disabled("无通道"),
                        Err(e) => ui.text_disabled(e),
                    }
                }
            }
            ui.table_next_column();
            if ui.button(format!("x##busrm{i}")) {
                remove = Some(i);
            }
        }
    }
    if let Some(i) = remove {
        app.remove_channel(i);
    }

    // FlexRay buses: the same table shape as the CAN one above -- one row per
    // 路, the same columns in the same order, because the questions are the
    // same: which bus is this, which description describes it, what timing does
    // it declare, which port feeds it. Two differences, and both come from the
    // protocol rather than from here: the timing is *read* out of the
    // description instead of typed (a FlexRay cluster's bit time is its
    // schedule, so changing it means editing the description), and a watch only
    // receives -- there is no transmit path yet. The schedule itself is not
    // listed: reading a slot/cycle table is a FIBEX/ARXML editor's job, and a
    // copy of it here would be a worse, staler one.
    ui.separator();
    let watches = app.snap.fr_watches.clone();
    // A channel already feeding a watch is not offered again: the second open
    // would fail in the driver, and two watches on one port are never what
    // "another cluster" means.
    let listed = ensure_fr_list(app);
    let free: Vec<_> = match &listed {
        Ok(list) => list
            .iter()
            .filter(|c| !watches.iter().any(|w| w.channel_index == c.index))
            .collect(),
        Err(_) => Vec::new(),
    };
    // The same header line CAN has: add a bus, and how many there are. A
    // FlexRay 路 *is* its cluster description, so "add" opens that picker and
    // the row appears with the file on it -- which is also why there is no
    // placeholder row for a bus nobody has added: cancelling the picker adds
    // nothing, exactly as cancelling CAN's DBC dialog leaves the bus empty
    // rather than inventing one.
    let rows = app.fr_bus_rows();
    if ui.button("+ Add FlexRay") {
        app.add_flexray_bus();
    }
    ui.same_line();
    ui.text(format!("{} 路", rows.len()));
    let mut detach = None;
    let mut forget = None;
    let mut attach: Option<(u8, i32)> = None;
    let mut load_for: Option<u8> = None;
    {
        let fr_opts = dear_imgui_rs::TableOptions::from(TableFlags::RESIZABLE)
            .sizing_policy(dear_imgui_rs::TableSizingPolicy::StretchProp);
        let Some(_t) = ui.begin_table_with_flags("fr_table", 5, fr_opts) else {
            return;
        };
        ui.table_setup_column_stretch_weight("Name", TableColumnFlags::NONE, 1.0);
        ui.table_setup_column_stretch_weight("FIBEX/ARXML", TableColumnFlags::NONE, 1.6);
        // Same widths as the CAN table, so the two line up column for column --
        // 190 because CAN's cell holds two combos side by side.
        ui.table_setup_column_fixed_width("kbit/s · 周期 ms", TableColumnFlags::NONE, 190.0);
        ui.table_setup_column_fixed_width("硬件", TableColumnFlags::NONE, 140.0);
        ui.table_setup_column_fixed_width("", TableColumnFlags::NONE, 26.0);
        ui.table_headers_row();
        for bus in rows {
            // Everything the row shows is read into owned values first: the
            // widgets below need `app` to themselves.
            let watch = watches.iter().find(|w| w.bus == bus).cloned();
            let path = app
                .fr_buses
                .get(&bus)
                .map(|c| c.path.clone())
                .unwrap_or_default();
            let timing = app
                .fr_db(bus)
                .map(|db| (db.params.speed_kbps, db.params.cycle_time_ms));
            let tag = fr_cluster_tag(app, bus);
            ui.table_next_row();
            if !ui.table_next_column() {
                continue;
            }
            ui.text(format!("FR{bus}"));
            if path.is_empty() && watch.is_none() {
                // Not a bus the user added: the log being replayed carries this
                // cluster's frames, and they stay nameless until a description
                // lands here. Say so -- an unnamed row with "(none)" in it reads
                // as a half-added bus.
                ui.same_line();
                ui.text_disabled("（日志里有这路流量）");
            }
            ui.table_next_column();
            // The description this 路 decodes against, and the one button that
            // changes it -- where CAN puts "Open...".
            ui.text(if path.is_empty() {
                "(none)".to_string()
            } else {
                file_name(&path)
            });
            // Value on the first line, actions on the second. A real file name
            // ("FR_Cluster_DP_30t_1209.xml") already fills the cell, and
            // anything after it is clipped by the column edge -- visible-but-
            // unreachable is the failure the CAN 硬件 column two-line layout
            // exists to avoid, so the button does not ride on the name's line.
            let load_label = if path.is_empty() { "加载…" } else { "换…" };
            if ui.button(format!("{load_label}##frload{bus}")) {
                load_for = Some(bus);
            }
            if ui.is_item_hovered() {
                ui.tooltip_text(
                    "挑一份这路 cluster 的 FIBEX/ARXML 描述：帧名、信号解码、占用率都按它来。放错路是静默的错（一路的槽会按另一路的调度去解名），所以按钮长在哪一行就归哪一路，不替你猜。\n只加载描述不开端口——回放两路录下来的日志用的就是这个。",
                );
            }
            if !tag.is_empty() {
                ui.same_line();
                ui.text_disabled(tag.trim_start_matches(" · "));
            }
            ui.table_next_column();
            match timing {
                Some((kbps, cycle)) => {
                    ui.text(format!("{kbps} / {cycle:.2}"));
                    if ui.is_item_hovered() {
                        ui.tooltip_text(
                            "描述声明的速率与宏周期算出的周期时间。不像 CAN 那一列可以在这里改：FlexRay 的位时就是调度表本身，要改得改描述文件。",
                        );
                    }
                }
                None => ui.text_disabled("-"),
            }
            ui.table_next_column();
            match &watch {
                Some(w) => {
                    ui.text(format!("[V] ch{}", w.channel_index));
                    ui.same_line();
                    ui.text_disabled("（只收）");
                    if !app.snap.real_bus {
                        ui.same_line();
                        ui.text_colored([1.0, 0.8, 0.4, 1.0], "已下线");
                        if ui.is_item_hovered() {
                            ui.tooltip_text(
                                "总线模式为 Simulated：监听保留配置但不收帧；顶部切到 Real bus 上线",
                            );
                        }
                    }
                    ui.same_line();
                    if ui.button(format!("断开##frdet{bus}")) {
                        detach = Some(bus);
                    }
                    if ui.is_item_hovered() {
                        ui.tooltip_text("关掉这端的接收并撤下它的配置（描述一并撤；只想撤描述就先断开再用行末的 x）");
                    }
                }
                None => match &listed {
                    Ok(_) if free.is_empty() => ui.text_disabled("无空闲 FlexRay 通道"),
                    Err(e) => ui.text_disabled(e.as_str()),
                    Ok(_) => {
                        // Picking a port from the combo is the attach, exactly
                        // like a CAN row: no second button to press.
                        let labels: Vec<String> = free
                            .iter()
                            .map(|c| format!("[V] ch{}: {}", c.index, c.name))
                            .collect();
                        let refs: Vec<&str> = labels.iter().map(|s| s.as_str()).collect();
                        ui.set_next_item_width(110.0);
                        let mut pick = 0usize;
                        if ui.combo_simple_string(format!("##frch{bus}"), &mut pick, &refs) {
                            let idx = free[pick.min(free.len() - 1)].index;
                            attach = Some((bus, idx));
                        }
                        if ui.is_item_hovered() {
                            ui.tooltip_text(
                                "给这路挂上只收监听（Vector 端口）：需要这行已经有集群描述——通道拿不到集群参数就收不到帧，没描述会先报出来。",
                            );
                        }
                    }
                },
            }
            ui.table_next_column();
            // The row's removal is the description's: a watch has its own
            // "断开" one column to the left. With nothing loaded there is
            // nothing to take away, so the cell stays empty rather than offering
            // an x that would silently do nothing.
            if path.is_empty() {
                ui.text("");
            } else {
                if ui.button(format!("x##frrm{bus}")) {
                    forget = Some(bus);
                }
                if ui.is_item_hovered() {
                    ui.tooltip_text("撤下这行的集群描述（不动别的路）。正在监听时先断开。");
                }
            }
        }
    }
    if let Some(bus) = detach {
        app.detach_fr_watch(bus);
    }
    if let Some(bus) = forget {
        app.forget_cluster_description(bus);
    }
    if let Some(bus) = load_for {
        app.pick_cluster_description(bus);
    }
    if let Some((bus, channel_index)) = attach {
        app.attach_fr_watch_on(bus, channel_index);
    }
}
