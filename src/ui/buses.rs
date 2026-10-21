use crate::app::App;
use dear_imgui_rs::{Condition, TableColumnFlags, TableFlags, Ui};

/// The Buses window's own state: open flag, the two rename drafts, and the
/// cached hardware enumerations. Everything that changes a bus or a port is
/// returned as a [`BusesCmd`].
#[derive(Default)]
pub struct BusesView {
    pub open: bool,
    pub bus_name_edit: Option<(usize, String)>,
    pub fr_name_edit: Option<(u8, String)>,
    pub hw_channels: Option<Result<Vec<crate::hw::AnyChannelInfo>, String>>,
    pub fr_channels: Option<Result<Vec<crate::hw::vector::ChannelInfo>, String>>,
}

pub enum BusesCmd {
    AddChannel,
    RemoveChannel(usize),
    SetChannelConfig {
        ch: u8,
        name: Option<String>,
        bitrate_kbps: Option<u32>,
        fd_data_kbps: Option<u32>,
    },
    PickDbc(usize),
    AttachDbc(usize),
    DetachDbcExtra {
        bus: usize,
        extra: usize,
    },
    DetachHardware(u8),
    AttachHardware {
        bus: u8,
        driver: crate::hw::HwDriver,
        index: i32,
        arb_kbps: u32,
        data_kbps: u32,
    },
    AddFrBus,
    RemoveFrBus(u8),
    SetFrName {
        bus: u8,
        name: String,
    },
    DetachFrWatch(u8),
    PickClusterDescription(u8),
    AttachFrWatch {
        bus: u8,
        channel_index: i32,
    },
    SetReplayMap(crate::replay_map::ReplayMap),
}

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
    let mut view = std::mem::take(&mut app.buses_view);
    let cmds = view.show(ui, app);
    app.buses_view = view;
    app.apply_buses_cmds(cmds);
}

impl BusesView {
    pub fn show(&mut self, ui: &Ui, app: &App) -> Vec<BusesCmd> {
        let mut cmds = Vec::new();
        if !self.open {
            return cmds;
        }
        let io = ui.io();
        let mut open = self.open;
        ui.window("Buses")
            .opened(&mut open)
            .position(
                [io.display_size()[0] * 0.38, io.display_size()[1] * 0.25],
                Condition::FirstUseEver,
            )
            .size([480.0, 240.0], Condition::FirstUseEver)
            .build(|| content(self, app, ui, &mut cmds));
        self.open = open;
        cmds
    }

    /// Enumerates the hardware channels of every supported driver once per
    /// session; later calls reuse the cached answer (including "driver
    /// unavailable").
    fn ensure_hw_list(&mut self) -> Result<Vec<crate::hw::AnyChannelInfo>, String> {
        self.hw_channels.clone().unwrap_or_else(|| {
            let fresh = crate::hw::enumerate_all();
            self.hw_channels = Some(fresh.clone());
            fresh
        })
    }

