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
use dear_imgui_rs::{Condition, Ui};
use std::collections::HashMap;
use std::hash::{Hash, Hasher};

const SOURCE_HEIGHT: f32 = 260.0;
const LOG_LINES: usize = 10;

/// The script windows' own state: which nodes have an editor open, the CTE
/// widgets (bound to the ImGui context), and the per-editor text/facts
/// bookkeeping. Kept out of `App` so the draw path only reads the application
/// and emits [`ScriptCmd`]s (see `AGENTS.md`).
#[derive(Default)]
pub struct ScriptEditorView {
    /// Node ids with an editor open, in open order.
    pub open: Vec<u64>,
    /// CTE editors, keyed by node id. Main creates them outside the frame's
    /// `Ui` borrow; the draw queues an id in `pending` for it.
    pub editors: HashMap<u64, dear_imgui_cte::TextEditor>,
    /// Node ids whose editor the UI asked for; main drains this queue.
    pub pending: Vec<u64>,
    /// The `node.source` each editor was last seeded with.
    pub(crate) synced: HashMap<u64, String>,
    /// Per-editor static facts (outline, send/receive/sysvar sets).
    pub(crate) facts: HashMap<u64, EditorFacts>,
}

impl ScriptEditorView {
    pub fn open(&mut self, id: u64) {
        if !self.open.contains(&id) {
            self.open.push(id);
        }
    }

    pub fn close(&mut self, id: u64) {
        self.open.retain(|&x| x != id);
        self.editors.remove(&id);
        self.synced.remove(&id);
        self.facts.remove(&id);
    }

    /// Drops the context-bound widgets when the render context goes away.
    pub fn clear_editors(&mut self) {
        self.editors.clear();
    }
}

