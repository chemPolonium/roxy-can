use crate::app::{App, GenRow, TOOLBAR_H, TX_CYCLE_MAX_MS, cycle_from_ms_text};
use crate::dbc::SignalInfo;
use crate::sim::{KINDS, SrcKind, ValueSrc};
use crate::ui::help::popup_is_open;
use dear_imgui_rs::{Condition, Drag, InputTextFlags, Key, NumericFormat, StyleVar, TreeNodeFlags, Ui};

/// Combo entries for a signal's source: "Constant" first so picking index 0
/// means "hold the base value", the rest in [`KINDS`] order.
///
/// The name is a label only. Index 0 stores no source at all and the value
/// lives in the message's base payload, so the persisted kind codes -- which
/// are [`KINDS`] positions -- do not move.
fn kind_labels() -> Vec<String> {
    let mut v = vec!["Constant".to_string()];
    v.extend(KINDS.iter().map(|k| k.label().to_string()));
    v
}

/// One signal as the generator's row draws it: position, encoding and declared
/// range. The two databases spell their fields differently, and the row code
/// should not have to know which bus it is editing.
struct SigRow {
    name: String,
    bit: u64,
    size: u64,
    big_endian: bool,
    signed: bool,
    factor: f64,
    offset: f64,
    min: f64,
    max: f64,
    unit: String,
}

impl From<&SignalInfo> for SigRow {
    fn from(s: &SignalInfo) -> Self {
        Self {
            name: s.name.clone(),
            bit: s.start_bit,
            size: s.size,
            big_endian: s.big_endian,
            signed: s.signed,
            factor: s.factor,
            offset: s.offset,
            min: s.min,
            max: s.max,
            unit: s.unit.clone(),
        }
    }
}

impl From<crate::fr_db::FrEditSignal> for SigRow {
    fn from(s: crate::fr_db::FrEditSignal) -> Self {
        Self {
            name: s.name,
            bit: s.bit,
            size: s.size,
            big_endian: s.big_endian,
            signed: s.signed,
            factor: s.factor,
            offset: s.offset,
            min: s.min,
            max: s.max,
            unit: s.unit,
        }
    }
}

/// Usable drag range for a signal: declared min/max when sane, otherwise the
/// raw bit-range scaled by factor/offset.
fn sig_range(s: &SigRow) -> (f32, f32) {
    if s.min.is_finite() && s.max.is_finite() && s.min < s.max {
        return (s.min as f32, s.max as f32);
    }
    let bits = s.size.min(48) as i32;
    let (rmin, rmax): (f64, f64) = if s.signed {
        let half = (2f64).powi(bits - 1);
        (-half, half - 1.0)
    } else {
        (0.0, (2f64).powi(bits) - 1.0)
    };
    (
        (rmin * s.factor + s.offset) as f32,
        (rmax * s.factor + s.offset) as f32,
    )
}

/// Comma-separated physical values for a step sequence. Blank entries are
/// dropped, so a trailing comma does not add a zero-length step.
fn parse_seq(s: &str) -> Vec<f64> {
    s.split(',')
        .map(|t| t.trim())
        .filter(|t| !t.is_empty())
        .filter_map(|t| t.parse::<f64>().ok())
        .collect()
}

pub fn render(app: &mut App, ui: &Ui) {
    let kinds = kind_labels();
    // The overview window: unassigned entries, the bulk switches and
    // add-by-id. Every DBC node's generator lives in the Network
    // window's node detail now.
    let main_open = render_overview(app, ui);
    // The modals serve the overview and the Network-embedded node
    // panels alike; they early-return while nothing is being edited.
    params_modal(app, ui, &kinds);
    cycle_modal(app, ui);
    app.show_tx = main_open;
}

/// Which entries the shared row renderer shows.
enum Scope {
    /// Entries that belong to no DBC node: the overview's whole list.
    Unassigned,
    /// One node's entries: the Network detail's generator panel.
    Node(u8, String),
}

/// The Interactive Generator overview: everything that has no node to
/// live under. Node-owned entries are edited in the Network window.
fn render_overview(app: &mut App, ui: &Ui) -> bool {
    let io = ui.io();
    let kinds = kind_labels();
    let mut open = app.show_tx;
    if !open {
        return false;
    }
    ui.window("Interactive Generator")
        .opened(&mut open)
        .position(
            [
                io.display_size()[0] * 0.62,
                TOOLBAR_H + 10.0,
            ],
            Condition::FirstUseEver,
        )
        .size([560.0, 340.0], Condition::FirstUseEver)
        .build(|| {
            ui.set_next_item_width(200.0);
            ui.input_text("##gsearch", &mut app.gen_search)
                .hint("search name / ID")
                .build();
            ui.same_line();
            if ui.button("Clear##gsc") {
                app.gen_search.clear();
            }
            ui.same_line();
            let active_count = app
                .snap
                .tx
                .iter()
                .filter(|t| t.active && t.node.is_empty())
                .count();
            ui.text(format!("{active_count} active (未分配)"));
            ui.separator();

            // Per-bus lines: add-by-id -- the way to put a message the
            // database does not know onto the wire. Bulk switches live
            // with the node entries in the Network window; this window
            // only owns the unassigned entries.
            for ch in 0..app.snap.channel_count {
                let ch8 = ch as u8;
                ui.text(app.channel_name(ch8));
                ui.same_line();
                let editing = matches!(&app.gen_add_buf, Some((r, _)) if *r == ch8);
                let mut buf = match &app.gen_add_buf {
                    Some((r, s)) if *r == ch8 => s.clone(),
                    _ => String::new(),
                };
                ui.set_next_item_width(90.0);
                ui.input_text(format!("##gaddid{ch}"), &mut buf)
                    .hint("hex id")
                    .flags(InputTextFlags::CHARS_HEXADECIMAL)
                    .build();
                if ui.is_item_active() {
                    app.gen_add_buf = Some((ch8, buf.clone()));
                }
                ui.same_line();
                let add = ui.button(format!("Add##gadd{ch}"));
                if ui.is_item_deactivated_after_edit() && !buf.is_empty() {
                    app.gen_add_buf = None;
                    add_hex_id(app, ch8, &buf);
                } else if editing && !ui.is_item_active() {
                    app.gen_add_buf = None;
                } else if add {
                    app.gen_add_buf = None;
                    add_hex_id(app, ch8, &buf);
                }
            }
            ui.separator();

            let tx = app.snap.tx.clone();
            render_rows(app, ui, &tx, &kinds, &Scope::Unassigned);
        });
    open
}

