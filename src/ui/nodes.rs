//! Nodes window: the script-node workbench. One card per node -- name,
//! channel, source editor, Apply, status, and the node's text log. All
//! node interaction is text; the window only manages the nodes.

use crate::app::App;
use imgui::Ui;

const LOG_LINES: usize = 8;
const SOURCE_HEIGHT: f32 = 160.0;

pub fn render(app: &mut App, ui: &Ui) {
    if !app.show_nodes {
        return;
    }
    let mut open = app.show_nodes;
    let disp = ui.io().display_size;
    ui.window("Nodes")
        .opened(&mut open)
        .position([disp[0] * 0.62, 40.0], imgui::Condition::FirstUseEver)
        .size([520.0, 420.0], imgui::Condition::FirstUseEver)
        .build(|| content(app, ui));
    app.show_nodes = open;
}

fn content(app: &mut App, ui: &Ui) {
    if ui.small_button("+ Node") {
        let n = app.snap.nodes.len();
        let name = format!("Node {}", n + 1);
        let channel = 0;
        app.send(crate::bus::BusCommand::AddNode { name, channel });
    }
    ui.same_line();
    ui.text_disabled("节点按文本交互：print 进日志，send 发帧到总线");

    ui.separator();
    let ids: Vec<u64> = app.snap.nodes.iter().map(|n| n.id).collect();
    for id in ids {
        let node = app.snap.nodes.iter().find(|n| n.id == id).expect("node");
        let node = node.clone();
        card(app, ui, &node);
        ui.spacing();
    }
}

/// One node card. Works on a clone of the view; every edit goes out as a
/// command keyed by the node's stable id.
fn card(app: &mut App, ui: &Ui, node: &crate::bus::NodeView) {
    let id = node.id;
    let open_token = ui
        .tree_node_config(format!("{}##node{}", node.name, id))
        .flags(imgui::TreeNodeFlags::DEFAULT_OPEN)
        .push();
    let Some(_t) = open_token else { return };

    // Header row: name, channel, enabled.
    ui.set_next_item_width(140.0);
    let mut name = node.name.clone();
    if ui.input_text(format!("##nname{id}"), &mut name).build() && !name.is_empty() {
        app.send(crate::bus::BusCommand::SetNodeName {
            id,
            name: name.clone(),
        });
    }
    ui.same_line();
    ui.set_next_item_width(80.0);
    let mut channel = node.channel as usize;
    let bus_names: Vec<String> = (0..app.snap.channel_count)
        .map(|ch| app.channel_name(ch as u8))
        .collect();
    let refs: Vec<&str> = bus_names.iter().map(|s| s.as_str()).collect();
    if ui.combo_simple_string(format!("##nch{id}"), &mut channel, &refs) {
        app.send(crate::bus::BusCommand::SetNodeChannel {
            id,
            channel: channel as u8,
        });
    }
    ui.same_line();
    let mut enabled = node.enabled;
    if ui.checkbox(format!("运行##nen{id}"), &mut enabled) {
        app.send(crate::bus::BusCommand::SetNodeEnabled { id, on: enabled });
    }
    ui.same_line();
    if ui.small_button(format!("删除##nrm{id}")) {
        app.send(crate::bus::BusCommand::RemoveNode { id });
        app.node_src_draft.remove(&id);
        return;
    }

    // Status line.
    if let Some(err) = state_text(node) {
        ui.text_colored([1.0, 0.55, 0.3, 1.0], err);
    } else if node.running {
        ui.text_colored([0.4, 0.95, 0.5, 1.0], "运行中");
    } else {
        ui.text_disabled("已停止");
    }

    // Source editor: local draft, applied on Apply (per-keystroke
    // recompiles would churn the core thread).
    let draft = app
        .node_src_draft
        .entry(id)
        .or_insert_with(|| node.source.clone());
    ui.set_next_item_width(-1.0);
    ui.input_text_multiline(format!("##nsrc{id}"), draft, [0.0, SOURCE_HEIGHT])
        .build();
    let dirty = *app.node_src_draft.get(&id).unwrap() != node.source;
    if ui.small_button(format!("Apply##napply{id}")) {
        let source = app.node_src_draft.get(&id).cloned().unwrap_or_default();
        app.send(crate::bus::BusCommand::SetNodeSource { id, source });
    }
    if dirty {
        ui.same_line();
        ui.text_colored([1.0, 0.8, 0.4, 1.0], "未应用");
    }

    // Log tail, newest at the bottom.
    if !node.log.is_empty() {
        ui.child_window(format!("##nlog{id}"))
            .size([0.0, 90.0])
            .build(|| {
                let show = node.log.len().saturating_sub(LOG_LINES);
                for line in &node.log[show..] {
                    ui.text(line);
                }
            });
    }
}

fn state_text(node: &crate::bus::NodeView) -> Option<String> {
    if node.errored {
        Some("出错（见日志；重新 Apply 或重启测量恢复）".to_string())
    } else {
        None
    }
}
