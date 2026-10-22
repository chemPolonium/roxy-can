use crate::app::{App, NodeRole};
use crate::ui::tx::{GeneratorCmd, GeneratorView};
use dear_imgui_rs::{Condition, StyleColor, Ui};

/// Which Ethernet subject the Network detail pane shows. Frames come from the
/// ARXML matrix, flows from captured traffic; both are read-only because this
/// build receives Ethernet and does not transmit it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum EthNetSel {
    Frame { bus: u8, index: usize },
    Flow(crate::aggregate::EthFlowKey),
}

/// The Network window's own state: selection, the Profile row, and replay
/// block drafts. Topology and node facts stay on the bus.
pub struct NetworkView {
    pub open: bool,
    pub net_selected: usize,
    pub net_fr_sel: Option<(u8, String)>,
    pub net_eth_sel: Option<EthNetSel>,
    pub(crate) profile_pick: usize,
    pub(crate) profile_draft_name: String,
    pub(crate) profile_delete_arm: bool,
    pub(crate) profile_names: Option<Result<Vec<String>, String>>,
    pub(crate) block_drafts: std::collections::HashMap<u64, crate::ui::BlockDraft>,
}

impl Default for NetworkView {
    fn default() -> Self {
        Self {
            open: true,
            net_selected: 0,
            net_fr_sel: None,
            net_eth_sel: None,
            profile_pick: 0,
            profile_draft_name: String::new(),
            profile_delete_arm: false,
            profile_names: None,
            block_drafts: std::collections::HashMap::new(),
        }
    }
}

/// What the Network detail asks the bus or a project file to do. Selection
/// and drafts are view state; roles, nodes, blocks and profiles cross here.
pub enum NetworkCmd {
    OpenScriptEditor(u64),
    SetNodeRole {
        bus: u8,
        node: String,
        role: NodeRole,
    },
    AddNode {
        name: String,
        channel: u8,
        attached: Option<(u8, String)>,
    },
    AddReplayBlock {
        channel: u8,
        name: String,
        node: String,
    },
    SetReplayBlockEnabled {
        id: u64,
        on: bool,
    },
    RemoveReplayBlock(u64),
    SetReplayBlock {
        id: u64,
        name: String,
        channel: u8,
        path: String,
        node_filter: Option<String>,
        attached: Option<(u8, String)>,
        ids: Vec<(u32, bool)>,
    },
    ApplyProfile(String),
    OpenProfileFile {
        name: String,
        path: std::path::PathBuf,
    },
    DeleteProfile(String),
    SaveProfile(String),
}

struct NodeInfo {
    name: String,
    tx: Vec<(u32, String)>,
    rx: Vec<((u32, bool), String, String)>,
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

/// One bus's tree section: the bus as the root, its DBC nodes as leaves.
/// The role tag colours the leaf ([S] simulated / [M] monitor / [-]
/// absent -- plain ASCII, the merged font has no geometric glyphs), a
/// green `*` tail marks "seen transmitting this run", and clicking a
/// leaf opens the node's detail below.
impl NetworkView {
    fn draw_tree_section(
        &mut self,
        ui: &Ui,
        app: &App,
        ch: usize,
        infos: &[NodeInfo],
        flat_base: usize,
        cmds: &mut Vec<NetworkCmd>,
    ) {
        // The root's ID is the channel index, not its name: two buses may be
        // renamed alike, and two visible tree nodes with one label would share one
        // ID.
        let _id = ui.push_id(ch as i32);
        let token = ui
            .tree_node_config(app.channel_name(ch as u8))
            .default_open(true)
            .push();
        if let Some(_t) = token {
            if infos.is_empty() {
                ui.text_disabled("no DBC loaded on this bus");
                return;
            }
            for (i, ni) in infos.iter().enumerate() {
                let role = app.node_role(ch as u8, &ni.name);
                let (tag, tag_color) = match role {
                    crate::app::NodeRole::Simulated => ("[S]", [0.95, 0.70, 0.20, 1.0]),
                    crate::app::NodeRole::Absent => ("[-]", [0.45, 0.45, 0.55, 1.0]),
                };
                ui.text_colored(tag_color, tag);
                if ui.is_item_hovered() {
                    ui.tooltip_text(role.hint());
                }
                ui.same_line();
                if ui
                    .selectable_config(format!("{}##net{ch}_{i}", ni.name))
                    .selected(
                        self.net_fr_sel.is_none()
                            && self.net_eth_sel.is_none()
                            && self.net_selected == flat_base + i,
                    )
                    .build()
                {
                    self.net_selected = flat_base + i;
                    // One selection at a time across the three trees: a CAN node,
                    // a FlexRay ECU and an Ethernet frame/flow cannot all own the
                    // detail pane.
                    self.net_fr_sel = None;
                    self.net_eth_sel = None;
                }
                // 绑定到该节点的脚本作为下一层树叶列在节点下：点击打开
                // 该脚本的编辑器。
                let bound: Vec<(u64, String, bool, bool, bool)> = app
                    .snap
                    .nodes
                    .iter()
                    .filter(|n| {
                        n.channel as usize == ch
                            && n.attached.as_ref().is_some_and(|a| a.1 == ni.name)
                    })
                    .map(|n| {
                        (
                            n.id,
                            n.name.clone(),
                            n.running && !n.errored,
                            n.enabled,
                            n.errored,
                        )
                    })
                    .collect();
                if !bound.is_empty() {
                    ui.indent();
                    for (nid, name, running, enabled, errored) in bound {
                        self.draw_script_leaf(ui, nid, &name, running, enabled, errored, cmds);
                    }
                    ui.unindent();
                }
                // 绑定到该节点的回放块同样列在节点下：点击打开 Replay
                // Blocks 窗口编辑。
                let blocks: Vec<(u64, String, bool)> = app
                    .snap
                    .blocks
                    .iter()
                    .filter(|b| {
                        b.attached
                            .as_ref()
                            .is_some_and(|a| a.0 == ch as u8 && a.1 == ni.name)
                    })
                    .map(|b| (b.id, b.name.clone(), b.enabled))
                    .collect();
                if !blocks.is_empty() {
                    ui.indent();
                    for (bid, name, enabled) in blocks {
                        let (marker, color) = if enabled {
                            ("[o]", [0.45, 0.62, 0.80, 1.0])
                        } else {
                            ("[-]", [0.5, 0.5, 0.55, 1.0])
                        };
                        ui.text_colored(color, marker);
                        if ui.is_item_hovered() {
                            ui.tooltip_text("Replay block / [o] enabled / [-] disabled");
                        }
                        ui.same_line();
                        if ui
                            .selectable_config(format!("{name}##netblock{bid}"))
                            .build()
                        {
                            // 块的编辑就在所属节点的详情里：点击树叶选中节点。
                            self.net_selected = flat_base + i;
                            self.net_eth_sel = None;
                        }
                    }
                    ui.unindent();
                }
            }
            // 自由脚本（未绑定 DBC 节点）列在总线根下，旧工程仍可见。
            let free: Vec<(u64, String, bool, bool, bool)> = app
                .snap
                .nodes
                .iter()
                .filter(|n| n.channel as usize == ch && n.attached.is_none())
                .map(|n| {
                    (
                        n.id,
                        n.name.clone(),
                        n.running && !n.errored,
                        n.enabled,
                        n.errored,
                    )
                })
                .collect();
            if !free.is_empty() {
                ui.indent();
                for (nid, name, running, enabled, errored) in free {
                    self.draw_script_leaf(ui, nid, &name, running, enabled, errored, cmds);
                }
                ui.unindent();
            }
        }
    }