/// Adds one manual entry by hex id (standard frames only -- extended
/// messages arrive through their DBC node). A duplicate is a no-op.
fn add_hex_id(app: &mut App, ch: u8, text: &str) {
    match u32::from_str_radix(text.trim(), 16) {
        Ok(id) if id <= 0x7FF => app.add_tx(ch, id),
        Ok(_) => app.status = "手动添加只支持标准帧 id（≤ 7FF）".to_string(),
        Err(_) => app.status = format!("hex id 解析失败：{text}"),
    }
}

/// One node's generator panel for the Network detail: the summary and
/// wire switch, this node's entries, its add combo, and its reaction
/// rules -- the interactive half of the node (the scripts are the other
/// half, right below in the same detail pane).
pub fn render_node_generator(app: &mut App, ui: &Ui, nch: u8, nname: &str) {
    let kinds = kind_labels();
    let total = app
        .snap
        .tx
        .iter()
        .filter(|t| t.channel == nch && t.node == nname)
        .count();
    let active_n = app
        .snap
        .tx
        .iter()
        .filter(|t| t.active && t.channel == nch && t.node == nname)
        .count();
    let role = app.node_role(nch, nname);
    let count_word = if role == crate::app::NodeRole::Simulated {
        "发送中"
    } else {
        "条启用 · 总关"
    };
    ui.text(format!("生成器：{active_n}/{total} {count_word}"));
    if ui.is_item_hovered() {
        ui.tooltip_text(role.hint());
    }

    // Add: this node's own messages from its bus's databases.
    let mut ids: Vec<u32> = Vec::new();
    let mut names: Vec<String> = Vec::new();
    if let Some(db) = app
        .snap
        .channels
        .get(nch as usize)
        .and_then(|c| c.dbc.as_ref())
    {
        let mut listed: std::collections::HashSet<u32> = std::collections::HashSet::new();
        for &(id, _) in &db.order {
            if !listed.insert(id) {
                continue;
            }
            if let Some(m) = db.message_of(id)
                && m.transmitter == nname
            {
                ids.push(id);
                names.push(format!("{id:03X}  {}", m.name));
            }
        }
    }
    if !ids.is_empty() {
        if app.tx_pick >= ids.len() {
            app.tx_pick = 0;
        }
        ui.set_next_item_width(260.0);
        ui.combo_simple_string("Message##netnode", &mut app.tx_pick, &names);
        ui.same_line();
        if ui.button("Add##netnodeadd") {
            app.add_tx(nch, ids[app.tx_pick]);
        }
    }

    let tx = app.snap.tx.clone();
    render_rows(app, ui, &tx, &kinds, &Scope::Node(nch, nname.to_string()));

    // Reactions aimed at this node's messages: the Send triggers that
    // answer this node's traffic. Managed in the Triggers window.
    let node_ids: std::collections::HashSet<u32> = tx
        .iter()
        .filter(|v| v.channel == nch && v.node == nname)
        .map(|v| v.id)
        .collect();
    let reactions: Vec<String> = app
        .snap
        .triggers
        .iter()
        .filter(|t| {
            matches!(&t.action,
                crate::trigger::TriggerAction::Send { ch, id }
                if *ch == nch && node_ids.contains(id))
        })
        .map(|t| t.cond.short())
        .collect();
    if !reactions.is_empty() {
        ui.separator();
        ui.text("响应规则（触发器 -> 发送到本节点报文）");
        for r in &reactions {
            ui.text(format!("  · {r}"));
        }
        if ui.button("在 Triggers 窗口管理##noder") {
            app.show_triggers = true;
        }
    }
}
/// The shared entry-row renderer: chip, row header, schedule controls
/// and the per-signal handles. `scope` picks whose entries show --
/// unassigned ones in the overview, one node's in the Network detail.
fn render_rows(app: &mut App, ui: &Ui, tx: &[crate::bus::TxView], kinds: &[String], scope: &Scope) {
    let query = app.gen_search.trim().to_ascii_lowercase();
    let mut remove: Option<(u8, u32)> = None;
    for (i, view) in tx.iter().enumerate() {
        let id = view.id;
        let ch = view.channel;
        let name = view.name.clone();
        match scope {
            Scope::Unassigned => {
                if !view.node.is_empty() {
                    continue;
                }
                if !query.is_empty() {
                    let hay =
                        format!("{} {} {:X}", app.channel_name(ch), name, id).to_ascii_lowercase();
                    if !hay.contains(&query) {
                        continue;
                    }
                }
            }
            Scope::Node(nch, nname) => {
                if ch != *nch || view.node != *nname {
                    continue;
                }
            }
        }
        let sigs: Vec<SignalInfo> = app
            .channel_dbc(ch)
            .and_then(|db| db.message_of(id))
            .map(|m| m.signals.clone())
            .unwrap_or_default();
        let driven = view.srcs.len();
        // The transmit state rides the header line, so the whole
        // list scans without expanding anything. MUTE is the
        // replay silencing, precomputed by the bus: the checkbox
        // keeps its state, but an id the replayed log carries
        // must not double-send. 总关 is the node-role gate: the
        // entry is on, but its node is not simulated.
        let (chip, color, hint) = if !view.active {
            (
                "OFF",
                [0.55, 0.58, 0.65, 1.0],
                "未发送：条目生成开关未勾选。",
            )
        } else if !view.gate_open {
            (
                "总关",
                [0.45, 0.60, 0.80, 1.0],
                "条目已启用，但所属节点的角色不是「模拟」——总开关关闭中。把节点角色切回「模拟」即恢复发车。",
            )
        } else if view.muted {
            (
                "MUTE",
                [1.0, 0.65, 0.2, 1.0],
                "本次回放期间静音：已加载的日志中带有此 ID，若再有第二个发送者，同一条信号的两路数据会混进曲线、统计等所有视图。On 勾选框保持原样——退出回放后照常发送。",
            )
        } else {
            (
                "ON",
                [0.4, 0.95, 0.5, 1.0],
                "正在发送：条目已勾选，且没有被任何机制抑制。",
            )
        };
        // Four cells wide in the monospace font, so the three
        // words -- and every row's header -- line up.
        ui.text_colored(color, format!("{chip:<4}"));
        if ui.is_item_hovered() {
            ui.tooltip_text(hint);
        }
        ui.same_line();
        let header_open = ui.collapsing_header(
            row_header(ch, &app.channel_name(ch), &name, id, driven),
            TreeNodeFlags::empty(),
        );
        if !header_open {
            continue;
        }
        ui.indent();
        let mut act = view.active;
        if ui.checkbox(format!("On##{i}"), &mut act) {
            // Routes through the model: activating anchors the
            // schedule at the current clock, so re-enabling an
            // entry never re-emits frames dated across the time
            // it was off.
            app.send(crate::bus::BusCommand::SetEntryActive { ch, id, on: act });
        }
        ui.same_line();
        // Not an inline number box any more: dragging one edits its
        // text in place, and every keystroke was applied, so dialing
        // in 100 put the message on the wire at 1 ms first. The
        // dialog drafts it and only writes on Apply.
        let cycle = view.cycle_us;
        let cyc = if cycle == 0 {
            "event".to_string()
        } else {
            format!("{} ms", cycle / 1000)
        };
        if ui.button_with_size(format!("{cyc}##cyc{i}"), [84.0, 0.0]) {
            app.tx_cycle_edit = Some(GenRow::Can(i));
            app.tx_cycle_buf = (cycle / 1000).to_string();
        }
        ui.same_line();
        let mut fd = view.fd;
        if ui.checkbox(format!("FD##{i}"), &mut fd) {
            app.send(crate::bus::BusCommand::SetEntryFd { ch, id, fd });
        }
        ui.same_line();
        // One frame now, off the schedule: base bytes and
        // waveforms as they stand, without touching the active
        // flag. A stopped bus drops the request silently.
        if ui.button(format!("Send now##now{i}")) {
            app.send(crate::bus::BusCommand::SendNow { ch, id });
        }
        // Only ever shown when the two disagree, so a row that
        // matches its database stays exactly as wide as before.
        let off = app.dbc_cycle_us(ch, id).filter(|d| *d != cycle);
        if let Some(declared) = off {
            ui.same_line();
            let label = if declared == 0 {
                "DBC event".to_string()
            } else {
                format!("DBC {}ms", declared / 1000)
            };
            if ui.button(format!("{label}##dbc{i}")) {
                app.send(crate::bus::BusCommand::SetEntryCycle {
                    ch,
                    id,
                    cycle_us: declared,
                });
            }
        }
        ui.same_line();
        // Values are edited through the signal handles below. For
        // a message the database knows, the box only shows the
        // bytes that actually go out -- base payload with every
        // driven source's value already laid over it, computed by
        // the bus into this frame's snapshot. Only a message
        // without DBC signals keeps an editable box, because it
        // has no handles to edit instead.
        if sigs.is_empty() {
            // The live edit buffer is frontend draft state: while
            // the box has focus the text lives in `tx_data_edit`,
            // and the bus only sees the payload when the edit
            // commits. Decoding waits for the box to be left --
            // parsing each keystroke meant retyping "11 22 33"
            // briefly put a one-byte frame on the bus.
            let editing = matches!(&app.tx_data_edit, Some((r, _)) if *r == i);
            let mut buf = match &app.tx_data_edit {
                Some((r, s)) if *r == i => s.clone(),
                _ => view.data_text.clone(),
            };
            ui.set_next_item_width(if off.is_some() { 200.0 } else { 260.0 });
            ui.input_text(format!("##data{i}"), &mut buf).build();
            if ui.is_item_active() {
                app.tx_data_edit = Some((i, buf.clone()));
            }
            if ui.is_item_deactivated_after_edit() {
                app.tx_data_edit = None;
                app.send(crate::bus::BusCommand::SetEntryHex { ch, id, text: buf });
            } else if editing && !ui.is_item_active() {
                app.tx_data_edit = None;
            }
        } else {
            ui.text_disabled(&view.sent_text);
        }
        ui.same_line();
        if ui.button(format!("x##{i}")) {
            remove = Some((ch, id));
        }

        if sigs.is_empty() {
            ui.text("(no signals in DBC)");
        }
        // The bytes that actually go out this instant: base with
        // every driven source laid over them, from the snapshot.
        // Driven rows read their displayed value back out of
        // these, so what you see is what the bus sees --
        // byte-width truncation and all. The raw computed number
        // never reaches the wire.
        // The bytes that actually go out this instant: base with every
        // driven source laid over them, from the snapshot.
        let rows: Vec<SigRow> = sigs.iter().map(SigRow::from).collect();
        signal_rows(
            app,
            ui,
            GenRow::Can(i),
            "c",
            kinds,
            RowSignals {
                sigs: &rows,
                data: &view.sent_data,
                srcs: &view.srcs,
            },
        );
        ui.unindent();
    }
    if let Some((ch, id)) = remove {
        app.send(crate::bus::BusCommand::RemoveEntry { ch, id });
    }
}

