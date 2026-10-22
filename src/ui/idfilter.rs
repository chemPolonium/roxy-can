use crate::app::{App, PopupTarget, SigScope};
use dear_imgui_rs::{Condition, Ui};
use std::collections::HashSet;

/// The selection popup's own state: whether it is open, which window it edits,
/// and the two search boxes. Kept out of `App` so the draw path only reads the
/// application and emits [`IdFilterCmd`]s (see `AGENTS.md`).
#[derive(Default)]
pub struct IdFilterView {
    pub open: bool,
    pub target: Option<PopupTarget>,
    pub msg_search: String,
    pub sig_search: String,
}

impl IdFilterView {
    /// Binds the popup to one window and opens it -- what every observer's
    /// "…" button does.
    pub fn open_for(&mut self, target: PopupTarget) {
        self.target = Some(target);
        self.open = true;
    }
}

/// What a click in the selection popup asks the application to change. The
/// draw never edits the window's pick set or signal list itself.
pub enum IdFilterCmd {
    TogglePick {
        target: PopupTarget,
        pick: crate::workspace::Pick,
        on: bool,
    },
    AddPicks {
        target: PopupTarget,
        picks: Vec<crate::workspace::Pick>,
    },
    ClearPicks {
        target: PopupTarget,
    },
    ToggleSignal {
        target: PopupTarget,
        key: crate::observe::SigKey,
        on: bool,
    },
    ClearSignals {
        target: PopupTarget,
        keys: Vec<crate::observe::SigKey>,
    },
}

struct MsgEntry {
    id: u32,
    name: String,
    signals: Vec<String>,
}

struct TreeMsg {
    id: u32,
    /// The entry's frame class: a standard and an extended message that
    /// share one numeric id list separately, each with its own signals.
    ext: bool,
    name: String,
    signals: Vec<String>,
}

/// Shared "Signals" selector used by Trace/Messages/Statistics: All buses /
/// one CAN channel / one FlexRay cluster / one Ethernet bus / Manual
/// selection. The "…" button opens the Message Selection popup bound to
/// `target`; selecting messages there switches the window to Manual.
///
/// The three bus numbering spaces are separate entries: a FlexRay cluster
/// index and an Ethernet bus index are different numbers for different
/// cables, so picking one here must not silently decide what the others
/// show. A scope naming a bus that no longer exists reads back as All buses
/// -- the widest view, never a hidden row.
pub fn scope_combo(
    app: &App,
    ui: &Ui,
    id: &str,
    scope: SigScope,
    target: PopupTarget,
    open_picker: &mut bool,
) -> SigScope {
    let mut items: Vec<String> = vec!["All buses".to_string()];
    let mut kinds: Vec<SigScope> = vec![SigScope::All];
    for ch in 0..app.snap.channel_count {
        items.push(format!("Bus: {}", app.channel_name(ch as u8)));
        kinds.push(SigScope::Bus(ch as u8));
    }
    for bus in app.fr_scope_buses(scope) {
        items.push(format!("FlexRay: {}", app.fr_bus_name(bus)));
        kinds.push(SigScope::FrBus(bus));
    }
    for bus in app.eth_scope_buses(scope) {
        items.push(format!("Ethernet: {}", app.eth_bus_label(bus)));
        kinds.push(SigScope::EthBus(bus));
    }
    let n = app.win_manual(target).map(|m| m.len()).unwrap_or(0);
    items.push(format!("Manual ({n})"));
    kinds.push(SigScope::Manual);
    let mut cur = kinds.iter().position(|k| *k == scope).unwrap_or(0);
    ui.set_next_item_width(140.0);
    ui.combo_simple_string(id, &mut cur, &items);
    ui.same_line();
    if ui.button(format!("…{id}")) {
        *open_picker = true;
    }
    if ui.is_item_hovered() {
        ui.tooltip_text("Open the selection popup: choose buses or hand-pick messages and signals");
    }
    kinds[cur.min(kinds.len() - 1)]
}