    /// One Ethernet bus as a tree section: the ARXML frames it declares and
    /// the L2 flows actually captured on it. There is no role switch or
    /// generator here -- this build receives Ethernet -- so the leaves open a
    /// read-only detail pane instead of a sender editor.
    fn draw_ethernet_section(&mut self, ui: &Ui, app: &App, bus: u8) {
        let db = app.eth_db(bus);
        let label = match db {
            Some(db) => {
                let cluster = match db.clusters.as_slice() {
                    [] => "-".to_string(),
                    [one] => one.name.clone(),
                    many => format!("{} clusters", many.len()),
                };
                format!("{} · {cluster}", app.eth_bus_label(bus))
            }
            None => format!("{} (no matrix loaded)", app.eth_bus_label(bus)),
        };
        let _id = ui.push_id(format!("eth{bus}").as_str());
        let Some(_t) = ui.tree_node_config(label).default_open(true).push() else {
            return;
        };
        if let Some(db) = db {
            if db.frames.is_empty() {
                crate::ui::text_wrapped_disabled(ui, "The matrix declares no frames");
            } else if let Some(_g) = ui
                .tree_node_config(format!("Frames ({})##ethframes{bus}", db.frames.len()))
                .default_open(true)
                .push()
            {
                for (i, f) in db.frames.iter().enumerate() {
                    let mut text = f.name.clone();
                    if let Some(vlan) = f.vlan {
                        text.push_str(&format!(" · VLAN {vlan}"));
                    }
                    if let Some(et) = f.ethertype_hint {
                        text.push_str(&format!(" · 0x{et:04X}"));
                    }
                    text.push_str(&format!(" · {} B", f.length));
                    let signals: usize = f.pdus.iter().map(|p| p.signals.len()).sum();
                    if signals > 0 {
                        text.push_str(&format!(" · {signals} sig"));
                    }
                    let selected = self.net_eth_sel == Some(EthNetSel::Frame { bus, index: i });
                    if ui
                        .selectable_config(format!("{text}##ethframe{bus}_{i}"))
                        .selected(selected)
                        .build()
                    {
                        self.net_eth_sel = Some(EthNetSel::Frame { bus, index: i });
                        self.net_fr_sel = None;
                    }
                    if ui.is_item_hovered() {
                        ui.tooltip_text(&text);
                    }
                }
            }
        } else {
            ui.text_disabled("No ARXML matrix is loaded; captured flows are listed below");
        }

        let flows: Vec<&crate::aggregate::EthAgg> = app
            .snap
            .eth_aggs
            .iter()
            .filter(|a| a.key.bus == bus)
            .collect();
        if flows.is_empty() {
            crate::ui::text_wrapped_disabled(ui, "No captured flows on this bus");
            return;
        }
        if let Some(_g) = ui
            .tree_node_config(format!("Flows seen ({})##ethflows{bus}", flows.len()))
            .default_open(true)
            .push()
        {
            for (i, agg) in flows.iter().enumerate() {
                let text = format!(
                    "{} > {} · VLAN {} · 0x{:04X} · {} frames",
                    crate::eth::db::mac_text(&agg.key.src),
                    crate::eth::db::mac_text(&agg.key.dst),
                    vlan_text(agg.key.vlan),
                    agg.key.ethertype,
                    agg.count
                );
                let selected = self.net_eth_sel == Some(EthNetSel::Flow(agg.key));
                if ui
                    .selectable_config(format!("{text}##ethflow{bus}_{i}"))
                    .selected(selected)
                    .build()
                {
                    self.net_eth_sel = Some(EthNetSel::Flow(agg.key));
                    self.net_fr_sel = None;
                }
                if ui.is_item_hovered() {
                    ui.tooltip_text(&text);
                }
            }
        }
    }

    /// The detail pane for either kind of Ethernet leaf. Both are read-only:
    /// the matrix says what a frame is, the capture says what arrived; there is
    /// no Ethernet transmit path to edit here.
    fn draw_ethernet_detail(&self, ui: &Ui, app: &App, sel: &EthNetSel) {
        match sel {
            EthNetSel::Frame { bus, index } => self.draw_eth_frame_detail(ui, app, *bus, *index),
            EthNetSel::Flow(key) => self.draw_eth_flow_detail(ui, app, key),
        }
    }