/// What the signal handles of one row work from: the signals the database
/// declares, the payload actually going out, and the sources driving it.
struct RowSignals<'a> {
    sigs: &'a [SigRow],
    data: &'a [u8],
    srcs: &'a [ValueSrc],
}

/// The signal handles of one generator row: the drag that pins a value into the
/// base payload, the shape combo that drives it over time, and the params
/// button. Shared by the CAN and FlexRay halves because the whole draft/commit
/// discipline -- and the failure it prevents -- is the same in both: a value
/// that cannot be encoded must never reach the wire as a partial one.
///
/// `tag` keeps the two lists' widget ids apart (`c3` and `f3` are different
/// rows); `data` is the payload actually going out, so a driven row reads its
/// displayed value back out of the bytes the bus will send, truncation and all.
fn signal_rows(app: &mut App, ui: &Ui, row: GenRow, tag: &str, kinds: &[String], s: RowSignals<'_>) {
    let RowSignals { sigs, data, srcs } = s;
    let id = row_index(row);
    for sig in sigs {
        let name = sig.name.as_str();
        let held = srcs.iter().find(|x| x.name == name).cloned();
        let raw = crate::decode::extract_raw(data, sig.bit, sig.size, sig.big_endian);
        let cur =
            crate::decode::to_physical(raw, sig.size, sig.signed, sig.factor, sig.offset);
        let (lo, hi) = sig_range(sig);
        // What the handle shows is the value in the bytes going out this
        // instant. For a driven signal that is the wave's own value, so the
        // handle is disabled: grabbing it used to pin the signal and silently
        // drop the source, which read like the wave simply breaking. Un-drive
        // through the kind combo's "Constant" instead.
        let model_shown = cur as f32;
        // Pinning rewrites the base payload and clears this signal's source, so
        // doing it per keystroke would let a half-typed 100 encode as 1 and cut
        // the wave off with it. The draft carries the preview; the model waits.
        let key = format!("sig{tag}{id}{name}");
        let mut shown = app.num_draft.shown(&key, model_shown as f64) as f32;
        ui.set_next_item_width(180.0);
        let mut v = shown;
        let _read_only = held.is_some().then(|| ui.begin_disabled_with_cond(true));
        let sig_fmt = NumericFormat::new("%g").expect("static format");
        let moved = Drag::new(format!("{name}##sig{tag}{id}_{name}"))
            .display_format(sig_fmt)
            .speed(((hi - lo) / 200.0).max(0.01))
            .range(lo, hi)
            .build(ui, &mut v);
        let ends = ui.is_item_deactivated();
        let committed = app.num_draft.step(
            &key,
            v as f64,
            moved,
            ui.is_item_deactivated_after_edit(),
            ends,
        );
        drop(_read_only);
        if let Some(val) = committed {
            // Fire-and-forget from the UI: whether the database can encode the
            // value is the bus's call, and a failed pin simply changes nothing.
            app.pin_gen_signal(row, name, val);
            shown = val as f32;
        }
        ui.same_line();
        ui.set_next_item_width(80.0);
        let mut pick = match held.as_ref() {
            None => 0,
            Some(h) => 1 + KINDS.iter().position(|k| *k == h.kind).unwrap_or(0),
        };
        if ui.combo_simple_string(format!("##src{tag}{id}_{name}"), &mut pick, kinds) {
            if pick == 0 {
                app.clear_gen_source(row, name);
            } else {
                let kind = KINDS[pick - 1];
                // Enabling snapshots lo/hi from the declared range; changing
                // shape afterwards keeps whatever the user has since edited in
                // the modal.
                let src = match held.as_ref() {
                    Some(h) => ValueSrc { kind, ..h.clone() },
                    None => ValueSrc::new(name, kind, lo as f64, hi as f64),
                };
                app.set_gen_source(row, src);
            }
        }
        if let Some(h) = &held {
            ui.same_line();
            if ui.button(format!("…##pp{tag}{id}_{name}")) {
                app.src_edit = Some((row, name.to_string()));
                app.src_draft = Some(h.clone());
                app.src_seq_buf = h
                    .seq
                    .iter()
                    .map(|v| format!("{v}"))
                    .collect::<Vec<_>>()
                    .join(", ");
            }
            ui.same_line();
            ui.text(format!("{shown} {}", sig.unit));
        } else {
            ui.same_line();
            ui.text(format!("{} {}", cur, sig.unit));
        }
    }
}

