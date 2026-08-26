use crate::app::{App, PopupTarget, SigScope};
use imgui::{Condition, TreeNodeFlags, Ui};
use std::collections::HashSet;

struct MsgEntry {
    id: u32,
    name: String,
    signals: Vec<String>,
}

struct TreeMsg {
    id: u32,
    name: String,
    signals: Vec<String>,
}

/// Shared "Signals" selector used by Trace/Messages/Statistics: All buses /
/// one bus / Manual selection. The "…" button opens the Message Selection
/// popup bound to `target`; selecting messages there switches the window to
/// Manual.
pub fn scope_combo(
    app: &mut App,
    ui: &Ui,
    id: &str,
    scope: SigScope,
    target: PopupTarget,
) -> SigScope {
    let mut items: Vec<String> = vec!["All buses".to_string()];
    for ch in 0..app.channels.len() {
        items.push(format!("Bus: {}", app.channel_name(ch as u8)));
    }
    let n = app.win_manual(target).map(|m| m.len()).unwrap_or(0);
    items.push(format!("Manual ({n})"));
    let manual_idx = items.len() - 1;
    let mut cur = match scope {
        SigScope::All => 0,
        SigScope::Bus(ch) => ((ch as usize) + 1).min(manual_idx),
        SigScope::Manual => manual_idx,
    };
    ui.set_next_item_width(140.0);
    ui.combo_simple_string(id, &mut cur, &items);
    ui.same_line();
    if ui.small_button(format!("…{id}")) {
        app.popup_target = Some(target);
        app.show_id_filter = true;
    }
    match cur {
        0 => SigScope::All,
        c if c == manual_idx => SigScope::Manual,
        c => SigScope::Bus((c - 1) as u8),
    }
}

/// Opens the selection popup for the window named by app.popup_target.
/// Trace/Messages/Statistics filter at the message level (flat list, hover a
/// message to see its signals); Graphics/Data filter at the signal level
/// (message tree with signal checkboxes).
pub fn render(app: &mut App, ui: &Ui) {
    if !app.show_id_filter {
        return;
    }
    let io = ui.io();
    let mut open = app.show_id_filter;
    let signal_level = matches!(
        app.popup_target,
        Some(PopupTarget::Graphics(_)) | Some(PopupTarget::Data(_))
    );
    let title = match app.popup_target {
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
    ui.window(title)
        .opened(&mut open)
        .position(
            [io.display_size[0] * 0.35, io.display_size[1] * 0.18],
            Condition::FirstUseEver,
        )
        .size([460.0, 480.0], Condition::FirstUseEver)
        .build(|| {
            if signal_level {
                signal_content(app, ui);
            } else {
                message_content(app, ui);
            }
        });
    app.show_id_filter = open;
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
    }
}