/// Opens the selection popup for the window it is bound to.
/// Trace/Messages/Statistics filter at the message level (flat list, hover a
/// message to see its signals); Graphics/Data filter at the signal level
/// (message tree with signal checkboxes).
pub fn render(app: &mut App, ui: &Ui) {
    let mut view = std::mem::take(&mut app.id_filter);
    let cmds = view.show(ui, app);
    app.id_filter = view;
    app.apply_id_filter_cmds(cmds);
}

impl IdFilterView {
    fn show(&mut self, ui: &Ui, app: &App) -> Vec<IdFilterCmd> {
        if !self.open {
            return Vec::new();
        }
        let io = ui.io();
        let mut open = self.open;
        let target = self.target;
        let signal_level = matches!(
            target,
            Some(PopupTarget::Graphics(_))
                | Some(PopupTarget::Data(_))
                | Some(PopupTarget::State(_))
                | Some(PopupTarget::Monitor)
        );
        let title = match target {
            Some(t) => format!(
                "{} — {}",
                if signal_level {
                    "Signal Selection"
                } else {
                    "Message Selection"
                },
                target_name(app, t)
            ),
            None => "Message Selection".to_string(),
        };
        let mut cmds = Vec::new();
        let (pos, size) = crate::ui::fit_first_use(
            io.display_size(),
            [io.display_size()[0] * 0.35, io.display_size()[1] * 0.18],
            [460.0, 480.0],
        );
        ui.window(title)
            .opened(&mut open)
            .position(pos, Condition::FirstUseEver)
            .size(size, Condition::FirstUseEver)
            .build(|| {
                if signal_level {
                    signal_content(self, app, ui, &mut cmds);
                } else {
                    message_content(self, app, ui, &mut cmds);
                }
            });
        self.open = open;
        cmds
    }
}

pub fn target_name(app: &App, t: PopupTarget) -> String {
    fn or_fallback(name: Option<&str>, fallback: String) -> String {
        match name {
            Some(n) if !n.trim().is_empty() => n.to_string(),
            _ => fallback,
        }
    }
    match t {
        PopupTarget::Trace(i) => or_fallback(
            app.trace_windows.get(i).map(|w| w.name.as_str()),
            format!("Trace {}", i + 1),
        ),
        PopupTarget::Messages(i) => or_fallback(
            app.msg_windows.get(i).map(|w| w.name.as_str()),
            format!("Messages {}", i + 1),
        ),
        PopupTarget::Stats(i) => or_fallback(
            app.stats_windows.get(i).map(|w| w.name.as_str()),
            format!("Statistics {}", i + 1),
        ),
        PopupTarget::Graphics(i) => or_fallback(
            app.graphics.get(i).map(|w| w.name.as_str()),
            format!("Graphics {}", i + 1),
        ),
        PopupTarget::Data(i) => or_fallback(
            app.data_windows.get(i).map(|w| w.name.as_str()),
            format!("Data {}", i + 1),
        ),
        PopupTarget::State(i) => or_fallback(
            app.state_trackers.get(i).map(|w| w.name.as_str()),
            format!("State Tracker {}", i + 1),
        ),
        PopupTarget::Monitor => "Monitor".to_string(),
    }
}

fn set_window_scope(app: &mut App, target: PopupTarget, scope: SigScope) {
    match target {
        PopupTarget::Trace(i) => {
            if let Some(w) = app.trace_windows.get_mut(i) {
                w.scope = scope;
            }
        }
        PopupTarget::Messages(i) => {
            if let Some(w) = app.msg_windows.get_mut(i) {
                w.scope = scope;
            }
        }
        PopupTarget::Stats(i) => {
            if let Some(w) = app.stats_windows.get_mut(i) {
                w.scope = scope;
            }
        }
        _ => {}
    }
}