    fn draw_eth_frame_detail(&self, ui: &Ui, app: &App, bus: u8, index: usize) {
        let Some(db) = app.eth_db(bus) else {
            ui.text("The ARXML matrix for this Ethernet bus is no longer loaded");
            return;
        };
        let Some(frame) = db.frames.get(index) else {
            ui.text("This frame is no longer in the loaded matrix");
            return;
        };
        {
            let _c = ui.push_style_color(StyleColor::Text, [0.95, 0.75, 0.35, 1.0]);
            ui.text_wrapped(format!(
                "{} / {}  —  ARXML frame",
                app.eth_bus_label(bus),
                frame.name
            ));
        }
        crate::ui::text_wrapped_disabled(
            ui,
            "Ethernet is receive-only in this build: the matrix names and decodes captured frames.",
        );
        ui.separator();
        ui.text(format!("Cluster: {}", frame.cluster));
        ui.text(format!("VLAN: {}", vlan_text(frame.vlan)));
        ui.text(format!("Declared length: {} bytes", frame.length));
        if let Some(et) = frame.ethertype_hint {
            ui.text(format!("EtherType: 0x{et:04X}"));
        }
        let signals: usize = frame.pdus.iter().map(|p| p.signals.len()).sum();
        ui.text(format!("PDUs: {} · signals: {}", frame.pdus.len(), signals));
        ui.separator();
        for pdu in &frame.pdus {
            ui.text_colored(
                [0.55, 0.80, 1.0, 1.0],
                format!("{}  ({} bytes)", pdu.name, pdu.length),
            );
            ui.indent();
            for sig in &pdu.signals {
                let mut line = format!(
                    "{}  bit {} · {} bit · {}",
                    sig.name,
                    sig.start_bit,
                    sig.size,
                    if sig.signed { "signed" } else { "unsigned" }
                );
                if !sig.unit.is_empty() {
                    line.push_str(&format!(" · {}", sig.unit));
                }
                ui.text(line);
            }
            ui.unindent();
        }

        ui.separator();
        ui.text("Captured flows matching this frame");
        let matching: Vec<&crate::aggregate::EthAgg> = app
            .snap
            .eth_aggs
            .iter()
            .filter(|a| {
                a.key.bus == bus
                    && db.match_flow(&a.key.flow(), app.eth_length_includes_header)
                        == crate::eth::db::EthMatch::Unique(index)
            })
            .collect();
        if matching.is_empty() {
            ui.text_disabled("No captured flow matches this frame yet.");
            return;
        }
        for agg in matching {
            let mut line = format!(
                "{} > {} · VLAN {} · {} frames",
                crate::eth::db::mac_text(&agg.key.src),
                crate::eth::db::mac_text(&agg.key.dst),
                vlan_text(agg.key.vlan),
                agg.count
            );
            if agg.count > 1 {
                line.push_str(&format!(
                    " · {:.1} ms ±{:.1}",
                    agg.cycle_us / 1000.0,
                    agg.jitter_us / 1000.0
                ));
            }
            ui.text(line);
            if let Some(decoded) = db.decode(
                &agg.key.flow(),
                &agg.payload,
                app.eth_length_includes_header,
            ) {
                ui.indent();
                for d in decoded.signals {
                    ui.text(format!(
                        "{} = {}",
                        d.name,
                        crate::dbc::fmt_signal_value(
                            d.physical,
                            &d.unit,
                            &d.type_tag,
                            d.label.as_deref()
                        )
                    ));
                }
                ui.unindent();
            }
        }
    }

