//! Per-node script editors: one window per node being edited, opened on
//! demand from the Entities table or the Network view. There is no
//! aggregate "all scripts" window -- a node's script belongs to the node,
//! and the directory (Entities) is where you decide which one to open.

use crate::app::App;
use imgui::{
    Condition, InputTextCallbackHandler, InputTextMultilineCallback, TextCallbackData, Ui,
};
use std::collections::HashMap;
use std::hash::{Hash, Hasher};

const SOURCE_HEIGHT: f32 = 220.0;
const LOG_LINES: usize = 10;

/// The source editor's cursor state: byte offset into the draft plus the
/// active selection, in bytes, and the inner scroll of the multiline's
/// text viewport. Recorded every frame the edit widget is active; the
/// sidebar inserts land here, the gutter scrolls with the offset, and
/// the space-dot overlay lines up with it.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct EditorCursor {
    pub pos: usize,
    pub sel: Option<(usize, usize)>,
    pub scroll: f32,
    pub hscroll: f32,
}

/// Records the cursor while the source widget runs (`CALLBACK_ALWAYS`
/// only fires while the widget is active, which is exactly the span the
/// position is meaningful and changing). During the callback the
/// multiline's inner child window is ImGui's current window, so the
/// scroll read there is the text viewport's own.
struct CursorSync<'a> {
    cursors: &'a mut HashMap<u64, EditorCursor>,
    id: u64,
}

impl InputTextCallbackHandler for CursorSync<'_> {
    fn on_always(&mut self, data: TextCallbackData) {
        let sel = data.selection();
        let sel = if sel.start < sel.end {
            Some((sel.start, sel.end))
        } else {
            None
        };
        let scroll = unsafe { imgui::sys::igGetScrollY() };
        let hscroll = unsafe { imgui::sys::igGetScrollX() };
        self.cursors.insert(
            self.id,
            EditorCursor {
                pos: data.cursor_pos(),
                sel,
                scroll,
                hscroll,
            },
        );
    }
}

/// Static facts of one editor's draft text: the handler outline and the
/// compile-derived send/receive/system-variable sets. Recomputed only
/// when the draft's hash changes, not per frame.
#[derive(Clone, Debug, Default)]
pub(crate) struct EditorFacts {
    pub hash: u64,
    /// `(line, label)` in draft order.
    pub outline: Vec<(u32, String)>,
    /// `(id, extended, arming handler)` from the compiler's send set.
    pub sends: Vec<(u32, bool, String)>,
    /// `(id, extended)` the handlers listen for.
    pub recvs: Vec<(u32, bool)>,
    pub recv_wildcard: bool,
    /// `ns::name` keys this script accesses.
    pub sysvars: Vec<String>,
    /// Response mapping: one row per armed one-shot timer.
    pub responses: Vec<ResponseRowLite>,
    /// Per-character highlight classes, aligned with the draft's
    /// `split('\n')` segments.
    pub classes: Vec<Vec<u8>>,
    /// Source line (1-based) of the compile error, for the gutter mark.
    pub error_line: Option<u32>,
    /// Compile error, when the draft no longer compiles.
    pub error: Option<String>,
}

/// One response row as the editor shows it: the timer, what arms it,
/// and what it sends.
#[derive(Clone, Debug, Default)]
pub(crate) struct ResponseRowLite {
    pub timer_label: String,
    pub armed_by: Vec<String>,
    pub sends: Vec<(u32, bool)>,
}

/// Per-character highlight classes (the tint overlay's input): 0 plain,
/// 1 keyword, 2 number, 3 string, 4 comment.
pub(crate) const PLAIN: u8 = 0;
pub(crate) const KEYWORD: u8 = 1;
pub(crate) const NUMBER: u8 = 2;
pub(crate) const STRING: u8 = 3;
pub(crate) const COMMENT: u8 = 4;