/// The index part of a generator row's widget id.
fn row_index(row: GenRow) -> usize {
    match row {
        GenRow::Can(i) | GenRow::Fr(i) => i,
    }
}

/// One FlexRay ECU's generator panel, in the Network detail: the frames the
/// description binds to this ECU as its sender, this node's entries, and the
/// same row controls a CAN node's panel has. Grouping by sender is the whole
/// point -- which node owns which frame is what the FIBEX/ARXML declares, so
/// what it does not declare gets no entry anywhere rather than a floating one
/// the tool would have to invent an owner for.
///
/// No role switch either: a role is the DBC node's gate and the FlexRay side has
/// none, so the entry's own On is the only switch (besides the replay's MUTE).
pub fn render_fr_ecu_generator(app: &mut App, ui: &Ui, bus: u8, ecu: &str) {
    let kinds = kind_labels();
    let all = app.snap.fr_tx.clone();
    let mine: Vec<(usize, crate::bus::FrTxView)> = all
        .iter()
        .enumerate()
        .filter(|(_, t)| t.bus == bus && t.node == ecu)
        .map(|(i, t)| (i, t.clone()))
        .collect();
    let active_n = mine.iter().filter(|(_, t)| t.active).count();
    ui.text(format!("生成器：{active_n}/{} 发送中", mine.len()));
    if ui.is_item_hovered() {
        ui.tooltip_text("端口只收：这些帧进入本会话（Trace、统计、规格、脚本、录制），不上线缆。");
    }
    ui.separator();

    // Add: the slots this ECU is the declared sender of.
    let mut slots: Vec<(u16, String)> = Vec::new();
    if let Some(db) = app.fr_db(bus) {
        for ix in 0..db.frames.len() {
            if db.frame_sender(ix) != Some(ecu) {
                continue;
            }
            let Some(f) = db.frame_index(ix) else {
                continue;
            };
            let slot = f.triggering.slot_id as u16;
            let label = if f.name.is_empty() {
                format!("slot{slot}")
            } else {
                f.name.clone()
            };
            match slots.iter_mut().find(|(s, _)| *s == slot) {
                Some((_, names)) => {
                    if !names.split('/').any(|n| n == label) {
                        names.push('/');
                        names.push_str(&label);
                    }
                }
                None => slots.push((slot, label)),
            }
        }
    }
    if slots.is_empty() {
        ui.text_disabled("描述没有把这路里的任何帧绑定给这个 ECU 发送：这里没有可加的条目");
    } else {
        let labels: Vec<String> = slots
            .iter()
            .map(|(slot, names)| format!("slot {slot}  {names}"))
            .collect();
        let refs: Vec<&str> = labels.iter().map(|s| s.as_str()).collect();
        let mut pick = app
            .fr_tx_pick
            .get(&bus)
            .copied()
            .unwrap_or(0)
            .min(labels.len() - 1);
        ui.set_next_item_width(260.0);
        if ui.combo_simple_string(format!("##frslot{bus}"), &mut pick, &refs) {
            app.fr_tx_pick.insert(bus, pick);
        }
        ui.same_line();
        if ui.button(format!("Add##fradd{bus}")) {
            app.add_fr_tx(bus, slots[pick].0);
        }
    }
    let mut remove: Option<(u8, u16)> = None;
    for (i, view) in &mine {
        if let Some(key) = fr_row(app, ui, *i, view, &kinds) {
            remove = Some(key);
        }
    }
    if let Some((bus, slot)) = remove {
        app.send(crate::bus::BusCommand::RemoveFrEntry { bus, slot });
    }
}

