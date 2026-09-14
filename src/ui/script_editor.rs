//! Per-node script editors: one window per node being edited, opened on
//! demand from the Entities table or the Network view. There is no
//! aggregate "all scripts" window -- a node's script belongs to the node,
//! and the directory (Entities) is where you decide which one to open.
//!
//! The source editor is the dear-imgui-cte widget (ImGuiColorTextEdit):
//! real syntax highlighting, line numbers, find/replace and error markers
//! are the widget's own, replacing the covered-and-repainted multiline
//! the tool shipped with before the dear-imgui-rs migration.

use crate::app::App;
use dear_imgui_cte::{CteUiExt, Position, ScrollAlignment, Selection};
use dear_imgui_rs::{Condition, TreeNodeFlags, Ui};
use std::hash::{Hash, Hasher};

const SOURCE_HEIGHT: f32 = 260.0;
const LOG_LINES: usize = 10;

/// Compile-error marker colors, packed ABGR (the native ImColor32
/// layout). The second color fills the whole line opaquely and doubles
/// as the hover-tooltip background, so it stays a dark, desaturated red
/// that ordinary syntax colors read on; the line number itself carries
/// the loud red.
const MARKER_LINE_NUMBER: u32 = 0xFF_58_50_E8;
const MARKER_LINE_FILL: u32 = 0xFF_20_1A_60;

/// Applies the editor defaults to a freshly created CTE editor: C
/// shaping (our language is C-flavoured -- `//` and `/* */` comments,
/// hex literals, and every control-flow keyword match), line numbers,
/// whitespace dots, four-space tabs. Lives here because the editor must
/// be created outside the frame's `Ui` borrow -- main (and the headless
/// harness) call it right after construction. Lua was tried first and
/// mangled comments: its single-line token is `--`, so `//` text was
/// plain punctuation and quotes inside comments flipped string state
/// across whole lines.
pub fn configure_new_editor(editor: &mut dear_imgui_cte::TextEditor) {
    editor.set_language(Some(dear_imgui_cte::Language::C));
    editor.set_show_line_numbers(true);
    editor.set_show_whitespaces(true);
    editor.set_auto_indent_enabled(true);
    let _ = editor.set_tab_size(4);
}

/// The script language keywords, for the autocomplete vocabulary.
const KEYWORDS: [&str; 15] = [
    "on", "fn", "if", "else", "while", "for", "return", "break", "continue", "switch", "case",
    "default", "true", "false", "let",
];

/// The autocomplete vocabulary for one node's editor: language keywords,
/// every builtin function name, and the message/signal names declared on
/// the node's bus. Snapshotted once when the editor is created -- the
/// completion callback is `'static` and cannot reach back into the app.
pub fn autocomplete_vocabulary(app: &App, channel: u8) -> Vec<String> {
    let mut words: Vec<String> = Vec::new();
    words.extend(KEYWORDS.iter().map(|w| (*w).to_string()));
    for (_, label, _) in SIDEBAR_ITEMS {
        words.push((*label).to_string());
    }
    if let Some(db) = app.channel_dbc(channel) {
        for &(id, ext) in &db.order {
            if let Some(m) = db.messages.get(&(id, ext)) {
                words.push(m.name.clone());
                for s in &m.signals {
                    words.push(s.name.clone());
                }
            }
        }
    }
    words.sort();
    words.dedup();
    words
}

/// Installs typing autocomplete fed from `words` (matched
/// case-insensitively on the identifier prefix, top 12 suggestions).
pub fn install_autocomplete(editor: &mut dear_imgui_cte::TextEditor, words: Vec<String>) {
    use dear_imgui_cte::AutocompleteConfig;
    let config = AutocompleteConfig::new();
    let _ = editor.set_autocomplete(&config, move |request| {
        let term = request.search_term().unwrap_or_default();
        if term.len() < 2 {
            return;
        }
        let needle = term.to_ascii_lowercase();
        let matches: Vec<String> = words
            .iter()
            .filter(|w| w.len() != term.len() && w.to_ascii_lowercase().starts_with(&needle))
            .take(12)
            .cloned()
            .collect();
        let _ = request.set_suggestions(matches);
    });
}