/// Classifies the draft character by character. A tiny lexer mirroring
/// `script/lexer`'s token shapes (`//` and `/* */` comments, `"strings"`
/// with backslash escapes, hex-friendly numbers, the keyword set); it
/// feeds the tint overlay only, never the compiler.
pub(crate) fn classify_lines(src: &str) -> Vec<Vec<u8>> {
    const KEYWORDS: [&str; 12] = [
        "on", "fn", "if", "else", "while", "for", "return", "break", "continue", "true", "false",
        "let",
    ];
    let mut out = Vec::new();
    let mut in_block = false;
    for line in src.split('\n') {
        let chars: Vec<char> = line.chars().collect();
        let mut classes = vec![PLAIN; chars.len()];
        let mut i = 0usize;
        while i < chars.len() {
            if in_block {
                classes[i] = COMMENT;
                if chars[i] == '*' && chars.get(i + 1) == Some(&'/') {
                    if i + 1 < classes.len() {
                        classes[i + 1] = COMMENT;
                    }
                    in_block = false;
                    i += 2;
                } else {
                    i += 1;
                }
                continue;
            }
            match chars[i] {
                '/' if chars.get(i + 1) == Some(&'/') => {
                    while i < classes.len() {
                        classes[i] = COMMENT;
                        i += 1;
                    }
                }
                '/' if chars.get(i + 1) == Some(&'*') => {
                    in_block = true;
                    classes[i] = COMMENT;
                    if i + 1 < classes.len() {
                        classes[i + 1] = COMMENT;
                    }
                    i += 2;
                }
                '"' => {
                    classes[i] = STRING;
                    i += 1;
                    while i < chars.len() {
                        classes[i] = STRING;
                        if chars[i] == '\\' && i + 1 < chars.len() {
                            i += 1;
                            classes[i] = STRING;
                        } else if chars[i] == '"' {
                            i += 1;
                            break;
                        }
                        i += 1;
                    }
                }
                c if c.is_ascii_digit() => {
                    while i < chars.len() && (chars[i].is_ascii_alphanumeric() || chars[i] == '_')
                    {
                        classes[i] = NUMBER;
                        i += 1;
                    }
                }
                c if c.is_alphabetic() || c == '_' => {
                    let start = i;
                    while i < chars.len() && (chars[i].is_alphanumeric() || chars[i] == '_') {
                        i += 1;
                    }
                    let word: String = chars[start..i].iter().collect();
                    if KEYWORDS.contains(&word.as_str()) {
                        for cl in &mut classes[start..i] {
                            *cl = KEYWORD;
                        }
                    }
                }
                _ => i += 1,
            }
        }
        out.push(classes);
    }
    out
}

/// Derives the static facts of a draft. The outline comes from a line
/// scan (it must work on broken, half-typed source); the sets come from
/// the compiler and are absent when it fails.
fn compute_facts(src: &str, hash: u64) -> EditorFacts {
    let mut facts = EditorFacts {
        hash,
        classes: classify_lines(src),
        ..Default::default()
    };
    for (i, raw) in src.lines().enumerate() {
        let line = raw.trim_start();
        const PREFIXES: [&str; 6] = [
            // Extended before plain: both are `starts_with` matches.
            "on extended message",
            "on message",
            "on timer",
            "on start",
            "on errorFrame",
            "fn ",
        ];
        let label = PREFIXES
            .iter()
            .find(|p| line.starts_with(**p))
            .map(|_| line.split('{').next().unwrap_or("").trim().to_string());
        if let Some(label) = label {
            facts.outline.push((i as u32 + 1, label));
        }
    }
    match crate::script::compile(src) {
        Ok(script) => {
            for (_, id, ext) in &script.send_refs {
                let entry = (*id, *ext);
                if !facts.sends.iter().any(|(i, e, _)| (*i, *e) == entry) {
                    let from = script
                        .send_refs
                        .iter()
                        .find(|(_, i, e)| (*i, *e) == entry)
                        .map(|(f, _, _)| f.clone())
                        .unwrap_or_default();
                    facts.sends.push((*id, *ext, from));
                }
            }
            for (id, ext) in &script.recv_refs {
                let entry = (*id, *ext);
                if !facts.recvs.contains(&entry) {
                    facts.recvs.push(entry);
                }
            }
            facts.recv_wildcard = script.recv_wildcard;
            for key in &script.sysvar_refs {
                if !facts.sysvars.contains(key) {
                    facts.sysvars.push(key.clone());
                }
            }
            for row in script.response_map() {
                // A timer nobody arms never fires: not a response.
                if row.armed_by.is_empty() {
                    continue;
                }
                facts.responses.push(ResponseRowLite {
                    timer_label: row.timer_label,
                    armed_by: row.armed_by,
                    sends: row.sends,
                });
            }
        }
        Err(e) => {
            facts.error_line = Some(e.line);
            facts.error = Some(e.to_string());
        }
    }
    facts
}

/// Cached facts for a draft, recomputed when its hash changed.
fn facts_for(app: &mut App, id: u64) -> EditorFacts {
    let src = app.node_src_draft.get(&id).cloned().unwrap_or_default();
    let mut h = std::collections::hash_map::DefaultHasher::new();
    src.hash(&mut h);
    let hash = h.finish();
    let cached = app.editor_facts.get(&id);
    if cached.is_some_and(|f| f.hash == hash) {
        return cached.unwrap().clone();
    }
    let facts = compute_facts(&src, hash);
    app.editor_facts.insert(id, facts.clone());
    facts
}