    fn draw_eth_flow_detail(&self, ui: &Ui, app: &App, key: &crate::aggregate::EthFlowKey) {
        let Some(agg) = app.snap.eth_aggs.iter().find(|a| a.key == *key) else {
            ui.text("This flow is no longer in the snapshot");
            return;
        };
        ui.text_colored(
            [0.95, 0.75, 0.35, 1.0],
            format!(
                "{} / {} > {}  —  Ethernet flow",
                app.eth_bus_label(key.bus),
                crate::eth::db::mac_text(&key.src),
                crate::eth::db::mac_text(&key.dst)
            ),
        );
        ui.text_disabled(
            "Read-only: Ethernet frames arrive from the log; this build does not transmit them.",
        );
        ui.separator();
        ui.text(format!("VLAN: {}", vlan_text(key.vlan)));
        ui.text(format!("EtherType: 0x{:04X}", key.ethertype));
        ui.text(format!("Payload: {} bytes · {} frames", key.len, agg.count));
        if agg.count > 1 {
            ui.text(format!(
                "Cycle: {:.1} ms ±{:.1} ms",
                agg.cycle_us / 1000.0,
                agg.jitter_us / 1000.0
            ));
        }
        let preview: String = agg
            .payload
            .iter()
            .take(64)
            .map(|b| format!("{b:02X} "))
            .collect();
        ui.text(format!(
            "Last payload: {}{}",
            preview.trim_end(),
            if agg.payload.len() > 64 { " …" } else { "" }
        ));
        ui.separator();

        let Some(db) = app.eth_db(key.bus) else {
            ui.text_disabled(
                "No ARXML matrix is loaded for this bus; the flow is shown as raw L2 only.",
            );
            return;
        };
        match db.match_flow(&key.flow(), app.eth_length_includes_header) {
            crate::eth::db::EthMatch::Unique(i) => {
                let frame = &db.frames[i];
                ui.text(format!("ARXML frame: {}", frame.name));
                if let Some(decoded) =
                    db.decode(&key.flow(), &agg.payload, app.eth_length_includes_header)
                {
                    ui.indent();
                    for d in decoded.signals {
                        ui.text(format!(
                            "{} = {}",
                            d.name,
                            crate::dbc::fmt_signal_value(
                                d.physical,
                                &d.unit,
                                &d.type_tag,
                                d.label.as_deref()
                            )
                        ));
                    }
                    ui.unindent();
                } else {
                    ui.text_disabled("The frame declares no signals this payload can decode.");
                }
            }
            crate::eth::db::EthMatch::Ambiguous(n) => {
                ui.text_colored(
                    [1.0, 0.8, 0.4, 1.0],
                    format!(
                        "{n} ARXML frames match this flow; it stays raw until the match is unique."
                    ),
                );
                ui.text_disabled(
                    "Check VLAN, payload length and the FRAME-LENGTH convention on the Buses window.",
                );
            }
            crate::eth::db::EthMatch::None => {
                ui.text_disabled("No ARXML frame matches this flow; it is shown as raw L2 only.");
            }
        }
        ui.separator();
        ui.text_disabled(
            "Signals can be plotted from Data, Graphics or State Tracker with the selection popup.",
        );
    }
}

/// VLAN as the tables print it: `-` when the frame was untagged.
fn vlan_text(vlan: Option<u16>) -> String {
    vlan.map(|v| v.to_string())
        .unwrap_or_else(|| "-".to_string())
}

/// The FlexRay clusters the topology lists: every bus with a description
/// loaded, plus any that carried traffic without one. A watched bus with no
/// description has no schedule to show, but leaving it out entirely would make
/// a live cable invisible in the one view that answers "what is on this bus".
fn fr_buses(app: &App) -> Vec<u8> {
    let mut buses: Vec<u8> = app.fr_buses.keys().copied().collect();
    for bus in app.snap.fr_loads.keys().copied() {
        if !buses.contains(&bus) {
            buses.push(bus);
        }
    }
    buses.sort_unstable();
    buses
}

/// The Ethernet buses the topology lists: every bus with a matrix loaded, plus
/// any bus that carried captured flows without one. As on the FlexRay side,
/// traffic alone is enough to make a cable visible.
fn eth_buses(app: &App) -> Vec<u8> {
    let mut buses: Vec<u8> = app.eth_buses.keys().copied().collect();
    for agg in &app.snap.eth_aggs {
        if !buses.contains(&agg.key.bus) {
            buses.push(agg.key.bus);
        }
    }
    buses.sort_unstable();
    buses
}

/// One FlexRay cluster as a tree section: which cluster this is, and the ECUs
/// its description declares. Nothing else -- the schedule (which frame holds
/// which slot in which cycle phase, with what repetition) is a different tool's
/// job, the same split as CANoe against a FIBEX/ARXML editor: this view answers
/// "who is on this bus", and a copy of the schedule here would be a worse,
/// staler duplicate of the document. What has actually arrived is in
/// Trace/Messages, the declared timing behind the occupancy figure in Bus
/// Statistics.
///
/// An ECU leaf is selectable, and its detail is the generator panel for the
/// slots this ECU is the declared sender of -- the same node-centred editing a
/// CAN node gets. There is no role switch: a role is the DBC node's gate, and
/// the description says who sends, not who is simulated.
impl NetworkView {
    fn draw_flexray_section(&mut self, ui: &Ui, app: &App, bus: u8) {
        let label = match app.fr_db(bus) {
            Some(db) => format!("{} · {}", app.fr_bus_name(bus), db.params.name),
            None => format!("{} (no description loaded)", app.fr_bus_name(bus)),
        };
        // The root's ID is the cluster index: two 路 can be renamed alike, and two
        // visible tree nodes with one label would share one ID.
        let _id = ui.push_id(format!("fr{bus}").as_str());
        let Some(_t) = ui.tree_node_config(label).default_open(true).push() else {
            return;
        };
        // The rows are the ECUs the description declares plus any name an existing
        // entry carries: swap the description out from under a stimulus setup and
        // its entries must still have a row to be reached through, rather than
        // vanish into a list the document no longer supports.
        let mut nodes: Vec<String> = match app.fr_db(bus) {
            Some(db) => db.ecus.clone(),
            None => Vec::new(),
        };
        for t in app.snap.fr_tx.iter().filter(|t| t.bus == bus) {
            if !nodes.contains(&t.node) {
                nodes.push(t.node.clone());
            }
        }
        let Some(db) = app.fr_db(bus) else {
            let seen: u64 = app.snap.fr_loads.get(&bus).map_or(0, |l| l.frames);
            ui.text_disabled(if seen > 0 {
                "This bus is receiving frames, but no cluster description is loaded, so ECUs and frame ownership cannot be shown"
            } else {
                "This bus is configured, no frames have arrived yet, and no cluster description is loaded"
            });
            if !nodes.is_empty() {
                self.draw_ecu_leaves(ui, bus, &nodes, &[]);
            }
            return;
        };
        if nodes.is_empty() {
            // An empty ECU list is a property of the document, not of the session:
            // a cluster export carries no ECUs at all (both bundled FIBEX files are
            // one), and saying so beats a group that looks like it failed to load.
            ui.text_disabled(
                "The description declares no ECUs (this file is a cluster parameter export with no ECUs section)",
            );
            return;
        }
        // How many frames each ECU is the declared sender of, so a leaf says what
        // its generator panel will be able to offer.
        let mut tx_counts: std::collections::HashMap<&str, usize> =
            std::collections::HashMap::new();
        for ix in 0..db.frames.len() {
            if let Some(s) = db.frame_sender(ix) {
                *tx_counts.entry(s).or_insert(0) += 1;
            }
        }
        let counts: Vec<usize> = nodes
            .iter()
            .map(|n| tx_counts.get(n.as_str()).copied().unwrap_or(0))
            .collect();
        if let Some(_e) = ui
            .tree_node_config(format!("ECU ({})##frecu{bus}", nodes.len()))
            .default_open(true)
            .push()
        {
            self.draw_ecu_leaves(ui, bus, &nodes, &counts);
        }
    }

