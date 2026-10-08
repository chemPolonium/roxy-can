use crate::app::App;
use dear_imgui_rs::{Condition, Ui};

struct NodeInfo {
    name: String,
    tx: Vec<(u32, String)>,
    rx: Vec<((u32, bool), String, String)>,
}

/// Per channel: the DBC node infos of that bus.
fn collect(app: &App) -> Vec<Vec<NodeInfo>> {
    let mut dbc_nodes = Vec::new();
    for channel in app.snap.channels.iter() {
        let infos: Vec<NodeInfo> = channel
            .dbc
            .as_ref()
            .map(|db| {
                db.nodes
                    .iter()
                    .map(|n| {
                        let tx = db
                            .node_tx_ids(n)
                            .into_iter()
                            .map(|id| {
                                let name = db.message_name(id).unwrap_or("-").to_string();
                                (id, name)
                            })
                            .collect();
                        let rx = db.node_rx_signals(n);
                        NodeInfo {
                            name: n.clone(),
                            tx,
                            rx,
                        }
                    })
                    .collect()
            })
            .unwrap_or_default();
        dbc_nodes.push(infos);
    }
    dbc_nodes
}

/// One bus's tree section: the bus as the root, its DBC nodes as leaves.
/// The role tag colours the leaf ([S] simulated / [M] monitor / [-]
/// absent -- plain ASCII, the merged font has no geometric glyphs), a
/// green `*` tail marks "seen transmitting this run", and clicking a
/// leaf opens the node's detail below.
fn draw_tree_section(app: &mut App, ui: &Ui, ch: usize, infos: &[NodeInfo], flat_base: usize) {
    // The root's ID is the channel index, not its name: two buses may be
    // renamed alike, and two visible tree nodes with one label would share one
    // ID.
    let _id = ui.push_id(ch as i32);
    let token = ui
        .tree_node_config(app.channel_name(ch as u8))
        .default_open(true)
        .push();
    if let Some(_t) = token {
        if infos.is_empty() {
            ui.text_disabled("no DBC loaded on this bus");
            return;
        }
        for (i, ni) in infos.iter().enumerate() {
            let role = app.node_role(ch as u8, &ni.name);
            let (tag, tag_color) = match role {
                crate::app::NodeRole::Simulated => ("[S]", [0.95, 0.70, 0.20, 1.0]),
                crate::app::NodeRole::Absent => ("[-]", [0.45, 0.45, 0.55, 1.0]),
            };
            ui.text_colored(tag_color, tag);
            if ui.is_item_hovered() {
                ui.tooltip_text(role.hint());
            }
            ui.same_line();
            if ui
                .selectable_config(format!("{}##net{ch}_{i}", ni.name))
                .selected(app.net_fr_sel.is_none() && app.net_selected == flat_base + i)
                .build()
            {
                app.net_selected = flat_base + i;
                // One selection at a time across the two trees: a CAN node and a
                // FlexRay ECU cannot both own the detail pane.
                app.net_fr_sel = None;
            }
            // 绑定到该节点的脚本作为下一层树叶列在节点下：点击打开
            // 该脚本的编辑器。
            let bound: Vec<(u64, String, bool, bool, bool)> = app
                .snap
                .nodes
                .iter()
                .filter(|n| {
                    n.channel as usize == ch && n.attached.as_ref().is_some_and(|a| a.1 == ni.name)
                })
                .map(|n| {
                    (
                        n.id,
                        n.name.clone(),
                        n.running && !n.errored,
                        n.enabled,
                        n.errored,
                    )
                })
                .collect();
            if !bound.is_empty() {
                ui.indent();
                for (nid, name, running, enabled, errored) in bound {
                    draw_script_leaf(app, ui, nid, &name, running, enabled, errored);
                }
                ui.unindent();
            }
            // 绑定到该节点的回放块同样列在节点下：点击打开 Replay
            // Blocks 窗口编辑。
            let blocks: Vec<(u64, String, bool)> = app
                .snap
                .blocks
                .iter()
                .filter(|b| {
                    b.attached
                        .as_ref()
                        .is_some_and(|a| a.0 == ch as u8 && a.1 == ni.name)
                })
                .map(|b| (b.id, b.name.clone(), b.enabled))
                .collect();
            if !blocks.is_empty() {
                ui.indent();
                for (bid, name, enabled) in blocks {
                    let (marker, color) = if enabled {
                        ("[o]", [0.45, 0.62, 0.80, 1.0])
                    } else {
                        ("[-]", [0.5, 0.5, 0.55, 1.0])
                    };
                    ui.text_colored(color, marker);
                    if ui.is_item_hovered() {
                        ui.tooltip_text("回放块 / [o] 启用 / [-] 停用");
                    }
                    ui.same_line();
                    if ui
                        .selectable_config(format!("{name}##netblock{bid}"))
                        .build()
                    {
                        // 块的编辑就在所属节点的详情里：点击树叶选中节点。
                        app.net_selected = flat_base + i;
                    }
                }
                ui.unindent();
            }
        }
        // 自由脚本（未绑定 DBC 节点）列在总线根下，旧工程仍可见。
        let free: Vec<(u64, String, bool, bool, bool)> = app
            .snap
            .nodes
            .iter()
            .filter(|n| n.channel as usize == ch && n.attached.is_none())
            .map(|n| {
                (
                    n.id,
                    n.name.clone(),
                    n.running && !n.errored,
                    n.enabled,
                    n.errored,
                )
            })
            .collect();
        if !free.is_empty() {
            ui.indent();
            for (nid, name, running, enabled, errored) in free {
                draw_script_leaf(app, ui, nid, &name, running, enabled, errored);
            }
            ui.unindent();
        }
    }
}