    /// Enumerates the Vector channels once per session, on the same
    /// cache-or-probe pattern as the CAN list. Portal devices (VN7640...)
    /// often do not report the FlexRay capability bit at all, so the FR
    /// section offers every channel -- the attach attempt is the real test,
    /// and its failure lands in the status line.
    fn ensure_fr_list(&mut self) -> Result<Vec<crate::hw::vector::ChannelInfo>, String> {
        self.fr_channels.clone().unwrap_or_else(|| {
            let fresh = crate::hw::vector::enumerate();
            self.fr_channels = Some(fresh.clone());
            fresh
        })
    }
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
/// user checks the right file landed on the right 路. The name stands alone
/// (no "cluster:" label): the cell is already under the FIBEX/ARXML header, on
/// the row that file is named in.
fn fr_cluster_tag(app: &App, bus: u8) -> String {
    match app.fr_db(bus) {
        Some(db) if !db.params.name.is_empty() => {
            format!("{}（{} 帧）", db.params.name, db.frames.len())
        }
        _ => String::new(),
    }
}

/// 回放通道映射：日志里的通道编号 → 本工程的哪一路。
///
/// 行来自本次回放真正见过的编号（映射表因此不需要先把文件读第二遍来数通道），
/// 再加上配过但这次没来过的——那是一份存进工程的选择，删掉就等于悄悄改回原样。
/// 每一行改完整张表发回核心：正在回放时下一条命令被抽干就生效，已经进环的帧不动。
fn replay_map_section(app: &App, ui: &Ui, cmds: &mut Vec<BusesCmd>) {
    use crate::replay_map::LogBusKind;

    ui.separator();
    ui.text("回放通道映射");
    if ui.is_item_hovered() {
        ui.tooltip_text("把日志里的通道编号指到本工程的哪一路；改完立即生效，随工程保存");
    }

    let map = app.snap.replay_map.clone();
    let can_rows = merged_rows(
        &app.snap.replay_census_can,
        &map.configured(LogBusKind::Can),
    );
    let fr_rows = merged_rows(&app.snap.replay_census_fr, &map.configured(LogBusKind::Fr));
    if can_rows.is_empty() && fr_rows.is_empty() {
        ui.text_disabled("回放一个日志之后，这里列出日志里有哪几路通道。");
        return;
    }

    // 读进自有值再画，理由与上面两张表一样：下面的下拉框要 `app` 独占。
    let can_names: Vec<String> = app.snap.channels.iter().map(|c| c.name.clone()).collect();
    let mut fr_buses: Vec<u8> = app.fr_buses.keys().copied().collect();
    fr_buses.sort_unstable();
    let fr_names: Vec<String> = fr_buses.iter().map(|bus| app.fr_bus_name(*bus)).collect();
    let mut edited = map.clone();

    // 一行一路。列就三样：日志里的编号、它带来了多少条、回放成哪一路——
    // 第二个数字正是挑错路时的凭据（哪一路的条数像自己熟悉的那条业务）。
    for (kind, rows, targets) in [
        (LogBusKind::Can, &can_rows, &can_names),
        (LogBusKind::Fr, &fr_rows, &fr_names),
    ] {
        let header = match kind {
            LogBusKind::Can => "日志里的 CAN 通道",
            LogBusKind::Fr => "日志里的 FlexRay 通道",
        };
        ui.text_disabled(header);
        let opts = dear_imgui_rs::TableOptions::from(TableFlags::NONE)
            .sizing_policy(dear_imgui_rs::TableSizingPolicy::StretchProp);
        let Some(_t) = ui.begin_table_with_flags(
            match kind {
                LogBusKind::Can => "replay_map_can",
                LogBusKind::Fr => "replay_map_fr",
            },
            3,
            opts,
        ) else {
            continue;
        };
        ui.table_setup_column_fixed_width("编号", TableColumnFlags::NONE, 120.0);
        ui.table_setup_column_fixed_width("条数", TableColumnFlags::NONE, 90.0);
        ui.table_setup_column_stretch_weight("回放成", TableColumnFlags::NONE, 1.0);
        ui.table_headers_row();
        for &(raw, count) in rows {
            ui.table_next_row();
            // 控件的身份是（哪一类总线, 编号）。两张表在同一个窗口里，而表格
            // 不给单元格另加 ID 种子，只按编号 push 会让 CAN 的 0 号与 FlexRay
            // 的 0 号撞成同一个控件。
            let _id = ui.push_id(replay_row_key(kind, raw).as_str());
            if !ui.table_next_column() {
                continue;
            }
            // 编号按文件里印出来的那个数写：CAN 的日志从 1 起（内部存的是
            // 起算后的下标），FlexRay 的 cluster 号与 Buses 表里 `#0` 同一套。
            ui.text(match kind {
                LogBusKind::Can => format!("通道 {}", raw + 1),
                LogBusKind::Fr => format!("#{raw}"),
            });
            ui.table_next_column();
            ui.text(count.to_string());
            if ui.is_item_hovered() {
                ui.tooltip_text("本次回放里这一路来了多少条；选了不回放的就是被挡掉的条数");
            }
            ui.table_next_column();
            // 原样 → 本工程的某一路 → 不回放。第一项是"表里没这一行"，所以
            // 选中它就等于取消这一行，以后加了新的一路还能自动接上。
            let mut labels: Vec<String> = Vec::with_capacity(targets.len() + 2);
            labels.push(match kind {
                LogBusKind::Can => format!("原样（通道 {}）", raw + 1),
                LogBusKind::Fr => format!("原样（#{raw}）"),
            });
            labels.extend(targets.iter().cloned());
            labels.push("不回放".to_string());
            let refs: Vec<&str> = labels.iter().map(|s| s.as_str()).collect();
            // 当前值问的是核心会怎么做，不是表里写了什么：编号在本工程对不上
            // 一路时核心就把它挡在门外，那一行在这里就该显示「不回放」。
            let effective = match kind {
                LogBusKind::Can => edited.can_channel(raw, can_names.len()),
                LogBusKind::Fr => edited.target(LogBusKind::Fr, raw),
            };
            let mut pick = match effective {
                None => labels.len() - 1,
                Some(t) => match kind {
                    LogBusKind::Can => t as usize + 1,
                    LogBusKind::Fr => fr_buses
                        .iter()
                        .position(|b| *b == t)
                        .map(|at| at + 1)
                        .unwrap_or(0),
                },
            };
            pick = pick.min(labels.len() - 1);
            ui.set_next_item_width(-1.0);
            if ui.combo_simple_string("##replaymap", &mut pick, &refs) {
                let to = if pick == 0 {
                    Some(raw)
                } else if pick == labels.len() - 1 {
                    None
                } else {
                    Some(match kind {
                        LogBusKind::Can => (pick - 1) as u8,
                        LogBusKind::Fr => fr_buses[pick - 1],
                    })
                };
                edited.set(kind, raw, to);
            }
            if ui.is_item_hovered() {
                ui.tooltip_text("这一路日志回放成工程的哪一路；不回放就不进 Trace");
            }
        }
    }
    if edited != map {
        cmds.push(BusesCmd::SetReplayMap(edited));
    }
}

/// 一行控件的身份：（哪一类总线, 日志里的编号）。两张表在同一个窗口里，
/// 而表格不给单元格另加 ID 种子，只用编号会让 CAN 的 0 号与 FlexRay 的
/// 0 号落在同一个控件上。
pub(crate) fn replay_row_key(kind: crate::replay_map::LogBusKind, raw: u8) -> String {
    match kind {
        crate::replay_map::LogBusKind::Can => format!("c{raw}"),
        crate::replay_map::LogBusKind::Fr => format!("f{raw}"),
    }
}

/// 见过的编号（带条数）与配过的编号合一行列表，按编号升序。配过但这次没来的
/// 条数印 0：行要在，人才改得回去。
fn merged_rows(counts: &[(u8, u64)], configured: &[(u8, Option<u8>)]) -> Vec<(u8, u64)> {
    let mut out: Vec<(u8, u64)> = counts.to_vec();
    for (raw, _) in configured {
        if !out.iter().any(|(seen, _)| seen == raw) {
            out.push((*raw, 0));
        }
    }
    out.sort_unstable_by_key(|(raw, _)| *raw);
    out
}

fn content(view: &mut BusesView, app: &App, ui: &Ui, cmds: &mut Vec<BusesCmd>) {
    if ui.button("+ Add bus") {
        cmds.push(BusesCmd::AddChannel);
    }
    ui.same_line();
    ui.text(format!("{} bus(es)", app.snap.channel_count));
    ui.same_line();
    // One global re-enumeration for the whole window: the channel list is
    // machine-wide, not per bus.
    if ui.button("刷新通道##hwref") {
        view.hw_channels = None;
        view.fr_channels = None;
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
            let editing = matches!(&view.bus_name_edit, Some((r, _)) if *r == i);
            let mut name_buf = match &view.bus_name_edit {
                Some((r, s)) if *r == i => s.clone(),
                _ => name,
            };
            ui.input_text(format!("##busname{i}"), &mut name_buf)
                .build();
            if ui.is_item_active() {
                view.bus_name_edit = Some((i, name_buf.clone()));
            }
            if ui.is_item_deactivated_after_edit() {
                view.bus_name_edit = None;
                cmds.push(BusesCmd::SetChannelConfig {
                    ch: i as u8,
                    name: Some(name_buf),
                    bitrate_kbps: None,
                    fd_data_kbps: None,
                });
            } else if editing && !ui.is_item_active() {
                view.bus_name_edit = None;
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
                cmds.push(BusesCmd::PickDbc(i));
            }
            if ui.is_item_hovered() {
                ui.tooltip_text("换这条总线的主 DBC 文件（解码与查名以它为先）");
            }
            ui.same_line();
            if ui.button(format!("+##busdbadd{i}")) {
                cmds.push(BusesCmd::AttachDbc(i));
            }
            if ui.is_item_hovered() {
                ui.tooltip_text("附加更多 DBC 文件到该总线");
            }
            // Extra attached databases: one line each with a detach button.
            for (e, extra) in extras.iter().enumerate() {
                ui.text_disabled(file_name(extra));
                ui.same_line();
                if ui.button(format!("x##busdbx{i}_{e}")) {
                    cmds.push(BusesCmd::DetachDbcExtra { bus: i, extra: e });
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
                cmds.push(BusesCmd::SetChannelConfig {
                    ch: i as u8,
                    name: None,
                    bitrate_kbps: Some(arb_vals[arb_pick.min(arb_vals.len() - 1)].max(1)),
                    fd_data_kbps: None,
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
                cmds.push(BusesCmd::SetChannelConfig {
                    ch: i as u8,
                    name: None,
                    bitrate_kbps: None,
                    fd_data_kbps: Some(data_vals[data_pick.min(data_vals.len() - 1)]),
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
            let attached = app.snap.hw.iter().find(|h| h.bus as usize == i).map(|h| {
                (
                    h.driver,
                    h.adapter,
                    h.kbps,
                    h.can_tx,
                    h.fd,
                    h.tx_fail.clone(),
                )
            });
            match attached {
                Some((driver, adapter, kbps, can_tx, fd, tx_fail)) => {
                    // 两行布局：上行状态、下行断开按钮——任何列宽下都完整
                    // 可见可点（单行放不下时按钮会被单元格裁掉）。
                    ui.text(format!(
                        "[{}] ch{adapter} {kbps}k{}",
                        driver.tag(),
                        if fd { " FD" } else { "" }
                    ));
                    // Simulated 模式下硬件已连接但暂不收发——列内写明，免得
                    // 用户以为帧发不出去是适配器坏了。
                    if !app.snap.real_bus {
                        ui.same_line();
                        ui.text_colored([1.0, 0.8, 0.4, 1.0], "未启用");
                        if ui.is_item_hovered() {
                            ui.tooltip_text(
                                "总线模式为 Simulated：硬件配置保留但不收不发；顶部切到 Real bus 即启用",
                            );
                        }
                    }
                    if ui.button(format!("{}##hwdet{i}", crate::channel::DETACH_LABEL)) {
                        cmds.push(BusesCmd::DetachHardware(i as u8));
                    }
                    if ui.is_item_hovered() {
                        // FD 数据段配在总线上但通道没带上 FD：降级原因
                        // 常驻提示（状态行早已被后续事件冲掉）。
                        ui.tooltip_text(if can_tx {
                            if fd {
                                "连接中（收发，FD 数据段参数已应用）"
                            } else if data_kbps > 0 {
                                "连接中（收发）。总线配了 FD 数据段波特率，但通道未带 FD——预设不匹配或硬件不支持，FD 帧无法经该通道发送。"
                            } else {
                                "连接中（收发）"
                            }
                        } else {
                            "连接中（仅接收：通道初始化访问被其他程序占用）"
                        });
                    }
                    // The wire refusing writes is the one failure a bench cannot
                    // see from the frames: they all stay on the internal bus and
                    // every view keeps scrolling. So the row carries the count,
                    // and the reason the driver gave, permanently.
                    if let Some((reason, n)) = &tx_fail {
                        ui.text_colored([1.0, 0.45, 0.35, 1.0], format!("发送被拒 ×{n}"));
                        if ui.is_item_hovered() {
                            ui.tooltip_text(format!("线路拒绝发送 {n} 次，第一次的原因：{reason}"));
                        }
                    }
                }
                None => {
                    let channels = view.ensure_hw_list();
                    match channels {
                        Ok(channels) if !channels.is_empty() => {
                            let labels: Vec<String> = channels
                                .iter()
                                .map(|c| format!("[{}] ch{}: {}", c.driver.tag(), c.index, c.name))
                                .collect();
                            let refs: Vec<&str> = labels.iter().map(|s| s.as_str()).collect();
                            ui.set_next_item_width(110.0);
                            // Show the port this row last chose, not the list's
                            // first entry: after a refused open the cell is the
                            // only place left that can say which port the
                            // status line is talking about.
                            let want = app.hw_pick.get(&(i as u8)).copied();
                            let mut pick = channels
                                .iter()
                                .position(|c| Some((c.driver, c.index)) == want)
                                .unwrap_or(0);
                            if ui.combo_simple_string(format!("##hw{i}"), &mut pick, &refs) {
                                let (driver, index) = (channels[pick].driver, channels[pick].index);
                                cmds.push(BusesCmd::AttachHardware {
                                    bus: i as u8,
                                    driver,
                                    index,
                                    arb_kbps,
                                    data_kbps,
                                });
                            }
                            if ui.is_item_hovered() {
                                ui.tooltip_text(
                                    "连接适配器：收到的帧进入总线，节点可经它发送（[K] Kvaser / [V] Vector）",
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
            if ui.is_item_hovered() {
                ui.tooltip_text(
                    "移除这条总线；后面的总线号依次前移，关联到它的曲线与条目随之删除或重编号。",
                );
            }
        }
    }
    if let Some(i) = remove {
        cmds.push(BusesCmd::RemoveChannel(i));
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
    let listed = view.ensure_fr_list();
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
        cmds.push(BusesCmd::AddFrBus);
    }
    if ui.is_item_hovered() {
        ui.tooltip_text("添加一条 FlexRay 路：先为它选择集群描述（取消则不添加）。");
    }
    ui.same_line();
    ui.text(format!("{} 路", rows.len()));
    let mut detach = None;
    let mut remove_fr = None;
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
            // The name, editable exactly like a CAN row's -- same fill width, so
            // the two tables' cells line up. The cluster index goes on the line
            // under the box: the record filter (`FR0:13`) and a script's
            // `fr_sig(0, ..)` are numbered by that index, so renaming a 路 must
            // not lose the only place on screen that says which number to type.
            let editing = matches!(&view.fr_name_edit, Some((r, _)) if *r == bus);
            let mut name_buf = match &view.fr_name_edit {
                Some((r, s)) if *r == bus => s.clone(),
                _ => app.fr_bus_name(bus),
            };
            ui.set_next_item_width(-1.0);
            ui.input_text(format!("##frname{bus}"), &mut name_buf)
                .build();
            if ui.is_item_active() {
                view.fr_name_edit = Some((bus, name_buf.clone()));
            }
            if ui.is_item_deactivated_after_edit() {
                view.fr_name_edit = None;
                cmds.push(BusesCmd::SetFrName {
                    bus,
                    name: name_buf,
                });
            } else if editing && !ui.is_item_active() {
                view.fr_name_edit = None;
            }
            let mut meta = format!("#{bus}");
            if path.is_empty() && watch.is_none() {
                // Not a bus the user added: the log being replayed carries this
                // cluster's frames, and they stay nameless until a description
                // lands here. Say so -- an unnamed row with "(none)" in it reads
                // as a half-added bus.
                meta.push_str(" · 日志里有这路流量");
            }
            ui.text_disabled(meta);
            ui.table_next_column();
            // The description this 路 decodes against, and the one button that
            // changes it -- where CAN puts "Open...".
            ui.text(if path.is_empty() {
                "(none)".to_string()
            } else {
                file_name(&path)
            });
            // The full path and the declared cluster name, on hover: the cell
            // cannot hold either in full, and a name cut in half is no use for
            // checking the right file landed on the right 路.
            if !path.is_empty() && ui.is_item_hovered() {
                ui.tooltip_text(format!("{path}\n{tag}"));
            }
            // Same verb as the CAN row's button for the same action -- pick the
            // file this row decodes against -- and the same one-line help. The
            // file name owns the first line, the button the second, the
            // declared cluster the third: anything side by side here is clipped
            // by the column edge (the CAN 硬件 column's two-line layout already
            // wrote down that lesson).
            if ui.button(format!("Open...##frload{bus}")) {
                load_for = Some(bus);
            }
            if ui.is_item_hovered() {
                ui.tooltip_text(
                    "换这路的集群描述文件（帧名与信号解码以它为先）；只加载描述，不打开端口。",
                );
            }
            if !tag.is_empty() {
                ui.text_disabled(&tag);
            }
            ui.table_next_column();
            match timing {
                Some((kbps, cycle)) => {
                    ui.text(format!("{kbps} / {cycle:.2}"));
                    if ui.is_item_hovered() {
                        ui.tooltip_text("描述声明的比特率与宏周期；此列只读——要改得改描述文件。");
                    }
                }
                None => ui.text_disabled("-"),
            }
            ui.table_next_column();
            match &watch {
                Some(w) => {
                    // 两行布局，与 CAN 行一致：状态一行、断开按钮一行。单行放不下
                    // 时按钮会被单元格右边缘裁掉（CAN 那列早已写下这条教训）。
                    ui.text(format!("[V] ch{}（仅接收）", w.channel_index));
                    // Simulated 模式下端口已连接但不收帧——列内写明，免得用户以为
                    // 适配器坏了。
                    if !app.snap.real_bus {
                        ui.same_line();
                        ui.text_colored([1.0, 0.8, 0.4, 1.0], "未启用");
                        if ui.is_item_hovered() {
                            ui.tooltip_text(
                                "总线模式为 Simulated：监听配置保留但不收帧；顶部切到 Real bus 即启用",
                            );
                        }
                    }
                    if ui.button(format!("{}##frdet{bus}", crate::channel::DETACH_LABEL)) {
                        detach = Some(bus);
                    }
                    if ui.is_item_hovered() {
                        ui.tooltip_text("关闭这路的接收端口；这路的描述与所有曲线保留。");
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
                        // The CAN row's rule, same reason: a refused open leaves
                        // no watch behind, and the row would otherwise read
                        // `ch0` while the bar names the port that failed.
                        let want = app.fr_pick.get(&bus).copied();
                        let mut pick = free.iter().position(|c| Some(c.index) == want).unwrap_or(0);
                        if ui.combo_simple_string(format!("##frch{bus}"), &mut pick, &refs) {
                            let idx = free[pick.min(free.len() - 1)].index;
                            attach = Some((bus, idx));
                        }
                        if ui.is_item_hovered() {
                            ui.tooltip_text(
                                "以仅接收方式连接（Vector 端口），选中即连接；这行需要先有集群描述。",
                            );
                        }
                    }
                },
            }
            ui.table_next_column();
            // The row's removal, the same `x` as a CAN row: the 路 goes -- port,
            // cluster description, and everything pointed at its slots. The
            // other 路 keep their numbers, because a cluster number is not a
            // position in a list. A row that only the log's traffic names has
            // nothing here to take away, so its cell stays empty rather than
            // offering an x that would do nothing.
            if path.is_empty() && watch.is_none() {
                ui.text("");
            } else {
                if ui.button(format!("x##frrm{bus}")) {
                    remove_fr = Some(bus);
                }
                if ui.is_item_hovered() {
                    ui.tooltip_text(
                        "删除这路 FlexRay：关闭端口、移除集群描述与引用它的所有曲线、发送条目和规则；其他各路编号不变。",
                    );
                }
            }
        }
    }
    if let Some(bus) = detach {
        cmds.push(BusesCmd::DetachFrWatch(bus));
    }
    if let Some(bus) = remove_fr {
        cmds.push(BusesCmd::RemoveFrBus(bus));
    }
    if let Some(bus) = load_for {
        cmds.push(BusesCmd::PickClusterDescription(bus));
    }
    if let Some((bus, channel_index)) = attach {
        cmds.push(BusesCmd::AttachFrWatch { bus, channel_index });
    }
    replay_map_section(app, ui, cmds);
}

impl App {
    pub(crate) fn apply_buses_cmds(&mut self, cmds: Vec<BusesCmd>) {
        for cmd in cmds {
            match cmd {
                BusesCmd::AddChannel => self.add_channel(),
                BusesCmd::RemoveChannel(i) => self.remove_channel(i),
                BusesCmd::SetChannelConfig {
                    ch,
                    name,
                    bitrate_kbps,
                    fd_data_kbps,
                } => {
                    self.send(crate::bus::BusCommand::SetChannelConfig {
                        ch,
                        name,
                        dbc_path: None,
                        bitrate_kbps,
                        fd_data_kbps,
                        node_roles: None,
                    });
                }
                BusesCmd::PickDbc(ch) => self.pick_dbc_for(ch),
                BusesCmd::AttachDbc(ch) => self.attach_dbc_dialog(ch),
                BusesCmd::DetachDbcExtra { bus, extra } => self.detach_dbc_extra(bus, extra),
                BusesCmd::DetachHardware(bus) => self.detach_hardware(bus),
                BusesCmd::AttachHardware {
                    bus,
                    driver,
                    index,
                    arb_kbps,
                    data_kbps,
                } => {
                    self.hw_pick.insert(bus, (driver, index));
                    self.set_hardware_channel(bus, driver, index, arb_kbps, Some(data_kbps));
                }
                BusesCmd::AddFrBus => self.add_flexray_bus(),
                BusesCmd::RemoveFrBus(bus) => self.remove_fr_bus(bus),
                BusesCmd::SetFrName { bus, name } => self.set_flexray_name(bus, &name),
                BusesCmd::DetachFrWatch(bus) => self.detach_flexray_watch(bus),
                BusesCmd::PickClusterDescription(bus) => self.pick_cluster_description(bus),
                BusesCmd::AttachFrWatch { bus, channel_index } => {
                    self.fr_pick.insert(bus, channel_index);
                    self.attach_fr_watch_on(bus, channel_index);
                }
                BusesCmd::SetReplayMap(map) => self.set_replay_map(map),
            }
        }
    }
}
