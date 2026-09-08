//! The Replay Blocks window: edit the recorded-traffic drivers. One card
//! per block -- name, bus, log file, the node/id filters, and the enable
//! toggle that decides whether the block joins the next measurement (or
//! the running one). All entity interaction is the same declaration the
//! project file stores.

use crate::app::App;
use imgui::{Condition, Ui};

pub fn render(app: &mut App, ui: &Ui) {
    if !app.show_blocks {
        return;
    }
    let io = ui.io();
    let mut open = app.show_blocks;
    ui.window("Replay Blocks")
        .opened(&mut open)
        .position(
            [io.display_size[0] * 0.45, io.display_size[1] * 0.35],
            Condition::FirstUseEver,
        )
        .size([560.0, 300.0], Condition::FirstUseEver)
        .build(|| content(app, ui));
    app.show_blocks = open;
}

fn content(app: &mut App, ui: &Ui) {
    if ui.small_button("+ Block") {
        let n = app.snap.blocks.len();
        app.add_replay_block(0, format!("Block {}", n + 1), String::new(), None);
    }
    ui.same_line();
    ui.text_disabled("回放块：把录制的日志按过滤条件注回仿真总线，仅仿真模式发车");

    ui.separator();
    let blocks: Vec<crate::bus::ReplayBlockView> = app.snap.blocks.clone();
    for b in &blocks {
        block_card(app, ui, b);
        ui.spacing();
    }
}

/// The card's text drafts as one owned snapshot, so the borrow of
/// `block_drafts` ends before any command goes out.
fn drafts_of(app: &mut App, id: u64) -> crate::ui::BlockDraft {
    app.block_drafts.entry(id).or_default().clone()
}

/// Parses the id filter text via the shared helper.
fn parse_ids(text: &str) -> Vec<(u32, bool)> {
    crate::ui::parse_id_filter(text)
}

fn block_card(app: &mut App, ui: &Ui, b: &crate::bus::ReplayBlockView) {
    let id = b.id;
    // Seed the drafts from the published declaration the first time.
    app.block_drafts.entry(id).or_insert_with(|| crate::ui::BlockDraft {
        name: b.name.clone(),
        path: b.path.clone(),
        ids_text: String::new(),
    });
    let open_token = ui
        .tree_node_config(format!("{}##block{}", b.name, id))
        .push();
    let Some(_t) = open_token else { return };

    // Header: name draft, bus, enable toggle.
    ui.set_next_item_width(130.0);
    let mut name = drafts_of(app, id).name;
    if ui.input_text(format!("##bname{id}"), &mut name).build()
        && let Some(d) = app.block_drafts.get_mut(&id)
    {
        d.name = name.clone();
    }
    ui.same_line();
    ui.set_next_item_width(90.0);
    let mut channel = b.channel as usize;
    let bus_names: Vec<String> = (0..app.snap.channel_count)
        .map(|ch| app.channel_name(ch as u8))
        .collect();
    let refs: Vec<&str> = bus_names.iter().map(|s| s.as_str()).collect();
    if ui.combo_simple_string(format!("##bch{id}"), &mut channel, &refs) {
        let d = drafts_of(app, id);
        app.send(crate::bus::BusCommand::SetReplayBlock {
            id,
            name: d.name,
            channel: channel as u8,
            path: d.path,
            node_filter: b.node_filter.clone(),
            ids: b.ids.clone(),
        });
    }
    ui.same_line();
    let mut enabled = b.enabled;
    if ui.checkbox(format!("##ben{id}"), &mut enabled) {
        app.set_replay_block_enabled(id, enabled);
    }
    ui.same_line();
    if ui.small_button(format!("删除##brm{id}")) {
        app.remove_replay_block(id);
        app.block_drafts.remove(&id);
        return;
    }

    // Log file row: draft text with a file picker beside it.
    ui.set_next_item_width(-72.0);
    let mut path = drafts_of(app, id).path;
    if ui.input_text(format!("##bpath{id}"), &mut path).build()
        && let Some(d) = app.block_drafts.get_mut(&id)
    {
        d.path = path.clone();
    }
    ui.same_line();
    if ui.small_button(format!("...##bfile{id}"))
        && let Some(p) = rfd::FileDialog::new()
            .set_title("选择回放日志")
            .add_filter("日志文件", &["asc", "blf"])
            .pick_file()
        && let Some(d) = app.block_drafts.get_mut(&id)
    {
        d.path = p.to_string_lossy().into_owned();
    }

    // Filter row: DBC node combo (None = every sender) plus an id list.
    let nodes: Vec<String> = app
        .channel_dbc(b.channel)
        .map(|db| db.nodes.clone())
        .unwrap_or_default();
    let selected = match &b.node_filter {
        Some(node) => nodes
            .iter()
            .position(|n| n == node)
            .map(|i| i + 1)
            .unwrap_or(0),
        None => 0,
    };
    let mut combo_labels: Vec<String> = vec!["(全部发送节点)".to_string()];
    combo_labels.extend(nodes.iter().cloned());
    let label_refs: Vec<&str> = combo_labels.iter().map(|s| s.as_str()).collect();
    ui.set_next_item_width(130.0);
    let mut sel = selected;
    if ui.combo_simple_string(format!("##bnode{id}"), &mut sel, &label_refs) {
        let node_filter = if sel == 0 {
            None
        } else {
            Some(nodes[sel - 1].clone())
        };
        let d = drafts_of(app, id);
        app.send(crate::bus::BusCommand::SetReplayBlock {
            id,
            name: d.name,
            channel: b.channel,
            path: d.path,
            node_filter,
            ids: b.ids.clone(),
        });
    }
    ui.same_line();
    ui.set_next_item_width(140.0);
    let mut ids_text = drafts_of(app, id).ids_text;
    if ui
        .input_text(format!("##bids{id}"), &mut ids_text)
        .hint("如 100, 3F4x")
        .build()
        && let Some(d) = app.block_drafts.get_mut(&id)
    {
        d.ids_text = ids_text.clone();
    }

    // Apply commits every text draft at once.
    if ui.small_button(format!("Apply##bapply{id}")) {
        let d = drafts_of(app, id);
        app.send(crate::bus::BusCommand::SetReplayBlock {
            id,
            name: d.name,
            channel: b.channel,
            path: d.path,
            node_filter: b.node_filter.clone(),
            ids: parse_ids(&d.ids_text),
        });
    }
    let d = drafts_of(app, id);
    if d.name != b.name || d.path != b.path || parse_ids(&d.ids_text) != b.ids {
        ui.same_line();
        ui.text_colored([1.0, 0.8, 0.4, 1.0], "未应用");
    }
    if let Some(e) = &b.last_error {
        ui.text_colored([1.0, 0.55, 0.3, 1.0], format!("载入失败：{e}"));
    } else if b.frames > 0 {
        ui.text_disabled(format!("已载入 {} 帧（时间从 0 归一化）", b.frames));
    } else {
        ui.text_disabled("未载入：选择日志并开启后自动载入");
    }
}