    /// The ECU leaves of one cluster. `counts` is the declared frame count per leaf;
    /// an empty slice means the description is gone and nothing can be claimed, so
    /// no count is printed -- which is not the same statement as "sends none".
    fn draw_ecu_leaves(&mut self, ui: &Ui, bus: u8, nodes: &[String], counts: &[usize]) {
        for (i, ecu) in nodes.iter().enumerate() {
            let text = match counts.get(i) {
                Some(0) | None => ecu.clone(),
                Some(n) => format!("{ecu} · sends {n} frames"),
            };
            if ui
                .selectable_config(format!("{text}##freculeaf{bus}_{i}"))
                .selected(
                    self.net_fr_sel
                        .as_ref()
                        .is_some_and(|(b, e)| *b == bus && e == ecu),
                )
                .build()
            {
                self.net_fr_sel = Some((bus, ecu.clone()));
                self.net_eth_sel = None;
            }
        }
    }

    /// One script node as a tree leaf: state marker + name, click opens the
    /// script editor.
    #[allow(clippy::too_many_arguments)]
    fn draw_script_leaf(
        &mut self,
        ui: &Ui,
        id: u64,
        name: &str,
        running: bool,
        enabled: bool,
        errored: bool,
        cmds: &mut Vec<NetworkCmd>,
    ) {
        let (marker, color) = if errored {
            ("[!]", [1.0, 0.55, 0.3, 1.0])
        } else if running {
            ("[*]", [0.45, 0.95, 0.45, 1.0])
        } else if enabled {
            ("[o]", [0.45, 0.62, 0.80, 1.0])
        } else {
            ("[-]", [0.5, 0.5, 0.55, 1.0])
        };
        ui.text_colored(color, marker);
        if ui.is_item_hovered() {
            ui.tooltip_text(
                "[*] running / [o] enabled, waiting for measurement / [-] disabled / [!] error (see the editor's log)",
            );
        }
        ui.same_line();
        if ui
            .selectable_config(format!("{name}##netscript{id}"))
            .build()
        {
            cmds.push(NetworkCmd::OpenScriptEditor(id));
        }
    }
}

pub fn render(app: &mut App, ui: &Ui) {
    let mut net = std::mem::take(&mut app.network_view);
    let mut genv = std::mem::take(&mut app.generator_view);
    let mut net_cmds = Vec::new();
    let mut gen_cmds = Vec::new();
    net.show(ui, app, &mut genv, &mut net_cmds, &mut gen_cmds);
    app.network_view = net;
    app.generator_view = genv;
    app.apply_network_cmds(net_cmds);
    app.apply_generator_cmds(gen_cmds);
}

impl NetworkView {
    fn show(
        &mut self,
        ui: &Ui,
        app: &App,
        genv: &mut GeneratorView,
        cmds: &mut Vec<NetworkCmd>,
        gen_cmds: &mut Vec<GeneratorCmd>,
    ) {
        let io = ui.io();
        let mut open = self.open;
        if open {
            let (pos, size) = crate::ui::fit_first_use(
                io.display_size(),
                [io.display_size()[0] * 0.3, io.display_size()[1] * 0.55],
                [860.0, 540.0],
            );
            ui.window("Network")
            .opened(&mut open)
            .position(pos, Condition::FirstUseEver)
            .size(size, Condition::FirstUseEver)
            .build(|| {
                // Profile row: enumerate the project's profiles/*.toml and
                // apply the selected one (all-or-nothing role overrides),
                // plus create-from-snapshot, open-in-editor and delete.
                let profile_dir = app
                    .project_path
                    .as_ref()
                    .and_then(|p| p.parent().map(|p| p.to_path_buf()))
                    .unwrap_or_default();
                if self.profile_names.is_none() {
                    self.profile_names = Some(Ok(crate::profile::list_profiles(&profile_dir)));
                }
                let Some(profile_list) =
                    self.profile_names.as_ref().and_then(|r| r.as_ref().ok()).cloned()
                else {
                    return;
                };
                ui.align_text_to_frame_padding();
                ui.text_disabled("Profile");
                if !profile_list.is_empty() {
                    ui.same_line();
                    let mut pick = self.profile_pick.min(profile_list.len() - 1);
                    ui.set_next_item_width(140.0);
                    if ui.combo_simple_string("##prof", &mut pick, &profile_list) {
                        self.profile_pick = pick;
                    }
                    ui.same_line();
                    if ui.button("Apply##profapply") {
                        let name =
                            profile_list[self.profile_pick.min(profile_list.len() - 1)].clone();
                        cmds.push(NetworkCmd::ApplyProfile(name));
                    }
                    if ui.is_item_hovered() {
                        ui.tooltip_text("Override roles from profiles/*.toml (all-or-nothing)");
                    }
                    ui.same_line();
                    if ui.button("Edit##profedit") {
                        let name =
                            profile_list[self.profile_pick.min(profile_list.len() - 1)].clone();
                        let path = profile_dir.join("profiles").join(format!("{name}.toml"));
                        cmds.push(NetworkCmd::OpenProfileFile { name, path });
                    }
                    if ui.is_item_hovered() {
                        ui.tooltip_text("Open this .toml in the system's default editor");
                    }
                    ui.same_line();
                    let name =
                        profile_list[self.profile_pick.min(profile_list.len() - 1)].clone();
                    if ui.button(if self.profile_delete_arm {
                        "Confirm delete##profdel"
                    } else {
                        "Delete##profdel"
                    }) {
                        if self.profile_delete_arm {
                            cmds.push(NetworkCmd::DeleteProfile(name));
                            self.profile_delete_arm = false;
                        } else {
                            self.profile_delete_arm = true;
                        }
                    }
                    if ui.is_item_hovered() {
                        ui.tooltip_text(
                            "Delete the selected profile file (click twice to confirm)",
                        );
                    }
                }
                ui.same_line();
                ui.set_next_item_width(120.0);
                ui.input_text("##profnew", &mut self.profile_draft_name)
                    .hint("new name")
                    .build();
                ui.same_line();
                if ui.button("Save current##profsave") {
                    cmds.push(NetworkCmd::SaveProfile(self.profile_draft_name.clone()));
                }
                if ui.is_item_hovered() {
                    ui.tooltip_text(
                        "Save the current roles and hardware connections as a new profile",
                    );
                }
                let dbc_nodes = collect(app);
                let total_dbc: usize = dbc_nodes.iter().map(|v| v.len()).sum();
                if self.net_selected >= total_dbc.max(1) {
                    self.net_selected = 0;
                }

                // 左右分栏：左边树形拓扑，右边所选节点的详情。两栏各自
                // 滚动——树的长度与详情的长度互不挤占。
                //
                // RESIZE_X makes the tree's right border draggable (ImGui's own
                // affordance, cursor turns into ↔), so a long node or cluster
                // name is the operator's to make room for; the width rides in the
                // project's layout. BORDERS stays because ImGui only draws the
                // persistent resize edge for a child that has one -- `border(true)`
                // and `child_flags` write the same field, so they are set here
                // together rather than chained.
                const TREE_W: f32 = 320.0;
                ui.child_window("net_tree")
                    .size([TREE_W, 0.0])
                    .child_flags(dear_imgui_rs::ChildFlags::RESIZE_X | dear_imgui_rs::ChildFlags::BORDERS)
                    .build(ui, || {
                        let mut flat_base = 0usize;
                        for (ch, infos) in dbc_nodes.iter().enumerate() {
                            self.draw_tree_section(ui, app, ch, infos, flat_base, cmds);
                            flat_base += infos.len();
                        }
                        // The FlexRay clusters after the CAN channels: same
                        // topology view, different numbering space on purpose --
                        // `FR{n}` is a cluster index, not a CAN channel.
                        for bus in fr_buses(app) {
                            self.draw_flexray_section(ui, app, bus);
                        }
                        // Ethernet after FlexRay: another numbering space, with
                        // matrix frames and captured L2 flows as its leaves.
                        for bus in eth_buses(app) {
                            self.draw_ethernet_section(ui, app, bus);
                        }
                    });
                ui.same_line();

                // Details scroll inside their own panel (which always fills the
                // remaining space), so long content never adds a scrollbar to the
                // outer window and shifts the topology sections.
                ui.child_window("node_details").size([0.0, 0.0]).build(ui, || {
                    if let Some(sel) = self.net_eth_sel.clone() {
                        self.draw_ethernet_detail(ui, app, &sel);
                        return;
                    }
                    if let Some((bus, ecu)) = self.net_fr_sel.clone() {
                        {
                            let _c =
                                ui.push_style_color(StyleColor::Text, [0.30, 0.80, 1.00, 1.0]);
                            ui.text_wrapped(format!(
                                "{} / {}  —  FlexRay sender",
                                app.fr_bus_label(bus),
                                ecu
                            ));
                        }
                        ui.separator();
                        gen_cmds.extend(genv.fr_ecu_generator(ui, app, bus, &ecu));
                        return;
                    }
                    if total_dbc == 0 {
                        // The other streams have their own selection models;
                        // an empty CAN tree is not "nothing to show" when a
                        // cluster or Ethernet bus is listed beside it.
                        if app.fr_buses.is_empty()
                            && app.snap.fr_loads.is_empty()
                            && app.eth_buses.is_empty()
                            && app.snap.eth_aggs.is_empty()
                        {
                            ui.text_wrapped("Nothing to display: no DBC, cluster description or Ethernet matrix is loaded");
                        } else {
                            ui.text_wrapped(
                                "No CAN nodes; pick an ECU under FlexRay or a frame/flow under Ethernet to see details",
                            );
                        }
                        return;
                    }
                    // Locate the selected DBC node (channel, index).
                    let mut remaining = self.net_selected;
                    let mut selected: Option<(usize, usize)> = None;
                    for (ch, infos) in dbc_nodes.iter().enumerate() {
                        if remaining < infos.len() {
                            selected = Some((ch, remaining));
                            break;
                        }
                        remaining -= infos.len();
                    }
                    let Some((ch, idx)) = selected else {
                        ui.text_wrapped("select a node to see its messages and signals");
                        return;
                    };
                    let ni = &dbc_nodes[ch][idx];
                    // The details pane can be narrow: wrapping the summary is
                    // better than clipping "receives N signal(s)" at the edge.
                    {
                        let _c =
                            ui.push_style_color(StyleColor::Text, [0.30, 0.80, 1.00, 1.0]);
                        ui.text_wrapped(format!(
                            "{} / {}  —  sends {} message(s), receives {} signal(s)",
                            app.channel_name(ch as u8),
                            ni.name,
                            ni.tx.len(),
                            ni.rx.len()
                        ));
                    }
                    let mut role_idx = crate::app::NodeRole::ALL
                        .iter()
                        .position(|r| *r == app.node_role(ch as u8, &ni.name))
                        .unwrap_or(0);
                    // An unlabelled combo under the node summary read as a
                    // stray field; name what it sets.
                    ui.align_text_to_frame_padding();
                    ui.text("Role");
                    ui.same_line();
                    ui.set_next_item_width(88.0);
                    let role_labels: Vec<&str> = crate::app::NodeRole::ALL
                        .iter()
                        .map(|r| r.label())
                        .collect();
                    if ui.combo_simple_string(format!("##netrole{ch}"), &mut role_idx, &role_labels)
                    {
                        let role = crate::app::NodeRole::ALL[role_idx];
                        cmds.push(NetworkCmd::SetNodeRole {
                            bus: ch as u8,
                            node: ni.name.clone(),
                            role,
                        });
                    }
                    ui.separator();

                    // 节点生成器：这个节点的条目、添加与响应规则，
                    // 紧接在角色声明之下——节点就是编辑单元。
                    genv.node_generator(ui, app, ch as u8, &ni.name, gen_cmds);
                    ui.separator();

                    // 脚本编辑入口在节点之下（而非总线）：新建的脚本
                    // 自动绑定到当前选中的 DBC 节点，发帧受其角色开关。
                    // 两个新建按钮统一排在标签同一行。
                    ui.align_text_to_frame_padding();
                    ui.text("Node scripts");
                    ui.same_line();
                    if ui.button(format!("+ Script node##netadd{ch}")) {
                        let name = format!("Node {}", app.snap.nodes.len() + 1);
                        let attached = Some((ch as u8, ni.name.clone()));
                        cmds.push(NetworkCmd::AddNode {
                            name,
                            channel: ch as u8,
                            attached,
                        });
                    }
                    ui.text_disabled("Click a script name to open its editor");
                    let node_scripts: Vec<(u64, String, bool, bool)> = app
                        .snap
                        .nodes
                        .iter()
                        .filter(|n| {
                            n.channel as usize == ch
                                && n.attached.as_ref().is_some_and(|a| a.1 == ni.name)
                        })
                        .map(|n| (n.id, n.name.clone(), n.running && !n.errored, n.enabled))
                        .collect();
                    for (nid, name, running, enabled) in node_scripts {
                        let dot = if running {
                            "[*]"
                        } else if enabled {
                            "[o]"
                        } else {
                            "[-]"
                        };
                        if ui
                            .selectable_config(format!("{dot} {name}##netscript{nid}"))
                            .build()
                        {
                            cmds.push(NetworkCmd::OpenScriptEditor(nid));
                        }
                        if ui.is_item_hovered() {
                            ui.tooltip_text(
                                "[*] running / [o] enabled, waiting for measurement / [-] disabled",
                            );
                        }
                    }
                    ui.separator();

                    // 回放块也是节点的一种驱动：该节点名下的块就近列出，
                    // 新建即绑定到本节点并默认只回放它的报文。
                    ui.align_text_to_frame_padding();
                    ui.text("Node replay blocks");
                    ui.same_line();
                    if ui.button(format!("+ Replay block##netaddblk{ch}")) {
                        let name = format!("Block {}", app.snap.blocks.len() + 1);
                        cmds.push(NetworkCmd::AddReplayBlock {
                            channel: ch as u8,
                            name,
                            node: ni.name.clone(),
                        });
                    }
                    if ui.is_item_hovered() {
                        ui.tooltip_text(
                            "Replay this node's recorded traffic onto the simulated bus (restbus); pick a log file below, then enable to send",
                        );
                    }
                    let node_blocks: Vec<(u64, String, bool, usize, Option<String>)> = app
                        .snap
                        .blocks
                        .iter()
                        .filter(|b| {
                            b.attached.as_ref().is_some_and(|a| a.0 == ch as u8 && a.1 == ni.name)
                        })
                        .map(|b| {
                            (
                                b.id,
                                b.name.clone(),
                                b.enabled,
                                b.frames,
                                b.last_error.clone(),
                            )
                        })
                        .collect();
                    for (bid, name, enabled, frames, last_error) in node_blocks {
                        // 启用开关即行首 checkbox；x 删除该块。
                        let mut on = enabled;
                        if ui.checkbox(format!("##blkon{bid}"), &mut on) {
                            cmds.push(NetworkCmd::SetReplayBlockEnabled { id: bid, on });
                        }
                        if ui.is_item_hovered() {
                            ui.tooltip_text(
                                "When enabled, sends with the recorded spacing during measurement (simulation only)",
                            );
                        }
                        ui.same_line();
                        ui.align_text_to_frame_padding();
                        ui.text(&name);
                        ui.same_line();
                        ui.text_disabled(format!("{frames} frames"));
                        if let Some(e) = &last_error {
                            ui.same_line();
                            ui.text_colored([1.0, 0.55, 0.3, 1.0], format!("load failed: {e}"));
                        }
                        ui.same_line();
                        if ui.button(format!("x##netblockrm{bid}")) {
                            cmds.push(NetworkCmd::RemoveReplayBlock(bid));
                            self.block_drafts.remove(&bid);
                            continue;
                        }
                        // 日志行（缩进）：路径草稿 + 文件选择，Apply 一并提交。
                        ui.indent();
                        let mut draft = self
                            .block_drafts
                            .entry(bid)
                            .or_insert_with(|| crate::ui::BlockDraft {
                                path: app
                                    .snap
                                    .blocks
                                    .iter()
                                    .find(|b| b.id == bid)
                                    .map(|b| b.path.clone())
                                    .unwrap_or_default(),
                                ids_text: String::new(),
                            })
                            .clone();
                        ui.set_next_item_width(-72.0);
                        if ui
                            .input_text(format!("##blpath{bid}"), &mut draft.path)
                            .build()
                            && let Some(d) = self.block_drafts.get_mut(&bid)
                        {
                            d.path = draft.path.clone();
                        }
                        ui.same_line();
                        if ui.button(format!("...##blfile{bid}"))
                            && let Some(p) = rfd::FileDialog::new()
                                .set_title("Select replay log")
                                .add_filter("Log files", &["asc", "blf"])
                                .pick_file()
                            && let Some(d) = self.block_drafts.get_mut(&bid)
                        {
                            d.path = p.to_string_lossy().into_owned();
                            draft.path = d.path.clone();
                        }
                        // id 过滤（可选）：留空 = 该节点的全部报文。
                        const BLOCK_IDS_HINT: &str = "id filter, e.g. 100, 3F4x";
                        ui.set_next_item_width(crate::ui::hint_width(
                            ui,
                            BLOCK_IDS_HINT,
                            140.0,
                        ));
                        if ui
                            .input_text(format!("##blids{bid}"), &mut draft.ids_text)
                            .hint(BLOCK_IDS_HINT)
                            .build()
                            && let Some(d) = self.block_drafts.get_mut(&bid)
                        {
                            d.ids_text = draft.ids_text.clone();
                        }
                        ui.same_line();
                        let saved = app
                            .snap
                            .blocks
                            .iter()
                            .find(|b| b.id == bid)
                            .map(|b| b.path.clone())
                            .unwrap_or_default();
                        if ui.button(format!("Apply##blapply{bid}"))
                            && let Some(d) = self.block_drafts.get(&bid)
                        {
                            let block = app.snap.blocks.iter().find(|b| b.id == bid);
                            cmds.push(NetworkCmd::SetReplayBlock {
                                id: bid,
                                name: name.clone(),
                                channel: ch as u8,
                                path: d.path.clone(),
                                node_filter: block.and_then(|b| b.node_filter.clone()),
                                attached: block.and_then(|b| b.attached.clone()),
                                ids: crate::ui::parse_id_filter(&d.ids_text),
                            });
                        }
                        if draft.path != saved {
                            ui.same_line();
                            ui.text_colored([1.0, 0.8, 0.4, 1.0], "Not applied");
                        }
                        ui.unindent();
                    }
                    ui.separator();
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
                    for (key, sig, sender) in &ni.rx {
                        let msg = app
                            .channel_dbc(ch as u8)
                            .and_then(|db| db.message_name_of(*key))
                            .unwrap_or("-");
                        let (id, ext) = *key;
                        let id_text = if ext {
                            format!("{id:03X} ext")
                        } else {
                            format!("{id:03X}")
                        };
                        ui.text(format!("  {sig}  <-  {sender}  ({msg} {id_text})"));
                    }
                });
            });
        }
        self.open = open;
    }
}

impl App {
    fn network_profile_dir(&self) -> std::path::PathBuf {
        self.project_path
            .as_ref()
            .and_then(|p| p.parent().map(|p| p.to_path_buf()))
            .unwrap_or_default()
    }