/// Static facts of one editor's text: the handler outline and the
/// compile-derived send/receive/system-variable sets. Recomputed only
/// when the text's hash changes, not per frame.
#[derive(Clone, Debug, Default)]
pub(crate) struct EditorFacts {
    pub hash: u64,
    /// `(line, label)` in source order.
    pub outline: Vec<(u32, String)>,
    /// `(id, extended, arming handler)` from the compiler's send set.
    pub sends: Vec<(u32, bool, String)>,
    /// `(id, extended)` the handlers listen for.
    pub recvs: Vec<(u32, bool)>,
    pub recv_wildcard: bool,
    /// `$Message::Signal` reads, as `(message, signal)` name pairs.
    pub named_refs: Vec<(String, String)>,
    /// The wildcard handler itself transmits: the self-excitation hint
    /// rides on this.
    pub wildcard_sends: bool,
    /// `ns::name` keys this script accesses.
    pub sysvars: Vec<String>,
    /// Response mapping: one row per armed one-shot timer.
    pub responses: Vec<ResponseRowLite>,
    /// Source line (1-based) of the compile error, for the editor mark.
    pub error_line: Option<u32>,
    /// Compile error, when the text no longer compiles.
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

/// Derives the static facts of a source text. The outline comes from a
/// line scan (it must work on broken, half-typed source); the sets come
/// from the compiler and are absent when it fails.
fn compute_facts(src: &str, hash: u64) -> EditorFacts {
    let mut facts = EditorFacts {
        hash,
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
            facts.wildcard_sends = script.wildcard_sends();
            for (msg, sig) in &script.named_signal_refs {
                if !facts.named_refs.contains(&(msg.clone(), sig.clone())) {
                    facts.named_refs.push((msg.clone(), sig.clone()));
                }
            }
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

/// Cached facts for an editor's text, recomputed when the text changed
/// (the widget reports `changed`) or never derived before.
fn facts_for(app: &mut App, id: u64, changed: bool) -> EditorFacts {
    let missing = !app.editor_facts.contains_key(&id);
    if (changed || missing)
        && let Some(editor) = app.editors.get(&id)
    {
        let src = editor.text().unwrap_or_default();
        let mut h = std::collections::hash_map::DefaultHasher::new();
        src.hash(&mut h);
        let facts = compute_facts(&src, h.finish());
        app.editor_facts.insert(id, facts.clone());
    }
    app.editor_facts.get(&id).cloned().unwrap_or_default()
}

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
        .size([640.0, 500.0], Condition::FirstUseEver)
        .build(|| content(app, ui, node));
    if !open {
        app.close_script_editor(id);
    }
}

/// Seeds the editor from the node's source when the model moved under it
/// (project load, Apply, a fresh Load). The editor owns the text while
/// being edited; this only reconciles external changes.
fn sync_from_model(app: &mut App, id: u64, source: &str) {
    let stale = app.editor_synced.get(&id).is_none_or(|s| s != source);
    if !stale {
        return;
    }
    if let Some(editor) = app.editors.get_mut(&id) {
        let _ = editor.set_text(source);
        app.editor_synced.insert(id, source.to_string());
        app.editor_facts.remove(&id);
    }
}

fn content(app: &mut App, ui: &Ui, node: &crate::bus::NodeView) {
    let id = node.id;

    // The editor is created by main, outside the frame's Ui borrow; the
    // first frame(s) show the sidebar alone until it exists.
    if !app.editors.contains_key(&id) && !app.pending_editors.contains(&id) {
        app.pending_editors.push(id);
    }
    sync_from_model(app, id, &node.source);

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
    let mut enabled = node.enabled;
    if ui.checkbox(format!("运行##een{id}"), &mut enabled) {
        app.send(crate::bus::BusCommand::SetNodeEnabled { id, on: enabled });
    }
    ui.same_line();
    if ui.button(format!("删除##erm{id}")) {
        app.send(crate::bus::BusCommand::RemoveNode { id });
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
        .build(ui, || sidebar(app, ui, id, node));

    ui.same_line();

    // Right main area: source + Apply/Save/Load + log.
    ui.child_window(format!("##emain{id}"))
        .size([0.0, avail[1]])
        .build(ui, || {
            // The widget reports whether its text changed this frame;
            // only then do the facts need re-deriving.
            let changed = match app.editors.get_mut(&id) {
                Some(editor) => {
                    let w = ui.content_region_avail()[0].max(1.0);
                    match ui
                        .text_editor(editor, format!("##esrc{id}"))
                        .size([w, SOURCE_HEIGHT])
                        .build()
                    {
                        Ok(changed) => changed,
                        Err(e) => {
                            eprintln!("text_editor build error: {e}");
                            false
                        }
                    }
                }
                None => false,
            };
            let facts = facts_for(app, id, changed);

            // The compile-error line gets the editor's own marker (a red
            // line highlight plus a gutter mark, both with the tooltip).
            // Refreshed every frame: markers are frame state, not layout
            // -- and the clear runs unconditionally, so the band lifts
            // the moment the draft compiles again.
            if let Some(editor) = app.editors.get_mut(&id) {
                editor.clear_markers();
                if let Some(line) = facts.error_line {
                    let tip = facts.error.clone().unwrap_or_default();
                    let _ = editor.add_marker(
                        (line - 1) as usize,
                        MARKER_LINE_NUMBER,
                        MARKER_LINE_FILL,
                        "编译错误",
                        &tip,
                    );
                }
            }

            // Apply, then Save/Load. The 未应用 tag reads the facts hash
            // against the model's source hash: the facts cache is fresh
            // whenever the text changed, so the tag tracks live state.
            let mut src_hash = std::collections::hash_map::DefaultHasher::new();
            node.source.hash(&mut src_hash);
            let unsaved = facts.hash != src_hash.finish();
            if ui.button(format!("Apply##eapply{id}"))
                && let Some(editor) = app.editors.get(&id)
            {
                let source = editor.text().unwrap_or_default();
                app.editor_synced.insert(id, source.clone());
                app.send(crate::bus::BusCommand::SetNodeSource { id, source });
            }
            if unsaved {
                ui.same_line();
                ui.text_colored([1.0, 0.8, 0.4, 1.0], "未应用");
            }
            ui.same_line();
            if ui.button(format!("保存##esave{id}"))
                && let Some(editor) = app.editors.get(&id)
            {
                let source = editor.text().unwrap_or_default();
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
                            if let Some(editor) = app.editors.get_mut(&id) {
                                let _ = editor.set_text(&src);
                            }
                            app.editor_synced.insert(id, src);
                            app.editor_facts.remove(&id);
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
                    .build(ui, || {
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
    ("控制流", "switch/case", "switch (value) {\n    case 1: {\n        \n    }\n    case 2: {\n        \n    }\n    default: {\n        \n    }\n}"),
    ("总线控制", "send", "send(0x000, 0x00);"),
    ("总线控制", "send_ext", "send_ext(0x000, 0x00);"),
    ("总线控制", "sig", "sig(0x000, \"Signal\")"),
    ("总线控制", "$信号", "$Message::Signal"),
    ("总线控制", "set_sig", "set_sig(buf, 0x000, \"Signal\", 0)"),
    ("总线控制", "emit_value", "emit_value(\"Name\", 0)"),
    ("系统变量", "sys_get", "sys_get(\"ns::name\")"),
    ("系统变量", "sys_set", "sys_set(\"ns::name\", 0)"),
    ("系统变量", "@sysvar", "@sysvar::ns::name"),
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
/// the source's handler outline; 收发 is the static send/receive/sysvar
/// fact table; SysVar lists the defined variables for one-click access.
/// Each tab's content scrolls in a child of its own so the tab bar stays
/// pinned at the top no matter how long the content grows.
fn sidebar(app: &mut App, ui: &Ui, id: u64, node: &crate::bus::NodeView) {
    let Some(_bar) = ui.tab_bar(format!("##esbtab{id}")) else {
        return;
    };
    if let Some(_t) = ui.tab_item("函数") {
        ui.child_window(format!("##esbscroll{id}f"))
            .size([0.0, 0.0])
            .build(ui, || fns_tab(app, ui, id, node));
    }
    if let Some(_t) = ui.tab_item("大纲") {
        ui.child_window(format!("##esbscroll{id}o"))
            .size([0.0, 0.0])
            .build(ui, || outline_tab(app, ui, id));
    }
    if let Some(_t) = ui.tab_item("收发") {
        ui.child_window(format!("##esbscroll{id}i"))
            .size([0.0, 0.0])
            .build(ui, || io_tab(app, ui, id, node));
    }
    if let Some(_t) = ui.tab_item("SysVar") {
        ui.child_window(format!("##esbscroll{id}s"))
            .size([0.0, 0.0])
            .build(ui, || sysvar_tab(app, ui, id));
    }
}

/// Inserts a template at the editor's own cursor (CTE tracks it, so a
/// click in the sidebar never throws the text at the file end), replacing
/// the selection when one is active. The snippet always lands on its own
/// line, and the cursor parks after it so the next insert chains on.
fn insert(app: &mut App, id: u64, text: &str) {
    let Some(editor) = app.editors.get_mut(&id) else {
        return;
    };
    let at = editor.main_cursor_position();
    // Own-line guarantee: break before unless the cursor opens a line
    // (the line's text before the cursor is empty), break after unless
    // the snippet already ends with one.
    let before = editor
        .line_text(at.line)
        .map(|line| {
            let cut = line.chars().count().min(at.column);
            line.chars().take(cut).collect::<String>()
        })
        .unwrap_or_default();
    let mut snippet = String::new();
    if !before.is_empty() {
        snippet.push('\n');
    }
    snippet.push_str(text);
    if !snippet.ends_with('\n') {
        snippet.push('\n');
    }
    if editor
        .replace_section(Selection::new(at, at), &snippet)
        .is_err()
    {
        return;
    }
    // Park the cursor at the start of the line after the snippet, where
    // the text after it now begins.
    let new_line = at.line + snippet.matches('\n').count();
    let _ = editor.set_cursor(Position::new(new_line, 0));
}

/// The insertable templates: categories with clickable items.
fn fns_tab(app: &mut App, ui: &Ui, id: u64, node: &crate::bus::NodeView) {
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

    // DBC-aware section: the bound node's own messages and signals.
    let dbc_items = owned_dbc_items(app, id);
    if !dbc_items.is_empty() {
        ui.separator();
        ui.text_disabled("本节点报文/信号");
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

    // The whole bus: every message/signal the bus's databases declare --
    // a script may read (`sig`) or send (`send`) anything on its wire.
    // Collapsed by default: a bus can declare hundreds of rows.
    let bus_items = bus_dbc_items(app, node.channel);
    if !bus_items.is_empty() {
        ui.separator();
        let open = ui.collapsing_header(
            format!(
                "总线全部报文（{}）##busall{id}",
                bus_items.len()
            ),
            TreeNodeFlags::empty(),
        );
        if open {
            for (msg_id, ext, msg_name, sigs) in &bus_items {
                let id_str = format!("{msg_id:03X}{}", if *ext { "x" } else { "" });
                if ui
                    .selectable_config(format!(
                        "{} {}##busmsg{id}_{msg_id}_{}",
                        msg_name,
                        id_str,
                        *ext as u8
                    ))
                    .build()
                {
                    insert(app, id, &format!("send({:#x});", msg_id));
                }
                for (sig_name, _) in sigs {
                    if ui
                        .selectable_config(format!(
                            "  {}##bussig{id}_{msg_id}_{}_{}",
                            sig_name,
                            *ext as u8,
                            sig_name
                        ))
                        .build()
                    {
                        insert(app, id, &format!("sig({:#x}, \"{}\")", msg_id, sig_name));
                    }
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

/// Every message the node's bus's databases declare (any transmitter):
/// `(id, ext, name, signals)` in DBC order.
fn bus_dbc_items(app: &App, ch: u8) -> Vec<DbcItem> {
    let Some(db) = app.channel_dbc(ch) else {
        return Vec::new();
    };
    db.order
        .iter()
        .filter_map(|&(id, ext)| {
            let m = db.messages.get(&(id, ext))?;
            let sigs: Vec<(String, u64)> =
                m.signals.iter().map(|s| (s.name.clone(), s.start_bit)).collect();
            Some((id, ext, m.name.clone(), sigs))
        })
        .collect()
}

/// The source's handler outline: every event handler and function with
/// its line number. Works on half-typed source; a compile error shows
/// alongside so the outline degrades gracefully.
fn outline_tab(app: &mut App, ui: &Ui, id: u64) {
    let facts = facts_for(app, id, false);
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
        if ui.selectable_config(format!("{label}##ol{line}")).build()
            && let Some(editor) = app.editors.get_mut(&id)
        {
            // Jump to the handler's line, centered in the viewport.
            let target = *line as usize - 1;
            let _ = editor.set_cursor(Position::new(target, 0));
            let _ = editor.scroll_to_line(target, ScrollAlignment::Middle);
        }
        if ui.is_item_hovered() {
            ui.tooltip_text(format!("第 {line} 行，点击跳转"));
        }
    }
}

/// The static fact table: what this script sends, what it listens for,
/// and which system variables it touches. Sends are marked against the
/// DBC's transmitter declaration when the script is bound: `*` own,
/// `!` foreign, `-` unknown.
fn io_tab(app: &mut App, ui: &Ui, id: u64, node: &crate::bus::NodeView) {
    let facts = facts_for(app, id, false);
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
        ui.text("  *  所有帧（转发者/记录者）");
        if ui.is_item_hovered() {
            ui.tooltip_text("发送集 = 接收集 ∩ DBC 声明；启动日志有同款 [info]");
        }
        if facts.wildcard_sends {
            ui.text_colored(
                [1.0, 0.75, 0.3, 1.0],
                "    通配 handler 内有发送：发出的帧会再次进入本 handler（自激成环风险）",
            );
            if ui.is_item_hovered() {
                ui.tooltip_text(
                    "转发前用 frame_id() 排除自己的报文（参考 examples/sniffer.rxcan）",
                );
            }
        }
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

    // `$Message::Signal` reads, marked against the DBC like the send set:
    // `*` the pair resolves to a real message+signal, `-` it does not.
    if !facts.named_refs.is_empty() {
        ui.separator();
        ui.text_disabled(format!("$速记（{}）", facts.named_refs.len()));
        for (msg, sig) in &facts.named_refs {
            let known = dbc.as_ref().is_some_and(|db| {
                db.message_id_by_name(msg)
                    .is_some_and(|id| db.message_of(id).is_some_and(|m| m.signals.iter().any(|s| &s.name == sig)))
            });
            let mark = if known { "*" } else { "-" };
            ui.text(format!("  {mark} ${msg}::{sig}"));
            if ui.is_item_hovered() && !known {
                ui.tooltip_text("报文名或信号名不在 DBC 中——启动检查会报 [check]");
            }
        }
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

    /// The compile error's line rides the facts for the editor mark.
    #[test]
    fn facts_carry_the_error_line() {
        let ok = compute_facts("on start { }", 1);
        assert_eq!(ok.error_line, None);
        let bad = compute_facts("on start {\n    send();\n}", 2);
        assert_eq!(bad.error_line, Some(2), "the send line is the offender");
    }
}