pub fn render(app: &mut App, ui: &Ui) {
    let ids = app.open_editors.clone();
    for id in ids {
        let Some(node) = app.snap.nodes.iter().find(|n| n.id == id).cloned() else {
            // The node went away (deleted, project replaced): its editor
            // has nothing left to edit.
            app.close_script_editor(id);
            app.editor_facts.remove(&id);
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
    // The script lives where its node lives: the binding is shown
    // read-only, there is no bus choice anywhere in the flow.
    match &node.attached {
        Some((ach, anode)) => {
            ui.text_disabled(format!(
                "节点 {anode} · 总线 {}",
                app.channel_name(*ach),
            ));
        }
        None => {
            ui.text_colored(
                [1.0, 0.8, 0.4, 1.0],
                format!(
                    "未绑定节点（总线 {}）——挂接 DBC 后自动收养",
                    app.channel_name(node.channel),
                ),
            );
        }
    }
    ui.same_line();
    // The wire-egress switch: shown whenever the node's bus has hardware
    // attached. 只收挂接在虚拟通道上同样能发车（驱动行为）。
    let bus_has_hw = app.snap.hw.iter().any(|h| h.bus == node.channel);
    if bus_has_hw {
        let mut via_hw = app
            .snap
            .hw_tx_nodes
            .contains(&(node.channel, node.name.clone()));
        if ui.checkbox(format!("经硬件##ehw{id}"), &mut via_hw) {
            app.set_node_hardware_tx(node.channel, &node.name, via_hw);
        }
        if ui.is_item_hovered() {
            ui.tooltip_text("该节点的发车同时上真实总线（Kvaser）");
        }
    }
    ui.same_line();
    let mut enabled = node.enabled;
    if ui.checkbox(format!("运行##een{id}"), &mut enabled) {
        app.send(crate::bus::BusCommand::SetNodeEnabled { id, on: enabled });
    }
    ui.same_line();
    if ui.button(format!("删除##erm{id}")) {
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

    // Source editor + sidebar: a two-column layout where the sidebar
    // lists available functions/constructs and the right side holds the
    // source editor, Apply/Save/Load, and the log. Both columns fill the
    // remaining height.
    let avail = ui.content_region_avail();
    const SIDEBAR_W: f32 = 180.0;

    // Left sidebar (full remaining height).
    ui.child_window(format!("##esidebar{id}"))
        .size([SIDEBAR_W, avail[1]])
        .border(true)
        .build(|| sidebar(app, ui, id, node));

    ui.same_line();

    // Right main area: source + Apply/Save/Load + log.
    ui.child_window(format!("##emain{id}"))
        .size([0.0, avail[1]])
        .build(|| {
            // Cached per-draft facts (highlight classes, error line, ...)
            // before the draft borrow starts.
            let facts = facts_for(app, id);
            let draft = app
                .node_src_draft
                .entry(id)
                .or_insert_with(|| node.source.clone());

            // Line-number gutter: the multiline never soft-wraps (ImGui
            // 1.89), so row k of the text viewport is exactly draft line k
            // at pitch `font_size`, offset by the widget's own scroll.
            let lh = ui.current_font_size();
            let frame_pad_y = unsafe { ui.style() }.frame_padding[1];
            let mut rows = draft.lines().count().max(1);
            if draft.ends_with('\n') {
                rows += 1;
            }
            let digits = (rows as f32).log10().floor() as usize + 1;
            let digit_w = ui.calc_text_size("0")[0];
            const GUTTER_PAD: f32 = 6.0;
            let gutter_w = GUTTER_PAD + digits as f32 * digit_w + GUTTER_PAD;

            let gutter_min = ui.cursor_screen_pos();
            let gutter_max = [gutter_min[0] + gutter_w, gutter_min[1] + SOURCE_HEIGHT];
            ui.dummy([gutter_w, SOURCE_HEIGHT]);
            ui.same_line();
            // Field-level split borrow: the cursor tracker takes the
            // cursors map while `draft` holds the source draft.
            let sync = CursorSync {
                cursors: &mut app.editor_cursors,
                id,
            };
            ui.set_next_item_width(-1.0);
            ui.input_text_multiline(format!("##esrc{id}"), draft, [0.0, SOURCE_HEIGHT])
                .callback(InputTextMultilineCallback::ALWAYS, sync)
                .build();

            // The callback has run by now when the widget is active, so
            // the stored viewport reflects the current frame.
            let (scroll, hscroll) = app
                .editor_cursors
                .get(&id)
                .map(|c| (c.scroll, c.hscroll))
                .unwrap_or((0.0, 0.0));
            let frame_pad_x = unsafe { ui.style() }.frame_padding[0];
            let widget_min = [gutter_max[0], gutter_min[1]];
            // The vertical scrollbar overlaps the widget's right edge.
            let widget_max = [
                widget_min[0] + ui.calc_item_width(),
                widget_min[1] + SOURCE_HEIGHT,
            ];
            let mut adv = |c: char| -> f32 {
                let w = app
                    .char_advance
                    .entry(c)
                    .or_insert_with(|| ui.calc_text_size(c.to_string())[0]);
                *w
            };
            let dot_r = (lh * 0.12).max(1.2);
            let dot_color = [0.62, 0.64, 0.68, 0.55];
            let draw_list = ui.get_window_draw_list();
            // Tint per highlight class: translucent bands drawn over the
            // glyphs, so keywords / numbers / strings / comments read as
            // colored without repainting the glyphs themselves.
            let tint_of = |class: u8| match class {
                KEYWORD => [0.45, 0.65, 1.0, 0.16],
                NUMBER => [1.0, 0.62, 0.25, 0.14],
                STRING => [0.4, 0.9, 0.5, 0.15],
                COMMENT => [0.65, 0.65, 0.68, 0.24],
                _ => [0.0, 0.0, 0.0, 0.0],
            };
            draw_list.with_clip_rect(widget_min, widget_max, || {
                for (li, line) in draft.split('\n').enumerate() {
                    let y = widget_min[1] + frame_pad_y + li as f32 * lh - scroll;
                    if y + lh < widget_min[1] || y > widget_max[1] {
                        continue;
                    }
                    let line_cls = facts.classes.get(li);
                    let mut x = widget_min[0] + frame_pad_x - hscroll;
                    let (mut run_class, mut run_w, mut run_x) = (PLAIN, 0.0f32, x);
                    let flush = |class: u8, w: f32, sx: f32| {
                        if class != PLAIN && w > 0.0 {
                            draw_list
                                .add_rect(
                                    [sx, y + lh * 0.08],
                                    [sx + w, y + lh * 0.92],
                                    tint_of(class),
                                )
                                .filled(true)
                                .build();
                        }
                    };
                    for (ci, c) in line.chars().enumerate() {
                        let w = adv(c);
                        let cl = line_cls
                            .and_then(|v| v.get(ci))
                            .copied()
                            .unwrap_or(PLAIN);
                        if cl != run_class {
                            flush(run_class, run_w, run_x);
                            run_class = cl;
                            run_x = x;
                            run_w = 0.0;
                        }
                        run_w += w;
                        if c == ' ' && x + w > widget_min[0] && x < widget_max[0] {
                            let cy = y + lh * 0.55;
                            draw_list
                                .add_circle([x + w * 0.5, cy], dot_r, dot_color)
                                .filled(true)
                                .build();
                        }
                        x += w;
                    }
                    flush(run_class, run_w, run_x);
                }
            });

            let first_row = ((scroll / lh).floor() as i32).max(0) as usize;
            let visible = (SOURCE_HEIGHT / lh).ceil() as usize + 1;
            let last_row = (first_row + visible).min(rows);
            let draw_list = ui.get_window_draw_list();
            draw_list.with_clip_rect(gutter_min, gutter_max, || {
                for i in first_row..last_row {
                    let y = gutter_min[1] + frame_pad_y + i as f32 * lh - scroll;
                    if y + lh < gutter_min[1] || y > gutter_max[1] {
                        continue;
                    }
                    // The compile-error line gets a red bar and a red
                    // number instead of the plain gray one.
                    let is_err = facts.error_line == Some(i as u32 + 1);
                    let num_color = if is_err {
                        [1.0, 0.45, 0.35, 1.0]
                    } else {
                        [0.5, 0.5, 0.5, 1.0]
                    };
                    if is_err {
                        draw_list
                            .add_rect(
                                [gutter_min[0] + 1.0, y],
                                [gutter_min[0] + 3.5, y + lh],
                                num_color,
                            )
                            .filled(true)
                            .build();
                    }
                    draw_list.add_text(
                        [gutter_min[0] + GUTTER_PAD, y],
                        num_color,
                        format!("{:>width$}", i + 1, width = digits),
                    );
                }
            });
            if ui.button(format!("Apply##eapply{id}")) {
                let source =
                    app.node_src_draft.get(&id).cloned().unwrap_or_default();
                app.send(crate::bus::BusCommand::SetNodeSource { id, source });
            }
            if *app.node_src_draft.get(&id).unwrap() != node.source {
                ui.same_line();
                ui.text_colored([1.0, 0.8, 0.4, 1.0], "未应用");
            }
            ui.same_line();
            if ui.button(format!("保存##esave{id}")) {
                let source =
                    app.node_src_draft.get(&id).cloned().unwrap_or_default();
                if let Some(path) = rfd::FileDialog::new()
                    .set_title("保存节点脚本")
                    .add_filter("节点脚本", &["rxcan"])
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
            if ui.button(format!("加载##eload{id}")) {
                let picked = rfd::FileDialog::new()
                    .set_title("加载节点脚本")
                    .add_filter("节点脚本", &["rxcan"])
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
        });
}

/// One insertable sidebar entry: display label and the text that goes
/// into the source when clicked.
const SIDEBAR_ITEMS: &[(&str, &str, &str)] = &[    // (category, label, insert_text)
    ("事件处理器", "on start", "on start {\n    \n}"),
    ("事件处理器", "on message", "on message 0x000 {\n    \n}"),
    ("事件处理器", "on ext message", "on extended message 0x000 {\n    \n}"),
    ("事件处理器", "on message *", "on message * {\n    \n}"),
    ("事件处理器", "on errorFrame", "on errorFrame {\n    \n}"),
    ("事件处理器", "on timer", "on timer 100 {\n    \n}"),
    ("事件处理器", "on timer \"name\"", "on timer \"name\" {\n    \n}"),
    ("总线控制", "send", "send(0x000, 0x00);"),
    ("总线控制", "send_ext", "send_ext(0x000, 0x00);"),
    ("总线控制", "sig", "sig(0x000, \"Signal\")"),
    ("总线控制", "set_sig", "set_sig(buf, 0x000, \"Signal\", 0)"),
    ("总线控制", "emit_value", "emit_value(\"Name\", 0)"),
    ("帧数据", "frame_byte", "frame_byte(0)"),
    ("帧数据", "frame_dlc", "frame_dlc()"),
    ("帧数据", "frame_id", "frame_id()"),
    ("定时器", "set_timer", "set_timer(\"name\", 100)"),
    ("定时器", "cancel_timer", "cancel_timer()"),
    ("定时器", "set_period", "set_period(100)"),
    ("波形", "ramp", "ramp(0, 255, 1000)"),
    ("波形", "sine_wave", "sine_wave(0, 255, 1000)"),
    ("波形", "triangle", "triangle(0, 255, 1000)"),
    ("波形", "square", "square(0, 255, 1000)"),
    ("波形", "counter", "counter(0, 255, 1000)"),
    ("波形", "random", "random(0, 255)"),
    ("缓冲/数组", "bytes", "bytes(8)"),
    ("缓冲/数组", "array", "array(4)"),
    ("缓冲/数组", "len", "len(v)"),
    ("数学", "abs", "abs(x)"),
    ("数学", "floor", "floor(x)"),
    ("数学", "ceil", "ceil(x)"),
    ("数学", "round", "round(x)"),
    ("数学", "sin", "sin(x)"),
    ("数学", "cos", "cos(x)"),
    ("数学", "min", "min(a, b)"),
    ("数学", "max", "max(a, b)"),
    ("数学", "clamp", "clamp(v, lo, hi)"),
    ("位运算", "bit_and", "bit_and(a, b)"),
    ("位运算", "bit_or", "bit_or(a, b)"),
    ("位运算", "bit_xor", "bit_xor(a, b)"),
    ("位运算", "bit_not", "bit_not(a)"),
    ("位运算", "bit_shl", "bit_shl(a, b)"),
    ("位运算", "bit_shr", "bit_shr(a, b)"),
    ("其他", "now", "now()"),
    ("其他", "srand", "srand(seed)"),
];

/// The sidebar: four tabs. 函数 lists the insertable templates; 大纲 is
/// the draft's handler outline; 收发 is the static send/receive/sysvar
/// fact table; SysVar lists the defined variables for one-click access.
fn sidebar(app: &mut App, ui: &Ui, id: u64, node: &crate::bus::NodeView) {
    let Some(_bar) = ui.tab_bar(format!("##esbtab{id}")) else {
        return;
    };
    if let Some(_t) = ui.tab_item("函数") {
        fns_tab(app, ui, id);
    }
    if let Some(_t) = ui.tab_item("大纲") {
        outline_tab(app, ui, id);
    }
    if let Some(_t) = ui.tab_item("收发") {
        io_tab(app, ui, id, node);
    }
    if let Some(_t) = ui.tab_item("SysVar") {
        sysvar_tab(app, ui, id);
    }
}

/// Inserts a template at the editor's tracked cursor (replacing the
/// selection, if any), falling back to the end of the draft when the
/// editor was never touched. The snippet always lands on its own line.
fn insert(app: &mut App, id: u64, text: &str) {
    let cur = app.editor_cursors.get(&id).copied();
    let draft = app.node_src_draft.entry(id).or_default();

    // Clamp onto char boundaries; imgui reports byte offsets, but a
    // stale record could point into the middle of a multi-byte char.
    let at_boundary = |s: &str, mut i: usize| {
        while i > 0 && !s.is_char_boundary(i) {
            i -= 1;
        }
        i
    };
    let (start, end) = match cur {
        Some(c) => {
            let a = at_boundary(draft, c.pos.min(draft.len()));
            let (s, e) = match c.sel {
                Some((sel_a, sel_z)) => (
                    at_boundary(draft, sel_a.min(draft.len())),
                    at_boundary(draft, sel_z.min(draft.len())),
                ),
                None => (a, a),
            };
            (s.min(e), e.max(s))
        }
        None => (draft.len(), draft.len()),
    };

    // Own-line guarantee: break before unless we are at a line start
    // (or at the very end of a line-terminated draft), break after
    // unless the snippet already ends with one.
    let mut snippet = String::new();
    let before = &draft[..start];
    if !before.is_empty() && !before.ends_with('\n') {
        snippet.push('\n');
    }
    snippet.push_str(text);
    if !snippet.ends_with('\n') {
        snippet.push('\n');
    }
    draft.replace_range(start..end, &snippet);
    // The next insert chains right after this one; the scroll is
    // untouched (nothing scrolled).
    let (scroll, hscroll) = cur.map(|c| (c.scroll, c.hscroll)).unwrap_or((0.0, 0.0));
    app.editor_cursors.insert(
        id,
        EditorCursor {
            pos: start + snippet.len(),
            sel: None,
            scroll,
            hscroll,
        },
    );
}

/// The insertable templates: categories with clickable items.
fn fns_tab(app: &mut App, ui: &Ui, id: u64) {
    let mut last_cat = "";
    for (cat, label, insert_text) in SIDEBAR_ITEMS {
        if *cat != last_cat {
            if !last_cat.is_empty() {
                ui.separator();
            }
            ui.text_disabled(*cat);
            last_cat = *cat;
        }
        if ui.selectable_config(*label).build() {
            insert(app, id, insert_text);
        }
    }

    // DBC-aware section: the bound node's messages and signals.
    let dbc_items = owned_dbc_items(app, id);
    if !dbc_items.is_empty() {
        ui.separator();
        ui.text_disabled("DBC 报文/信号");
        for (msg_id, ext, msg_name, sigs) in &dbc_items {
            let id_str = format!("{msg_id:03X}{}", if *ext { "x" } else { "" });
            if ui
                .selectable_config(format!("{} {}##dbcmsg{}", msg_name, id_str, msg_id))
                .build()
            {
                insert(app, id, &format!("send({:#x});", msg_id));
            }
            for (sig_name, _) in sigs {
                if ui
                    .selectable_config(format!("  {}##dbcsig{}_{}", sig_name, msg_id, sig_name))
                    .build()
                {
                    insert(app, id, &format!("sig({:#x}, \"{}\")", msg_id, sig_name));
                }
            }
        }
    }
}

/// `(bus, node)` binding of the editor's node, if any.
fn binding_of(app: &App, id: u64) -> Option<(u8, String)> {
    let n = app.snap.nodes.iter().find(|n| n.id == id)?;
    n.attached.clone()
}

/// The bound node's own DBC messages: `(id, ext, name, signals)`.
type DbcItem = (u32, bool, String, Vec<(String, u64)>);

fn owned_dbc_items(app: &App, id: u64) -> Vec<DbcItem> {
    let Some((bus, attached_node)) = binding_of(app, id) else {
        return Vec::new();
    };
    let Some(db) = app.channel_dbc(bus) else {
        return Vec::new();
    };
    db.order
        .iter()
        .filter_map(|&(id, ext)| {
            let m = db.messages.get(&(id, ext))?;
            if m.transmitter != attached_node {
                return None;
            }
            let sigs: Vec<(String, u64)> =
                m.signals.iter().map(|s| (s.name.clone(), s.start_bit)).collect();
            Some((id, ext, m.name.clone(), sigs))
        })
        .collect()
}

/// The draft's handler outline: every event handler and function with
/// its line number. Works on half-typed source; a compile error shows
/// alongside so the outline degrades gracefully.
fn outline_tab(app: &mut App, ui: &Ui, id: u64) {
    let facts = facts_for(app, id);
    if let Some(e) = &facts.error {
        ui.text_colored([1.0, 0.55, 0.3, 1.0], "草稿未编译通过");
        if ui.is_item_hovered() {
            ui.tooltip_text(e);
        }
    }
    if facts.outline.is_empty() {
        ui.text_disabled("（尚无处理器）");
        return;
    }
    for (line, label) in &facts.outline {
        ui.selectable_config(format!("{label}##ol{line}")).build();
        if ui.is_item_hovered() {
            ui.tooltip_text(format!("第 {line} 行"));
        }
    }
}

/// The static fact table: what this script sends, what it listens for,
/// and which system variables it touches. Sends are marked against the
/// DBC's transmitter declaration when the script is bound: `*` own,
/// `!` foreign, `-` unknown.
fn io_tab(app: &mut App, ui: &Ui, id: u64, node: &crate::bus::NodeView) {
    let facts = facts_for(app, id);
    if let Some(e) = &facts.error {
        ui.text_colored([1.0, 0.55, 0.3, 1.0], "草稿未编译通过");
        if ui.is_item_hovered() {
            ui.tooltip_text(e);
        }
        return;
    }

    ui.text_disabled(format!("发送（{}）", facts.sends.len()));
    let dbc = node
        .attached
        .as_ref()
        .and_then(|(bus, _)| app.channel_dbc(*bus));
    let attached_node = node.attached.as_ref().map(|(_, n)| n.clone());
    if facts.sends.is_empty() {
        ui.text_disabled("  （无）");
    }
    for (msg_id, ext, from) in &facts.sends {
        let mark = match (&dbc, &attached_node) {
            (Some(db), Some(owner)) => match db.messages.get(&(*msg_id, *ext)) {
                Some(m) if &m.transmitter == owner => "*",
                Some(_) => "!",
                None => "-",
            },
            _ => "-",
        };
        ui.text(format!("{mark} 0x{msg_id:03X}{}", if *ext { "x" } else { "" }));
        if ui.is_item_hovered() {
            ui.tooltip_text(format!("来自 {from}"));
        }
    }

    // Reverse check mirrored in place: what the node's DBC declares but
    // the draft never sends (the start log reports the same as [info]).
    if let (Some(db), Some(owner)) = (&dbc, &attached_node)
        && !facts.recv_wildcard
    {
        let mut missing: Vec<(u32, bool, &str)> = db
            .messages
            .iter()
            .filter(|(k, m)| {
                m.transmitter == *owner
                    && !facts.sends.iter().any(|(id, ext, _)| (*id, *ext) == **k)
            })
            .map(|(k, m)| (k.0, k.1, m.name.as_str()))
            .collect();
        missing.sort();
        if !missing.is_empty() {
            ui.separator();
            ui.text_disabled(format!("未实现（{}）", missing.len()));
            for (msg_id, ext, name) in &missing {
                ui.text(format!(
                    "  0x{msg_id:03X}{} {name}",
                    if *ext { "x" } else { "" }
                ));
            }
            if ui.is_item_hovered() {
                ui.tooltip_text("可能由生成器代发；启动日志有同款 [info]");
            }
        }
    }

    ui.separator();
    ui.text_disabled(format!("接收（{}）", facts.recvs.len() + usize::from(facts.recv_wildcard)));
    if facts.recv_wildcard {
        ui.text("  *  所有帧");
    }
    if facts.recvs.is_empty() && !facts.recv_wildcard {
        ui.text_disabled("  （无）");
    }
    for (msg_id, ext) in &facts.recvs {
        ui.text(format!("  0x{msg_id:03X}{}", if *ext { "x" } else { "" }));
    }

    ui.separator();
    ui.text_disabled(format!("系统变量（{}）", facts.sysvars.len()));
    if facts.sysvars.is_empty() {
        ui.text_disabled("  （无）");
    }
    for key in &facts.sysvars {
        ui.text(format!("  {key}"));
    }

    ui.separator();
    ui.text_disabled(format!("响应映射（{}）", facts.responses.len()));
    if facts.responses.is_empty() {
        ui.text_disabled("  （无未被武装的单次定时器）");
    }
    for row in &facts.responses {
        let replies = row
            .sends
            .iter()
            .map(|(id, ext)| format!("{id:#X}{}", if *ext { "x" } else { "" }))
            .collect::<Vec<_>>()
            .join(", ");
        ui.text(format!("  {} -> {replies}", row.timer_label));
        if ui.is_item_hovered() {
            ui.tooltip_text(format!("由 {} 武装", row.armed_by.join(", ")));
        }
    }
}

/// The defined system variables, grouped by namespace: 读 inserts a
/// `sys_get`, 写 inserts a `sys_set` template.
fn sysvar_tab(app: &mut App, ui: &Ui, id: u64) {
    let mut vars: Vec<(String, Vec<(String, bool)>)> = Vec::new(); // (ns, [(name, has_bounds)])
    for v in &app.snap.sysvars {
        match vars.iter_mut().find(|(ns, _)| *ns == v.def.namespace) {
            Some((_, list)) => list.push((v.def.name.clone(), v.def.min.is_some() || v.def.max.is_some())),
            None => vars.push((
                v.def.namespace.clone(),
                vec![(v.def.name.clone(), v.def.min.is_some() || v.def.max.is_some())],
            )),
        }
    }
    if vars.is_empty() {
        ui.text_disabled("（尚未定义）");
        ui.text_disabled("View > System Variables 中管理");
        return;
    }
    for (ns, names) in &vars {
        ui.text_disabled(ns);
        for (name, _) in names {
            if ui.selectable_config(format!("读  {name}##svr{ns}{name}")).build() {
                insert(app, id, &format!("sys_get(\"{ns}::{name}\")"));
            }
            if ui.selectable_config(format!("写  {name}##svw{ns}{name}")).build() {
                insert(app, id, &format!("sys_set(\"{ns}::{name}\", 0)"));
            }
        }
    }
}

#[cfg(test)]
mod facts_tests {
    use super::*;

    /// The response mapping rides the facts: a one-shot timer someone
    /// actually arms, with its replies. A timer nobody arms is not a
    /// response and does not appear.
    #[test]
    fn facts_carry_the_response_mapping() {
        let src = r#"
            on message 0x100 { set_timer("resp", 200); }
            on timer "resp" { send(0x200); }
            on timer "idle" { send(0x300); }
        "#;
        let f = compute_facts(src, 7);
        assert_eq!(f.responses.len(), 1, "only the armed timer maps: {:?}", f.responses);
        let row = &f.responses[0];
        assert_eq!(row.timer_label, "<on timer \"resp\">");
        assert!(
            row.armed_by.iter().any(|a| a.contains("0x100")),
            "the arming handler is recorded: {:?}",
            row.armed_by
        );
        assert_eq!(row.sends, &[(0x200, false)]);
    }

    /// The compile error's line rides the facts for the gutter mark.
    #[test]
    fn facts_carry_the_error_line() {
        let ok = compute_facts("on start { }", 1);
        assert_eq!(ok.error_line, None);
        let bad = compute_facts("on start {\n    send();\n}", 2);
        assert_eq!(bad.error_line, Some(2), "the send line is the offender");
    }

    /// Classification mirrors the lexer's shapes: keywords, numbers
    /// (including hex), strings with escapes, // and /* */ comments --
    /// and multi-byte characters stay plain without breaking alignment.
    #[test]
    fn classification_covers_keywords_numbers_strings_comments() {
        let src = concat!(
            "on start {\n",                        // 1: keyword
            "    // line note\n",                  // 2: comment
            "    let id = 0x1F4 + 7;\n",           // 3: keyword + numbers
            "    /* block\n",                      // 4: comment start
            "    still */ print(\"a\\\"b\");\n",   // 5: comment tail + string
            "    sig(0x100, \"转速\");\n",         // 6: number + string with CJK
            "}\n",                                 // 7: keyword? no: plain brace
        );
        let classes = classify_lines(src);
        assert_eq!(classes.len(), 8, "trailing newline makes an empty last line");

        let kw = |row: &[u8], i: usize| row[i] == KEYWORD;
        assert!(kw(&classes[0], 0) && kw(&classes[0], 1) && !kw(&classes[0], 3));

        assert!(
            classes[1].iter().skip(4).all(|&c| c == COMMENT),
            "// to EOL (leading spaces stay plain)"
        );

        assert!(kw(&classes[2], 4), "let");
        assert_eq!(classes[2][13], NUMBER, "0x1F4 all classified");
        assert_eq!(classes[2][21], NUMBER, "the 7 too");

        assert!(
            classes[3].iter().skip(4).all(|&c| c == COMMENT),
            "block opener from the slash on"
        );
        assert!(classes[4].iter().take(9).all(|&c| c == COMMENT), "block tail");
        assert!(
            classes[4].iter().skip(19).take(6).all(|&c| c == STRING),
            "escaped string incl. quotes"
        );

        assert!(classes[5].iter().any(|&c| c == NUMBER), "hex id");
        assert!(classes[5].iter().filter(|&&c| c == STRING).count() >= 4, "CJK string");
    }
}
