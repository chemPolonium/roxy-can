//! Per-node script editors: one window per node being edited, opened on
//! demand from the Entities table or the Network view. There is no
//! aggregate "all scripts" window -- a node's script belongs to the node,
//! and the directory (Entities) is where you decide which one to open.

use crate::app::App;
use imgui::{Condition, Ui};

const SOURCE_HEIGHT: f32 = 220.0;
const LOG_LINES: usize = 10;

pub fn render(app: &mut App, ui: &Ui) {
    let ids = app.open_editors.clone();
    for id in ids {
        let Some(node) = app.snap.nodes.iter().find(|n| n.id == id).cloned() else {
            // The node went away (deleted, project replaced): its editor
            // has nothing left to edit.
            app.close_script_editor(id);
            continue;
        };
        editor_window(app, ui, &node);
    }
}

fn editor_window(app: &mut App, ui: &Ui, node: &crate::bus::NodeView) {
    let id = node.id;
    // `###` pins the window identity to the node, so a rename moves the
    // title without losing position or open state.
    let mut open = true;
    ui.window(format!("脚本 · {}###script{}", node.name, id))
        .opened(&mut open)
        .position(
            [340.0 + (id as f32 % 6.0) * 28.0, 80.0 + (id as f32 % 6.0) * 24.0],
            Condition::FirstUseEver,
        )
        .size([520.0, 460.0], Condition::FirstUseEver)
        .build(|| content(app, ui, node));
    if !open {
        app.close_script_editor(id);
    }
}

fn content(app: &mut App, ui: &Ui, node: &crate::bus::NodeView) {
    let id = node.id;

    // Header: node name, channel binding, run switch, delete. The name
    // commits per keystroke -- it is a cheap string write and the
    // Entities table shows it live.
    let mut name = node.name.clone();
    ui.set_next_item_width(140.0);
    if ui.input_text(format!("##ename{id}"), &mut name).build() {
        app.send(crate::bus::BusCommand::SetNodeName { id, name });
    }
    ui.same_line();
    let mut channel = node.channel as usize;
    let bus_names: Vec<String> = (0..app.snap.channel_count)
        .map(|ch| app.channel_name(ch as u8))
        .collect();
    let refs: Vec<&str> = bus_names.iter().map(|s| s.as_str()).collect();
    if ui.combo_simple_string(format!("##ech{id}"), &mut channel, &refs) {
        app.send(crate::bus::BusCommand::SetNodeChannel {
            id,
            channel: channel as u8,
        });
    }
    ui.same_line();
    let mut enabled = node.enabled;
    if ui.checkbox(format!("运行##een{id}"), &mut enabled) {
        app.send(crate::bus::BusCommand::SetNodeEnabled { id, on: enabled });
    }
    ui.same_line();
    if ui.small_button(format!("删除##erm{id}")) {
        app.send(crate::bus::BusCommand::RemoveNode { id });
        app.node_src_draft.remove(&id);
        app.close_script_editor(id);
        return;
    }

    // Status line.
    if node.errored {
        ui.text_colored([1.0, 0.55, 0.3, 1.0], "出错（见日志；重新 Apply 或重启测量恢复）");
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
    ui.input_text_multiline(format!("##esrc{id}"), draft, [0.0, SOURCE_HEIGHT])
        .build();
    let dirty = *app.node_src_draft.get(&id).unwrap() != node.source;
    if ui.small_button(format!("Apply##eapply{id}")) {
        let source = app.node_src_draft.get(&id).cloned().unwrap_or_default();
        app.send(crate::bus::BusCommand::SetNodeSource { id, source });
    }
    if dirty {
        ui.same_line();
        ui.text_colored([1.0, 0.8, 0.4, 1.0], "未应用");
    }
    ui.same_line();
    if ui.small_button(format!("保存##esave{id}")) {
        let source = app.node_src_draft.get(&id).cloned().unwrap_or_default();
        if let Some(path) = rfd::FileDialog::new()
            .set_title("保存节点脚本")
            .add_filter("节点脚本", &["capl"])
            .save_file()
        {
            let path = path.to_string_lossy().into_owned();
            if let Err(e) = std::fs::write(&path, &source) {
                app.status = format!("保存失败: {e}");
            } else {
                app.status = format!("已保存 {path}");
            }
        }
    }
    ui.same_line();
    if ui.small_button(format!("加载##eload{id}")) {
        let picked = rfd::FileDialog::new()
            .set_title("加载节点脚本")
            .add_filter("节点脚本", &["capl"])
            .pick_file();
        if let Some(p) = picked {
            let path = p.to_string_lossy().into_owned();
            match std::fs::read_to_string(&path) {
                Ok(src) => {
                    app.node_src_draft.insert(id, src);
                }
                Err(e) => {
                    app.status = format!("加载失败: {e}");
                }
            }
        }
    }

    // Log tail, newest at the bottom.
    if !node.log.is_empty() {
        ui.child_window(format!("##elog{id}"))
            .size([0.0, 110.0])
            .build(|| {
                let show = node.log.len().saturating_sub(LOG_LINES);
                for line in &node.log[show..] {
                    ui.text(line);
                }
            });
    }
}
