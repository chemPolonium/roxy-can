/// A seeded session without a project file: open every DBC node's gate,
/// switch every generator entry on, and drive the curves from the sample
/// databases. Tests use this to stand in for a live file-based project.
#[cfg(test)]
fn seed_test_session(app: &mut App) {
    seed_sample_entries(app);
    app.start_virtual();
}

fn seed_sample_entries(app: &mut App) {
    let bus_count = app.snap.channel_count;
    for ch in 0..bus_count {
        let nodes: Vec<String> = app
            .snap
            .channels
            .get(ch)
            .and_then(|c| c.dbc.as_ref())
            .map(|db| db.nodes.clone())
            .unwrap_or_default();
        for node in nodes {
            app.set_node_role(ch as u8, &node, NodeRole::Simulated);
        }
    }
    let cycle_us = std::env::var("ROXY_WEB_CYCLE_US")
        .ok()
        .and_then(|value| value.parse::<u64>().ok())
        .filter(|value| *value >= 1_000)
        .unwrap_or(SAMPLE_CYCLE_US);
    let entries: Vec<(u8, u32)> = app
        .snap
        .tx
        .iter()
        .map(|entry| (entry.channel, entry.id))
        .collect();
    for (ch, id) in entries {
        app.send(BusCommand::SetEntryActive { ch, id, on: true });
        app.send(BusCommand::SetEntryCycle { ch, id, cycle_us });
    }
    for spec in &SAMPLE_SIGNALS {
        let mut src = ValueSrc::new(spec.name, spec.kind, spec.lo, spec.hi);
        src.period_us = spec.period_us;
        src.phase_us = spec.phase_us;
        src.redraw_us = spec.redraw_us;
        // Random wheels would otherwise share one sequence and overlap
        // exactly on the plot.
        src.seed ^= (spec.color as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15);
        app.send(BusCommand::SetEntrySource {
            ch: spec.ch,
            id: spec.id,
            src,
        });
        app.subscribe(app.can_key(spec.ch, spec.id, false, spec.name.to_string()));
    }
}

const SAMPLE_PROJECT_FILE: &str = "roxy-can-sample.rxproj";
const SAMPLE_SCRIPT_BAND_SET: &str = "speed-bands";
const SAMPLE_SCRIPT: &str = r#"let ticks = 0;

fn send_beacon(value) {
    let msg = frame(EngineStatus);
    msg.EngineSpeed = value;
    send_frame(msg);
}

on timer 200 {
    ticks = (ticks + 1) % 16;
    send_beacon(1000 + ticks * 400);
}

on panel_action("Beacon") {
    send_beacon(7400);
}
"#;

/// Writes the built-in starter databases next to a fresh project file, so
/// the sample behaves like any other file-based project. Picking a folder
/// that already holds this sample only opens it again.
fn write_sample_project(parent: &std::path::Path) -> Result<std::path::PathBuf, String> {
    let dir = sample_project_dir(parent)?;
    let project_path = dir.join(SAMPLE_PROJECT_FILE);
    if project_path.exists() {
        return Ok(project_path);
    }
    std::fs::create_dir_all(&dir).map_err(|e| format!("sample folder {}: {e}", dir.display()))?;
    let databases = dir.join("databases");
    std::fs::create_dir_all(&databases)
        .map_err(|e| format!("sample databases {}: {e}", databases.display()))?;
    let sample_dbc = databases.join("sample.dbc");
    let motbus_dbc = databases.join("motbus.dbc");
    std::fs::write(&sample_dbc, crate::sample_data::SAMPLE_DBC)
        .map_err(|e| format!("sample.dbc: {e}"))?;
    std::fs::write(&motbus_dbc, crate::sample_data::MOTBUS_DBC)
        .map_err(|e| format!("motbus.dbc: {e}"))?;

    let mut app = App::headless();
    replace_can_dbc(&mut app, 0, "builtin:sample.dbc", &sample_dbc);
    replace_can_dbc(&mut app, 1, "builtin:motbus.dbc", &motbus_dbc);
    seed_sample_entries(&mut app);
    seed_sample_script(&mut app, &dir.join("scripts"))?;
    seed_sample_band_set(&mut app);
    seed_sample_panel(&mut app);
    for spec in &SAMPLE_SIGNALS {
        let key = app.can_key(spec.ch, spec.id, false, spec.name.to_string());
        chart_select_key(&mut app, key);
    }
    if !app.save_project(Some(project_path.clone())) {
        return Err(app.status.clone());
    }
    Ok(project_path)
}