/// What a script window's controls ask the application to change. Editor
/// text, cursor moves and palette inserts are view state and stay in the
/// draw; anything that touches the node model or writes a file is a command.
pub enum ScriptCmd {
    SetName { id: u64, name: String },
    SetEnabled { id: u64, on: bool },
    Remove { id: u64 },
    Apply { id: u64, source: String },
    Save { path: String, source: String },
    Status(String),
}

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
                    // The CANoe-style read sugar completes as a whole:
                    // typing `$` or `$Engine` suggests the full path.
                    words.push(format!("${}::{}", m.name, s.name));
                }
            }
        }
    }
    // Defined system variables, both the plain key (for sys_get/sys_set)
    // and the `@sysvar::` sugar form.
    for v in &app.snap.sysvars {
        let key = format!("{}::{}", v.def.namespace, v.def.name);
        words.push(key.clone());
        words.push(format!("@sysvar::{key}"));
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
        const PREFIXES: [&str; 7] = [
            // Extended before plain: both are `starts_with` matches.
            "on extended message",
            "on message",
            "on timer",
            "on start",
            "on errorFrame",
            "on fr",
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
fn facts_for(view: &mut ScriptEditorView, id: u64, changed: bool) -> EditorFacts {
    let missing = !view.facts.contains_key(&id);
    if (changed || missing)
        && let Some(editor) = view.editors.get(&id)
    {
        let src = editor.text().unwrap_or_default();
        let mut h = std::collections::hash_map::DefaultHasher::new();
        src.hash(&mut h);
        let facts = compute_facts(&src, h.finish());
        view.facts.insert(id, facts.clone());
    }
    view.facts.get(&id).cloned().unwrap_or_default()
}

pub fn render(app: &mut App, ui: &Ui) {
    let mut view = std::mem::take(&mut app.script_editor);
    let mut cmds = Vec::new();
    let ids = view.open.clone();
    for id in ids {
        let Some(node) = app.snap.nodes.iter().find(|n| n.id == id).cloned() else {
            // The node went away (deleted, project replaced): its editor
            // has nothing left to edit.
            view.close(id);
            continue;
        };
        editor_window(&mut view, app, ui, &node, &mut cmds);
    }
    app.script_editor = view;
    app.apply_script_cmds(cmds);
}

fn editor_window(
    view: &mut ScriptEditorView,
    app: &App,
    ui: &Ui,
    node: &crate::bus::NodeView,
    cmds: &mut Vec<ScriptCmd>,
) {
    let id = node.id;
    // `###` pins the window identity to the node, so a rename moves the
    // title without losing position or open state.
    let mut open = true;
    ui.window(format!("Script · {}###script{}", node.name, id))
        .opened(&mut open)
        .position(
            [
                340.0 + (id as f32 % 6.0) * 28.0,
                80.0 + (id as f32 % 6.0) * 24.0,
            ],
            Condition::FirstUseEver,
        )
        .size([640.0, 500.0], Condition::FirstUseEver)
        .build(|| content(view, app, ui, node, cmds));
    if !open {
        view.close(id);
    }
}

/// Seeds the editor from the node's source when the model moved under it
/// (project load, Apply, a fresh Load). The editor owns the text while
/// being edited; this only reconciles external changes.
fn sync_from_model(view: &mut ScriptEditorView, id: u64, source: &str) {
    let stale = view.synced.get(&id).is_none_or(|s| s != source);
    if !stale {
        return;
    }
    if let Some(editor) = view.editors.get_mut(&id) {
        let _ = editor.set_text(source);
        view.synced.insert(id, source.to_string());
        view.facts.remove(&id);
    }
}

fn content(
    view: &mut ScriptEditorView,
    app: &App,
    ui: &Ui,
    node: &crate::bus::NodeView,
    cmds: &mut Vec<ScriptCmd>,
) {
    let id = node.id;

    // The editor is created by main, outside the frame's Ui borrow; the
    // first frame(s) show the sidebar alone until it exists.
    if !view.editors.contains_key(&id) && !view.pending.contains(&id) {
        view.pending.push(id);
    }
    sync_from_model(view, id, &node.source);

    // Header: node name, channel binding, run switch, delete. The name
    // commits per keystroke -- it is a cheap string write and the
    // Entities table shows it live.
    let mut name = node.name.clone();
    ui.set_next_item_width(140.0);
    if ui.input_text(format!("##ename{id}"), &mut name).build() {
        cmds.push(ScriptCmd::SetName { id, name });
    }
    ui.same_line();
    // The script lives where its node lives: the binding is shown
    // read-only, there is no bus choice anywhere in the flow.
    match &node.attached {
        Some((ach, anode)) => {
            ui.text_disabled(format!("node {anode} · bus {}", app.channel_name(*ach),));
        }
        None => {
            ui.text_colored(
                [1.0, 0.8, 0.4, 1.0],
                format!(
                    "Unbound node (bus {}); once a DBC loads it joins that bus's first node",
                    app.channel_name(node.channel),
                ),
            );
        }
    }
    ui.same_line();
    let mut enabled = node.enabled;
    if ui.checkbox(format!("Run##een{id}"), &mut enabled) {
        cmds.push(ScriptCmd::SetEnabled { id, on: enabled });
    }
    ui.same_line();
    if ui.button(format!("Delete##erm{id}")) {
        cmds.push(ScriptCmd::Remove { id });
        view.close(id);
        return;
    }

    // Status line.
    if node.errored {
        ui.text_colored(
            [1.0, 0.55, 0.3, 1.0],
            "Error (see the log; Apply again or restart the measurement)",
        );
    } else if node.running {
        ui.text_colored([0.4, 0.95, 0.5, 1.0], "Running");
    } else {
        ui.text_disabled("Stopped");
    }

    // Three columns, the CAPL Browser shape: left = the source's outline
    // and its send/receive facts; center = the editor, Apply/Save/Load
    // and the log; right = the insertable palette (functions, SysVars,
    // every message the bus declares). All three fill the remaining
    // height. The center width is explicit: a fill-everything middle
    // child would push the palette past the window edge.
    let avail = ui.content_region_avail();
    const LEFT_W: f32 = 160.0;
    const PALETTE_W: f32 = 190.0;
    let main_w = (avail[0] - LEFT_W - PALETTE_W - 16.0).max(240.0);

    // Left: outline + facts.
    ui.child_window(format!("##eleft{id}"))
        .size([LEFT_W, avail[1]])
        .border(true)
        .build(ui, || left_tabs(app, view, ui, id, node));

    ui.same_line();

    // Center main area: source + Apply/Save/Load + log.
    ui.child_window(format!("##emain{id}"))
        .size([main_w, avail[1]])
        .build(ui, || {
            // The widget reports whether its text changed this frame;
            // only then do the facts need re-deriving.
            let changed = match view.editors.get_mut(&id) {
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
            let facts = facts_for(view, id, changed);

            // The compile-error line gets the editor's own marker (a red
            // line highlight plus a gutter mark, both with the tooltip).
            // Refreshed every frame: markers are frame state, not layout
            // -- and the clear runs unconditionally, so the band lifts
            // the moment the draft compiles again.
            if let Some(editor) = view.editors.get_mut(&id) {
                editor.clear_markers();
                if let Some(line) = facts.error_line {
                    let tip = facts.error.clone().unwrap_or_default();
                    let _ = editor.add_marker(
                        (line - 1) as usize,
                        MARKER_LINE_NUMBER,
                        MARKER_LINE_FILL,
                        "compile error",
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
                && let Some(editor) = view.editors.get(&id)
            {
                let source = editor.text().unwrap_or_default();
                view.synced.insert(id, source.clone());
                cmds.push(ScriptCmd::Apply { id, source });
            }
            if unsaved {
                ui.same_line();
                ui.text_colored([1.0, 0.8, 0.4, 1.0], "Not applied");
            }
            ui.same_line();
            if ui.button(format!("Save##esave{id}"))
                && let Some(editor) = view.editors.get(&id)
            {
                let source = editor.text().unwrap_or_default();
                if let Some(path) = rfd::FileDialog::new()
                    .set_title("Save node script")
                    .add_filter("Node scripts", &["rxcan"])
                    .save_file()
                {
                    cmds.push(ScriptCmd::Save {
                        path: path.to_string_lossy().into_owned(),
                        source,
                    });
                }
            }
            ui.same_line();
            if ui.button(format!("Load##eload{id}")) {
                let picked = rfd::FileDialog::new()
                    .set_title("Load node script")
                    .add_filter("Node scripts", &["rxcan"])
                    .pick_file();
                if let Some(p) = picked {
                    let path = p.to_string_lossy().into_owned();
                    // GBK-encoded scripts (ANSI tooling with Chinese
                    // comments) decode through the same fallback as DBCs.
                    match std::fs::read(&path)
                        .map_err(|e| e.to_string())
                        .map(crate::dbc::text_from_bytes)
                    {
                        Ok(src) => {
                            if let Some(editor) = view.editors.get_mut(&id) {
                                let _ = editor.set_text(&src);
                            }
                            view.synced.insert(id, src);
                            view.facts.remove(&id);
                        }
                        Err(e) => {
                            cmds.push(ScriptCmd::Status(format!("load failed: {e}")));
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

    // Right: the insertable palette.
    ui.same_line();
    ui.child_window(format!("##epalette{id}"))
        .size([PALETTE_W, avail[1]])
        .border(true)
        .build(ui, || palette_tabs(app, view, ui, id, node));
}

/// One insertable sidebar entry: display label and the text that goes
/// into the source when clicked.
const SIDEBAR_ITEMS: &[(&str, &str, &str)] = &[
    // (category, label, insert_text)
    ("Handlers", "on start", "on start {\n    \n}"),
    ("Handlers", "on message", "on message 0x000 {\n    \n}"),
    (
        "Handlers",
        "on ext message",
        "on extended message 0x000 {\n    \n}",
    ),
    ("Handlers", "on message *", "on message * {\n    \n}"),
    ("Handlers", "on errorFrame", "on errorFrame {\n    \n}"),
    ("Handlers", "on timer", "on timer 100 {\n    \n}"),
    (
        "Handlers",
        "on timer \"name\"",
        "on timer \"name\" {\n    \n}",
    ),
    ("Handlers", "on fr slot", "on fr slot 0 {\n    \n}"),
    ("Handlers", "on fr 0 slot", "on fr 0 slot 0 {\n    \n}"),
    (
        "Control flow",
        "switch/case",
        "switch (value) {\n    case 1: {\n        \n    }\n    case 2: {\n        \n    }\n    default: {\n        \n    }\n}",
    ),
    ("Bus access", "send", "send(0x000, 0x00);"),
    ("Bus access", "send_ext", "send_ext(0x000, 0x00);"),
    ("Bus access", "sig", "sig(0x000, \"Signal\")"),
    ("Bus access", "fr_sig", "fr_sig(0, 0, \"Signal\")"),
    ("Bus access", "fr_send", "fr_send(0, 0, 0x00);"),
    (
        "Bus access",
        "set_fr_sig",
        "set_fr_sig(0, 0, \"Signal\", 0)",
    ),
    ("Bus access", "$signal", "$Message::Signal"),
    (
        "Bus access",
        "set_sig",
        "set_sig(buf, 0x000, \"Signal\", 0)",
    ),
    ("Bus access", "emit_value", "emit_value(\"Name\", 0)"),
    ("SysVars", "sys_get", "sys_get(\"ns::name\")"),
    ("SysVars", "sys_set", "sys_set(\"ns::name\", 0)"),
    ("SysVars", "@sysvar", "@sysvar::ns::name"),
    ("Frame data", "frame_byte", "frame_byte(0)"),
    ("Frame data", "frame_dlc", "frame_dlc()"),
    ("Frame data", "frame_id", "frame_id()"),
    ("Frame data", "fr_cycle", "fr_cycle()"),
    ("Timers", "set_timer", "set_timer(\"name\", 100)"),
    ("Timers", "cancel_timer", "cancel_timer()"),
    ("Timers", "set_period", "set_period(100)"),
    ("Waveforms", "ramp", "ramp(0, 255, 1000)"),
    ("Waveforms", "sine_wave", "sine_wave(0, 255, 1000)"),
    ("Waveforms", "triangle", "triangle(0, 255, 1000)"),
    ("Waveforms", "square", "square(0, 255, 1000)"),
    ("Waveforms", "counter", "counter(0, 255, 1000)"),
    ("Waveforms", "random", "random(0, 255)"),
    ("Buffers", "bytes", "bytes(8)"),
    ("Buffers", "array", "array(4)"),
    ("Buffers", "len", "len(v)"),
    ("Math", "abs", "abs(x)"),
    ("Math", "floor", "floor(x)"),
    ("Math", "ceil", "ceil(x)"),
    ("Math", "round", "round(x)"),
    ("Math", "sin", "sin(x)"),
    ("Math", "cos", "cos(x)"),
    ("Math", "min", "min(a, b)"),
    ("Math", "max", "max(a, b)"),
    ("Math", "clamp", "clamp(v, lo, hi)"),
    ("Bitwise", "bit_and", "bit_and(a, b)"),
    ("Bitwise", "bit_or", "bit_or(a, b)"),
    ("Bitwise", "bit_xor", "bit_xor(a, b)"),
    ("Bitwise", "bit_not", "bit_not(a)"),
    ("Bitwise", "bit_shl", "bit_shl(a, b)"),
    ("Bitwise", "bit_shr", "bit_shr(a, b)"),
    ("Other", "now", "now()"),
    ("Other", "format", "format(\"%.0f\", v)"),
    ("Other", "srand", "srand(seed)"),
];

/// The left column: the source's handler outline and the static
/// send/receive/sysvar fact table. Each tab's content scrolls in a child
/// of its own so the tab bar stays pinned at the top.
fn left_tabs(
    app: &App,
    view: &mut ScriptEditorView,
    ui: &Ui,
    id: u64,
    node: &crate::bus::NodeView,
) {
    let Some(_bar) = ui.tab_bar(format!("##elefttab{id}")) else {
        return;
    };
    if let Some(_t) = ui.tab_item("Outline") {
        ui.child_window(format!("##esbscroll{id}o"))
            .size([0.0, 0.0])
            .build(ui, || outline_tab(view, ui, id));
    }
    if let Some(_t) = ui.tab_item("I/O") {
        ui.child_window(format!("##esbscroll{id}i"))
            .size([0.0, 0.0])
            .build(ui, || io_tab(app, view, ui, id, node));
    }
}

/// The right column: insertable functions, system variables, every message the
/// bus's databases declare, and the FlexRay signals the loaded cluster
/// descriptions decode -- click one to push it into the source at the editor
/// cursor.
fn palette_tabs(
    app: &App,
    view: &mut ScriptEditorView,
    ui: &Ui,
    id: u64,
    node: &crate::bus::NodeView,
) {
    let Some(_bar) = ui.tab_bar(format!("##esbtab{id}")) else {
        return;
    };
    if let Some(_t) = ui.tab_item("Functions") {
        ui.child_window(format!("##esbscroll{id}f"))
            .size([0.0, 0.0])
            .build(ui, || fns_tab(app, view, ui, id));
    }
    if let Some(_t) = ui.tab_item("SysVar") {
        ui.child_window(format!("##esbscroll{id}s"))
            .size([0.0, 0.0])
            .build(ui, || sysvar_tab(app, view, ui, id));
    }
    if let Some(_t) = ui.tab_item("Messages") {
        ui.child_window(format!("##esbscroll{id}b"))
            .size([0.0, 0.0])
            .build(ui, || bus_tab(app, view, ui, id, node));
    }
    if let Some(_t) = ui.tab_item("FlexRay") {
        ui.child_window(format!("##esbscroll{id}r"))
            .size([0.0, 0.0])
            .build(ui, || fr_tab(app, view, ui, id));
    }
}

impl App {
    /// Applies a script window's model/file actions after its draw put the
    /// view back. Cursor, text and palette edits never reach here.
    pub(crate) fn apply_script_cmds(&mut self, cmds: Vec<ScriptCmd>) {
        for cmd in cmds {
            match cmd {
                ScriptCmd::SetName { id, name } => {
                    self.send(crate::bus::BusCommand::SetNodeName { id, name });
                }
                ScriptCmd::SetEnabled { id, on } => {
                    self.send(crate::bus::BusCommand::SetNodeEnabled { id, on });
                }
                ScriptCmd::Remove { id } => {
                    self.send(crate::bus::BusCommand::RemoveNode { id });
                }
                ScriptCmd::Apply { id, source } => {
                    self.send(crate::bus::BusCommand::SetNodeSource { id, source });
                }
                ScriptCmd::Save { path, source } => {
                    if let Err(e) = std::fs::write(&path, &source) {
                        self.status = format!("save failed: {e}");
                    } else {
                        self.status = format!("saved {path}");
                    }
                }
                ScriptCmd::Status(text) => self.status = text,
            }
        }
    }
}

/// Inserts a template at the editor's own cursor (CTE tracks it, so a
/// click in the sidebar never throws the text at the file end), replacing
/// the selection when one is active. The snippet always lands on its own
/// line, and the cursor parks after it so the next insert chains on.
fn insert(view: &mut ScriptEditorView, id: u64, text: &str) {
    let Some(editor) = view.editors.get_mut(&id) else {
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
fn fns_tab(app: &App, view: &mut ScriptEditorView, ui: &Ui, id: u64) {
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
            insert(view, id, insert_text);
        }
    }

    // DBC-aware section: the bound node's own messages and signals.
    let dbc_items = owned_dbc_items(app, id);
    if !dbc_items.is_empty() {
        ui.separator();
        ui.text_disabled("Node messages / signals");
        for (msg_id, ext, msg_name, sigs) in &dbc_items {
            let id_str = format!("{msg_id:03X}{}", if *ext { "x" } else { "" });
            if ui
                .selectable_config(format!("{} {}##dbcmsg{}", msg_name, id_str, msg_id))
                .build()
            {
                insert(view, id, &format!("send({:#x});", msg_id));
            }
            for (sig_name, _) in sigs {
                if ui
                    .selectable_config(format!("  {}##dbcsig{}_{}", sig_name, msg_id, sig_name))
                    .build()
                {
                    insert(view, id, &format!("sig({:#x}, \"{}\")", msg_id, sig_name));
                }
            }
        }
    }
}

/// The FlexRay signals every loaded cluster description offers to read.
/// `fr_sig` takes a cluster index, a slot and a signal name, and none of the
/// three is something a script author should have to look up in another window
/// -- so the list carries them: one group per slot, named alongside the frame
/// scheduled in it, since one slot is held by several frames in turn.
///
/// Its own tab rather than a section under 函数: it is a list of *this
/// project's* signals, like 报文 is for CAN, and it is the long one -- a real
/// cluster has hundreds of slots, and burying that under the templates pushed
/// the templates off the panel.
///
/// `pub(crate)` for the smoke test only: a tab body draws just while its tab is
/// selected, which no headless frame can arrange, so the list is drawn directly
/// the way the siglist probe draws its table.
pub(crate) fn fr_tab(app: &App, view: &mut ScriptEditorView, ui: &Ui, id: u64) {
    let fr_items: Vec<(u8, u16, String, Vec<String>)> = app
        .fr_buses
        .iter()
        .flat_map(|(bus, cfg)| {
            cfg.db
                .slot_signals()
                .into_iter()
                .map(move |(slot, frame, sigs)| (*bus, slot, frame, sigs))
        })
        .collect();
    if fr_items.is_empty() {
        ui.text_disabled("(no FlexRay cluster description loaded)");
        return;
    }
    for (bus, slot, frame, sigs) in &fr_items {
        // The index stays visible next to a user-chosen name: this is the tab
        // whose entries end up typed as `fr_sig(bus, slot, ..)`.
        ui.text_disabled(format!("{} slot {slot} · {frame}", app.fr_bus_label(*bus)));
        for sig in sigs {
            if ui
                .selectable_config(format!("  {sig}##frsig{bus}_{slot}_{frame}_{sig}"))
                .build()
            {
                insert(view, id, &format!("fr_sig({bus}, {slot}, \"{sig}\")"));
            }
        }
    }
}

/// The whole bus: every message/signal the bus's databases declare -- a
/// script may read (`sig`) or send (`send`) anything on its wire.
fn bus_tab(app: &App, view: &mut ScriptEditorView, ui: &Ui, id: u64, node: &crate::bus::NodeView) {
    let bus_items = bus_dbc_items(app, node.channel);
    if bus_items.is_empty() {
        ui.text_disabled("(no DBC messages on this bus)");
        return;
    }
    for (msg_id, ext, msg_name, sigs) in &bus_items {
        let id_str = format!("{msg_id:03X}{}", if *ext { "x" } else { "" });
        if ui
            .selectable_config(format!(
                "{} {}##busmsg{id}_{msg_id}_{}",
                msg_name, id_str, *ext as u8
            ))
            .build()
        {
            insert(view, id, &format!("send({:#x});", msg_id));
        }
        for (sig_name, _) in sigs {
            if ui
                .selectable_config(format!(
                    "  {}##bussig{id}_{msg_id}_{}_{}",
                    sig_name, *ext as u8, sig_name
                ))
                .build()
            {
                insert(view, id, &format!("sig({:#x}, \"{}\")", msg_id, sig_name));
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
            let sigs: Vec<(String, u64)> = m
                .signals
                .iter()
                .map(|s| (s.name.clone(), s.start_bit))
                .collect();
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
            let sigs: Vec<(String, u64)> = m
                .signals
                .iter()
                .map(|s| (s.name.clone(), s.start_bit))
                .collect();
            Some((id, ext, m.name.clone(), sigs))
        })
        .collect()
}

/// The source's handler outline: every event handler and function with
/// its line number. Works on half-typed source; a compile error shows
/// alongside so the outline degrades gracefully.
fn outline_tab(view: &mut ScriptEditorView, ui: &Ui, id: u64) {
    let facts = facts_for(view, id, false);
    if let Some(e) = &facts.error {
        ui.text_colored([1.0, 0.55, 0.3, 1.0], "Draft does not compile");
        if ui.is_item_hovered() {
            ui.tooltip_text(e);
        }
    }
    if facts.outline.is_empty() {
        ui.text_disabled("(no handlers yet)");
        return;
    }
    for (line, label) in &facts.outline {
        if ui.selectable_config(format!("{label}##ol{line}")).build()
            && let Some(editor) = view.editors.get_mut(&id)
        {
            // Jump to the handler's line, centered in the viewport.
            let target = *line as usize - 1;
            let _ = editor.set_cursor(Position::new(target, 0));
            let _ = editor.scroll_to_line(target, ScrollAlignment::Middle);
        }
        if ui.is_item_hovered() {
            ui.tooltip_text(format!("line {line}; click to jump"));
        }
    }
}

/// The static fact table: what this script sends, what it listens for,
/// and which system variables it touches. Sends are marked against the
/// DBC's transmitter declaration when the script is bound: `*` own,
/// `!` foreign, `-` unknown.
fn io_tab(app: &App, view: &mut ScriptEditorView, ui: &Ui, id: u64, node: &crate::bus::NodeView) {
    let facts = facts_for(view, id, false);
    if let Some(e) = &facts.error {
        ui.text_colored([1.0, 0.55, 0.3, 1.0], "Draft does not compile");
        if ui.is_item_hovered() {
            ui.tooltip_text(e);
        }
        return;
    }

    ui.text_disabled(format!("Sends ({})", facts.sends.len()));
    let dbc = node
        .attached
        .as_ref()
        .and_then(|(bus, _)| app.channel_dbc(*bus));
    let attached_node = node.attached.as_ref().map(|(_, n)| n.clone());
    if facts.sends.is_empty() {
        ui.text_disabled("  (none)");
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
        ui.text(format!(
            "{mark} 0x{msg_id:03X}{}",
            if *ext { "x" } else { "" }
        ));
        if ui.is_item_hovered() {
            ui.tooltip_text(format!("from {from}"));
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
            ui.text_disabled(format!("Not implemented ({})", missing.len()));
            for (msg_id, ext, name) in &missing {
                ui.text(format!(
                    "  0x{msg_id:03X}{} {name}",
                    if *ext { "x" } else { "" }
                ));
            }
            if ui.is_item_hovered() {
                ui.tooltip_text("may be sent by the generator; the start log has the same [info]");
            }
        }
    }

    ui.separator();
    ui.text_disabled(format!(
        "Receives ({})",
        facts.recvs.len() + usize::from(facts.recv_wildcard)
    ));
    if facts.recv_wildcard {
        ui.text("  *  all frames (forwarder / recorder)");
        if ui.is_item_hovered() {
            ui.tooltip_text(
                "send set = receive set ∩ DBC declarations; the start log has the same [info]",
            );
        }
        if facts.wildcard_sends {
            ui.text_colored(
                [1.0, 0.75, 0.3, 1.0],
                "    wildcard handler sends: emitted frames re-enter this handler (loop risk)",
            );
            if ui.is_item_hovered() {
                ui.tooltip_text(
                    "exclude your own frames with frame_id() before forwarding (see examples/sniffer.rxcan)",
                );
            }
        }
    }
    if facts.recvs.is_empty() && !facts.recv_wildcard {
        ui.text_disabled("  (none)");
    }
    for (msg_id, ext) in &facts.recvs {
        ui.text(format!("  0x{msg_id:03X}{}", if *ext { "x" } else { "" }));
    }

    ui.separator();
    ui.text_disabled(format!("System variables ({})", facts.sysvars.len()));
    if facts.sysvars.is_empty() {
        ui.text_disabled("  (none)");
    }
    for key in &facts.sysvars {
        ui.text(format!("  {key}"));
    }

    // `$Message::Signal` reads, marked against the DBC like the send set:
    // `*` the pair resolves to a real message+signal, `-` it does not.
    if !facts.named_refs.is_empty() {
        ui.separator();
        ui.text_disabled(format!("$shorthand ({})", facts.named_refs.len()));
        for (msg, sig) in &facts.named_refs {
            let known = dbc.as_ref().is_some_and(|db| {
                db.message_id_by_name(msg).is_some_and(|id| {
                    db.message_of(id)
                        .is_some_and(|m| m.signals.iter().any(|s| &s.name == sig))
                })
            });
            let mark = if known { "*" } else { "-" };
            ui.text(format!("  {mark} ${msg}::{sig}"));
            if ui.is_item_hovered() && !known {
                ui.tooltip_text(
                    "message or signal name not in the DBC; the start check reports [check]",
                );
            }
        }
    }

    ui.separator();
    ui.text_disabled(format!("Response map ({})", facts.responses.len()));
    if facts.responses.is_empty() {
        ui.text_disabled("  (no armed one-shot timers)");
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
            ui.tooltip_text(format!("armed by {}", row.armed_by.join(", ")));
        }
    }
}

/// The defined system variables, grouped by namespace: 读 inserts a
/// `sys_get`, 写 inserts a `sys_set` template.
fn sysvar_tab(app: &App, view: &mut ScriptEditorView, ui: &Ui, id: u64) {
    let mut vars: Vec<(String, Vec<(String, bool)>)> = Vec::new(); // (ns, [(name, has_bounds)])
    for v in &app.snap.sysvars {
        match vars.iter_mut().find(|(ns, _)| *ns == v.def.namespace) {
            Some((_, list)) => list.push((
                v.def.name.clone(),
                v.def.min.is_some() || v.def.max.is_some(),
            )),
            None => vars.push((
                v.def.namespace.clone(),
                vec![(
                    v.def.name.clone(),
                    v.def.min.is_some() || v.def.max.is_some(),
                )],
            )),
        }
    }
    if vars.is_empty() {
        ui.text_disabled("(none defined)");
        ui.text_disabled("Manage in View > System Variables");
        return;
    }
    for (ns, names) in &vars {
        ui.text_disabled(ns);
        for (name, _) in names {
            if ui
                .selectable_config(format!("get  {name}##svr{ns}{name}"))
                .build()
            {
                insert(view, id, &format!("sys_get(\"{ns}::{name}\")"));
            }
            if ui
                .selectable_config(format!("set  {name}##svw{ns}{name}"))
                .build()
            {
                insert(view, id, &format!("sys_set(\"{ns}::{name}\", 0)"));
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
        assert_eq!(
            f.responses.len(),
            1,
            "only the armed timer maps: {:?}",
            f.responses
        );
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