impl App {
    /// Applies the selection popup's clicks after its draw put the view back.
    pub(crate) fn apply_id_filter_cmds(&mut self, cmds: Vec<IdFilterCmd>) {
        for cmd in cmds {
            match cmd {
                IdFilterCmd::TogglePick { target, pick, on } => {
                    if let Some(m) = self.win_manual_mut(target) {
                        if on {
                            m.insert(pick);
                        } else {
                            m.remove(&pick);
                        }
                    }
                    set_window_scope(self, target, SigScope::Manual);
                }
                IdFilterCmd::AddPicks { target, picks } => {
                    if let Some(m) = self.win_manual_mut(target) {
                        m.extend(picks);
                    }
                    set_window_scope(self, target, SigScope::Manual);
                }
                IdFilterCmd::ClearPicks { target } => {
                    if let Some(m) = self.win_manual_mut(target) {
                        m.clear();
                    }
                    set_window_scope(self, target, SigScope::All);
                }
                IdFilterCmd::ToggleSignal { target, key, on } => {
                    self.set_win_signal(target, key, on);
                }
                IdFilterCmd::ClearSignals { target, keys } => {
                    for key in keys {
                        self.set_win_signal(target, key, false);
                    }
                }
            }
        }
    }
}

/// Message-level selection for Trace/Messages/Statistics: one checkbox per
/// message.
fn message_content(view: &mut IdFilterView, app: &App, ui: &Ui, cmds: &mut Vec<IdFilterCmd>) {
    let Some(target) = view.target else {
        return;
    };
    ui.set_next_item_width(ui.content_region_avail()[0]);
    ui.input_text("##idf_search", &mut view.msg_search)
        .hint("search messages / signals")
        .build();
    if ui.button("Select all matching") {
        let q = view.msg_search.trim().to_ascii_uppercase();
        let mut keys: Vec<crate::workspace::Pick> = Vec::new();
        for (ch, channel) in app.snap.channels.iter().enumerate() {
            let Some(db) = &channel.dbc else {
                continue;
            };
            for &(id, _) in &db.order {
                if msg_matches(db.message_name(id), id, &q) {
                    keys.push(crate::workspace::Pick::Can { ch: ch as u8, id });
                }
            }
        }
        // The FlexRay slots whose frame names match the same query: one slot per
        // pick, because that is what the window's Manual scope admits or drops
        // (a slot held by several frames is picked as a slot).
        keys.extend(
            fr_slot_hits(app, &q)
                .into_iter()
                .map(|(bus, slot, _)| crate::workspace::Pick::Fr { bus, slot }),
        );
        // Ethernet flows the same query offers: one pick per captured L2
        // flow, which is the unit a Trace row and a Messages tally share.
        keys.extend(
            app.snap
                .eth_aggs
                .iter()
                .filter(|a| eth_flow_matches(app, &a.key, &q))
                .map(|a| crate::workspace::Pick::Eth(a.key)),
        );
        cmds.push(IdFilterCmd::AddPicks {
            target,
            picks: keys,
        });
    }
    ui.same_line();
    if ui.button("Clear") {
        cmds.push(IdFilterCmd::ClearPicks { target });
    }
    ui.same_line();
    let n_sel = app.win_manual(target).map(|m| m.len()).unwrap_or(0);
    ui.text(format!("{n_sel} message(s) / slot(s) selected"));
    ui.separator();

    let q = view.msg_search.trim().to_ascii_uppercase();

    // Clone the message/signal layout first so the checkbox callbacks can
    // mutate the window's manual set without borrow conflicts.
    let channel_names: Vec<String> = (0..app.snap.channel_count)
        .map(|ch| app.channel_name(ch as u8))
        .collect();
    let per_channel: Vec<Vec<MsgEntry>> = app
        .snap
        .channels
        .iter()
        .map(|channel| {
            channel
                .dbc
                .as_ref()
                .map(|db| {
                    // The selection model is keyed by bare id, so a standard
                    // and an extended twin list once (standard wins).
                    let mut seen: std::collections::HashSet<u32> = std::collections::HashSet::new();
                    db.order
                        .iter()
                        .filter_map(|&(id, _)| {
                            if !seen.insert(id) {
                                return None;
                            }
                            let m = db.message_of(id)?;
                            Some(MsgEntry {
                                id,
                                name: m.name.clone(),
                                signals: m.signals.iter().map(|s| s.name.clone()).collect(),
                            })
                        })
                        .collect()
                })
                .unwrap_or_default()
        })
        .collect();

    for (ch, entries) in per_channel.iter().enumerate() {
        ui.text_colored(
            [0.30, 0.80, 1.00, 1.0],
            format!("{}  ({})", channel_names[ch], entries.len()),
        );
        if entries.is_empty() {
            ui.text("(no DBC loaded on this bus)");
            continue;
        }
        for e in entries {
            let sig_match = e
                .signals
                .iter()
                .any(|s| s.to_ascii_uppercase().contains(&q));
            if !msg_matches(Some(&e.name), e.id, &q) && !sig_match {
                continue;
            }
            let key = crate::workspace::Pick::Can {
                ch: ch as u8,
                id: e.id,
            };
            let mut on = app.win_manual(target).is_some_and(|m| m.contains(&key));
            // ID includes the bus: the same DBC on two buses would otherwise
            // produce identical labels and colliding widget IDs.
            let label = format!("{:03X}  {}##msgsel{ch}_{:X}", e.id, e.name, e.id);
            if ui.checkbox(label, &mut on) {
                cmds.push(IdFilterCmd::TogglePick {
                    target,
                    pick: key,
                    on,
                });
            }
        }
    }

    // The FlexRay slots the same query offers. The pick is the **slot**, because
    // that is the unit a window admits or drops rows by, so the frames sharing
    // one slot by cycle phase are named together on one line instead of
    // pretending to be separate choices.
    let fr_rows = fr_slot_hits(app, &q);
    if !fr_rows.is_empty() {
        ui.separator();
        ui.text_colored([0.55, 0.8, 1.0, 1.0], "FlexRay");
        for (bus, slot, frames) in &fr_rows {
            let key = crate::workspace::Pick::Fr {
                bus: *bus,
                slot: *slot,
            };
            let mut on = app.win_manual(target).is_some_and(|m| m.contains(&key));
            // Labelled with the 路's name, keyed by its index: the widget id
            // must not follow a rename, or the checkbox loses its state as soon
            // as the user names the bus.
            let label = format!(
                "{} slot {slot}  {frames}##frsel{bus}_{slot}",
                app.fr_bus_name(*bus)
            );
            if ui.checkbox(label, &mut on) {
                cmds.push(IdFilterCmd::TogglePick {
                    target,
                    pick: key,
                    on,
                });
            }
        }
    }

    // The Ethernet flows the same query offers. A flow has no address or
    // slot, so its L2 identity is the pick -- the identical key Messages,
    // Statistics and Trace filter that row by.
    let eth_rows: Vec<(crate::aggregate::EthFlowKey, String)> = app
        .snap
        .eth_aggs
        .iter()
        .filter(|a| eth_flow_matches(app, &a.key, &q))
        .map(|a| (a.key, eth_flow_label(app, &a.key)))
        .collect();
    if !eth_rows.is_empty() {
        ui.separator();
        ui.text_colored([0.90, 0.70, 0.30, 1.0], "Ethernet");
        for (flow, label) in &eth_rows {
            let key = crate::workspace::Pick::Eth(*flow);
            let mut on = app.win_manual(target).is_some_and(|m| m.contains(&key));
            let label = format!("{label}##ethsel{}", eth_flow_tag(flow));
            if ui.checkbox(label, &mut on) {
                cmds.push(IdFilterCmd::TogglePick {
                    target,
                    pick: key,
                    on,
                });
            }
        }
    }
}