    fn refresh_network_profiles(&mut self) {
        let dir = self.network_profile_dir();
        self.network_view.profile_names = Some(Ok(crate::profile::list_profiles(&dir)));
    }

    pub(crate) fn apply_network_cmds(&mut self, cmds: Vec<NetworkCmd>) {
        for cmd in cmds {
            match cmd {
                NetworkCmd::OpenScriptEditor(id) => self.open_script_editor(id),
                NetworkCmd::SetNodeRole { bus, node, role } => {
                    self.set_node_role(bus, &node, role);
                }
                NetworkCmd::AddNode {
                    name,
                    channel,
                    attached,
                } => {
                    self.send(crate::bus::BusCommand::AddNode {
                        name,
                        channel,
                        attached,
                    });
                    self.settle();
                    if let Some(newest) = self.snap.nodes.iter().map(|n| n.id).max() {
                        self.open_script_editor(newest);
                    }
                }
                NetworkCmd::AddReplayBlock {
                    channel,
                    name,
                    node,
                } => {
                    self.add_replay_block(
                        channel,
                        name,
                        String::new(),
                        Some(node.clone()),
                        Some((channel, node)),
                    );
                }
                NetworkCmd::SetReplayBlockEnabled { id, on } => {
                    self.set_replay_block_enabled(id, on);
                }
                NetworkCmd::RemoveReplayBlock(id) => self.remove_replay_block(id),
                NetworkCmd::SetReplayBlock {
                    id,
                    name,
                    channel,
                    path,
                    node_filter,
                    attached,
                    ids,
                } => {
                    self.send(crate::bus::BusCommand::SetReplayBlock {
                        id,
                        name,
                        channel,
                        path,
                        node_filter,
                        attached,
                        ids,
                    });
                }
                NetworkCmd::ApplyProfile(name) => {
                    let dir = self.network_profile_dir();
                    self.status = match crate::profile::apply_profile(self, &dir, &name) {
                        Ok(s) => s,
                        Err(e) => e,
                    };
                    self.refresh_network_profiles();
                }
                NetworkCmd::OpenProfileFile { name, path } => {
                    let opened = std::process::Command::new("cmd")
                        .args(["/C", "start", ""])
                        .arg(&path)
                        .spawn();
                    self.status = match opened {
                        Ok(_) => format!("Opened {name}.toml in the system editor"),
                        Err(e) => format!("failed to open editor: {e}"),
                    };
                }
                NetworkCmd::DeleteProfile(name) => {
                    let dir = self.network_profile_dir();
                    self.status = match crate::profile::delete_profile(&dir, &name) {
                        Ok(()) => format!("profile `{name}` deleted"),
                        Err(e) => e,
                    };
                    self.refresh_network_profiles();
                }
                NetworkCmd::SaveProfile(name) => {
                    let dir = self.network_profile_dir();
                    match crate::profile::save_profile(self, &dir, &name) {
                        Ok(s) => {
                            self.status = s;
                            self.network_view.profile_draft_name.clear();
                            self.refresh_network_profiles();
                        }
                        Err(e) => self.status = e,
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The topology lists a bus for either reason: a loaded matrix, or traffic
    /// that arrived without one. Missing the second case is what made a live
    /// Ethernet cable invisible in Network.
    #[test]
    fn eth_buses_lists_matrix_only_and_traffic_only_buses() {
        let mut app = App::headless();
        app.eth_buses.insert(
            2,
            crate::app::EthBusCfg {
                path: "synthetic.arxml".to_string(),
                db: std::sync::Arc::new(crate::eth::db::EthDb::default()),
            },
        );
        let flow = crate::aggregate::EthFlowKey {
            bus: 1,
            vlan: None,
            dst: [0; 6],
            src: [1; 6],
            ethertype: 0x0800,
            len: 8,
        };
        app.eth_aggs.insert(
            flow,
            crate::aggregate::EthAgg {
                key: flow,
                ..Default::default()
            },
        );
        app.refresh_snapshot();
        assert_eq!(eth_buses(&app), vec![1, 2]);
    }
}
