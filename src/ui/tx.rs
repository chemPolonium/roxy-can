use crate::app::{App, TOOLBAR_H};
use crate::can::frame::{FrameFlags, MAX_CAN_FD_LEN};
use crate::dbc::SignalInfo;
use imgui::{Condition, Ui};

fn parse_hex_bytes(s: &str) -> Option<Vec<u8>> {
    let toks: Vec<&str> = s.split_whitespace().collect();
    if toks.is_empty() || toks.len() > MAX_CAN_FD_LEN {
        return None;
    }
    let mut out = Vec::with_capacity(toks.len());
    for t in toks {
        out.push(u8::from_str_radix(t, 16).ok()?);
    }
    Some(out)
}

fn data_text(data: &[u8], len: u8) -> String {
    data[..len.min(data.len() as u8) as usize]
        .iter()
        .map(|b| format!("{b:02X}"))
        .collect::<Vec<_>>()
        .join(" ")
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

pub fn render(app: &mut App, ui: &Ui) {
    let io = ui.io();
    let mut open = app.show_tx;
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

                    let header_open = ui.collapsing_header(
                        format!("{}  {}  ({:X})##tx{i}", app.channel_name(ch), name, id),
                        imgui::TreeNodeFlags::empty(),
                    );
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
                        app.tx_list[i].flags = if fd {
                            FrameFlags::FD
                        } else {
                            FrameFlags::NONE
                        };
                    }
                    ui.same_line();
                    ui.set_next_item_width(260.0);
                    if ui
                        .input_text(format!("##data{i}"), &mut app.tx_list[i].data_text)
                        .build()
                    {
                        if let Some(bytes) = parse_hex_bytes(&app.tx_list[i].data_text) {
                            let mut data = [0u8; MAX_CAN_FD_LEN];
                            data[..bytes.len()].copy_from_slice(&bytes);
                            app.tx_list[i].data = data;
                            app.tx_list[i].len = bytes.len() as u8;
                            if bytes.len() > 8 {
                                app.tx_list[i].flags =
                                    app.tx_list[i].flags.union(FrameFlags::FD);
                            }
                        }
                    }
                    ui.same_line();
                    if ui.small_button(format!("x##{i}")) {
                        remove_idx = Some(i);
                    }

                    if sigs.is_empty() {
                        ui.text("(no signals in DBC)");
                    }
                    let data = app.tx_list[i].data;
                    let msg_size = app
                        .channel_dbc(ch)
                        .and_then(|db| db.messages.get(&id))
                        .map(|m| m.dlc.min(MAX_CAN_FD_LEN as u64) as u8)
                        .unwrap_or(app.tx_list[i].len);
                    for s in &sigs {
                        let raw =
                            crate::decode::extract_raw(&data, s.start_bit, s.size, s.big_endian);
                        let cur =
                            crate::decode::to_physical(raw, s.size, s.signed, s.factor, s.offset);
                        let (lo, hi) = sig_range(s);
                        ui.set_next_item_width(180.0);
                        let mut v = cur as f32;
                        if imgui::Drag::new(format!("{}##sig{i}_{}", s.name, s.name))
                            .speed(((hi - lo) / 200.0).max(0.01))
                            .range(lo, hi)
                            .build(ui, &mut v)
                        {
                            let mut nd = data;
                            if app
                                .channel_dbc(ch)
                                .is_some_and(|db| db.encode_signal(id, &s.name, v as f64, &mut nd))
                            {
                                let newlen = app.tx_list[i].len.max(msg_size);
                                if newlen > 8 {
                                    app.tx_list[i].flags =
                                        app.tx_list[i].flags.union(FrameFlags::FD);
                                }
                                app.tx_list[i].len = newlen;
                                app.tx_list[i].data = nd;
                                app.tx_list[i].data_text = data_text(&nd, newlen);
                            }
                        }
                        ui.same_line();
                        ui.text(format!("{} {}", cur, s.unit));
                    }
                    ui.unindent();
                }
                if let Some(i) = remove_idx {
                    app.tx_list.remove(i);
                }
            });
    }
    app.show_tx = open;
}
