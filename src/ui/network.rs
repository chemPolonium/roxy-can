use crate::app::App;
use imgui::{Condition, Ui};

const BOX_W: f32 = 118.0;
const BOX_H: f32 = 36.0;
const SECTION_H: f32 = 150.0;

struct NodeInfo {
    name: String,
    tx: Vec<(u32, String)>,
    rx: Vec<(u32, String, String)>,
}

/// Per channel: the DBC node infos of that bus.
fn collect(app: &App) -> Vec<Vec<NodeInfo>> {
    let mut dbc_nodes = Vec::new();
    for channel in app.snap.channels.iter() {
        let infos: Vec<NodeInfo> = channel
            .dbc
            .as_ref()
            .map(|db| {
                db.nodes
                    .iter()
                    .map(|n| {
                        let tx = db
                            .node_tx_ids(n)
                            .into_iter()
                            .map(|id| {
                                let name = db.message_name(id).unwrap_or("-").to_string();
                                (id, name)
                            })
                            .collect();
                        let rx = db.node_rx_signals(n);
                        NodeInfo {
                            name: n.clone(),
                            tx,
                            rx,
                        }
                    })
                    .collect()
            })
            .unwrap_or_default();
        dbc_nodes.push(infos);
    }
    dbc_nodes
}

fn draw_section(app: &mut App, ui: &Ui, ch: usize, infos: &[NodeInfo], flat_base: usize) {
    let p0 = ui.cursor_screen_pos();
    let avail = ui.content_region_avail();
    let w = avail[0].max(120.0);
    let dl = ui.get_window_draw_list();

    dl.add_rect(
        [p0[0], p0[1]],
        [p0[0] + w, p0[1] + SECTION_H],
        [0.08, 0.08, 0.10, 1.0],
    )
    .filled(true)
    .build();
    dl.add_rect(
        [p0[0], p0[1]],
        [p0[0] + w, p0[1] + SECTION_H],
        [0.20, 0.20, 0.25, 1.0],
    )
    .build();

    let bus_y = p0[1] + SECTION_H - 30.0;
    dl.add_line(
        [p0[0] + 12.0, bus_y],
        [p0[0] + w - 12.0, bus_y],
        [0.30, 0.80, 1.00, 1.0],
    )
    .thickness(2.0)
    .build();
    dl.add_line(
        [p0[0] + 12.0, bus_y + 4.0],
        [p0[0] + w - 12.0, bus_y + 4.0],
        [0.30, 0.80, 1.00, 1.0],
    )
    .thickness(2.0)
    .build();
    dl.add_text(
        [p0[0] + 16.0, bus_y - 18.0],
        [0.30, 0.80, 1.00, 1.0],
        app.channel_name(ch as u8),
    );

    if infos.is_empty() {
        dl.add_text(
            [p0[0] + 16.0, p0[1] + 30.0],
            [0.5, 0.5, 0.6, 1.0],
            "no DBC loaded on this bus",
        );
        ui.set_cursor_screen_pos([p0[0], p0[1] + SECTION_H + 4.0]);
        return;
    }

    let total = infos.len();
    let box_y = p0[1] + 18.0;
    let x_of = |i: usize| p0[0] + w * (i + 1) as f32 / (total + 1) as f32;

    for (i, ni) in infos.iter().enumerate() {
        let cx = x_of(i);
        let bx = cx - BOX_W / 2.0;
        dl.add_line([cx, box_y + BOX_H], [cx, bus_y], [0.45, 0.45, 0.55, 1.0])
            .build();

        let sel = app.net_selected == flat_base + i;
        let active = ni.tx.iter().any(|(id, _)| {
            app.snap
                .aggs
                .iter()
                .any(|a| a.channel == ch as u8 && a.id == *id && a.count > 0)
        });
        let bg = if sel {
            [0.16, 0.28, 0.42, 1.0]
        } else {
            [0.12, 0.12, 0.16, 1.0]
        };
        let border = if sel {
            [0.30, 0.80, 1.00, 1.0]
        } else {
            [0.35, 0.35, 0.45, 1.0]
        };
        dl.add_rect([bx, box_y], [bx + BOX_W, box_y + BOX_H], bg)
            .rounding(6.0)
            .filled(true)
            .build();
        dl.add_rect([bx, box_y], [bx + BOX_W, box_y + BOX_H], border)
            .rounding(6.0)
            .build();
        // Amber bar on the left edge: "I transmit as this ECU". Deliberately a
        // different channel from the green dot, which means "I have seen this
        // ECU send" -- a simulated node that is also real shows both.
        if app.is_node_simulated(ch as u8, &ni.name) {
            dl.add_rect(
                [bx + 3.0, box_y + 7.0],
                [bx + 6.0, box_y + BOX_H - 7.0],
                [0.95, 0.70, 0.20, 1.0],
            )
            .filled(true)
            .build();
        }
        let size = ui.calc_text_size(ni.name.clone());
        dl.add_text(
            [cx - size[0] / 2.0, box_y + (BOX_H - size[1]) / 2.0],
            [0.9, 0.9, 0.95, 1.0],
            ni.name.clone(),
        );
        if active {
            dl.add_circle(
                [bx + BOX_W - 8.0, box_y + 8.0],
                4.0,
                [0.45, 0.95, 0.45, 1.0],
            )
            .filled(true)
            .build();
        }
    }

    for i in 0..infos.len() {
        let cx = x_of(i);
        ui.set_cursor_screen_pos([cx - BOX_W / 2.0, box_y]);
        if ui.invisible_button(format!("node{ch}_{i}##net"), [BOX_W, BOX_H]) {
            app.net_selected = flat_base + i;
        }
    }
    ui.set_cursor_screen_pos([p0[0], p0[1] + SECTION_H + 4.0]);
}