/// One FlexRay slot entry: the ON/OFF/MUTE header a CAN row has, then On, the
/// cycle dialog's button, Send now, the payload, and the signal handles. The
/// header's identity is `###`-suffixed with `(bus, slot)` so a changing badge or
/// frame name cannot collapse the row mid-edit.
fn fr_row(
    app: &mut App,
    ui: &Ui,
    i: usize,
    view: &crate::bus::FrTxView,
    kinds: &[String],
) -> Option<(u8, u16)> {
    let (bus, slot) = (view.bus, view.slot);
    let (chip, color, hint) = if !view.active {
        (
            "OFF",
            [0.55, 0.58, 0.65, 1.0],
            "未发送：条目生成开关未勾选。",
        )
    } else if view.muted {
        (
            "MUTE",
            [1.0, 0.65, 0.2, 1.0],
            "本次回放期间静音：日志里这一路本来就带这个槽的帧，再发一份会让同一条信号有两个发送者，混进每个视图。On 勾选框保持原样——退出回放后照常发车。",
        )
    } else {
        (
            "ON",
            [0.4, 0.95, 0.5, 1.0],
            "正在发出：条目已勾选，没有被任何机制抑制。端口只收，所以这些帧进的是本会话（Trace、统计、规格、脚本、录制），不是总线线缆。",
        )
    };
    ui.text_colored(color, format!("{chip:<4}"));
    if ui.is_item_hovered() {
        ui.tooltip_text(hint);
    }
    ui.same_line();
    let badge = if view.srcs.is_empty() {
        String::new()
    } else {
        format!("  {} driven", view.srcs.len())
    };
    let header = format!("slot {slot}  {}{badge}###frgen{bus}_{slot}", view.name);
    if !ui.collapsing_header(header, TreeNodeFlags::empty()) {
        return None;
    }
    ui.indent();
    let mut act = view.active;
    if ui.checkbox(format!("On##fron{i}"), &mut act) {
        app.send(crate::bus::BusCommand::SetFrEntryActive {
            bus,
            slot,
            on: act,
        });
    }
    ui.same_line();
    let cycle = view.cycle_us;
    // The period, in the unit the bus counts in: cycles for a scheduled slot,
    // milliseconds for anything else. "不发" rather than CAN's "event" for a zero
    // period, because a static slot has no event-triggered mode -- 0 there just
    // means the entry never sends.
    let cycle_time = app.fr_cycle_time_us(bus);
    let cycles = crate::generator::cycles_of(cycle, cycle_time.unwrap_or(0));
    let (cyc, cyc_hint) = match (cycle, cycles) {
        (0, _) => (
            "不发".to_string(),
            "周期 0：这个条目不会自己发车（Send now 仍然可以送一帧）。要恢复正常，点这个按钮填周期数。".to_string(),
        ),
        (_, Some(n)) => (
            format!("{n} 周期"),
            format!(
                "每 {n} 个通信周期发一帧 = {} ms。静态槽由调度表决定什么时候发，所以这里数的是周期，不是毫秒。",
                cycle / 1000
            ),
        ),
        _ => (
            format!("{} ms", cycle / 1000),
            match cycle_time {
                Some(_) => "不在周期网格的整倍数上：这一路描述声明的周期数除不进这个值。".to_string(),
                None => "这一路没有集群描述（或没声明周期时间），只能按毫秒发。".to_string(),
            },
        ),
    };
    if ui.button_with_size(format!("{cyc}##frcyc{i}"), [84.0, 0.0]) {
        app.tx_cycle_edit = Some(GenRow::Fr(i));
        app.tx_cycle_buf = match cycles {
            Some(n) => n.to_string(),
            None => (cycle / 1000).to_string(),
        };
    }
    if ui.is_item_hovered() {
        ui.tooltip_text(cyc_hint);
    }
    ui.same_line();
    if ui.button(format!("Send now##frnow{i}")) {
        app.send(crate::bus::BusCommand::SendFrNow { bus, slot });
    }
    // Only when the two disagree, exactly like the CAN row's button: one click
    // puts the slot back on the period its own schedule declares.
    let off = app.fr_declared_period_us(bus, slot).filter(|d| *d != cycle);
    if let Some(declared) = off {
        ui.same_line();
        let label = match crate::generator::cycles_of(declared, cycle_time.unwrap_or(0)) {
            Some(n) => format!("描述 {n} 周期"),
            None => format!("描述 {}ms", declared / 1000),
        };
        if ui.button(format!("{label}##frdbc{i}")) {
            app.send(crate::bus::BusCommand::SetFrEntryCycle {
                bus,
                slot,
                cycle_us: declared,
            });
        }
    }
    ui.same_line();
    let remove = if ui.button(format!("x##frrm{i}")) {
        Some((bus, slot))
    } else {
        None
    };
    let sigs: Vec<SigRow> = app
        .fr_db(bus)
        .and_then(|db| Some((db, db.frame_ix_of_slot(slot)?)))
        .map(|(db, ix)| db.edit_signals(ix).into_iter().map(SigRow::from).collect())
        .unwrap_or_default();
    if sigs.is_empty() {
        // Nothing describes this slot's payload, so the bytes are the whole
        // edit -- the same rule as a CAN message with no DBC signals.
        let editing = matches!(&app.fr_data_edit, Some((r, _)) if *r == i);
        let mut buf = match &app.fr_data_edit {
            Some((r, s)) if *r == i => s.clone(),
            _ => view.data_text.clone(),
        };
        ui.set_next_item_width(260.0);
        ui.input_text(format!("##frdata{i}"), &mut buf).build();
        if ui.is_item_hovered() {
            // The width is the schedule's, not the operator's: say so before the
            // first refusal, not after. An undescribed slot falls back to the
            // length its entry carries.
            let wide = app.fr_declared_len(bus, slot).unwrap_or_else(|| {
                crate::generator::hex_len_of(&view.data_text)
            });
            ui.tooltip_text(format!(
                "载荷 {wide} 字节（这个槽的宽度由调度表规定）：不足补 0，多出来的不收"
            ));
        }
        if ui.is_item_active() {
            app.fr_data_edit = Some((i, buf.clone()));
        }
        if ui.is_item_deactivated_after_edit() {
            app.fr_data_edit = None;
            app.send(crate::bus::BusCommand::SetFrEntryHex {
                bus,
                slot,
                text: buf,
            });
        } else if editing && !ui.is_item_active() {
            app.fr_data_edit = None;
        }
    } else {
        ui.text_disabled(&view.sent_text);
    }
    if view.undescribed {
        ui.text_disabled("该路没有集群描述、或描述未调度这个槽：载荷按原样发出，无人解码它。");
    }
    signal_rows(
        app,
        ui,
        GenRow::Fr(i),
        "f",
        kinds,
        RowSignals {
            sigs: &sigs,
            data: &view.sent_data,
            srcs: &view.srcs,
        },
    );
    ui.unindent();
    remove
}