/// What an Ethernet flow's picker row says: its matrix frame name when the
/// flow matches exactly one definition, else the MAC pair, always with the bus.
fn eth_flow_label(app: &App, key: &crate::aggregate::EthFlowKey) -> String {
    let vlan = key
        .vlan
        .map(|v| format!("VLAN {v}"))
        .unwrap_or_else(|| "no VLAN".to_string());
    format!(
        "{}  {vlan}  {}",
        app.eth_bus_label(key.bus),
        app.eth_flow_name(key)
    )
}

/// Whether an Ethernet flow answers a picker query: its frame name, MAC pair,
/// bus label, VLAN and EtherType -- the same facts the Trace text filter reads.
fn eth_flow_matches(app: &App, key: &crate::aggregate::EthFlowKey, q: &str) -> bool {
    q.is_empty()
        || eth_flow_label(app, key).to_ascii_uppercase().contains(q)
        || format!("{:04X}", key.ethertype).contains(q)
        || key.vlan.is_some_and(|v| v.to_string().contains(q))
}

/// A compact, stable widget-id token for one flow. Names repeat (two VLANs can
/// carry the same frame definition), so the label alone cannot key the row.
fn eth_flow_tag(key: &crate::aggregate::EthFlowKey) -> String {
    let macs: String = key.src.iter().map(|b| format!("{b:02X}")).collect();
    let dmacs: String = key.dst.iter().map(|b| format!("{b:02X}")).collect();
    format!(
        "{}_{}_{}_{}_{:04X}_{}",
        key.bus,
        key.vlan
            .map(|v| v.to_string())
            .unwrap_or_else(|| "-".into()),
        macs,
        dmacs,
        key.ethertype,
        key.len
    )
}

