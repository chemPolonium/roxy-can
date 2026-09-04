use crate::app::{App, TOOLBAR_H, TX_CYCLE_MAX_MS, cycle_from_ms_text};
use crate::dbc::SignalInfo;
use crate::sim::{KINDS, SrcKind, ValueSrc};
use crate::ui::help::popup_is_open;
use imgui::{Condition, InputTextFlags, Key, Ui};

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

/// Usable drag range for a signal: DBC min/max when sane, otherwise the raw
/// bit-range scaled by factor/offset.
fn sig_range(s: &SignalInfo) -> (f32, f32) {
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
    let io = ui.io();
    let mut open = app.show_tx;
    let kinds = kind_labels();
    if open {
        ui.window("Interactive Generator")
            .opened(&mut open)
            .position(
                [io.display_size[0] * 0.62, TOOLBAR_H + 10.0],
                Condition::FirstUseEver,
            )
            .size([560.0, 340.0], Condition::FirstUseEver)
            .build(|| {
                let (ids, names): (Vec<(u8, u32)>, Vec<String>) = {
                    let mut ids = Vec::new();
                    let mut names = Vec::new();
                    for (ch, channel) in app.snap.channels.iter().enumerate() {
                        let Some(db) = &channel.dbc else {
                            continue;
                        };
                        for &id in &db.order {
                            if let Some(m) = db.messages.get(&id) {
                                ids.push((ch as u8, id));
                                names.push(format!(
                                    "{}  {:03X}  {}",
                                    app.channel_name(ch as u8),
                                    id,
                                    m.name
                                ));
                            }
                        }
                    }
                    (ids, names)
                };
                if ids.is_empty() {
                    ui.text("no DBC loaded");
                } else {
                    if app.tx_pick >= ids.len() {
                        app.tx_pick = 0;
                    }
                    ui.set_next_item_width(260.0);
                    ui.combo_simple_string("Message##ig", &mut app.tx_pick, &names);
                    ui.same_line();
                    if ui.button("Add") {
                        let (ch, id) = ids[app.tx_pick];
                        app.add_tx(ch, id);
                    }
                }
                ui.same_line();
                ui.text(format!(
                    "{} active",
                    app.snap.tx.iter().filter(|t| t.active).count()
                ));
                ui.separator();

                ui.set_next_item_width(200.0);
                ui.input_text("##gsearch", &mut app.gen_search)
                    .hint("search name / ID")
                    .build();
                ui.same_line();
                if ui.small_button("Clear##gsc") {
                    app.gen_search.clear();
                }

                // Per-bus bulk switches: one click enables or disables
                // every message of that bus. All groups share one row, and
                // each group after the first re-anchors to the row's frame
                // top: aligning the bus name's text baseline leaves the
                // cursor a few pixels low, and a plain `same_line` inherits
                // that low position, stacking every later group's buttons
                // visibly beneath the first group's.
                let mut row_top: Option<f32> = None;
                let mut first_bus = true;
                for ch in 0..app.snap.channel_count {
                    let ch8 = ch as u8;
                    if !app.snap.tx.iter().any(|t| t.channel == ch8) {
                        continue;
                    }
                    if !first_bus {
                        ui.same_line();
                        if let Some(top) = row_top {
                            let p = ui.cursor_pos();
                            ui.set_cursor_pos([p[0], top]);
                        }
                    } else {
                        row_top = Some(ui.cursor_pos()[1]);
                    }
                    first_bus = false;
                    if ui.small_button(format!("All On##gon{ch}")) {
                        app.set_bus_tx(ch8, true);
                    }
                    ui.same_line();
                    if ui.small_button(format!("All Off##goff{ch}")) {
                        app.set_bus_tx(ch8, false);
                    }
                    ui.same_line();
                    ui.align_text_to_frame_padding();
                    ui.text(app.channel_name(ch8));
                }

                let query = app.gen_search.trim().to_ascii_lowercase();
                // The rows read this frame's snapshot, cloned once so the
                // per-row widgets can send commands without aliasing.
                let tx = app.snap.tx.clone();
                let mut remove: Option<(u8, u32)> = None;
                for (i, view) in tx.iter().enumerate() {
                    let id = view.id;
                    let ch = view.channel;
                    let name = view.name.clone();
                    if !query.is_empty() {
                        let hay = format!("{} {} {:X}", app.channel_name(ch), name, id)
                            .to_ascii_lowercase();
                        if !hay.contains(&query) {
                            continue;
                        }
                    }
                    let sigs: Vec<SignalInfo> = app
                        .channel_dbc(ch)
                        .and_then(|db| db.messages.get(&id))
                        .map(|m| m.signals.clone())
                        .unwrap_or_default();
                    let driven = view.srcs.len();
                    // The transmit state rides the header line, so the whole
                    // list scans without expanding anything. MUTE is the
                    // replay silencing, precomputed by the bus: the checkbox
                    // keeps its state, but an id the replayed log carries
                    // must not double-send.
                    let (chip, color, hint) = if !view.active {
                        ("OFF", [0.55, 0.58, 0.65, 1.0], "未发送：On 勾选框未勾选。")
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
                        imgui::TreeNodeFlags::empty(),
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
                        app.tx_cycle_edit = Some(i);
                        app.tx_cycle_buf = (cycle / 1000).to_string();
                    }
                    ui.same_line();
                    let mut fd = view.fd;
                    if ui.checkbox(format!("FD##{i}"), &mut fd) {
                        app.send(crate::bus::BusCommand::SetEntryFd { ch, id, fd });
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
                        if ui.small_button(format!("{label}##dbc{i}")) {
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
                    if ui.small_button(format!("x##{i}")) {
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
                    let data = view.sent_data;
                    for s in &sigs {
                        let held = view
                            .srcs
                            .iter()
                            .find(|x| x.name == s.name)
                            .cloned();
                        let raw =
                            crate::decode::extract_raw(&data, s.start_bit, s.size, s.big_endian);
                        let cur =
                            crate::decode::to_physical(raw, s.size, s.signed, s.factor, s.offset);
                        let (lo, hi) = sig_range(s);
                        // A driven row shows the live value, so the handle rides
                        // the wave. The value belongs to the source, so there
                        // the handle is disabled: grabbing it used to pin the
                        // signal and silently drop the source, which read like
                        // the wave simply breaking. Un-drive through the kind
                        // combo's "Constant" instead.
                        let model_shown = match held.as_ref() {
                            Some(_) => {
                                let raw = crate::decode::extract_raw(
                                    &data,
                                    s.start_bit,
                                    s.size,
                                    s.big_endian,
                                );
                                crate::decode::to_physical(
                                    raw,
                                    s.size,
                                    s.signed,
                                    s.factor,
                                    s.offset,
                                ) as f32
                            }
                            None => cur as f32,
                        };
                        // Pinning rewrites the base payload and clears this
                        // signal's source, so doing it per keystroke would let a
                        // half-typed 100 encode as 1 and cut the wave off with
                        // it. The draft carries the preview; the model waits.
                        let key = format!("sig{i}{}", s.name);
                        let mut shown = app.num_draft.shown(&key, model_shown as f64) as f32;
                        ui.set_next_item_width(180.0);
                        let mut v = shown;
                        let _read_only = held.is_some().then(|| ui.begin_disabled(true));
                        let moved = imgui::Drag::new(format!("{}##sig{i}_{}", s.name, s.name))
                            .display_format("%g")
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
                            // Fire-and-forget from the UI: whether the
                            // database can encode the value is the bus's
                            // call, and a failed pin simply changes nothing.
                            app.send(crate::bus::BusCommand::PinEntrySignal {
                                ch,
                                id,
                                name: s.name.clone(),
                                phys: val,
                            });
                            shown = val as f32;
                        }
                        ui.same_line();
                        ui.set_next_item_width(90.0);
                        let mut pick = match held.as_ref() {
                            None => 0,
                            Some(h) => 1 + KINDS.iter().position(|k| *k == h.kind).unwrap_or(0),
                        };
                        if ui.combo_simple_string(format!("##src{i}_{}", s.name), &mut pick, &kinds)
                        {
                            if pick == 0 {
                                app.send(crate::bus::BusCommand::ClearEntrySource {
                                    ch,
                                    id,
                                    name: s.name.clone(),
                                });
                            } else {
                                let kind = KINDS[pick - 1];
                                // Enabling snapshots lo/hi from the DBC range;
                                // changing shape afterwards keeps whatever the
                                // user has since edited in the modal.
                                let src = match held.as_ref() {
                                    Some(h) => ValueSrc { kind, ..h.clone() },
                                    None => ValueSrc::new(&s.name, kind, lo as f64, hi as f64),
                                };
                                app.send(crate::bus::BusCommand::SetEntrySource { ch, id, src });
                            }
                        }
                        if let Some(h) = &held {
                            ui.same_line();
                            if ui.small_button(format!("...##pp{i}_{}", s.name)) {
                                app.src_edit = Some((i, s.name.clone()));
                                app.src_draft = Some(h.clone());
                                app.src_seq_buf = h
                                    .seq
                                    .iter()
                                    .map(|v| format!("{v}"))
                                    .collect::<Vec<_>>()
                                    .join(", ");
                            }
                            ui.same_line();
                            ui.text(format!("~{shown} {}", s.unit));
                        } else {
                            ui.same_line();
                            ui.text(format!("{} {}", cur, s.unit));
                        }
                    }
                    ui.unindent();
                }
                if let Some((ch, id)) = remove {
                    app.send(crate::bus::BusCommand::RemoveEntry { ch, id });
                }
            });
    }
    app.show_tx = open;
    if open {
        params_modal(app, ui, &kinds);
        cycle_modal(app, ui);
    } else {
        app.src_edit = None;
        app.src_draft = None;
        app.tx_cycle_edit = None;
    }
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

/// Send period of one message, drafted rather than written in place. The row
/// held an inline number box before, and those apply every keystroke, so
/// dialing in 100 put the message on the wire at 1 ms and then 10 ms on the way
/// there. Nothing here touches the schedule until Apply.
fn cycle_modal(app: &mut App, ui: &Ui) {
    const ID: &str = "Send cycle##cycmodal";
    let Some(row) = app.tx_cycle_edit else {
        return;
    };
    let Some(tx) = app.snap.tx.get(row) else {
        app.tx_cycle_edit = None;
        return;
    };
    let (ch, id, current, name) = (tx.channel, tx.id, tx.cycle_us, tx.name.clone());
    let declared = app.dbc_cycle_us(ch, id);
    if !popup_is_open(ui, ID) {
        ui.open_popup(ID);
    }
    let mut open = true;
    // `opened()` keeps `open` borrowed for the whole frame, so the widgets
    // inside record a dismissal here instead.
    let mut dismissed = false;
    let mut confirmed: Option<u64> = None;
    let min = ui.push_style_var(imgui::StyleVar::WindowMinSize([420.0, 0.0]));
    ui.modal_popup_config(ID).opened(&mut open).build(|| {
        ui.text(format!("{}  {:X}  on {}", name, id, app.channel_name(ch)));
        ui.separator();
        // Focus and select on open, so click the row, type, Enter is the whole
        // gesture; the old value is highlighted rather than left to delete.
        if ui.is_window_appearing() {
            ui.set_keyboard_focus_here();
        }
        ui.set_next_item_width(120.0);
        let entered = ui
            .input_text("ms", &mut app.tx_cycle_buf)
            .flags(InputTextFlags::CHARS_DECIMAL | InputTextFlags::AUTO_SELECT_ALL)
            .enter_returns_true(true)
            .build();
        // Still draggable for a coarse sweep, but it writes the same draft.
        ui.same_line();
        ui.set_next_item_width(220.0);
        let mut sweep = cycle_from_ms_text(&app.tx_cycle_buf).unwrap_or(0) as f32 / 1000.0;
        if imgui::Drag::new("##cycsweep")
            .display_format("%g")
            .speed(1.0)
            .range(0.0f32, TX_CYCLE_MAX_MS as f32)
            .build(ui, &mut sweep)
        {
            app.tx_cycle_buf = (sweep.round().max(0.0) as u64).to_string();
        }
        let draft = cycle_from_ms_text(&app.tx_cycle_buf);
        match draft {
            Some(0) => {
                ui.text_colored(
                    [0.95, 0.70, 0.20, 1.0],
                    "event-triggered: never sent on a timer",
                );
            }
            Some(us) => {
                ui.text(format!(
                    "every {} ms  ({:.2} frames/s)",
                    us / 1000,
                    1_000_000.0 / us as f64
                ));
            }
            None => {
                ui.text_colored(
                    [0.60, 0.60, 0.65, 1.0],
                    format!("whole milliseconds, 1 to {TX_CYCLE_MAX_MS} -- or 0 for event"),
                );
            }
        }
        if let Some(d) = declared.filter(|d| *d != current) {
            ui.text(format!(
                "DBC declares {}",
                if d == 0 {
                    "event".to_string()
                } else {
                    format!("{} ms", d / 1000)
                }
            ));
            ui.same_line();
            if ui.small_button("use it") {
                app.tx_cycle_buf = (d / 1000).to_string();
            }
        }
        ui.separator();
        if ui.is_key_pressed(Key::Escape) {
            dismissed = true;
        }
        let dis = ui.begin_disabled(draft.is_none());
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
        app.send(crate::bus::BusCommand::SetEntryCycle {
            ch,
            id,
            cycle_us: us,
        });
    }
    if !open || dismissed || confirmed.is_some() {
        app.tx_cycle_edit = None;
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
        _ => match app
            .snap
            .tx
            .get(row)
            .and_then(|t| t.srcs.iter().find(|s| s.name == sig).cloned())
        {
            Some(h) => h,
            None => {
                app.src_edit = None;
                app.src_draft = None;
                return;
            }
        },
    };
    let desc = app.snap.tx.get(row).and_then(|t| {
        app.channel_dbc(t.channel)
            .and_then(|db| db.messages.get(&t.id))
            .and_then(|m| m.signals.iter().find(|s| s.name == sig))
            .map(|s| (t.name.clone(), t.id, s.unit.clone()))
    });
    let (msg_name, msg_id, unit) = desc.unwrap_or_else(|| (String::new(), 0, String::new()));

    if !popup_is_open(ui, ID) {
        ui.open_popup(ID);
    }
    let mut open = true;
    // `opened()` borrows `open` for the whole frame, so the buttons below note
    // their own outcome instead of writing it.
    let mut dismissed = false;
    let mut confirmed = false;
    let mut applied = false;
    let min = ui.push_style_var(imgui::StyleVar::WindowMinSize([520.0, 240.0]));
    ui.modal_popup_config(ID).opened(&mut open).build(|| {
        applied = true;
        ui.text(format!("{msg_name}  {msg_id:X}  /  {sig} {unit}"));
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
        if imgui::Drag::new("lo")
            .display_format("%g")
            .speed(speed as f32)
            .build(ui, &mut lo)
        {
            src.lo = lo;
        }
        ui.same_line();
        ui.set_next_item_width(220.0);
        let mut hi = src.hi;
        if imgui::Drag::new("hi")
            .display_format("%g")
            .speed(speed as f32)
            .build(ui, &mut hi)
        {
            src.hi = hi;
        }
        if src.kind == SrcKind::Random {
            ui.set_next_item_width(340.0);
            let mut ms = src.redraw_us as f64 / 1000.0;
            if imgui::Drag::new("redraw ms")
                .display_format("%g")
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
            if imgui::Drag::new("seed")
                .display_format("%.0f")
                .speed(1.0)
                .build(ui, &mut seed)
            {
                src.seed = seed.max(0.0) as u64;
            }
        } else {
            ui.set_next_item_width(340.0);
            let mut ms = src.period_us as f64 / 1000.0;
            if imgui::Drag::new("period ms")
                .display_format("%g")
                .speed(10.0)
                .range(1.0f64, 600_000.0)
                .build(ui, &mut ms)
            {
                src.period_us = (ms * 1000.0).max(1.0) as u64;
            }
            ui.set_next_item_width(340.0);
            let mut ms = src.phase_us as f64 / 1000.0;
            if imgui::Drag::new("phase ms")
                .display_format("%g")
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
        app.set_source(row, src);
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