/// One row's header.
///
/// The `###` suffix carries the row's identity, and it has to be `###` rather
/// than `##`: imgui resets the id hash at `###` and hashes only what follows
/// (`ImHashStr`, imgui.cpp:1916), while still displaying only the text *before*
/// it. A `##` suffix is appended to the hash instead, so the badge below would
/// rename the item -- and a renamed tree node is a brand new one, which opens
/// closed. That is how adding or removing a stimulus used to collapse the row
/// out from under whoever was editing it.
///
/// The identity is the `(bus, message)` pair, not the loop index: `add_tx`
/// allows one entry per pair, and rows are removed from the middle, so an index
/// would silently move every later row's open state onto its neighbour.
fn row_header(ch: u8, bus: &str, name: &str, id: u32, driven: usize) -> String {
    let badge = if driven == 0 {
        String::new()
    } else {
        format!("  {driven} driven")
    };
    format!("{bus}  {name}  ({id:X}){badge}###tx{ch}_{id:X}")
}

/// Send period of one entry, drafted rather than written in place. The row held
/// an inline number box before, and those apply every keystroke, so dialing in
/// 100 put the message on the wire at 1 ms and then 10 ms on the way there.
/// Nothing here touches the schedule until Apply.
///
/// The two buses are asked in **different units, because they keep time
/// differently**: CAN counts milliseconds (it has no other clock -- any node may
/// request a frame whenever the bus is free), while a FlexRay static slot
/// transmits in the cycles the cluster schedule assigns it. Asking that slot for
/// a millisecond figure invites the operator to fight a schedule that is not
/// theirs to change, so its dialog counts cycles and shows the time each choice
/// means. An entry whose 路 has no description has no cycle grid to count in, and
/// falls back to milliseconds rather than inventing one.
fn cycle_modal(app: &mut App, ui: &Ui) {
    const ID: &str = "Send cycle##cycmodal";
    let Some(target) = app.tx_cycle_edit else {
        return;
    };
    let Some(edit) = cycle_edit(app, target) else {
        app.tx_cycle_edit = None;
        return;
    };
    let title = edit.title;
    let (current, declared, declared_by, cycle_us) =
        (edit.current_us, edit.declared_us, edit.declared_by, edit.cycle_us);
    // The draft box's meaning, and the period Apply writes.
    let draft = match cycle_us {
        Some(ct) => crate::generator::fr_cycle_draft(&app.tx_cycle_buf, ct).ok().map(|(_, us)| us),
        None => cycle_from_ms_text(&app.tx_cycle_buf),
    };
    let note = match cycle_us {
        Some(ct) => match crate::generator::fr_cycle_draft(&app.tx_cycle_buf, ct) {
            Ok((n, us)) => (format!("每 {n} 个通信周期 = {} ms", us / 1000), false),
            Err(why) => (why.to_string(), true),
        },
        None => match draft {
            Some(0) => (
                "event-triggered: never sent on a timer".to_string(),
                false,
            ),
            Some(us) => (
                format!(
                    "every {} ms  ({:.2} frames/s)",
                    us / 1000,
                    1_000_000.0 / us as f64
                ),
                false,
            ),
            None => (
                format!("whole milliseconds, 1 to {TX_CYCLE_MAX_MS} -- or 0 for event"),
                false,
            ),
        },
    };
    // What the schedule declares, in the unit this dialog edits.
    let declared_line = declared
        .filter(|d| *d != current)
        .map(|d| match cycle_us {
            Some(ct) => match crate::generator::cycles_of(d, ct) {
                Some(n) => (format!("{declared_by} 声明的周期：每 {n} 周期（{} ms）", d / 1000), n.to_string()),
                None => (
                    format!("{declared_by} 声明的周期：{} ms", d / 1000),
                    (d / 1000).to_string(),
                ),
            },
            None => (
                format!(
                    "{declared_by} 声明的周期：{}",
                    if d == 0 {
                        "event（无周期）".to_string()
                    } else {
                        format!("{} ms", d / 1000)
                    }
                ),
                (d / 1000).to_string(),
            ),
        });

    if !popup_is_open(ui, ID) {
        ui.open_popup(ID);
    }
    let mut open = true;
    // `opened()` keeps `open` borrowed for the whole frame, so the widgets
    // inside record a dismissal here instead.
    let mut dismissed = false;
    let mut confirmed: Option<u64> = None;
    let min = ui.push_style_var(StyleVar::WindowMinSize([420.0, 0.0]));
    ui.modal_popup_with_opened(ID, &mut open, || {
        ui.text(&title);
        ui.separator();
        // Focus and select on open, so click the row, type, Enter is the whole
        // gesture; the old value is highlighted rather than left to delete.
        if ui.is_window_appearing() {
            ui.set_keyboard_focus_here();
        }
        ui.set_next_item_width(120.0);
        let entered = ui
            .input_text(
                if cycle_us.is_some() { "周期" } else { "ms" },
                &mut app.tx_cycle_buf,
            )
            .flags(InputTextFlags::CHARS_DECIMAL | InputTextFlags::AUTO_SELECT_ALL)
            .enter_returns_true(true)
            .build();
        let (text, warn) = &note;
        if *warn {
            ui.same_line();
            ui.text_colored([0.95, 0.70, 0.20, 1.0], text);
        } else {
            ui.same_line();
            ui.text_disabled(text);
        }
        if let Some((line, use_it)) = &declared_line {
            ui.text(line);
            ui.same_line();
            if ui.button("use it") {
                app.tx_cycle_buf = use_it.clone();
            }
        }
        ui.separator();
        if ui.is_key_pressed(Key::Escape) {
            dismissed = true;
        }
        let dis = ui.begin_disabled_with_cond(draft.is_none());
        if ui.button_with_size("Apply", [90.0, 0.0]) || (entered && draft.is_some()) {
            confirmed = draft;
            ui.close_current_popup();
        }
        dis.end();
        ui.same_line();
        if ui.button_with_size("Cancel", [90.0, 0.0]) {
            dismissed = true;
        }
    });
    min.pop();
    if let Some(us) = confirmed {
        match target {
            GenRow::Can(row) => {
                if let Some(tx) = app.snap.tx.get(row) {
                    let (ch, id) = (tx.channel, tx.id);
                    app.send(crate::bus::BusCommand::SetEntryCycle {
                        ch,
                        id,
                        cycle_us: us,
                    });
                }
            }
            GenRow::Fr(row) => {
                if let Some(tx) = app.snap.fr_tx.get(row) {
                    let (bus, slot) = (tx.bus, tx.slot);
                    app.send(crate::bus::BusCommand::SetFrEntryCycle {
                        bus,
                        slot,
                        cycle_us: us,
                    });
                }
            }
        }
    }
    if !open || dismissed || confirmed.is_some() {
        app.tx_cycle_edit = None;
    }
}