/// The FlexRay slots a query offers, as `(bus, slot, frame names)`. Both the
/// checkbox list and "Select all matching" read this, so the two cannot drift
/// into admitting different slots. A slot matches on its number or on any frame
/// that holds it in some cycle phase; names are joined for the label, which is
/// also why they are listed rather than a single frame being named.
fn fr_slot_hits(app: &App, q: &str) -> Vec<(u8, u16, String)> {
    app.fr_buses
        .iter()
        .flat_map(|(bus, cfg)| cfg.db.scheduled_slots().into_iter().map(move |s| (*bus, s)))
        .filter_map(|(bus, (slot, frames))| {
            let hit =
                frames.iter().any(|f| f.to_ascii_uppercase().contains(q)) || slot.to_string() == q;
            hit.then(|| (bus, slot, frames.join(" / ")))
        })
        .collect()
}

/// Signal-level selection for one Graphics/Data window: a message → signal
/// checkbox tree over all buses (buses are grouping headers only; a window
/// may mix signals from any bus). Dear ImGui has no built-in multi-select
/// tree, so this composes TreeNode + Checkbox with cascading selection:
/// checking a message selects every signal listed below it, and labels show
/// (selected/total) counts.
fn signal_content(view: &mut IdFilterView, app: &App, ui: &Ui, cmds: &mut Vec<IdFilterCmd>) {
    let Some(target) = view.target else {
        return;
    };
    let selected: Vec<crate::observe::SigKey> = match target {
        PopupTarget::Graphics(i) => {
            let Some(w) = app.graphics.get(i) else {
                return;
            };
            w.signals.iter().map(|s| s.key.clone()).collect()
        }
        PopupTarget::Data(i) => {
            let Some(w) = app.data_windows.get(i) else {
                return;
            };
            w.signals.iter().map(|s| s.key.clone()).collect()
        }
        PopupTarget::State(i) => {
            let Some(w) = app.state_trackers.get(i) else {
                return;
            };
            w.signals.iter().map(|s| s.key.clone()).collect()
        }
        PopupTarget::Monitor => app
            .monitor_view
            .rows
            .iter()
            .map(|r| r.key.clone())
            .collect(),
        _ => return,
    };

    ui.set_next_item_width(ui.content_region_avail()[0]);
    ui.input_text("##sig_search", &mut view.sig_search)
        .hint("search messages / signals")
        .build();
    if ui.button("Clear##sigsel") {
        cmds.push(IdFilterCmd::ClearSignals {
            target,
            keys: selected.clone(),
        });
    }
    ui.same_line();
    ui.text(format!("{} signal(s) selected", selected.len()));
    ui.separator();

    let q = view.sig_search.trim().to_ascii_uppercase();
    let sel: HashSet<crate::observe::SigKey> = selected.into_iter().collect();
    let bus_names: Vec<String> = (0..app.snap.channel_count)
        .map(|ch| app.channel_name(ch as u8))
        .collect();

    // Clone the bus/message/signal layout first so toggle actions can be
    // applied afterwards without borrow conflicts. Every order entry lists:
    // a standard and an extended twin are two different messages, with two
    // distinct signal sets.
    let layout: Vec<Vec<TreeMsg>> = app
        .snap
        .channels
        .iter()
        .map(|channel| {
            channel
                .dbc
                .as_ref()
                .map(|db| {
                    db.order
                        .iter()
                        .filter_map(|&(id, ext)| {
                            let m = db.messages.get(&(id, ext))?;
                            let msg_hit = q.is_empty() || m.name.to_ascii_uppercase().contains(&q);
                            let signals: Vec<String> = m
                                .signals
                                .iter()
                                .filter(|s| {
                                    msg_hit
                                        || q.is_empty()
                                        || s.name.to_ascii_uppercase().contains(&q)
                                })
                                .map(|s| s.name.clone())
                                .collect();
                            if signals.is_empty() {
                                return None;
                            }
                            Some(TreeMsg {
                                id,
                                ext,
                                name: m.name.clone(),
                                signals,
                            })
                        })
                        .collect()
                })
                .unwrap_or_default()
        })
        .collect();

    let filtering = !q.is_empty();
    let mut actions: Vec<(crate::observe::SigKey, bool)> = Vec::new();

    for (ch, msgs) in layout.iter().enumerate() {
        let bus_keys: Vec<crate::observe::SigKey> = msgs
            .iter()
            .flat_map(|m| {
                m.signals
                    .iter()
                    .map(move |s| crate::observe::SigKey::can(ch as u8, m.id, m.ext, s.clone()))
            })
            .collect();
        let sel_n = bus_keys.iter().filter(|k| sel.contains(k)).count();
        ui.text_colored(
            [0.30, 0.80, 1.00, 1.0],
            format!("{}  ({sel_n}/{})", bus_names[ch], bus_keys.len()),
        );
        if msgs.is_empty() {
            ui.text("(no DBC or no matching messages on this bus)");
            continue;
        }
        for m in msgs {
            let msg_keys: Vec<crate::observe::SigKey> = m
                .signals
                .iter()
                .map(|s| crate::observe::SigKey::can(ch as u8, m.id, m.ext, s.clone()))
                .collect();
            let m_sel = msg_keys.iter().filter(|k| sel.contains(k)).count();
            let m_tot = msg_keys.len();
            let mut msg_on = m_sel == m_tot;
            if ui.checkbox(
                format!("##msgchk{ch}_{:X}{}", m.id, m.ext as u8),
                &mut msg_on,
            ) {
                for k in &msg_keys {
                    actions.push((k.clone(), msg_on));
                }
            }
            ui.same_line();
            let mtoken = ui
                .tree_node_config(format!(
                    "{:03X}{}  {} ({m_sel}/{m_tot})###selmsg{ch}_{:X}{}",
                    m.id,
                    if m.ext { "x" } else { "" },
                    m.name,
                    m.id,
                    if m.ext { "x" } else { "" }
                ))
                .default_open(filtering)
                .push();
            if mtoken.is_some() {
                for s in &m.signals {
                    let key = crate::observe::SigKey::can(ch as u8, m.id, m.ext, s.clone());
                    let mut son = sel.contains(&key);
                    if ui.checkbox(
                        format!("{s}##selsig{ch}_{:X}{}", m.id, m.ext as u8),
                        &mut son,
                    ) {
                        actions.push((key, son));
                    }
                }
            }
        }
    }

    emitted_section(app, ui, &sel, &mut actions);
    flexray_section(app, ui, &sel, &mut actions, &q);
    eth_section(app, ui, &sel, &mut actions, &q);

    cmds.extend(
        actions
            .into_iter()
            .map(|(key, on)| IdFilterCmd::ToggleSignal { target, key, on }),
    );
}