fn replace_can_dbc(app: &mut App, bus: u8, builtin: &str, path: &std::path::Path) {
    app.remove_can_dbc(bus, builtin);
    app.add_can_dbc(bus, &path.to_string_lossy());
}

/// One linked script bound to an already-simulated sample node. The project
/// saves only the relative path; the source itself lives in the `.rxcan`.
fn seed_sample_script(app: &mut App, scripts: &std::path::Path) -> Result<(), String> {
    std::fs::create_dir_all(scripts)
        .map_err(|error| format!("sample scripts: {error}"))?;
    let path = scripts.join("engine_beacon.rxcan");
    std::fs::write(&path, SAMPLE_SCRIPT)
        .map_err(|error| format!("engine_beacon.rxcan: {error}"))?;
    app.send(crate::bus::BusCommand::AddNode {
        name: "Engine Beacon".to_string(),
        channel: 0,
        attached: Some((0, "EngineECU".to_string())),
    });
    let id = app
        .snap
        .nodes
        .last()
        .expect("the sample script node")
        .id;
    app.send(crate::bus::BusCommand::SetNodeFile {
        id,
        path: path.to_string_lossy().into_owned(),
        file_source: SAMPLE_SCRIPT.to_string(),
    });
    app.send(crate::bus::BusCommand::SetNodeSource {
        id,
        source: SAMPLE_SCRIPT.to_string(),
    });
    Ok(())
}

fn seed_sample_band_set(app: &mut App) {
    app.band_sets.push(crate::band::BandSet {
        id: SAMPLE_SCRIPT_BAND_SET.to_string(),
        name: "Engine speed bands".to_string(),
        bands: vec![
            crate::band::Band {
                upper: Some(2_000.0),
                inclusive: false,
                text: "idle".to_string(),
                color: Some(1),
            },
            crate::band::Band {
                upper: Some(5_000.0),
                inclusive: false,
                text: "cruise".to_string(),
                color: Some(2),
            },
            crate::band::Band {
                upper: None,
                inclusive: false,
                text: "boost".to_string(),
                color: Some(3),
            },
        ],
    });
}

fn seed_sample_panel(app: &mut App) {
    let speed = app
        .can_key(0, 0x100, false, "EngineSpeed".to_string())
        .text();
    let vehicle = app
        .can_key(0, 0x200, false, "VehicleSpeed".to_string())
        .text();
    let brake = app
        .can_key(0, 0x320, false, "BrakePressure".to_string())
        .text();
    app.panel_pages = vec![serde_json::json!({
        "format": "roxy-can.panel-page",
        "version": 1,
        "page": {
            "name": "Overview",
            "width": 160,
            "height": 90,
            "widgets": [
                {
                    "kind": "value",
                    "label": "Engine speed",
                    "source": speed,
                    "digits": 0,
                    "labelSet": SAMPLE_SCRIPT_BAND_SET,
                    "x": 2, "y": 2, "w": 18, "h": 6
                },
                {
                    "kind": "bar",
                    "label": "Vehicle speed",
                    "source": vehicle,
                    "min": 0,
                    "max": 300,
                    "digits": 1,
                    "x": 24, "y": 2, "w": 20, "h": 7
                },
                {
                    "kind": "lamp",
                    "label": "Brake",
                    "source": brake,
                    "threshold": 20,
                    "color": 2,
                    "x": 2, "y": 10, "w": 14, "h": 4
                },
                {
                    "kind": "button",
                    "label": "Beacon",
                    "action": "script",
                    "script": "Beacon",
                    "x": 24, "y": 11, "w": 16, "h": 5
                }
            ]
        }
    })];
}