/// Everything the cycle dialog shows, read out of whichever list the row came
/// from: the two halves of the generator share this one dialog.
struct CycleEdit {
    title: String,
    current_us: u64,
    declared_us: Option<u64>,
    declared_by: &'static str,
    /// The cluster's cycle time: `Some` when this row is scheduled in cycles, and
    /// then the box counts them.
    cycle_us: Option<u64>,
}

fn cycle_edit(app: &App, target: GenRow) -> Option<CycleEdit> {
    match target {
        GenRow::Can(row) => {
            let tx = app.snap.tx.get(row)?;
            let (ch, id, cycle_us, name) = (tx.channel, tx.id, tx.cycle_us, tx.name.clone());
            let (declared, bus) = (app.dbc_cycle_us(ch, id), app.channel_name(ch));
            Some(CycleEdit {
                title: format!("{name}  {id:X}  on {bus}"),
                current_us: cycle_us,
                declared_us: declared,
                declared_by: "DBC",
                cycle_us: None,
            })
        }
        GenRow::Fr(row) => {
            let tx = app.snap.fr_tx.get(row)?;
            let (bus, slot, cycle_us, name) = (tx.bus, tx.slot, tx.cycle_us, tx.name.clone());
            let (declared, bus_name) = (app.fr_declared_period_us(bus, slot), app.fr_bus_name(bus));
            Some(CycleEdit {
                title: format!("{name}  slot {slot}  on {bus_name}"),
                current_us: cycle_us,
                declared_us: declared,
                declared_by: "描述",
                cycle_us: app.fr_cycle_time_us(bus),
            })
        }
    }
}