fn set_target_scope(app: &mut App, scope: SigScope) {
    if let Some(t) = app.popup_target {
        match t {
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
}

/// Message-level selection for Trace/Messages/Statistics: one checkbox per
/// message.
fn message_content(app: &mut App, ui: &Ui) {
    let Some(target) = app.popup_target else {
        return;
    };
    ui.set_next_item_width(ui.content_region_avail()[0]);
    ui.input_text("##idf_search", &mut app.id_filter_search)
        .hint("search messages / signals")
        .build();
    if ui.small_button("Select all matching") {
        let q = app.id_filter_search.trim().to_ascii_uppercase();
        let mut keys: Vec<(u8, u32)> = Vec::new();
        for (ch, channel) in app.channels.iter().enumerate() {
            let Some(db) = &channel.dbc else {
                continue;
            };
            for &id in &db.order {
                if msg_matches(db.message_name(id), id, &q) {
                    keys.push((ch as u8, id));
                }
            }
        }
        if let Some(m) = app.win_manual_mut(target) {
            for k in keys {
                m.insert(k);
            }
        }
        set_target_scope(app, SigScope::Manual);
    }
    ui.same_line();
    if ui.small_button("Clear") {
        if let Some(m) = app.win_manual_mut(target) {
            m.clear();
        }
        set_target_scope(app, SigScope::All);
    }
    ui.same_line();
    let n_sel = app.win_manual(target).map(|m| m.len()).unwrap_or(0);
    ui.text(format!("{n_sel} message(s) selected"));
    ui.separator();

    let q = app.id_filter_search.trim().to_ascii_uppercase();

    // Clone the message/signal layout first so the checkbox callbacks can
    // mutate the window's manual set without borrow conflicts.
    let channel_names: Vec<String> = (0..app.channels.len())
        .map(|ch| app.channel_name(ch as u8))
        .collect();
    let per_channel: Vec<Vec<MsgEntry>> = app
        .channels
        .iter()
        .map(|channel| {
            channel
                .dbc
                .as_ref()
                .map(|db| {
                    db.order
                        .iter()
                        .filter_map(|&id| {
                            let m = db.messages.get(&id)?;
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
            let key = (ch as u8, e.id);
            let mut on = app.win_manual(target).is_some_and(|m| m.contains(&key));
            // ID includes the bus: the same DBC on two buses would otherwise
            // produce identical labels and colliding widget IDs.
            let label = format!("{:03X}  {}##msgsel{ch}_{:X}", e.id, e.name, e.id);
            if ui.checkbox(label, &mut on) {
                if let Some(m) = app.win_manual_mut(target) {
                    if on {
                        m.insert(key);
                    } else {
                        m.remove(&key);
                    }
                }
                set_target_scope(app, SigScope::Manual);
            }
        }
    }
}

/// Signal-level selection for one Graphics/Data window: a message → signal
/// checkbox tree over all buses (buses are grouping headers only; a window
/// may mix signals from any bus). Dear ImGui has no built-in multi-select
/// tree, so this composes TreeNode + Checkbox with cascading selection:
/// checking a message selects every signal listed below it, and labels show
/// (selected/total) counts.
fn signal_content(app: &mut App, ui: &Ui) {
    let Some(target) = app.popup_target else {
        return;
    };
    let selected: Vec<(u8, u32, String)> = match target {
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
        _ => return,
    };

    ui.set_next_item_width(ui.content_region_avail()[0]);
    ui.input_text("##sig_search", &mut app.symbol_search)
        .hint("search messages / signals")
        .build();
    if ui.small_button("Clear##sigsel") {
        for key in selected.clone() {
            app.set_win_signal(target, key, false);
        }
    }
    ui.same_line();
    ui.text(format!("{} signal(s) selected", selected.len()));
    ui.separator();

    let q = app.symbol_search.trim().to_ascii_uppercase();
    let sel: HashSet<(u8, u32, String)> = selected.into_iter().collect();
    let bus_names: Vec<String> = (0..app.channels.len())
        .map(|ch| app.channel_name(ch as u8))
        .collect();

    // Clone the bus/message/signal layout first so toggle actions can be
    // applied afterwards without borrow conflicts.
    let layout: Vec<Vec<TreeMsg>> = app
        .channels
        .iter()
        .map(|channel| {
            channel
                .dbc
                .as_ref()
                .map(|db| {
                    db.order
                        .iter()
                        .filter_map(|&id| {
                            let m = db.messages.get(&id)?;
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
    let open_flags = if filtering {
        TreeNodeFlags::DEFAULT_OPEN
    } else {
        TreeNodeFlags::empty()
    };
    let mut actions: Vec<((u8, u32, String), bool)> = Vec::new();

    for (ch, msgs) in layout.iter().enumerate() {
        let bus_keys: Vec<(u8, u32, String)> = msgs
            .iter()
            .flat_map(|m| m.signals.iter().map(move |s| (ch as u8, m.id, s.clone())))
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
            let msg_keys: Vec<(u8, u32, String)> = m
                .signals
                .iter()
                .map(|s| (ch as u8, m.id, s.clone()))
                .collect();
            let m_sel = msg_keys.iter().filter(|k| sel.contains(k)).count();
            let m_tot = msg_keys.len();
            let mut msg_on = m_sel == m_tot;
            if ui.checkbox(format!("##msgchk{ch}_{:X}", m.id), &mut msg_on) {
                for k in &msg_keys {
                    actions.push((k.clone(), msg_on));
                }
            }
            ui.same_line();
            let mtoken = ui
                .tree_node_config(format!(
                    "{:03X}  {} ({m_sel}/{m_tot})###selmsg{ch}_{:X}",
                    m.id, m.name, m.id
                ))
                .flags(open_flags)
                .push();
            if mtoken.is_some() {
                for s in &m.signals {
                    let key = (ch as u8, m.id, s.clone());
                    let mut son = sel.contains(&key);
                    if ui.checkbox(format!("{s}##selsig{ch}_{:X}", m.id), &mut son) {
                        actions.push((key, son));
                    }
                }
            }
        }
    }

    for (key, on) in actions {
        app.set_win_signal(target, key, on);
    }
}

fn msg_matches(name: Option<&str>, id: u32, q: &str) -> bool {
    if q.is_empty() {
        return true;
    }
    let hex = format!("{id:X}");
    hex.contains(q) || name.is_some_and(|n| n.to_ascii_uppercase().contains(q))
}