/// A dedicated `roxy-can-sample` folder under the chosen parent; an existing
/// sample is reused, while unrelated non-empty folders get the next suffix.
fn sample_project_dir(parent: &std::path::Path) -> Result<std::path::PathBuf, String> {
    for index in 0..100 {
        let dir = if index == 0 {
            parent.join("roxy-can-sample")
        } else {
            parent.join(format!("roxy-can-sample-{index}"))
        };
        if !dir.exists() {
            return Ok(dir);
        }
        if dir.join(SAMPLE_PROJECT_FILE).exists() {
            return Ok(dir);
        }
        let empty = std::fs::read_dir(&dir)
            .map(|mut entries| entries.next().is_none())
            .unwrap_or(false);
        if empty {
            return Ok(dir);
        }
    }
    Err("no free roxy-can-sample folder name in the selected directory".to_string())
}

/// Chronological trace buffer owned by the app thread. The API sends a seed
/// on first contact and only the frames appended since the client's last
/// sequence afterwards; the frontend keeps the 5k window and virtualizes it,
/// so a full table never costs a full JSON snapshot per poll.
#[derive(Default)]
struct TraceStore {
    epoch: u64,
    next_seq: u64,
    log: VecDeque<(u64, WebFrame)>,
    /// Total frames the source ring has ever held this run (`len + lost`).
    /// Using the total, not `len`, lets a full ring trim its head without
    /// looking like a reset on every publish.
    last_total: u64,
    /// The FR and Ethernet rings expose no head-loss counter; a shrinking
    /// length is a new run (or a cleared ring), so it triggers a reseed.
    last_fr_len: usize,
    last_eth_len: usize,
    primed: bool,
}

impl TraceStore {
    fn seed_rows(app: &App) -> Vec<WebFrame> {
        let mut rows: Vec<WebFrame> = app
            .snap
            .trace
            .iter()
            .map(|frame| frame_dto(app, frame))
            .chain(app.snap.fr_trace.iter().map(|row| fr_frame_dto(app, row)))
            .chain(
                app.snap
                    .eth_trace
                    .iter()
                    .map(|frame| eth_frame_dto(app, frame)),
            )
            .collect();
        rows.sort_by_key(|row| row.t_us);
        if rows.len() > TRACE_LOG_CAP {
            rows.drain(..rows.len() - TRACE_LOG_CAP);
        }
        rows
    }

    fn sync(&mut self, app: &App) {
        let len = app.snap.trace.len();
        let loss = app.snap.trace.head_loss().0;
        let total = len as u64 + loss;
        let fr_len = app.snap.fr_trace.len();
        let eth_len = app.snap.eth_trace.len();
        let reset = !self.primed
            || total < self.last_total
            || fr_len < self.last_fr_len
            || eth_len < self.last_eth_len;
        if reset {
            self.primed = true;
            self.epoch = self.epoch.wrapping_add(1).max(1);
            self.next_seq = 0;
            self.log.clear();
            for row in Self::seed_rows(app) {
                self.push(row);
            }
        } else {
            let mut rows = Vec::new();
            let new_count = total.saturating_sub(self.last_total) as usize;
            if new_count > 0 {
                rows.extend(
                    app.snap
                        .trace
                        .iter()
                        .rev()
                        .take(new_count)
                        .map(|frame| frame_dto(app, frame)),
                );
            }
            if fr_len > self.last_fr_len {
                let take = fr_len - self.last_fr_len;
                rows.extend(
                    app.snap
                        .fr_trace
                        .iter()
                        .rev()
                        .take(take)
                        .map(|row| fr_frame_dto(app, row)),
                );
            }
            if eth_len > self.last_eth_len {
                let take = eth_len - self.last_eth_len;
                rows.extend(
                    app.snap
                        .eth_trace
                        .iter_rev()
                        .take(take)
                        .map(|frame| eth_frame_dto(app, frame)),
                );
            }
            // The three rings are drained in arrival order within one
            // publish; sorting the small delta keeps the merged table
            // chronological without touching the stored log.
            rows.sort_by_key(|row| row.t_us);
            for row in rows {
                self.push(row);
            }
        }
        self.last_total = total;
        self.last_fr_len = fr_len;
        self.last_eth_len = eth_len;
    }

    fn push(&mut self, frame: WebFrame) {
        self.log.push_back((self.next_seq, frame));
        self.next_seq += 1;
        while self.log.len() > TRACE_LOG_CAP {
            self.log.pop_front();
        }
    }

