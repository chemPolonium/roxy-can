async fn scripts_handler(State(api): State<ApiState>) -> Json<Vec<WebScript>> {
    Json(
        api.scripts
            .lock()
            .map(|scripts| scripts.clone())
            .unwrap_or_default(),
    )
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct WebScriptFunction {
    name: &'static str,
    usage: &'static str,
}

fn script_help_items() -> Vec<WebScriptFunction> {
    crate::script::SCRIPT_HELP
        .iter()
        .map(|(name, usage)| WebScriptFunction { name, usage })
        .collect()
}

/// The script editor's completion vocabulary. Names and call shapes live in
/// the language module, so a new builtin cannot be documented twice.
async fn script_help_handler() -> Json<Vec<WebScriptFunction>> {
    Json(script_help_items())
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct WebDebugValue {
    name: String,
    value: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct WebDebugFrame {
    function: String,
    line: u32,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct WebDebug {
    enabled: bool,
    node: Option<u64>,
    paused: bool,
    line: Option<u32>,
    function: Option<String>,
    breakpoints: Vec<u32>,
    locals: Vec<WebDebugValue>,
    globals: Vec<WebDebugValue>,
    stack: Vec<WebDebugFrame>,
    note: Option<String>,
}

impl From<crate::script::debug::DebugView> for WebDebug {
    fn from(view: crate::script::debug::DebugView) -> Self {
        Self {
            enabled: view.enabled,
            node: view.node,
            paused: view.paused,
            line: view.line,
            function: view.function,
            breakpoints: view.breakpoints,
            locals: view
                .locals
                .into_iter()
                .map(|value| WebDebugValue {
                    name: value.name,
                    value: value.value,
                })
                .collect(),
            globals: view
                .globals
                .into_iter()
                .map(|value| WebDebugValue {
                    name: value.name,
                    value: value.value,
                })
                .collect(),
            stack: view
                .stack
                .into_iter()
                .map(|frame| WebDebugFrame {
                    function: frame.function,
                    line: frame.line,
                })
                .collect(),
            note: view.note,
        }
    }
}

#[derive(Deserialize)]
#[serde(tag = "action", rename_all = "snake_case")]
enum WebDebugCommand {
    Arm { node_id: u64, lines: Vec<u32> },
    SetBreakpoints { lines: Vec<u32> },
    Continue,
    Step,
    Pause,
    Stop,
}

async fn debug_view_handler() -> Json<WebDebug> {
    Json(crate::script::debug::view().into())
}

async fn debug_command_handler(Json(command): Json<WebDebugCommand>) -> Json<WebDebug> {
    let view = match command {
        WebDebugCommand::Arm { node_id, lines } => crate::script::debug::arm(node_id, lines),
        WebDebugCommand::SetBreakpoints { lines } => crate::script::debug::set_breakpoints(lines),
        WebDebugCommand::Continue => crate::script::debug::resume(false),
        WebDebugCommand::Step => crate::script::debug::resume(true),
        WebDebugCommand::Pause => crate::script::debug::pause(),
        WebDebugCommand::Stop => crate::script::debug::stop(),
    };
    Json(view.into())
}

/// Fingerprint of everything the scripts dialog shows. Source edits are
/// infrequent, but they must reach the editor without shipping every
/// script on every 10 Hz state poll; `/api/nodes` is refreshed only when
/// this changes.
fn scripts_signature(app: &App) -> u64 {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    for node in app.snap.nodes.iter() {
        node.id.hash(&mut hasher);
        node.name.hash(&mut hasher);
        node.source.hash(&mut hasher);
        node.enabled.hash(&mut hasher);
        node.running.hash(&mut hasher);
        node.errored.hash(&mut hasher);
        if let Some(issue) = &node.issue {
            issue.line.hash(&mut hasher);
            issue.col.hash(&mut hasher);
            issue.message.hash(&mut hasher);
        } else {
            0u8.hash(&mut hasher);
        }
        node.attached.hash(&mut hasher);
        node.log.len().hash(&mut hasher);
        node.log.last().hash(&mut hasher);
        for test in &node.tests {
            test.name.hash(&mut hasher);
            test.runs.hash(&mut hasher);
            test.failures.hash(&mut hasher);
            test.line.hash(&mut hasher);
        }
        for name in &node.panel_actions {
            name.hash(&mut hasher);
        }
    }
    hasher.finish()
}

fn scripts_snapshot(app: &App) -> Vec<WebScript> {
    app.snap
        .nodes
        .iter()
        .map(|node| WebScript {
            id: node.id,
            name: node.name.clone(),
            channel: node.channel,
            attached_bus: node.attached.as_ref().map(|(bus, _)| *bus),
            attached_node: node.attached.as_ref().map(|(_, name)| name.clone()),
            enabled: node.enabled,
            running: node.running,
            errored: node.errored,
            issue: node.issue.as_ref().map(|issue| WebScriptIssue {
                line: issue.line,
                col: issue.col,
                message: issue.message.clone(),
            }),
            file_path: node.file_path.clone().unwrap_or_default(),
            source: node.source.clone(),
            tests: node
                .tests
                .iter()
                .map(|test| WebScriptTest {
                    name: test.name.clone(),
                    runs: test.runs,
                    failures: test.failures,
                    line: test.line,
                    time_s: test.time_s,
                })
                .collect(),
            log: node.log.clone(),
            panel_actions: node.panel_actions.clone(),
        })
        .collect()
}

/// Runs the core exporter for a web export request. Trace picks ASC or PCAP
/// by the path's extension; the others write CSV/TXT. Exporters target the
/// first matching observer window, which starts as an All-buses view.
/// Adopts a browser workspace only if it belongs to the current project
/// revision; a debounce from the previous project is dropped.
fn set_web_workspace(app: &mut App, rev: u64, workspace: serde_json::Value) {
    if rev == app.workspace_rev {
        app.web_workspace = Some(workspace);
    }
}

fn export_file(app: &mut App, kind: &str, path: &str, range: Option<(u64, u64)>) {
    if path.trim().is_empty() {
        app.fail("export needs a path");
        return;
    }
    match kind {
        "trace" => match range {
            Some((from, to)) => app.export_trace_range(0, path, from, to),
            None => app.export_trace(0, path),
        },
        "messages" => app.export_messages_csv(0, path),
        "stats" => app.export_stats_csv(0, path),
        "write" => app.export_write_txt(path),
        "data" => app.export_data_csv(0, path),
        "state" => app.export_state_csv(0, path),
        "graphics" => app.export_graphics_csv(0, path),
        "spec" => app.export_spec_csv(path),
        other => app.fail(format!("unknown export kind: {other}")),
    }
}

/// Seeds the chart from a project's first Graphics window. The project file
/// persists curve selection there; the web keeps its own lane metadata but
/// must not start empty when the same project is opened.
fn seed_chart_selection(app: &mut App, store: &mut SignalStore) {
    let keys: Vec<SigKey> = app
        .graphics
        .first()
        .map(|window| {
            window
                .signals
                .iter()
                .filter(|signal| signal.visible)
                .map(|signal| signal.key.clone())
                .collect()
        })
        .unwrap_or_default();
    for key in keys {
        store.select_key(app, key.clone());
        app.subscribe(key);
    }
}

/// Selecting a chart lane also records it on the first Graphics window, so
/// project save/load carries the selection. The panel stays the source of
/// truth for lane colors; `store` is updated by the caller.
fn checked_band_set(
    id: String,
    name: String,
    bands: Vec<crate::band::Band>,
) -> Result<crate::band::BandSet, String> {
    let id = id.trim().to_string();
    let name = name.trim().to_string();
    if id.is_empty() || id.chars().count() > 64 {
        return Err("band set id must be 1–64 characters".into());
    }
    if name.is_empty() || name.chars().count() > 40 {
        return Err("band set name must be 1–40 characters".into());
    }
    let mut bands = bands;
    if bands.is_empty() || bands.len() > 64 {
        return Err("band set needs 1–64 bands".into());
    }
    for band in &mut bands {
        if band.upper.is_some_and(|upper| !upper.is_finite()) {
            return Err("band bounds must be finite".into());
        }
        band.text = band.text.trim().to_string();
        if band.text.chars().count() > 80 {
            return Err("band text is limited to 80 characters".into());
        }
        if band.color.is_some_and(|color| color > 63) {
            return Err("band color slot must be 0–63".into());
        }
    }
    bands.sort_by(|a, b| match (a.upper, b.upper) {
        (Some(left), Some(right)) => left
            .total_cmp(&right)
            .then_with(|| a.inclusive.cmp(&b.inclusive)),
        (Some(_), None) => std::cmp::Ordering::Less,
        (None, Some(_)) => std::cmp::Ordering::Greater,
        (None, None) => std::cmp::Ordering::Equal,
    });
    if bands.iter().filter(|band| band.upper.is_none()).count() != 1 {
        return Err("band set needs exactly one open-ended final band".into());
    }
    if bands.last().is_some_and(|band| band.upper.is_some()) {
        return Err("the open-ended band must be last".into());
    }
    if bands
        .windows(2)
        .any(|pair| pair[0].upper == pair[1].upper && pair[0].inclusive == pair[1].inclusive)
    {
        return Err("band set needs unique bound/mode pairs".into());
    }
    Ok(crate::band::BandSet { id, name, bands })
}

#[cfg(test)]
fn chart_select(app: &mut App, ch: u8, id: u32, ext: bool, name: &str) {
    let key = app.can_key(ch, id, ext, name.to_string());
    chart_select_key(app, key);
}

/// Refreshes the core's chart-keep set from the session selection. Called
/// just before an observer drops a row: the chart owns that subscription even
/// when the lane never entered the project's Graphics window (a session-only
/// web pick), so pruning must not unmount it.
fn sync_chart_keep(app: &mut App, signals: &std::sync::Mutex<SignalStore>) {
    if let Ok(store) = signals.lock() {
        app.set_session_chart_keep(store.keys());
    }
}

fn chart_select_key(app: &mut App, key: SigKey) {
    if app.graphics.is_empty() {
        app.new_graphics_window();
    }
    app.set_win_signal(
        crate::workspace::PopupTarget::Graphics(0),
        key.clone(),
        true,
    );
    app.subscribe(key);
}

fn chart_unselect(app: &mut App, key: &SigKey) {
    app.set_win_signal(
        crate::workspace::PopupTarget::Graphics(0),
        key.clone(),
        false,
    );
    // Session-only web lanes may not live in a Graphics window; prune still
    // drops the subscription unless another observer (Monitor/Data/State) uses
    // it.
    app.prune_signal(key);
}

/// Selects a signal as a plotted lane or a cursor-only read signal. Both
/// keep the bus subscription; only the lane state differs. Switching a
/// plotted signal to read removes its Graphics row but the refreshed chart
/// keep set protects the subscription from the prune.
fn set_signal_mode(app: &mut App, store: &mut SignalStore, key: SigKey, read: bool) {
    let known = store.is_selected(&key);
    if known && store.is_read(&key) == read {
        return;
    }
    if !known {
        store.select_key_mode(app, key.clone(), read);
        if read {
            app.subscribe(key);
        } else {
            chart_select_key(app, key);
        }
        app.set_session_chart_keep(store.keys());
        return;
    }
    store.select_key_mode(app, key.clone(), read);
    if read {
        app.set_session_chart_keep(store.keys());
        chart_unselect(app, &key);
    } else {
        chart_select_key(app, key);
        app.set_session_chart_keep(store.keys());
    }
}

/// Applies a Signals context-menu action. Watching only adds a reader;
/// removing a chart lane leaves any observer that already watches it alone.
fn chart_signal_action(app: &mut App, store: &mut SignalStore, key: &SigKey, action: &str) -> bool {
    match action {
        "monitor" => app.set_win_signal(crate::workspace::PopupTarget::Monitor, key.clone(), true),
        "data" => {
            if app.data_windows.is_empty() {
                app.new_data_window();
            }
            app.set_win_signal(crate::workspace::PopupTarget::Data(0), key.clone(), true);
        }
        "state" => {
            if app.state_trackers.is_empty() {
                app.new_state_window();
            }
            app.set_win_signal(crate::workspace::PopupTarget::State(0), key.clone(), true);
        }
        "remove" => {
            store.unselect(key);
            app.set_session_chart_keep(store.keys());
            chart_unselect(app, key);
        }
        _ => return false,
    }
    true
}

/// Moves a chart lane in both the session cache and the first Graphics
/// window. The window order is what a project save persists, while the cache
/// order is what the web UI draws.
fn chart_move(app: &mut App, store: &mut SignalStore, key: &SigKey, to: usize) {
    if !store.move_selected(key, to) {
        return;
    }
    let order = store.chart_keys();
    let Some(window) = app.graphics.first_mut() else {
        return;
    };
    let Some(position) = window.signals.iter().position(|signal| &signal.key == key) else {
        return;
    };
    let moved = window.signals.remove(position);
    // Insert relative to the selected neighbours, leaving hidden rows in
    // place around the reordered web lanes.
    let insert_at = order
        .get(to + 1)
        .and_then(|next| window.signals.iter().position(|signal| &signal.key == next))
        .or_else(|| {
            to.checked_sub(1)
                .and_then(|previous| order.get(previous))
                .and_then(|previous| {
                    window
                        .signals
                        .iter()
                        .position(|signal| &signal.key == previous)
                        .map(|index| index + 1)
                })
        })
        .unwrap_or(window.signals.len());
    window.signals.insert(insert_at, moved);
}

/// Replaces a State row's custom value-definition bands. Cuts are sorted
/// and deduplicated; names are padded so every band has a label.
fn set_state_rule(
    app: &mut App,
    key: &SigKey,
    bands: Vec<crate::band::Band>,
    band_set_id: Option<String>,
) {
    let Some(window) = app.state_trackers.first_mut() else {
        app.fail("state rule needs a State Tracker window");
        return;
    };
    window.rules.insert(
        key.clone(),
        crate::observe::StateRule { bands, band_set_id },
    );
}

fn clear_state_rule(app: &mut App, key: &SigKey) {
    if let Some(window) = app.state_trackers.first_mut() {
        window.rules.remove(key);
    }
}