/// The FlexRay clusters the topology lists: every bus with a description
/// loaded, plus any that carried traffic without one. A watched bus with no
/// description has no schedule to show, but leaving it out entirely would make
/// a live cable invisible in the one view that answers "what is on this bus".
fn fr_buses(app: &App) -> Vec<u8> {
    let mut buses: Vec<u8> = app.fr_buses.keys().copied().collect();
    for bus in app.snap.fr_loads.keys().copied() {
        if !buses.contains(&bus) {
            buses.push(bus);
        }
    }
    buses.sort_unstable();
    buses
}

/// One FlexRay cluster as a tree section: which cluster this is, and the ECUs
/// its description declares. Nothing else -- the schedule (which frame holds
/// which slot in which cycle phase, with what repetition) is a different tool's
/// job, the same split as CANoe against a FIBEX/ARXML editor: this view answers
/// "who is on this bus", and a copy of the schedule here would be a worse,
/// staler duplicate of the document. What has actually arrived is in
/// Trace/Messages, the declared timing behind the occupancy figure in Bus
/// Statistics.
///
/// An ECU leaf is selectable, and its detail is the generator panel for the
/// slots this ECU is the declared sender of -- the same node-centred editing a
/// CAN node gets. There is no role switch: a role is the DBC node's gate, and
/// the description says who sends, not who is simulated.
fn draw_flexray_section(app: &mut App, ui: &Ui, bus: u8) {
    let label = match app.fr_db(bus) {
        Some(db) => format!("{} · {}", app.fr_bus_name(bus), db.params.name),
        None => format!("{}（未加载描述）", app.fr_bus_name(bus)),
    };
    // The root's ID is the cluster index: two 路 can be renamed alike, and two
    // visible tree nodes with one label would share one ID.
    let _id = ui.push_id(format!("fr{bus}").as_str());
    let Some(_t) = ui.tree_node_config(label).default_open(true).push() else {
        return;
    };
    // The rows are the ECUs the description declares plus any name an existing
    // entry carries: swap the description out from under a stimulus setup and
    // its entries must still have a row to be reached through, rather than
    // vanish into a list the document no longer supports.
    let mut nodes: Vec<String> = match app.fr_db(bus) {
        Some(db) => db.ecus.clone(),
        None => Vec::new(),
    };
    for t in app.snap.fr_tx.iter().filter(|t| t.bus == bus) {
        if !nodes.contains(&t.node) {
            nodes.push(t.node.clone());
        }
    }
    let Some(db) = app.fr_db(bus) else {
        let seen: u64 = app
            .snap
            .fr_loads
            .get(&bus)
            .map_or(0, |l| l.frames);
        ui.text_disabled(if seen > 0 {
            "该路正在接收帧，但未加载集群描述，ECU 与帧归属无法显示"
        } else {
            "该路已配置，尚无帧到达，且未加载集群描述"
        });
        if !nodes.is_empty() {
            draw_ecu_leaves(app, ui, bus, &nodes, &[]);
        }
        return;
    };
    if nodes.is_empty() {
        // An empty ECU list is a property of the document, not of the session:
        // a cluster export carries no ECUs at all (both bundled FIBEX files are
        // one), and saying so beats a group that looks like it failed to load.
        ui.text_disabled("描述未声明 ECU（该文件是集群参数导出，不含 ECUs 一节）");
        return;
    }
    // How many frames each ECU is the declared sender of, so a leaf says what
    // its generator panel will be able to offer.
    let mut tx_counts: std::collections::HashMap<&str, usize> = std::collections::HashMap::new();
    for ix in 0..db.frames.len() {
        if let Some(s) = db.frame_sender(ix) {
            *tx_counts.entry(s).or_insert(0) += 1;
        }
    }
    let counts: Vec<usize> = nodes
        .iter()
        .map(|n| tx_counts.get(n.as_str()).copied().unwrap_or(0))
        .collect();
    if let Some(_e) = ui
        .tree_node_config(format!("ECU ({})##frecu{bus}", nodes.len()))
        .default_open(true)
        .push()
    {
        draw_ecu_leaves(app, ui, bus, &nodes, &counts);
    }
}