    fn reset(&mut self) {
        self.primed = false;
        self.log.clear();
        self.next_seq = 0;
        self.last_total = 0;
        self.last_fr_len = 0;
        self.last_eth_len = 0;
    }

    fn first_seq(&self) -> u64 {
        self.log
            .front()
            .map(|(seq, _)| *seq)
            .unwrap_or(self.next_seq)
    }

    /// Newest first, ready to prepend on the client.
    fn seed(&self) -> Vec<WebFrame> {
        self.log
            .iter()
            .rev()
            .map(|(_, frame)| frame.clone())
            .collect()
    }

    fn after(&self, seq: u64) -> Vec<WebFrame> {
        self.log
            .iter()
            .rev()
            .filter(|(s, _)| *s > seq)
            .map(|(_, frame)| frame.clone())
            .collect()
    }
}

fn frame_dto(app: &App, frame: &CanFrame) -> WebFrame {
    let channel = app.snap.channels.get(frame.channel as usize);
    WebFrame {
        kind: "can".to_string(),
        addr: if frame.extended {
            format!("0x{:08X}", frame.id)
        } else {
            format!("0x{:03X}", frame.id)
        },
        t_us: frame.t_us,
        bus: frame.channel,
        bus_name: channel
            .map(|c| c.name.clone())
            .unwrap_or_else(|| format!("CAN{}", frame.channel + 1)),
        id: frame.id,
        extended: frame.extended,
        name: app
            .message_name(frame.channel, frame.id, frame.extended)
            .unwrap_or_default(),
        dir: match frame.dir {
            Direction::Rx => "Rx",
            Direction::Tx => "Tx",
        },
        len: frame.len,
        data: hex(frame.payload()),
        fd: frame.is_fd(),
        brs: frame.brs(),
    }
}

fn fr_frame_dto(app: &App, row: &crate::trace::FrRow) -> WebFrame {
    WebFrame {
        kind: "fr".to_string(),
        addr: format!("slot {}", row.slot),
        t_us: row.t_us,
        bus: row.bus,
        bus_name: app.fr_bus_name(row.bus),
        id: row.slot as u32,
        extended: false,
        name: app.fr_row_name(row).unwrap_or("").to_string(),
        dir: "Rx",
        len: row.payload.len().min(u8::MAX as usize) as u8,
        data: hex(&row.payload),
        fd: false,
        brs: false,
    }
}

fn eth_frame_dto(app: &App, frame: &crate::eth::EthFrame) -> WebFrame {
    const SHOWN_BYTES: usize = 32;
    let shown = frame.payload.len().min(SHOWN_BYTES);
    let mut data = hex(&frame.payload[..shown]);
    if frame.payload.len() > SHOWN_BYTES {
        data.push_str(" …");
    }
    WebFrame {
        kind: "eth".to_string(),
        addr: format!("0x{:04X}", frame.ethertype),
        t_us: frame.t_us,
        bus: frame.bus,
        bus_name: app.eth_bus_label(frame.bus),
        id: frame.ethertype as u32,
        extended: false,
        name: app.eth_row_name(frame),
        dir: match frame.dir {
            Direction::Rx => "Rx",
            Direction::Tx => "Tx",
        },
        len: frame.payload.len().min(u8::MAX as usize) as u8,
        data,
        fd: false,
        brs: false,
    }
}

#[derive(Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
struct SignalSeries {
    key: String,
    name: String,
    bus: String,
    unit: String,
    color: usize,
    /// Latest DBC `VAL_` label, used as the text fallback when no band names
    /// the current value.
    #[serde(skip_serializing_if = "Option::is_none")]
    label: Option<String>,
    /// True for a signal subscribed only for cursor readout: no lane, but
    /// its history participates in the A/B table.
    read: bool,
    /// `[seconds, value]`, oldest first.
    points: Vec<[f64; 2]>,
}

struct SignalBuffer {
    meta: SignalSeries,
    points: VecDeque<(u64, f64)>,
}

#[derive(Clone)]
struct SelectedSignal {
    key: SigKey,
    name: String,
    bus: String,
    unit: String,
    color: usize,
    /// Read-only signal: subscribed and shown at cursors, never drawn.
    read: bool,
}

const SIGNAL_PALETTE: usize = 8;