/// Shape, range and timing of one driven signal, kept out of the row so the
/// generator stays one-signal-per-line.
fn params_modal(app: &mut App, ui: &Ui, kinds: &[String]) {
    const ID: &str = "Signal Value Source##srcparams";
    let Some((row, sig)) = app.src_edit.clone() else {
        return;
    };
    let mut src = match app.src_draft.clone() {
        Some(d) if d.name == sig => d,
        _ => match app.gen_source(row, &sig) {
            Some(h) => h,
            None => {
                app.src_edit = None;
                app.src_draft = None;
                return;
            }
        },
    };
    let (title, unit) = match app.gen_title(row) {
        Some((title, _)) => (title, app.gen_signal_unit(row, &sig)),
        None => {
            app.src_edit = None;
            app.src_draft = None;
            return;
        }
    };

    if !popup_is_open(ui, ID) {
        ui.open_popup(ID);
    }
    let mut open = true;
    // `opened()` borrows `open` for the whole frame, so the buttons below note
    // their own outcome instead of writing it.
    let mut dismissed = false;
    let mut confirmed = false;
    let mut applied = false;
    let min = ui.push_style_var(StyleVar::WindowMinSize([520.0, 240.0]));
    ui.modal_popup_with_opened(ID, &mut open, || {
        applied = true;
        ui.text(format!("{title}  /  {sig} {unit}"));
        ui.separator();
        ui.set_next_item_width(240.0);
        let mut pick = KINDS.iter().position(|k| *k == src.kind).unwrap_or(0);
        // Skip the leading "Constant": the row combo picks the shape.
        let shapes = &kinds[1..];
        if ui.combo_simple_string("Shape", &mut pick, shapes) {
            src.kind = KINDS[pick];
        }
        let speed = ((src.hi - src.lo).abs() / 100.0).max(0.01);
        ui.set_next_item_width(220.0);
        let mut lo = src.lo;
        let lo_fmt = NumericFormat::new("%g").expect("static format");
        if Drag::new("lo")
            .display_format(lo_fmt)
            .speed(speed as f32)
            .build(ui, &mut lo)
        {
            src.lo = lo;
        }
        ui.same_line();
        ui.set_next_item_width(220.0);
        let mut hi = src.hi;
        let hi_fmt = NumericFormat::new("%g").expect("static format");
        if Drag::new("hi")
            .display_format(hi_fmt)
            .speed(speed as f32)
            .build(ui, &mut hi)
        {
            src.hi = hi;
        }
        if src.kind == SrcKind::Random {
            ui.set_next_item_width(340.0);
            let mut ms = src.redraw_us as f64 / 1000.0;
            let redraw_fmt = NumericFormat::new("%g").expect("static format");
            if Drag::new("redraw ms")
                .display_format(redraw_fmt)
                .speed(1.0)
                .range(0.0f64, 600_000.0)
                .build(ui, &mut ms)
            {
                src.redraw_us = (ms * 1000.0).max(0.0) as u64;
            }
            ui.set_next_item_width(340.0);
            let mut seed = src.seed as f64;
            // An integer identifier: a %g would render big seeds in exponent
            // notation, so this one gets whole numbers instead.
            let seed_fmt = NumericFormat::new("%.0f").expect("static format");
            if Drag::new("seed")
                .display_format(seed_fmt)
                .speed(1.0)
                .build(ui, &mut seed)
            {
                src.seed = seed.max(0.0) as u64;
            }
        } else {
            ui.set_next_item_width(340.0);
            let mut ms = src.period_us as f64 / 1000.0;
            let period_fmt = NumericFormat::new("%g").expect("static format");
            if Drag::new("period ms")
                .display_format(period_fmt)
                .speed(10.0)
                .range(1.0f64, 600_000.0)
                .build(ui, &mut ms)
            {
                src.period_us = (ms * 1000.0).max(1.0) as u64;
            }
            ui.set_next_item_width(340.0);
            let mut ms = src.phase_us as f64 / 1000.0;
            let phase_fmt = NumericFormat::new("%g").expect("static format");
            if Drag::new("phase ms")
                .display_format(phase_fmt)
                .speed(10.0)
                .range(0.0f64, src.period_us as f64 / 1000.0)
                .build(ui, &mut ms)
            {
                src.phase_us = (ms * 1000.0).max(0.0) as u64;
            }
            if src.kind == SrcKind::Step {
                ui.set_next_item_width(480.0);
                if ui
                    .input_text("steps", &mut app.src_seq_buf)
                    .hint("0, 30, 60, 90")
                    .build()
                {
                    let seq = parse_seq(&app.src_seq_buf);
                    // Ignore a text that parses to nothing: an empty seq already
                    // means "toggle lo/hi", so clearing the box by mistake must
                    // not silently drop a defined sequence.
                    if !seq.is_empty() {
                        src.seq = seq;
                    }
                }
            }
        }
        ui.separator();
        if ui.is_key_pressed(Key::Escape) {
            dismissed = true;
        }
        if ui.button_with_size("Apply", [90.0, 0.0]) {
            confirmed = true;
            ui.close_current_popup();
        }
        ui.same_line();
        if ui.button_with_size("Cancel", [90.0, 0.0]) {
            dismissed = true;
        }
        ui.same_line();
        ui.text_colored([0.6, 0.6, 0.65, 1.0], "Apply is what changes the waveform");
    });
    min.pop();
    if confirmed {
        app.set_gen_source(row, src);
        app.src_edit = None;
        app.src_draft = None;
    } else if !open || dismissed {
        app.src_edit = None;
        app.src_draft = None;
    } else if applied {
        app.src_draft = Some(src);
    }
}

#[cfg(test)]
mod tests {
    use super::row_header;

    /// What imgui hashes into the item id. `###` restarts the hash and folds in
    /// only the tail; without it the whole label is the id source, because `##`
    /// hides text from the display but does not drop it from the hash.
    fn identity(header: &str) -> &str {
        match header.split_once("###") {
            Some((_, after)) => after,
            None => header,
        }
    }

    /// What the row shows: rendering stops at the first `##`, of either kind.
    fn label(header: &str) -> &str {
        let cut = header.find("##").unwrap_or(header.len());
        &header[..cut]
    }

    /// The reported bug. Every change to the badge used to rename the item, and
    /// a renamed tree node opens closed, so configuring a stimulus collapsed the
    /// row the user was working in.
    #[test]
    fn a_row_keeps_its_identity_while_the_badge_moves() {
        let before = row_header(1, "CAN2", "EngineData", 0x64, 0);
        let after = row_header(1, "CAN2", "EngineData", 0x64, 2);
        assert_eq!(
            identity(&before),
            identity(&after),
            "adding a stimulus must not move the row"
        );
        assert!(!label(&before).contains("driven"));
        assert_eq!(label(&after), "CAN2  EngineData  (64)  2 driven");
    }

    /// The shape this row shipped with before the fix, rebuilt here so the
    /// assertion says why the marker has to be `###`: `##` leaves the label in
    /// the hash, so `identity` moves with the badge and the row reopens
    /// collapsed. Without this, a test comparing `identity` could pass against a
    /// helper that simply ignored the marker.
    #[test]
    fn a_double_hash_suffix_would_move_with_the_badge() {
        let fixed = row_header(1, "CAN2", "EngineData", 0x64, 2);
        let before_fix = fixed.replace("###", "##");
        assert!(
            identity(&before_fix).contains("driven"),
            "## folds the badge into the id, which is the bug"
        );
        assert_eq!(label(&before_fix), label(&fixed), "both display the same");
    }

    /// And the bus has to be in that tail, or two buses carrying the same
    /// message id would share one open/closed state.
    #[test]
    fn the_identity_starts_with_the_bus() {
        let a = row_header(0, "CAN1", "EngineData", 0x64, 0);
        let b = row_header(1, "CAN2", "EngineData", 0x64, 0);
        assert!(identity(&a).starts_with("tx0_"), "{a}");
        assert!(identity(&b).starts_with("tx1_"), "{b}");
    }

    /// Two buses can carry the same message id, and a name is editable, so
    /// neither may be the whole identity.
    #[test]
    fn rows_on_different_buses_are_different_rows() {
        assert_ne!(
            identity(&row_header(0, "CAN1", "EngineData", 0x64, 0)),
            identity(&row_header(1, "CAN1", "EngineData", 0x64, 0))
        );
        assert_ne!(
            identity(&row_header(0, "CAN1", "EngineData", 0x64, 0)),
            identity(&row_header(0, "CAN1", "EngineData", 0x65, 0))
        );
    }

    /// A row that is renamed -- a DBC reloaded under a new message name -- keeps
    /// its place in the list rather than its label.
    #[test]
    fn the_identity_ignores_the_displayed_names() {
        assert_eq!(
            identity(&row_header(1, "CAN2", "EngineData", 0x64, 1)),
            identity(&row_header(1, "Bus B", "EngineSpeed", 0x64, 1))
        );
    }
}