/// The ECU leaves of one cluster. `counts` is the declared frame count per leaf;
/// an empty slice means the description is gone and nothing can be claimed, so
/// no count is printed -- which is not the same statement as "sends none".
fn draw_ecu_leaves(app: &mut App, ui: &Ui, bus: u8, nodes: &[String], counts: &[usize]) {
    for (i, ecu) in nodes.iter().enumerate() {
        let text = match counts.get(i) {
            Some(0) | None => ecu.clone(),
            Some(n) => format!("{ecu} · 发 {n} 帧"),
        };
        if ui
            .selectable_config(format!("{text}##freculeaf{bus}_{i}"))
            .selected(app.net_fr_sel.as_ref().is_some_and(|(b, e)| *b == bus && e == ecu))
            .build()
        {
            app.net_fr_sel = Some((bus, ecu.clone()));
        }
    }
}

/// One script node as a tree leaf: state marker + name, click opens the
/// script editor.
fn draw_script_leaf(
    app: &mut App,
    ui: &Ui,
    id: u64,
    name: &str,
    running: bool,
    enabled: bool,
    errored: bool,
) {
    let (marker, color) = if errored {
        ("[!]", [1.0, 0.55, 0.3, 1.0])
    } else if running {
        ("[*]", [0.45, 0.95, 0.45, 1.0])
    } else if enabled {
        ("[o]", [0.45, 0.62, 0.80, 1.0])
    } else {
        ("[-]", [0.5, 0.5, 0.55, 1.0])
    };
    ui.text_colored(color, marker);
    if ui.is_item_hovered() {
        ui.tooltip_text("[*] 运行中 / [o] 已启用待测量 / [-] 未启用 / [!] 出错（编辑器里看日志）");
    }
    ui.same_line();
    if ui
        .selectable_config(format!("{name}##netscript{id}"))
        .build()
    {
        app.open_script_editor(id);
    }
}

