use crate::app::App;
use imgui::{Condition, TreeNodeFlags, Ui};

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
    let token = ui
        .tree_node_config(app.channel_name(ch as u8))
        .flags(TreeNodeFlags::DEFAULT_OPEN)
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
                crate::app::NodeRole::Monitor => ("[M]", [0.45, 0.62, 0.80, 1.0]),
                crate::app::NodeRole::Absent => ("[-]", [0.45, 0.45, 0.55, 1.0]),
            };
            ui.text_colored(tag_color, tag);
            if ui.is_item_hovered() {
                ui.tooltip_text(role.hint());
            }
            ui.same_line();
            if ui
                .selectable_config(format!("{}##net{ch}_{i}", ni.name))
                .selected(app.net_selected == flat_base + i)
                .build()
            {
                app.net_selected = flat_base + i;
            }
            // 绑定到该节点的脚本作为下一层树叶挂在节点下：点击打开
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
            // 绑定到该节点的回放块同样挂在节点下：点击打开 Replay
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
        // 自由脚本（未绑定 DBC 节点）挂在总线根下，旧工程仍可见。
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
                [io.display_size[0] * 0.3, io.display_size[1] * 0.55],
                Condition::FirstUseEver,
            )
            .size([860.0, 540.0], Condition::FirstUseEver)
            .build(|| {
                // Profile row: enumerate the project's profiles/*.toml and
                // apply the selected one (all-or-nothing role overrides).
                if app.profile_names.is_none() {
                    let dir = app
                        .project_path
                        .as_ref()
                        .and_then(|p| p.parent().map(|p| p.to_path_buf()))
                        .unwrap_or_default();
                    app.profile_names = Some(Ok(crate::profile::list_profiles(&dir)));
                }
                let Some(Ok(profile_list)) = app.profile_names.as_ref() else {
                    return;
                };
                if !profile_list.is_empty() {
                    ui.text_disabled("Profile");
                    ui.same_line();
                    let mut pick = app.profile_pick.min(profile_list.len() - 1);
                    ui.set_next_item_width(140.0);
                    if ui.combo_simple_string("##prof", &mut pick, profile_list) {
                        app.profile_pick = pick;
                    }
                    ui.same_line();
                    if ui.button("应用##profapply") {
                        let dir = app
                            .project_path
                            .as_ref()
                            .and_then(|p| p.parent().map(|p| p.to_path_buf()))
                            .unwrap_or_default();
                        let name =
                            profile_list[app.profile_pick.min(profile_list.len() - 1)].clone();
                        match crate::profile::apply_profile(app, &dir, &name) {
                            Ok(s) => app.status = s,
                            Err(e) => app.status = e,
                        }
                    }
                    if ui.is_item_hovered() {
                        ui.tooltip_text("按 profiles/*.toml 覆盖角色（all-or-nothing）");
                    }
                }
                let dbc_nodes = collect(app);
                let total_dbc: usize = dbc_nodes.iter().map(|v| v.len()).sum();
                if app.net_selected >= total_dbc.max(1) {
                    app.net_selected = 0;
                }

                // 左右分栏：左边树形拓扑，右边所选节点的详情。两栏各自
                // 滚动——树的长度与详情的长度互不挤占。
                const TREE_W: f32 = 240.0;
                ui.child_window("net_tree")
                    .size([TREE_W, 0.0])
                    .border(true)
                    .build(|| {
                        let mut flat_base = 0usize;
                        for (ch, infos) in dbc_nodes.iter().enumerate() {
                            draw_tree_section(app, ui, ch, infos, flat_base);
                            flat_base += infos.len();
                        }
                    });
                ui.same_line();

                // Details scroll inside their own panel (which always fills the
                // remaining space), so long content never adds a scrollbar to the
                // outer window and shifts the topology sections.
                ui.child_window("node_details").size([0.0, 0.0]).build(|| {
                    if total_dbc == 0 {
                        ui.text("no DBC nodes to display");
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
                    // The selector alone never explains the roles; the
                    // selected one's declaration sits right below it.
                    ui.text_disabled(crate::app::NodeRole::ALL[role_idx].hint());
                    ui.same_line();
                    ui.text_disabled("节点角色");
                    ui.same_line();
                    if ui.button(format!("Simulate all##netsimall{ch}")) {
                        app.simulate_all_nodes(ch as u8);
                    }
                    ui.same_line();
                    if ui.button(format!("Stop all##netstop{ch}")) {
                        app.stop_all_nodes(ch as u8);
                    }
                    if ni.tx.is_empty() {
                        ui.text_colored(
                            [0.5, 0.5, 0.6, 1.0],
                            "  (sends nothing -- the role is still recorded)",
                        );
                    }
                    ui.separator();

                    // 节点生成器：这个节点的条目、添加与响应规则，
                    // 就近挂在角色声明之下——节点就是编辑单元。
                    crate::ui::tx::render_node_generator(app, ui, ch as u8, &ni.name);
                    ui.separator();

                    // 脚本编辑入口在节点之下（而非总线）：新建的脚本
                    // 自动绑定到当前选中的 DBC 节点，发帧受其角色闸。
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
                            "把该节点录制的真实流量注回仿真总线（restbus）；新建后在下方选择日志文件，启用即发车",
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
                            ui.tooltip_text("启用后测量中按录制间距发车（仅仿真模式）");
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
                        ui.set_next_item_width(140.0);
                        if ui
                            .input_text(format!("##blids{bid}"), &mut draft.ids_text)
                            .hint("id 过滤，如 100, 3F4x")
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