/// Adds a checkbox per emitted signal emitted by the script nodes
/// (`emit_value`): grouped by node below the DBC tree, because a derived
/// signal is selected exactly like a database one.
fn emitted_section(
    app: &App,
    ui: &Ui,
    sel: &HashSet<crate::observe::SigKey>,
    actions: &mut Vec<(crate::observe::SigKey, bool)>,
) {
    let streams: Vec<(crate::observe::SigKey, String)> = app.snap.emitted.clone();
    if streams.is_empty() {
        return;
    }
    ui.spacing();
    ui.separator();
    ui.text_colored(
        [0.75, 0.55, 1.0, 1.0],
        format!(
            "Derived signals  ({}/{})",
            streams.iter().filter(|(k, _)| sel.contains(k)).count(),
            streams.len()
        ),
    );
    ui.text_disabled("Derived signals published by script nodes via emit_value");
    for (key, node_name) in &streams {
        let mut on = sel.contains(key);
        // The id suffix only has to be unique per stream, so it spells
        // whatever identity the key carries: arbitration id for a CAN
        // (derived streams are CAN by construction), slot for a FlexRay one.
        let (ch, id) = match key {
            crate::observe::SigKey::Can { ch, id, .. } => (*ch, *id),
            crate::observe::SigKey::Fr { bus, slot, .. } => (*bus, u32::from(*slot)),
            crate::observe::SigKey::Eth { bus, .. } => (*bus, 0),
        };
        if ui.checkbox(
            format!("{}##emit{ch}_{id:X}_{node_name}", key.name()),
            &mut on,
        ) {
            actions.push((key.clone(), on));
        }
        ui.same_line();
        ui.text_disabled(format!("← {node_name}"));
    }
}