pub fn render(app: &mut App, ui: &Ui) {
    let io = ui.io();
    let mut open = app.show_network;
    if open {
        ui.window("Network")
            .opened(&mut open)
            .position(
                [
                    io.display_size()[0] * 0.3,
                    io.display_size()[1] * 0.55,
                ],
                Condition::FirstUseEver,
            )
            .size([860.0, 540.0], Condition::FirstUseEver)
            .build(|| {
                // Profile row: enumerate the project's profiles/*.toml and
                // apply the selected one (all-or-nothing role overrides),
                // plus create-from-snapshot, open-in-editor and delete.
                let profile_dir = app
                    .project_path
                    .as_ref()
                    .and_then(|p| p.parent().map(|p| p.to_path_buf()))
                    .unwrap_or_default();
                if app.profile_names.is_none() {
                    app.profile_names = Some(Ok(crate::profile::list_profiles(&profile_dir)));
                }
                let Some(profile_list) =
                    app.profile_names.as_ref().and_then(|r| r.as_ref().ok()).cloned()
                else {
                    return;
                };
                ui.text_disabled("Profile");
                if !profile_list.is_empty() {
                    ui.same_line();
                    let mut pick = app.profile_pick.min(profile_list.len() - 1);
                    ui.set_next_item_width(140.0);
                    if ui.combo_simple_string("##prof", &mut pick, &profile_list) {
                        app.profile_pick = pick;
                    }
                    ui.same_line();
                    if ui.button("应用##profapply") {
                        let name =
                            profile_list[app.profile_pick.min(profile_list.len() - 1)].clone();
                        match crate::profile::apply_profile(app, &profile_dir, &name) {
                            Ok(s) => app.status = s,
                            Err(e) => app.status = e,
                        }
                    }
                    if ui.is_item_hovered() {
                        ui.tooltip_text("按 profiles/*.toml 覆盖角色（all-or-nothing）");
                    }
                    ui.same_line();
                    if ui.button("编辑##profedit") {
                        let name =
                            profile_list[app.profile_pick.min(profile_list.len() - 1)].clone();
                        let path = profile_dir.join("profiles").join(format!("{name}.toml"));
                        let opened = std::process::Command::new("cmd")
                            .args(["/C", "start", ""])
                            .arg(&path)
                            .spawn();
                        app.status = match opened {
                            Ok(_) => format!("已在系统编辑器打开 {name}.toml"),
                            Err(e) => format!("打开编辑器失败: {e}"),
                        };
                    }
                    if ui.is_item_hovered() {
                        ui.tooltip_text("用系统默认编辑器打开这个 .toml");
                    }
                    ui.same_line();
                    let name =
                        profile_list[app.profile_pick.min(profile_list.len() - 1)].clone();
                    if ui.button(if app.profile_delete_arm {
                        "确认删除##profdel"
                    } else {
                        "删除##profdel"
                    }) {
                        if app.profile_delete_arm {
                            match crate::profile::delete_profile(&profile_dir, &name) {
                                Ok(()) => {
                                    app.status = format!("profile `{name}` 已删除");
                                    app.profile_names =
                                        Some(Ok(crate::profile::list_profiles(&profile_dir)));
                                }
                                Err(e) => app.status = e,
                            }
                            app.profile_delete_arm = false;
                        } else {
                            app.profile_delete_arm = true;
                            app.status = format!("再点一次「删除」确认删掉 `{name}.toml`");
                        }
                    }
                    if ui.is_item_hovered() {
                        ui.tooltip_text("删除选中的 profile 文件（点两次确认）");
                    }
                }
                ui.same_line();
                ui.set_next_item_width(120.0);
                ui.input_text("##profnew", &mut app.profile_draft_name)
                    .hint("新名字")
                    .build();
                ui.same_line();
                if ui.button("存当前##profsave") {
                    match crate::profile::save_profile(app, &profile_dir, &app.profile_draft_name)
                    {
                        Ok(s) => {
                            app.status = s;
                            app.profile_draft_name.clear();
                            app.profile_names =
                                Some(Ok(crate::profile::list_profiles(&profile_dir)));
                        }
                        Err(e) => app.status = e,
                    }
                }
                if ui.is_item_hovered() {
                    ui.tooltip_text("把当前角色与硬件连接快照存为新 profile");
                }
                let dbc_nodes = collect(app);
                let total_dbc: usize = dbc_nodes.iter().map(|v| v.len()).sum();
                if app.net_selected >= total_dbc.max(1) {
                    app.net_selected = 0;
                }

                // 左右分栏：左边树形拓扑，右边所选节点的详情。两栏各自
                // 滚动——树的长度与详情的长度互不挤占。宽度按树里最长的
                // 一行取（DBC 节点名与 FlexRay 的 cluster 标题同一量级）。
                const TREE_W: f32 = 320.0;
                ui.child_window("net_tree")
                    .size([TREE_W, 0.0])
                    .border(true)
                    .build(ui, || {
                        let mut flat_base = 0usize;
                        for (ch, infos) in dbc_nodes.iter().enumerate() {
                            draw_tree_section(app, ui, ch, infos, flat_base);
                            flat_base += infos.len();
                        }
                        // The FlexRay clusters after the CAN channels: same
                        // topology view, different numbering space on purpose --
                        // `FR{n}` is a cluster index, not a CAN channel.
                        for bus in fr_buses(app) {
                            draw_flexray_section(app, ui, bus);
                        }
                    });
                ui.same_line();

                // Details scroll inside their own panel (which always fills the
                // remaining space), so long content never adds a scrollbar to the
                // outer window and shifts the topology sections.
                ui.child_window("node_details").size([0.0, 0.0]).build(ui, || {
                    if let Some((bus, ecu)) = app.net_fr_sel.clone() {
                        ui.text_colored(
                            [0.30, 0.80, 1.00, 1.0],
                            format!("{} / {}  —  FlexRay 发送方", app.fr_bus_label(bus), ecu),
                        );
                        ui.separator();
                        crate::ui::tx::render_fr_ecu_generator(app, ui, bus, &ecu);
                        return;
                    }
                    if total_dbc == 0 {
                        // The FlexRay side has no selection model yet (no roles,
                        // no generator: the bus cannot transmit), so an empty CAN
                        // tree with clusters listed beside it is not "nothing to
                        // show" -- say where to look instead.
                        if app.fr_buses.is_empty() && app.snap.fr_loads.is_empty() {
                            ui.text("no DBC nodes to display");
                        } else {
                            ui.text("没有 DBC 节点；点左侧 FlexRay 一节里的 ECU，这里就是它的生成器");
                        }
                        return;
                    }
                    // Locate the selected DBC node (channel, index).
                    let mut remaining = app.net_selected;
                    let mut selected: Option<(usize, usize)> = None;
                    for (ch, infos) in dbc_nodes.iter().enumerate() {
                        if remaining < infos.len() {
                            selected = Some((ch, remaining));
                            break;
                        }
                        remaining -= infos.len();
                    }
                    let Some((ch, idx)) = selected else {
                        ui.text("select a node to see its messages and signals");
                        return;
                    };
                    let ni = &dbc_nodes[ch][idx];
                    ui.text_colored(
                        [0.30, 0.80, 1.00, 1.0],
                        format!(
                            "{} / {}  —  sends {} message(s), receives {} signal(s)",
                            app.channel_name(ch as u8),
                            ni.name,
                            ni.tx.len(),
                            ni.rx.len()
                        ),
                    );
                    let mut role_idx = crate::app::NodeRole::ALL
                        .iter()
                        .position(|r| *r == app.node_role(ch as u8, &ni.name))
                        .unwrap_or(0);
                    ui.set_next_item_width(88.0);
                    let role_labels: Vec<&str> = crate::app::NodeRole::ALL
                        .iter()
                        .map(|r| r.label())
                        .collect();
                    if ui.combo_simple_string(format!("##netrole{ch}"), &mut role_idx, &role_labels)
                    {
                        let role = crate::app::NodeRole::ALL[role_idx];
                        app.set_node_role(ch as u8, &ni.name, role);
                    }
                    ui.separator();

                    // 节点生成器：这个节点的条目、添加与响应规则，
                    // 紧接在角色声明之下——节点就是编辑单元。
                    crate::ui::tx::render_node_generator(app, ui, ch as u8, &ni.name);
                    ui.separator();

                    // 脚本编辑入口在节点之下（而非总线）：新建的脚本
                    // 自动绑定到当前选中的 DBC 节点，发帧受其角色开关。
                    // 两个新建按钮统一排在标签同一行。
                    ui.text("本节点脚本");
                    ui.same_line();
                    if ui.button(format!("+ 脚本节点##netadd{ch}")) {
                        let name = format!("Node {}", app.snap.nodes.len() + 1);
                        let attached = Some((ch as u8, ni.name.clone()));
                        app.send(crate::bus::BusCommand::AddNode {
                            name,
                            channel: ch as u8,
                            attached,
                        });
                        app.settle();
                        if let Some(newest) = app.snap.nodes.iter().map(|n| n.id).max() {
                            app.open_script_editor(newest);
                        }
                    }
                    ui.same_line();
                    ui.text_disabled("点名字打开脚本编辑器");
                    let node_scripts: Vec<(u64, String, bool, bool)> = app
                        .snap
                        .nodes
                        .iter()
                        .filter(|n| {
                            n.channel as usize == ch
                                && n.attached.as_ref().is_some_and(|a| a.1 == ni.name)
                        })
                        .map(|n| (n.id, n.name.clone(), n.running && !n.errored, n.enabled))
                        .collect();
                    for (nid, name, running, enabled) in node_scripts {
                        let dot = if running {
                            "[*]"
                        } else if enabled {
                            "[o]"
                        } else {
                            "[-]"
                        };
                        if ui
                            .selectable_config(format!("{dot} {name}##netscript{nid}"))
                            .build()
                        {
                            app.open_script_editor(nid);
                        }
                        if ui.is_item_hovered() {
                            ui.tooltip_text("[*] 运行中 / [o] 已启用待测量 / [-] 未启用");
                        }
                    }
                    ui.separator();

                    // 回放块也是节点的一种驱动：该节点名下的块就近列出，
                    // 新建即绑定到本节点并默认只回放它的报文。
                    ui.text("本节点回放块");
                    ui.same_line();
                    if ui.button(format!("+ 回放块##netaddblk{ch}")) {
                        let name = format!("Block {}", app.snap.blocks.len() + 1);
                        app.add_replay_block(
                            ch as u8,
                            name,
                            String::new(),
                            Some(ni.name.clone()),
                            Some((ch as u8, ni.name.clone())),
                        );
                    }
                    if ui.is_item_hovered() {
                        ui.tooltip_text(
                            "把该节点录制的真实流量重新发到仿真总线上（restbus）；新建后在下方选择日志文件，启用即发送",
                        );
                    }
                    let node_blocks: Vec<(u64, String, bool, usize, Option<String>)> = app
                        .snap
                        .blocks
                        .iter()
                        .filter(|b| {
                            b.attached.as_ref().is_some_and(|a| a.0 == ch as u8 && a.1 == ni.name)
                        })
                        .map(|b| {
                            (
                                b.id,
                                b.name.clone(),
                                b.enabled,
                                b.frames,
                                b.last_error.clone(),
                            )
                        })
                        .collect();
                    for (bid, name, enabled, frames, last_error) in node_blocks {
                        // 启用开关即行首 checkbox；x 删除该块。
                        let mut on = enabled;
                        if ui.checkbox(format!("##blkon{bid}"), &mut on) {
                            app.set_replay_block_enabled(bid, on);
                        }
                        if ui.is_item_hovered() {
                            ui.tooltip_text("启用后测量中按录制间距发送（仅仿真模式）");
                        }
                        ui.same_line();
                        ui.text(&name);
                        ui.same_line();
                        ui.text_disabled(format!("{frames} 帧"));
                        if let Some(e) = &last_error {
                            ui.same_line();
                            ui.text_colored([1.0, 0.55, 0.3, 1.0], format!("载入失败：{e}"));
                        }
                        ui.same_line();
                        if ui.button(format!("x##netblockrm{bid}")) {
                            app.remove_replay_block(bid);
                            app.block_drafts.remove(&bid);
                            continue;
                        }
                        // 日志行（缩进）：路径草稿 + 文件选择，Apply 一并提交。
                        ui.indent();
                        let mut draft = app
                            .block_drafts
                            .entry(bid)
                            .or_insert_with(|| crate::ui::BlockDraft {
                                path: app
                                    .snap
                                    .blocks
                                    .iter()
                                    .find(|b| b.id == bid)
                                    .map(|b| b.path.clone())
                                    .unwrap_or_default(),
                                ids_text: String::new(),
                            })
                            .clone();
                        ui.set_next_item_width(-72.0);
                        if ui
                            .input_text(format!("##blpath{bid}"), &mut draft.path)
                            .build()
                            && let Some(d) = app.block_drafts.get_mut(&bid)
                        {
                            d.path = draft.path.clone();
                        }
                        ui.same_line();
                        if ui.button(format!("...##blfile{bid}"))
                            && let Some(p) = rfd::FileDialog::new()
                                .set_title("选择回放日志")
                                .add_filter("日志文件", &["asc", "blf"])
                                .pick_file()
                            && let Some(d) = app.block_drafts.get_mut(&bid)
                        {
                            d.path = p.to_string_lossy().into_owned();
                            draft.path = d.path.clone();
                        }
                        // id 过滤（可选）：留空 = 该节点的全部报文。
                        const BLOCK_IDS_HINT: &str = "id 过滤，如 100, 3F4x";
                        ui.set_next_item_width(crate::ui::hint_width(
                            ui,
                            BLOCK_IDS_HINT,
                            140.0,
                        ));
                        if ui
                            .input_text(format!("##blids{bid}"), &mut draft.ids_text)
                            .hint(BLOCK_IDS_HINT)
                            .build()
                            && let Some(d) = app.block_drafts.get_mut(&bid)
                        {
                            d.ids_text = draft.ids_text.clone();
                        }
                        ui.same_line();
                        let saved = app
                            .snap
                            .blocks
                            .iter()
                            .find(|b| b.id == bid)
                            .map(|b| b.path.clone())
                            .unwrap_or_default();
                        if ui.button(format!("Apply##blapply{bid}"))
                            && let Some(d) = app.block_drafts.get(&bid)
                        {
                            let block = app.snap.blocks.iter().find(|b| b.id == bid);
                            app.send(crate::bus::BusCommand::SetReplayBlock {
                                id: bid,
                                name: name.clone(),
                                channel: ch as u8,
                                path: d.path.clone(),
                                node_filter: block.and_then(|b| b.node_filter.clone()),
                                attached: block.and_then(|b| b.attached.clone()),
                                ids: crate::ui::parse_id_filter(&d.ids_text),
                            });
                        }
                        if draft.path != saved {
                            ui.same_line();
                            ui.text_colored([1.0, 0.8, 0.4, 1.0], "未应用");
                        }
                        ui.unindent();
                    }
                    ui.separator();
                    ui.text("Sent messages");
                    for (id, name) in &ni.tx {
                        let (count, cycle) = app
                            .snap
                            .aggs
                            .iter()
                            .find(|a| a.channel == ch as u8 && a.id == *id)
                            .map(|a| (a.count, a.cycle_us / 1000.0))
                            .unwrap_or((0, 0.0));
                        let cycle_s = if count >= 2 {
                            format!("  ~{cycle:.1} ms")
                        } else {
                            String::new()
                        };
                        ui.text(format!("  {id:03X}  {name}  count {count}{cycle_s}"));
                    }
                    ui.text("Received signals");
                    for (key, sig, sender) in &ni.rx {
                        let msg = app
                            .channel_dbc(ch as u8)
                            .and_then(|db| db.message_name_of(*key))
                            .unwrap_or("-");
                        let (id, ext) = *key;
                        let id_text = if ext {
                            format!("{id:03X} ext")
                        } else {
                            format!("{id:03X}")
                        };
                        ui.text(format!("  {sig}  <-  {sender}  ({msg} {id_text})"));
                    }
                });
            });
    }
    app.show_network = open;
}
