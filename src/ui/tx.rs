use crate::app::{App, TOOLBAR_H};
use crate::can::frame::FrameFlags;
use crate::dbc::SignalInfo;
use crate::sim::{KINDS, SrcKind, ValueSrc, eval_phys};
use crate::ui::help::popup_is_open;
use imgui::{Condition, Ui};

/// Combo entries for a signal's source: "Off" first so picking index 0 means
/// "stop driving this signal", the rest in [`KINDS`] order.
fn kind_labels() -> Vec<String> {
    let mut v = vec!["Off".to_string()];
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
                    for (ch, channel) in app.channels.iter().enumerate() {
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
                    app.tx_list.iter().filter(|t| t.active).count()
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
                // every message of that bus.
                let mut first_bus = true;
                for ch in 0..app.channels.len() {
                    let ch8 = ch as u8;
                    if !app.tx_list.iter().any(|t| t.channel == ch8) {
                        continue;
                    }
                    if !first_bus {
                        ui.same_line();
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
                let n = app.tx_list.len();
                let mut remove_idx: Option<usize> = None;
                for i in 0..n {
                    let id = app.tx_list[i].id;
                    let ch = app.tx_list[i].channel;
                    let name = app.tx_list[i].name.clone();
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
                    let driven = app.tx_list[i].srcs.len();

                    let header = if driven == 0 {
                        format!("{}  {}  ({:X})##tx{i}", app.channel_name(ch), name, id)
                    } else {
                        format!(
                            "{}  {}  ({:X})  {driven} driven##tx{i}",
                            app.channel_name(ch),
                            name,
                            id
                        )
                    };
                    let header_open = ui.collapsing_header(header, imgui::TreeNodeFlags::empty());
                    if !header_open {
                        continue;
                    }
                    ui.indent();
                    let mut act = app.tx_list[i].active;
                    if ui.checkbox(format!("On##{i}"), &mut act) {
                        app.tx_list[i].active = act;
                        if act {
                            app.tx_list[i].next_t_us = 0;
                        }
                    }
                    ui.same_line();
                    ui.set_next_item_width(90.0);
                    let mut ms = app.tx_list[i].cycle_us as f32 / 1000.0;
                    if imgui::Drag::new(format!("ms##cyc{i}"))
                        .speed(1.0)
                        .range(1.0f32, 60_000.0)
                        .build(ui, &mut ms)
                    {
                        app.tx_list[i].cycle_us = ((ms as f64) * 1000.0) as u64;
                        app.tx_list[i].next_t_us = 0;
                    }
                    ui.same_line();
                    let mut fd = app.tx_list[i].flags.contains(FrameFlags::FD);
                    if ui.checkbox(format!("FD##{i}"), &mut fd) {
                        app.tx_list[i].flags = if fd { FrameFlags::FD } else { FrameFlags::NONE };
                    }
                    ui.same_line();
                    ui.set_next_item_width(260.0);
                    if ui
                        .input_text(format!("##data{i}"), &mut app.tx_list[i].data_text)
                        .build()
                    {
                        let text = app.tx_list[i].data_text.clone();
                        // Bad text simply stays unapplied until the box parses;
                        // active sources are never cleared by a hex edit.
                        app.set_tx_hex(i, &text);
                    }
                    if driven > 0 {
                        // The box edits the base payload only; what goes out is
                        // base plus the sources layered on top of it.
                        ui.same_line();
                        ui.text("base");
                    }
                    ui.same_line();
                    if ui.small_button(format!("x##{i}")) {
                        remove_idx = Some(i);
                    }

                    if sigs.is_empty() {
                        ui.text("(no signals in DBC)");
                    }
                    let data = app.tx_list[i].data;
                    let sim = app.sim_t_us;
                    for s in &sigs {
                        let held = app.tx_list[i]
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
                        // the wave; grabbing it pins the signal right there.
                        let shown = held
                            .as_ref()
                            .map_or(cur as f32, |h| eval_phys(h, sim) as f32);
                        ui.set_next_item_width(180.0);
                        let mut v = shown;
                        if imgui::Drag::new(format!("{}##sig{i}_{}", s.name, s.name))
                            .speed(((hi - lo) / 200.0).max(0.01))
                            .range(lo, hi)
                            .build(ui, &mut v)
                        {
                            app.pin_signal(i, &s.name, v as f64);
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
                                app.clear_source(i, &s.name);
                            } else {
                                let kind = KINDS[pick - 1];
                                // Enabling snapshots lo/hi from the DBC range;
                                // changing shape afterwards keeps whatever the
                                // user has since edited in the modal.
                                let src = match held.as_ref() {
                                    Some(h) => ValueSrc { kind, ..h.clone() },
                                    None => ValueSrc::new(&s.name, kind, lo as f64, hi as f64),
                                };
                                app.set_source(i, src);
                            }
                        }
                        if let Some(h) = &held {
                            ui.same_line();
                            if ui.small_button(format!("...##pp{i}_{}", s.name)) {
                                app.src_edit = Some((i, s.name.clone()));
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
                if let Some(i) = remove_idx {
                    app.tx_list.remove(i);
                }
            });
    }
    app.show_tx = open;
    if open {
        params_modal(app, ui, &kinds);
    } else {
        app.src_edit = None;
    }
}

/// Shape, range and timing of one driven signal, kept out of the row so the
/// generator stays one-signal-per-line.
fn params_modal(app: &mut App, ui: &Ui, kinds: &[String]) {
    const ID: &str = "Signal Value Source##srcparams";
    let Some((row, sig)) = app.src_edit.clone() else {
        return;
    };
    let held = app
        .tx_list
        .get(row)
        .and_then(|t| t.srcs.iter().find(|s| s.name == sig).cloned());
    let Some(mut src) = held else {
        app.src_edit = None;
        return;
    };
    let desc = app.tx_list.get(row).and_then(|t| {
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
    let mut applied = false;
    let min = ui.push_style_var(imgui::StyleVar::WindowMinSize([520.0, 240.0]));
    ui.modal_popup_config(ID).opened(&mut open).build(|| {
        applied = true;
        ui.text(format!("{msg_name}  {msg_id:X}  /  {sig} {unit}"));
        ui.separator();
        ui.set_next_item_width(240.0);
        let mut pick = KINDS.iter().position(|k| *k == src.kind).unwrap_or(0);
        // Skip the leading "Off": the row combo turns a source off.
        let shapes = &kinds[1..];
        if ui.combo_simple_string("Shape", &mut pick, shapes) {
            src.kind = KINDS[pick];
        }
        let speed = ((src.hi - src.lo).abs() / 100.0).max(0.01);
        ui.set_next_item_width(220.0);
        let mut lo = src.lo;
        if imgui::Drag::new("lo")
            .speed(speed as f32)
            .build(ui, &mut lo)
        {
            src.lo = lo;
        }
        ui.same_line();
        ui.set_next_item_width(220.0);
        let mut hi = src.hi;
        if imgui::Drag::new("hi")
            .speed(speed as f32)
            .build(ui, &mut hi)
        {
            src.hi = hi;
        }
        if src.kind == SrcKind::Random {
            ui.set_next_item_width(340.0);
            let mut ms = src.redraw_us as f64 / 1000.0;
            if imgui::Drag::new("redraw ms")
                .speed(1.0)
                .range(0.0f64, 600_000.0)
                .build(ui, &mut ms)
            {
                src.redraw_us = (ms * 1000.0).max(0.0) as u64;
            }
            ui.set_next_item_width(340.0);
            let mut seed = src.seed as f64;
            if imgui::Drag::new("seed").speed(1.0).build(ui, &mut seed) {
                src.seed = seed.max(0.0) as u64;
            }
        } else {
            ui.set_next_item_width(340.0);
            let mut ms = src.period_us as f64 / 1000.0;
            if imgui::Drag::new("period ms")
                .speed(10.0)
                .range(1.0f64, 600_000.0)
                .build(ui, &mut ms)
            {
                src.period_us = (ms * 1000.0).max(1.0) as u64;
            }
            ui.set_next_item_width(340.0);
            let mut ms = src.phase_us as f64 / 1000.0;
            if imgui::Drag::new("phase ms")
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
    });
    min.pop();
    if applied {
        app.set_source(row, src);
    }
    if !open {
        app.src_edit = None;
    }
}