/// Adds a checkbox per FlexRay signal in every loaded cluster description,
/// grouped by bus, static slot and frame name below the derived-signal
/// section. A FlexRay signal joins the same subscription space as CAN signals
/// under its own `SigKey::Fr` identity, so selecting one plots its live values
/// exactly like a DBC signal. One row per frame that declares signals -- a slot
/// scheduled to several frames by cycle phase lists each of them, since the
/// first one alone would hide the rest of the cluster. Hidden until a FlexRay
/// database loads.
fn flexray_section(
    app: &App,
    ui: &Ui,
    sel: &HashSet<crate::observe::SigKey>,
    actions: &mut Vec<(crate::observe::SigKey, bool)>,
    q: &str,
) {
    if app.fr_buses.is_empty() {
        return;
    }
    // One entry per (bus, slot, frame): the same slot number on two clusters
    // names two different frames, and one slot on one cluster can be scheduled
    // for several frames in alternate cycles -- the bus is part of the identity
    // everywhere here, and the frame is what the entry documents.
    let hits: Vec<(u8, u16, String, Vec<String>)> = app
        .fr_buses
        .iter()
        .flat_map(|(bus, cfg)| {
            cfg.db
                .slot_signals()
                .into_iter()
                .map(move |(slot, frame, sigs)| (*bus, slot, frame, sigs))
        })
        .filter(|(_, slot, frame, sigs)| {
            q.is_empty()
                || frame.to_ascii_uppercase().contains(q)
                || sigs.iter().any(|s| s.to_ascii_uppercase().contains(q))
                || slot.to_string() == q
        })
        .collect();
    if hits.is_empty() {
        return;
    }
    let key = |bus: u8, slot: u16, s: &str| crate::app::fr_signal_key(bus, slot, s);
    let total: usize = hits.iter().map(|(_, _, _, s)| s.len()).sum();
    let sel_n: usize = hits
        .iter()
        .flat_map(|(bus, slot, _, sigs)| sigs.iter().map(move |s| key(*bus, *slot, s)))
        .filter(|k| sel.contains(k))
        .count();
    let multi = app.fr_buses.len() > 1;
    ui.spacing();
    ui.separator();
    ui.text_colored(
        [0.35, 0.80, 0.60, 1.0],
        format!("FlexRay ({sel_n}/{total})"),
    );
    ui.text_disabled("Signals decoded from the FlexRay description database");
    for (bus, slot, frame, sigs) in &hits {
        let keys: Vec<_> = sigs.iter().map(|s| key(*bus, *slot, s)).collect();
        let m_sel = keys.iter().filter(|k| sel.contains(k)).count();
        let mut msg_on = m_sel == keys.len() && !keys.is_empty();
        // The frame is part of the widget id: a slot scheduled to several
        // frames lists one row per occupant, and two rows sharing an id would
        // make ImGui draw the same checkbox twice.
        if ui.checkbox(format!("##frmsgchk{bus}_{slot}_{frame}"), &mut msg_on) {
            for k in &keys {
                actions.push((k.clone(), msg_on));
            }
        }
        ui.same_line();
        let mtoken = ui
            .tree_node_config(format!(
                "{}slot {slot}  {frame} ({m_sel}/{})",
                if multi {
                    format!("{}  ", app.fr_bus_name(*bus))
                } else {
                    String::new()
                },
                keys.len()
            ))
            .default_open(!q.is_empty())
            .push();
        if mtoken.is_some() {
            for (s, k) in sigs.iter().zip(&keys) {
                let mut son = sel.contains(k);
                if ui.checkbox(format!("{s}##frsig{bus}_{slot}_{frame}"), &mut son) {
                    actions.push((k.clone(), son));
                }
            }
        }
    }
}