pub fn render(app: &mut App, ui: &Ui) {
    let io = ui.io();
    let mut open = app.show_network;
    if open {
        ui.window("Network")
            .opened(&mut open)
            .position(
                [io.display_size[0] * 0.3, io.display_size[1] * 0.55],
                Condition::FirstUseEver,
            )
            .size([720.0, 520.0], Condition::FirstUseEver)
            .build(|| {
                let dbc_nodes = collect(app);
                let total_dbc: usize = dbc_nodes.iter().map(|v| v.len()).sum();
                if total_dbc == 0 {
                    ui.text("no DBC nodes to display");
                    return;
                }
                if app.net_selected >= total_dbc {
                    app.net_selected = 0;
                }

                let mut flat_base = 0usize;
                for (ch, infos) in dbc_nodes.iter().enumerate() {
                    draw_section(app, ui, ch, infos, flat_base);
                    flat_base += infos.len();
                }
                ui.separator();

                // Details scroll inside their own panel (which always fills the
                // remaining space), so long content never adds a scrollbar to the
                // outer window and shifts the topology sections.
                ui.child_window("node_details").size([0.0, 0.0]).build(|| {
                    // Locate the selected DBC node (channel, index).
                    let mut remaining = app.net_selected;
                    let mut selected: Option<(usize, usize)> = None;
                    for (ch, infos) in dbc_nodes.iter().enumerate() {
                        if remaining < infos.len() {
                            selected = Some((ch, remaining));
                            break;
                        }
                        remaining -= infos.len();
                    }
                    let Some((ch, idx)) = selected else {
                        ui.text("select a node to see its messages and signals");
                        return;
                    };
                    let ni = &dbc_nodes[ch][idx];
                    ui.text_colored(
                        [0.30, 0.80, 1.00, 1.0],
                        format!(
                            "{} / {}  —  sends {} message(s), receives {} signal(s)",
                            app.channel_name(ch as u8),
                            ni.name,
                            ni.tx.len(),
                            ni.rx.len()
                        ),
                    );
                    let mut sim = app.is_node_simulated(ch as u8, &ni.name);
                    if ui.checkbox("Simulate this node", &mut sim) {
                        app.set_node_sim(ch as u8, &ni.name, sim);
                    }
                    if ni.tx.is_empty() {
                        ui.text_colored(
                            [0.5, 0.5, 0.6, 1.0],
                            "  (sends nothing -- ticking only records the intent)",
                        );
                    }
                    ui.text("Sent messages");
                    for (id, name) in &ni.tx {
                        let (count, cycle) = app
                            .snap
                            .aggs
                            .iter()
                            .find(|a| a.channel == ch as u8 && a.id == *id)
                            .map(|a| (a.count, a.cycle_us / 1000.0))
                            .unwrap_or((0, 0.0));
                        let cycle_s = if count >= 2 {
                            format!("  ~{cycle:.1} ms")
                        } else {
                            String::new()
                        };
                        ui.text(format!("  {id:03X}  {name}  count {count}{cycle_s}"));
                    }
                    ui.text("Received signals");
                    for (id, sig, sender) in &ni.rx {
                        let msg = app
                            .channel_dbc(ch as u8)
                            .and_then(|db| db.message_name(*id))
                            .unwrap_or("-");
                        ui.text(format!("  {sig}  <-  {sender}  ({msg} {id:03X})"));
                    }
                });
            });
    }
    app.show_network = open;
}