/// Adds a checkbox per signal in every loaded ARXML Ethernet frame, grouped
/// by bus and frame below the FlexRay section. The identity is
/// `SigKey::Eth { bus, frame, name }`, the same key the core folds decoded
/// frames into, so a selection here samples and plots exactly like a DBC
/// signal. Hidden until an Ethernet matrix loads.
fn eth_section(
    app: &App,
    ui: &Ui,
    sel: &HashSet<crate::observe::SigKey>,
    actions: &mut Vec<(crate::observe::SigKey, bool)>,
    q: &str,
) {
    if app.eth_buses.is_empty() {
        return;
    }
    let hits: Vec<(u8, String, Vec<String>)> = app
        .eth_buses
        .iter()
        .flat_map(|(bus, cfg)| {
            cfg.db.frames.iter().map(move |f| {
                let sigs: Vec<String> = f
                    .pdus
                    .iter()
                    .flat_map(|p| p.signals.iter().map(|s| s.name.clone()))
                    .collect();
                (*bus, f.name.clone(), sigs)
            })
        })
        .filter(|(_, frame, sigs)| {
            q.is_empty()
                || frame.to_ascii_uppercase().contains(q)
                || sigs.iter().any(|s| s.to_ascii_uppercase().contains(q))
        })
        .collect();
    if hits.is_empty() {
        return;
    }
    let key = |bus: u8, frame: &str, s: &str| crate::observe::SigKey::eth(bus, frame, s);
    let total: usize = hits.iter().map(|(_, _, s)| s.len()).sum();
    let sel_n: usize = hits
        .iter()
        .flat_map(|(bus, frame, sigs)| sigs.iter().map(move |s| key(*bus, frame, s)))
        .filter(|k| sel.contains(k))
        .count();
    let multi = app.eth_buses.len() > 1;
    ui.spacing();
    ui.separator();
    ui.text_colored(
        [0.90, 0.70, 0.30, 1.0],
        format!("Ethernet ({sel_n}/{total})"),
    );
    ui.text_disabled("Signals decoded from the ARXML Ethernet matrix");
    for (bus, frame, sigs) in &hits {
        let keys: Vec<_> = sigs.iter().map(|s| key(*bus, frame, s)).collect();
        let m_sel = keys.iter().filter(|k| sel.contains(k)).count();
        let mut msg_on = m_sel == keys.len() && !keys.is_empty();
        if ui.checkbox(format!("##ethmsgchk{bus}_{frame}"), &mut msg_on) {
            for k in &keys {
                actions.push((k.clone(), msg_on));
            }
        }
        ui.same_line();
        let mtoken = ui
            .tree_node_config(format!(
                "{}{frame} ({m_sel}/{})",
                if multi {
                    format!("{}  ", app.eth_bus_label(*bus))
                } else {
                    String::new()
                },
                keys.len()
            ))
            .default_open(!q.is_empty())
            .push();
        if mtoken.is_some() {
            for (s, k) in sigs.iter().zip(&keys) {
                let mut son = sel.contains(k);
                if ui.checkbox(format!("{s}##ethsig{bus}_{frame}"), &mut son) {
                    actions.push((k.clone(), son));
                }
            }
        }
    }
}

fn msg_matches(name: Option<&str>, id: u32, q: &str) -> bool {
    if q.is_empty() {
        return true;
    }
    let hex = format!("{id:X}");
    hex.contains(q) || name.is_some_and(|n| n.to_ascii_uppercase().contains(q))
}
