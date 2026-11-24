fn build_catalog(app: &App) -> WebCatalog {
    let mut channels: Vec<WebCatalogChannel> = app
        .snap
        .channels
        .iter()
        .enumerate()
        .map(|(ch, channel)| WebCatalogChannel {
            kind: "can".to_string(),
            index: ch as u8,
            name: channel.name.clone(),
            aliases: Vec::new(),
            messages: app
                .model_comms
                .get(ch)
                .and_then(Option::as_ref)
                .map(|comm| can_catalog_messages(comm))
                .unwrap_or_default(),
        })
        .collect();
    for bus in app.fr_buses.keys() {
        let messages = app
            .fr_models
            .get(bus)
            .map(|comm| fr_catalog_messages(comm))
            .unwrap_or_default();
        let aliases = app
            .fr_models
            .get(bus)
            .map(|comm| catalog_aliases(comm))
            .unwrap_or_default();
        channels.push(WebCatalogChannel {
            kind: "fr".to_string(),
            index: *bus,
            name: app.fr_bus_name(*bus),
            aliases,
            messages,
        });
    }
    for bus in app.eth_buses.keys() {
        let messages = app
            .eth_models
            .get(bus)
            .map(|comm| eth_catalog_messages(comm, app.eth_length_includes_header))
            .unwrap_or_default();
        let aliases = app
            .eth_models
            .get(bus)
            .map(|comm| catalog_aliases(comm))
            .unwrap_or_default();
        channels.push(WebCatalogChannel {
            kind: "eth".to_string(),
            index: *bus,
            name: app.eth_bus_label(*bus),
            aliases,
            messages: {
                let mut messages = messages;
                messages.extend(someip_catalog_messages(app, *bus));
                messages
            },
        });
    }
    WebCatalog { channels }
}

#[derive(Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
struct WebSomeIpService {
    id: u16,
    name: String,
    bus: u8,
    bus_name: String,
    major: u8,
    source: String,
    methods: Vec<WebSomeIpMember>,
    events: Vec<WebSomeIpMember>,
    instances: Vec<WebSomeIpInstance>,
}

#[derive(Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
struct WebSomeIpMember {
    id: u16,
    name: String,
    fields: Vec<String>,
    field_defs: Vec<WebSomeIpField>,
    input_field_defs: Vec<WebSomeIpField>,
    output_field_defs: Vec<WebSomeIpField>,
}

#[derive(Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
struct WebSomeIpField {
    name: String,
    /// `number` or `text`; unions are flattened into a `$selector` plus their
    /// variant fields, so the picker can drive them like any other record.
    kind: String,
    unit: String,
    min: Option<f64>,
    max: Option<f64>,
    /// Enum labels, when the ARXML carries a TEXTTABLE for the field.
    labels: Vec<(i64, String)>,
}

#[derive(Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
struct WebSomeIpInstance {
    id: u16,
    major: u8,
    minor: u32,
    eventgroups: Vec<u16>,
    endpoint: String,
}

#[derive(Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
struct WebReplaySource {
    path: String,
    name: String,
    folder: String,
    size: u64,
    modified_ms: u64,
}

#[derive(Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
struct WebState {
    measuring: bool,
    trace_paused: bool,
    recording: bool,
    record_file: String,
    record_eth_file: String,
    record_filter: String,
    mode: String,
    /// Bus-event marker times in seconds, oldest first.
    markers: Vec<f64>,
    frames: u64,
    fps: f64,
    trace_len: usize,
    sim_time_us: u64,
    status: String,
    project: String,
    project_path: String,
    dirty: bool,
    log: String,
    log_info: String,
    run_mode: String,
    replay_pos_us: Option<u64>,
    replay_total_us: Option<u64>,
    replay_speed: f64,
    recent_projects: Vec<String>,
    recent_logs: Vec<String>,
    replay_folders: Vec<String>,
    replay_files: Vec<String>,
    replay_recordings: Vec<WebReplaySource>,
    /// Workspace adopted from the saved project plus the revision that owns
    /// it; the browser applies it once per revision.
    workspace: Option<serde_json::Value>,
    panel_pages: Vec<serde_json::Value>,
    someip: Vec<crate::eth::someip::SomeIpEvent>,
    someip_subscriptions: Vec<crate::bus::SomeIpSubscriptionView>,
    someip_issues: Vec<String>,
    someip_services: Vec<WebSomeIpService>,
    workspace_rev: u64,
    reference: Option<crate::reference::ReferenceRun>,
    reference_rev: u64,
    /// Definitions the editor lists; the values ride the normal signal
    /// streams under `derived:<id>` keys.
    derived: Vec<WebDerivedDef>,
    replay_map: WebReplayMap,
    /// One row per CAN/FlexRay/Ethernet bus; `can`/`flexray`/`ethernet`
    /// carries the kind-specific detail.
    buses: Vec<WebBus>,
    /// The last scanned ARXML's cluster list, shown as the binding chooser.
    #[serde(skip_serializing_if = "Option::is_none")]
    arxml_scan: Option<WebArxmlScan>,
    real_bus: bool,
    hw_refresh: u64,
    tx: Vec<WebTx>,
    fr_tx: Vec<WebFrTx>,
    blocks: Vec<WebReplayBlock>,
    triggers: Vec<WebTrigger>,
    /// Recent trigger edges, oldest first; the replay bar walks these.
    trigger_fires: Vec<WebTriggerFire>,
    trigger_limits: WebTriggerLimits,
    sysvars: Vec<WebSysVar>,
    band_sets: Vec<crate::band::BandSet>,
    monitor: Vec<WebMonitorRow>,
    /// One or two rows per line on the Monitor page.
    monitor_columns: u8,
    data: Vec<WebDataRow>,
    state_rows: Vec<WebStateRow>,
    state_window_s: f64,
    scripts: Vec<WebScriptInfo>,
    stats: Vec<WebStatsRow>,
    bus_stats: Vec<WebBusStats>,
    spec_rows: Vec<WebSpecRow>,
    spec_tolerance: u64,
    spec_grace: u64,
    spec_periodic: usize,
    channels: Vec<WebChannel>,
    messages: Vec<WebMessage>,
    write: Vec<WebLine>,
    series: Vec<WebSeriesPoint>,
}

/// One chart sample: total plus every bus. Rates are frames/s, loads are
/// percent. The frontend draws solid rate lines and dashed load lines.
#[derive(Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
struct WebSeriesPoint {
    total: f64,
    bus_rates: Vec<f64>,
    bus_loads: Vec<f64>,
}

#[derive(Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
struct WebFrBus {
    cluster: String,
    attached: bool,
    channel: i32,
}

#[derive(Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
struct WebEthBus {
    clusters: usize,
    pdus: usize,
    endpoints: usize,
    pdu_names: Vec<String>,
    endpoint_list: Vec<WebEthEndpoint>,
    #[serde(skip_serializing_if = "Option::is_none")]
    attached: Option<String>,
}

#[derive(Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
struct WebEthEndpoint {
    name: String,
    ip: String,
    port: u16,
    protocol: String,
    connector: String,
    pdu: String,
}

#[derive(Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
struct WebArxmlScan {
    path: String,
    clusters: Vec<WebArxmlCluster>,
}

#[derive(Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
struct WebArxmlCluster {
    kind: String,
    name: String,
}

fn description_view(source: &crate::model::ClusterSource) -> WebDescription {
    match source {
        crate::model::ClusterSource::Arxml { path, cluster } => WebDescription {
            kind: "arxml".to_string(),
            path: path.clone(),
            cluster: cluster.clone(),
        },
        crate::model::ClusterSource::Fibex { path } => WebDescription {
            kind: "fibex".to_string(),
            path: path.clone(),
            cluster: String::new(),
        },
        _ => WebDescription {
            kind: "manual".to_string(),
            path: String::new(),
            cluster: String::new(),
        },
    }
}

#[derive(Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
struct WebReplayBlock {
    id: u64,
    name: String,
    channel: u8,
    path: String,
    node_filter: Option<String>,
    ids: Vec<(u32, bool)>,
    enabled: bool,
    frames: usize,
    error: Option<String>,
}

#[derive(Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
struct WebFrTx {
    bus: u8,
    bus_name: String,
    slot: u16,
    name: String,
    node: String,
    active: bool,
    cycle_us: u64,
    data_text: String,
    sent_text: String,
    muted: bool,
    undescribed: bool,
}

#[derive(Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
struct WebNode {
    name: String,
    simulated: bool,
    entries: usize,
    /// Unified ECU identity when the ARXML model names the same node.
    #[serde(skip_serializing_if = "Option::is_none")]
    ecu: Option<u32>,
}

#[derive(Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
struct WebReplayRow {
    raw: u8,
    count: u64,
    /// The bus the core would actually use; `None` = these frames are dropped.
    target: Option<u8>,
    /// Whether the table has a deliberate choice for this channel. Without
    /// it, "As-is" and "No replay" both look like `target: None` for a
    /// channel whose number names no bus.
    configured: bool,
}

#[derive(Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
struct WebReplayMap {
    can: Vec<WebReplayRow>,
    fr: Vec<WebReplayRow>,
    eth: Vec<WebReplayRow>,
}

/// Census rows plus configured rows whose channel has not arrived yet, with
/// the target the core would actually use (`None` = no replay).
fn replay_rows(
    census: &[(u8, u64)],
    map: &crate::replay_map::ReplayMap,
    kind: crate::replay_map::LogBusKind,
    effective: impl Fn(u8) -> Option<u8>,
) -> Vec<WebReplayRow> {
    use crate::replay_map::ReplayTarget;
    let mut rows: std::collections::BTreeMap<u8, u64> = census.iter().copied().collect();
    for (raw, _) in map.configured(kind) {
        rows.entry(raw).or_insert(0);
    }
    rows.into_iter()
        .map(|(raw, count)| WebReplayRow {
            raw,
            count,
            target: effective(raw),
            configured: !matches!(map.target(kind, raw), ReplayTarget::Same),
        })
        .collect()
}

/// Applies one channel-map choice. The frontend picks a bus; the map stores
/// the cluster it names, so the choice survives channel renumbering. Errors
/// are user-facing status text, not a failed request.
fn apply_replay_channel(
    app: &mut App,
    kind: crate::replay_map::LogBusKind,
    raw: u8,
    target: ReplayChannelChoice,
) -> Result<(), String> {
    let mut map = app.snap.replay_map.clone();
    match target {
        ReplayChannelChoice::Word(word) if word == "same" => map.forget(kind, raw),
        ReplayChannelChoice::Word(word) if word == "drop" => map.set(kind, raw, None),
        ReplayChannelChoice::Word(word) => {
            return Err(format!("unknown replay target: {word}"));
        }
        ReplayChannelChoice::Bus(bus) => {
            let Some(cluster) = app.cluster_of_bus(kind, bus) else {
                return Err(format!(
                    "replay target bus {bus} has no cluster in this project"
                ));
            };
            map.set(kind, raw, Some(cluster));
        }
    }
    app.send(BusCommand::SetReplayMap(map));
    Ok(())
}

#[derive(Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
struct WebBus {
    index: u8,
    id: u32,
    kind: String,
    name: String,
    frames: usize,
    source: String,
    /// Every description bound to the bus, DBC and ARXML alike, in binding
    /// order.
    descriptions: Vec<WebDescription>,
    #[serde(skip_serializing_if = "Option::is_none")]
    can: Option<WebCanBus>,
    #[serde(skip_serializing_if = "Option::is_none")]
    flexray: Option<WebFrBus>,
    #[serde(skip_serializing_if = "Option::is_none")]
    ethernet: Option<WebEthBus>,
}

#[derive(Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
struct WebCanBus {
    bitrate_kbps: u32,
    fd_data_kbps: u32,
    nodes: Vec<WebNode>,
    attached: Option<WebHwAttachment>,
}

#[derive(Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
struct WebDescription {
    kind: String,
    path: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    cluster: String,
}

#[derive(Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
struct WebHwAttachment {
    driver: String,
    adapter: i32,
    kbps: u32,
    fd: bool,
    can_tx: bool,
    tx_fail: Option<WebTxFail>,
}

#[derive(Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
struct WebTxFail {
    reason: String,
    count: u64,
}

#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct WebSrc {
    name: String,
    kind: String,
    lo: f64,
    hi: f64,
    period_us: u64,
    phase_us: u64,
    seq: Vec<f64>,
    seed: u64,
    redraw_us: u64,
}

#[derive(Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
struct WebTx {
    channel: u8,
    bus: String,
    id: u32,
    name: String,
    node: String,
    active: bool,
    gate_open: bool,
    fd: bool,
    brs: bool,
    cycle_us: u64,
    data_text: String,
    sent_text: String,
    muted: bool,
    srcs: Vec<WebSrc>,
}

#[derive(Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
struct WebTrigger {
    summary: String,
    action: String,
    kind: String,
    enabled: bool,
    fired: u64,
    level: bool,
    last_fire_us: u64,
    ch: u8,
    id: u32,
    ext: bool,
    signal: String,
    threshold: f64,
    rising: bool,
    bus: u8,
    slot: u16,
    var_key: String,
    derived_id: String,
    action_kind: String,
    send_ch: u8,
    send_id: u32,
    cooldown_ms: u32,
    max_fires: u32,
    pre_frames: Option<u32>,
    post_frames: Option<u32>,
    sys_var_key: String,
    sys_var_value: f64,
    script_name: String,
    script_value: Option<f64>,
    send_frame: String,
    send_data: String,
    someip_method: u16,
    someip_client: u16,
    someip_session: u16,
    someip_type: u8,
    cond_someip_method: Option<u16>,
    cond_someip_type: Option<u8>,
    someip_values: std::collections::BTreeMap<String, f64>,
    someip_transport: String,
    someip_host: String,
    someip_port: u16,
    offset: u8,
    mask: u8,
    value: u8,
    vlan: Option<u16>,
}

#[derive(Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
struct WebTriggerFire {
    index: usize,
    at_us: u64,
    text: String,
    action: String,
    result: String,
    ok: bool,
}

#[derive(Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
struct WebTriggerLimits {
    pre_frames: usize,
    post_frames: u32,
    marker_cap: usize,
}

#[derive(Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
struct WebSpecRow {
    name: String,
    addr: String,
    bus: String,
    rule: String,
    declared: String,
    measured: String,
    count: u64,
    first: String,
    last: String,
}

#[derive(Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
struct WebBusStatRow {
    label: String,
    current: String,
    min: String,
    max: String,
    avg: String,
}

#[derive(Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
struct WebBusStats {
    name: String,
    kbps: u32,
    load_now: f64,
    load_history: Vec<f64>,
    rows: Vec<WebBusStatRow>,
}

#[derive(Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
struct WebStatsRow {
    label: String,
    bus: String,
    count: u64,
    min_ms: String,
    avg_ms: String,
    max_ms: String,
    jitter_ms: String,
    share: String,
    len: u8,
}

#[derive(Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
struct WebMonitorRow {
    kind: String,
    ch: u8,
    id: u32,
    ext: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    frame: Option<String>,
    name: String,
    /// Canonical `sig:` key; the move command resolves it to reorder rows.
    key: String,
    label: String,
    bus: String,
    /// `Frame / PDU / Signal` from the loaded model; empty for derived rows.
    #[serde(skip_serializing_if = "String::is_empty")]
    path: String,
    value: f64,
    has_value: bool,
    unit: String,
    digits: usize,
    bands: Vec<crate::band::Band>,
    #[serde(skip_serializing_if = "Option::is_none")]
    band_set_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    enum_label: Option<String>,
}

#[derive(Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
struct WebDataRow {
    kind: String,
    ch: u8,
    id: u32,
    ext: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    frame: Option<String>,
    name: String,
    /// Canonical `sig:` key; the move command resolves it to reorder rows.
    key: String,
    bus: String,
    /// `Frame / PDU / Signal` from the loaded model; empty for derived rows.
    #[serde(skip_serializing_if = "String::is_empty")]
    path: String,
    value: String,
    unit: String,
    raw: String,
    min: String,
    avg: String,
    max: String,
    frac: f64,
    visible: bool,
}

#[derive(Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
struct WebDerivedDef {
    id: String,
    name: String,
    unit: String,
    expression: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    band_set_id: Option<String>,
    bands: Vec<crate::band::Band>,
    sources: Vec<crate::derived::DerivedSource>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<String>,
}

#[derive(Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
struct WebStateBand {
    start: f64,
    end: f64,
    label: String,
    color: usize,
}

#[derive(Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
struct WebStateRule {
    bands: Vec<crate::band::Band>,
    #[serde(skip_serializing_if = "Option::is_none")]
    band_set_id: Option<String>,
}

#[derive(Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
struct WebStateRow {
    kind: String,
    ch: u8,
    id: u32,
    ext: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    frame: Option<String>,
    name: String,
    /// Canonical `sig:` key, shared with the catalog and observer rows.
    key: String,
    bus: String,
    current: String,
    bands: Vec<WebStateBand>,
    rule: Option<WebStateRule>,
}

#[derive(Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
struct WebScriptIssue {
    line: u32,
    col: Option<u32>,
    message: String,
}

#[derive(Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
struct WebScriptTest {
    name: String,
    runs: u64,
    failures: u64,
    line: u32,
    time_s: f64,
}

#[derive(Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
struct WebScript {
    id: u64,
    name: String,
    channel: u8,
    attached_bus: Option<u8>,
    attached_node: Option<String>,
    enabled: bool,
    running: bool,
    errored: bool,
    issue: Option<WebScriptIssue>,
    file_path: String,
    source: String,
    tests: Vec<WebScriptTest>,
    /// `on panel_action("…")` names this script declares.
    panel_actions: Vec<String>,
    log: Vec<String>,
}

#[derive(Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
struct WebScriptInfo {
    id: u64,
    name: String,
    channel: u8,
    attached_bus: Option<u8>,
    attached_node: Option<String>,
    enabled: bool,
    running: bool,
    errored: bool,
    file_path: String,
    log_len: usize,
    last_log: String,
    /// `on panel_action("…")` names this script declares.
    panel_actions: Vec<String>,
}

#[derive(Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
struct WebSysVar {
    namespace: String,
    name: String,
    value: f64,
    init: f64,
    min: Option<f64>,
    max: Option<f64>,
    unit: String,
    comment: String,
    label_sets: Vec<crate::bus::SysVarLabelSet>,
    band_set_id: Option<String>,
}

#[derive(Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
struct WebHwChannel {
    driver: String,
    index: i32,
    name: String,
    detail: Option<String>,
}

#[derive(Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
struct WebHardware {
    channels: Vec<WebHwChannel>,
    fr_channels: Vec<WebHwChannel>,
    eth_channels: Vec<WebEthChannel>,
    error: String,
    fr_error: String,
    eth_error: String,
}

#[derive(Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
struct WebEthChannel {
    device: String,
    name: String,
}

#[derive(Default)]
struct HardwareCache {
    generation: u64,
    data: Option<WebHardware>,
}

#[derive(Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
struct WebChannel {
    name: String,
    bitrate_kbps: u32,
    fd_data_kbps: u32,
}

#[derive(Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
struct WebFrame {
    kind: String,
    addr: String,
    t_us: u64,
    bus: u8,
    bus_name: String,
    id: u32,
    extended: bool,
    name: String,
    dir: &'static str,
    len: u8,
    data: String,
    fd: bool,
    brs: bool,
}

#[derive(Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
struct WebMessageSignal {
    name: String,
    /// Formatted physical value.
    value: String,
    raw: i64,
    unit: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    label: Option<String>,
}

/// E2E counter continuity for a protected message. The CRC verdict is not
/// included: authentication stays unverified.
#[derive(Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
struct WebE2e {
    profile: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    data_id: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    counter: Option<u8>,
    #[serde(skip_serializing_if = "Option::is_none")]
    crc: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    crc_ok: Option<bool>,
    crc_checked: u64,
    crc_fail: u64,
    frames: u64,
    repeats: u64,
    gaps: u64,
    jumps: u64,
}

fn e2e_dto(track: &crate::bus::E2eView) -> WebE2e {
    WebE2e {
        profile: track.profile,
        data_id: track.data_id,
        counter: track.counter,
        crc: track.crc,
        crc_ok: track.crc_ok,
        crc_checked: track.crc_checked,
        crc_fail: track.crc_fail,
        frames: track.frames,
        repeats: track.repeats,
        gaps: track.gaps,
        jumps: track.jumps,
    }
}

#[derive(Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
struct WebMessage {
    /// Stable identity for expansion and tree keys.
    key: String,
    kind: String,
    addr: String,
    bus: u8,
    bus_name: String,
    id: u32,
    extended: bool,
    name: String,
    count: u64,
    cycle_ms: f64,
    len: u8,
    data: String,
    /// How many DBC/description signals this row carries.
    signal_count: usize,
    /// Decoded children, present only while the row is expanded.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    signals: Vec<WebMessageSignal>,
    /// Counter continuity for an E2E-protected message; absent otherwise.
    #[serde(skip_serializing_if = "Option::is_none")]
    e2e: Option<WebE2e>,
}

#[derive(Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
struct WebLine {
    wall_us: u64,
    kind: &'static str,
    text: String,
}

fn expanded_signal_dto(rows: &[crate::bus::ExpandedSignal]) -> Vec<WebMessageSignal> {
    rows.iter()
        .map(|row| WebMessageSignal {
            value: row.value.clone(),
            name: row.name.clone(),
            raw: row.raw,
            unit: row.unit.clone(),
            label: row.label.clone(),
        })
        .collect()
}

fn eth_message_key(key: &crate::aggregate::EthFlowKey) -> String {
    key.message_key()
}

/// Test and performance hook: the byte size of the state JSON one browser
/// poll receives for this app (empty live-series queue).
#[cfg(test)]
pub(crate) fn state_json_size(app: &App) -> usize {
    let series = VecDeque::new();
    serde_json::to_vec(&build_state(app, &series, 0.0))
        .map(|json| json.len())
        .unwrap_or(0)
}

fn build_state(app: &App, series: &VecDeque<WebSeriesPoint>, fps: f64) -> WebState {
    let snap = &app.snap;
    let mut buses: Vec<WebBus> = snap
        .channels
        .iter()
        .enumerate()
        .map(|(index, ch)| {
            let descriptions: Vec<WebDescription> = ch
                .dbc_paths
                .iter()
                .filter(|path| !path.trim().is_empty())
                .map(|path| WebDescription {
                    kind: "dbc".to_string(),
                    path: path.clone(),
                    cluster: String::new(),
                })
                .chain(app.can_arxml.get(&(index as u8)).into_iter().flatten().map(
                    |(path, cluster)| WebDescription {
                        kind: "arxml".to_string(),
                        path: path.clone(),
                        cluster: cluster.clone(),
                    },
                ))
                .collect();
            let frames = {
                let model = snap
                    .model_frames
                    .keys()
                    .filter(|(ch, _, _)| usize::from(*ch) == index)
                    .count();
                if model > 0 {
                    model
                } else {
                    ch.dbc.as_ref().map_or(0, |db| db.order.len())
                }
            };
            WebBus {
                index: index as u8,
                id: snap.cluster_ids.can.get(index).copied().unwrap_or(0),
                kind: "can".to_string(),
                name: ch.name.clone(),
                frames,
                source: descriptions
                    .first()
                    .map(|description| description.path.clone())
                    .unwrap_or_default(),
                descriptions,
                can: Some(WebCanBus {
                    bitrate_kbps: ch.bitrate_kbps,
                    fd_data_kbps: ch.fd_data_kbps,
                    nodes: ch
                        .dbc
                        .as_ref()
                        .map(|db| {
                            db.nodes
                                .iter()
                                .map(|node| WebNode {
                                    name: node.clone(),
                                    simulated: ch.role_of(node) == crate::app::NodeRole::Simulated,
                                    ecu: ch.node_ecus.get(node).copied(),
                                    entries: snap
                                        .tx
                                        .iter()
                                        .filter(|entry| {
                                            entry.channel == index as u8 && entry.node == *node
                                        })
                                        .count(),
                                })
                                .collect()
                        })
                        .unwrap_or_default(),
                    attached: snap.hw.iter().find(|hw| hw.bus == index as u8).map(|hw| {
                        WebHwAttachment {
                            driver: hw.driver.word().to_string(),
                            adapter: hw.adapter,
                            kbps: hw.kbps,
                            fd: hw.fd,
                            can_tx: hw.can_tx,
                            tx_fail: hw.tx_fail.as_ref().map(|(reason, count)| WebTxFail {
                                reason: reason.clone(),
                                count: *count,
                            }),
                        }
                    }),
                }),
                flexray: None,
                ethernet: None,
            }
        })
        .collect();

    for (bus, cfg) in &app.fr_buses {
        let watch = app.snap.fr_watches.iter().find(|watch| watch.bus == *bus);
        let model = app
            .fr_models
            .get(bus)
            .and_then(|comm| comm.clusters.values().next());
        let descriptions: Vec<WebDescription> = app
            .fr_sources
            .get(bus)
            .into_iter()
            .flatten()
            .map(description_view)
            .collect();
        let frames = model
            .map(|cluster| {
                app.fr_models
                    .get(bus)
                    .map(|comm| comm.frames_in_cluster(cluster.id).count())
                    .unwrap_or(0)
            })
            .filter(|count| *count > 0)
            .unwrap_or(cfg.frames.len());
        buses.push(WebBus {
            index: *bus,
            id: snap.cluster_ids.fr.get(bus).copied().unwrap_or(0),
            kind: "flexray".to_string(),
            name: app.fr_bus_name(*bus),
            frames,
            source: descriptions
                .first()
                .map(|description| description.path.clone())
                .unwrap_or_default(),
            descriptions,
            can: None,
            flexray: Some(WebFrBus {
                cluster: app.fr_cluster_label(*bus),
                attached: watch.is_some(),
                channel: watch.map(|watch| watch.channel_index).unwrap_or(-1),
            }),
            ethernet: None,
        });
    }

    for (bus, cfg) in &app.eth_buses {
        let descriptions: Vec<WebDescription> = app
            .eth_sources
            .get(bus)
            .into_iter()
            .flatten()
            .map(description_view)
            .collect();
        let clusters = app
            .eth_models
            .get(bus)
            .map(|comm| comm.clusters.len())
            .unwrap_or(cfg.clusters.len());
        let frames = app
            .eth_models
            .get(bus)
            .map(|comm| {
                comm.frames
                    .values()
                    .filter(|frame| {
                        matches!(frame.address, crate::model::FrameAddress::Ethernet { .. })
                    })
                    .count()
            })
            .filter(|count| *count > 0)
            .unwrap_or(cfg.frames.len());
        buses.push(WebBus {
            index: *bus,
            id: snap.cluster_ids.eth.get(bus).copied().unwrap_or(0),
            kind: "ethernet".to_string(),
            name: app.eth_bus_label(*bus),
            frames,
            source: descriptions
                .first()
                .map(|description| description.path.clone())
                .unwrap_or_default(),
            descriptions,
            can: None,
            flexray: None,
            ethernet: Some(WebEthBus {
                clusters,
                pdus: app.eth_db(*bus).map(|db| db.pdus.len()).unwrap_or(0),
                endpoints: app.eth_db(*bus).map(|db| db.endpoints.len()).unwrap_or(0),
                pdu_names: app
                    .eth_db(*bus)
                    .map(|db| {
                        db.pdus
                            .iter()
                            .map(|pdu| {
                                if pdu.signals.is_empty() {
                                    pdu.name.clone()
                                } else {
                                    format!("{} · {} signals", pdu.name, pdu.signals.len())
                                }
                            })
                            .collect()
                    })
                    .unwrap_or_default(),
                endpoint_list: app
                    .eth_db(*bus)
                    .map(|db| {
                        db.endpoints
                            .iter()
                            .map(|endpoint| WebEthEndpoint {
                                name: endpoint.name.clone(),
                                ip: endpoint.ip.clone(),
                                port: endpoint.port,
                                protocol: endpoint.protocol.clone(),
                                connector: endpoint.connector.clone(),
                                pdu: endpoint.pdu.clone(),
                            })
                            .collect()
                    })
                    .unwrap_or_default(),
                attached: snap.eth_attached.get(bus).cloned(),
            }),
        });
    }
    buses.sort_by_key(|bus| bus.id);

    let mut tx: Vec<WebTx> = snap
        .tx
        .iter()
        .map(|entry| WebTx {
            channel: entry.channel,
            bus: snap
                .channels
                .get(entry.channel as usize)
                .map(|channel| channel.name.clone())
                .unwrap_or_else(|| format!("CAN{}", entry.channel + 1)),
            id: entry.id,
            name: entry.name.clone(),
            node: entry.node.clone(),
            active: entry.active,
            gate_open: entry.gate_open,
            fd: entry.fd,
            brs: entry.brs,
            cycle_us: entry.cycle_us,
            data_text: entry.data_text.clone(),
            sent_text: entry.sent_text.clone(),
            muted: entry.muted,
            srcs: entry
                .srcs
                .iter()
                .map(|src| WebSrc {
                    name: src.name.clone(),
                    kind: src.kind.label().to_string(),
                    lo: src.lo,
                    hi: src.hi,
                    period_us: src.period_us,
                    phase_us: src.phase_us,
                    seq: src.seq.clone(),
                    seed: src.seed,
                    redraw_us: src.redraw_us,
                })
                .collect(),
        })
        .collect();
    tx.sort_by_key(|entry| (entry.channel, entry.id));

    let blocks: Vec<WebReplayBlock> = snap
        .blocks
        .iter()
        .map(|block| WebReplayBlock {
            id: block.id,
            name: block.name.clone(),
            channel: block.channel,
            path: block.path.clone(),
            node_filter: block.node_filter.clone(),
            ids: block.ids.clone(),
            enabled: block.enabled,
            frames: block.frames,
            error: block.last_error.clone(),
        })
        .collect();
    let mut fr_tx: Vec<WebFrTx> = snap
        .fr_tx
        .iter()
        .map(|entry| WebFrTx {
            bus: entry.bus,
            bus_name: app.fr_bus_name(entry.bus),
            slot: entry.slot,
            name: entry.name.clone(),
            node: entry.node.clone(),
            active: entry.active,
            cycle_us: entry.cycle_us,
            data_text: entry.data_text.clone(),
            sent_text: entry.sent_text.clone(),
            muted: entry.muted,
            undescribed: entry.undescribed,
        })
        .collect();
    fr_tx.sort_by_key(|entry| (entry.bus, entry.slot));

    let total_frames: u64 = snap
        .aggs
        .iter()
        .map(|agg| agg.count)
        .chain(snap.fr_aggs.iter().map(|agg| agg.count))
        .chain(snap.eth_aggs.iter().map(|agg| agg.count))
        .sum();
    let share_text = |count: u64| {
        format!(
            "{:.1}%",
            if total_frames > 0 {
                count as f64 / total_frames as f64 * 100.0
            } else {
                0.0
            }
        )
    };
    let ms = |value: f64, count: u64| {
        if count >= 2 {
            format!("{:.2}", value / 1000.0)
        } else {
            "-".to_string()
        }
    };
    let mut stats: Vec<WebStatsRow> = snap
        .aggs
        .iter()
        .map(|agg| {
            let name = snap
                .model_frames
                .get(&(agg.channel, agg.id, agg.extended))
                .map(|frame| frame.name.as_str())
                .or_else(|| {
                    snap.channels
                        .get(agg.channel as usize)
                        .and_then(|channel| channel.dbc.as_ref())
                        .and_then(|db| db.message_name(agg.id))
                })
                .unwrap_or("-");
            let id = if agg.extended {
                format!("{:08X}x", agg.id)
            } else {
                format!("{:03X}", agg.id)
            };
            WebStatsRow {
                label: format!("{id}  {name}"),
                bus: snap
                    .channels
                    .get(agg.channel as usize)
                    .map(|channel| channel.name.clone())
                    .unwrap_or_else(|| format!("CAN{}", agg.channel + 1)),
                count: agg.count,
                min_ms: ms(agg.min_us, agg.count),
                avg_ms: ms(agg.cycle_us, agg.count),
                max_ms: ms(agg.max_us, agg.count),
                jitter_ms: ms(agg.jitter_us, agg.count),
                share: share_text(agg.count),
                len: agg.len,
            }
        })
        .collect();
    for agg in &snap.fr_aggs {
        let name = App::fr_frame_name(agg);
        stats.push(WebStatsRow {
            label: if name.is_empty() {
                format!("slot {}", agg.slot)
            } else {
                format!("slot {}  {name}", agg.slot)
            },
            bus: app.fr_bus_name(agg.bus),
            count: agg.count,
            min_ms: ms(agg.min_us, agg.count),
            avg_ms: ms(agg.cycle_us, agg.count),
            max_ms: ms(agg.max_us, agg.count),
            jitter_ms: ms(agg.jitter_us, agg.count),
            share: share_text(agg.count),
            len: agg.payload.len().min(u8::MAX as usize) as u8,
        });
    }
    for agg in &snap.eth_aggs {
        let vlan = agg
            .key
            .vlan
            .map(|vlan| format!("VLAN {vlan}  "))
            .unwrap_or_default();
        stats.push(WebStatsRow {
            label: format!("{vlan}{}", app.eth_flow_name(&agg.key)),
            bus: app.eth_bus_label(agg.key.bus),
            count: agg.count,
            min_ms: ms(agg.min_us, agg.count),
            avg_ms: ms(agg.cycle_us, agg.count),
            max_ms: ms(agg.max_us, agg.count),
            jitter_ms: ms(agg.jitter_us, agg.count),
            share: share_text(agg.count),
            len: agg.key.len.min(u8::MAX as usize) as u8,
        });
    }

    let triggers: Vec<WebTrigger> = snap
        .triggers
        .iter()
        .enumerate()
        .map(|(index, trigger)| {
            let (send_frame, send_data) = match &trigger.action {
                TriggerAction::SendEth { frame, data, .. } => {
                    (frame.clone(), crate::generator::hex_of(data))
                }
                _ => (String::new(), String::new()),
            };
            let (
                someip_method,
                someip_client,
                someip_session,
                someip_type,
                someip_values,
                someip_transport,
                someip_host,
                someip_port,
            ) = match &trigger.action {
                TriggerAction::SendSomeIp {
                    method,
                    client,
                    session,
                    message_type,
                    values,
                    transport,
                    host,
                    port,
                    ..
                } => (
                    *method,
                    *client,
                    *session,
                    *message_type,
                    values.clone(),
                    transport.clone(),
                    host.clone(),
                    *port,
                ),
                _ => (
                    0,
                    0,
                    0,
                    0,
                    std::collections::BTreeMap::new(),
                    "udp".to_string(),
                    String::new(),
                    30490,
                ),
            };
            let mut view = WebTrigger {
                summary: app.trigger_summary(index),
                action: trigger.action.label(),
                kind: trigger_kind(&trigger.cond).to_string(),
                enabled: trigger.enabled,
                fired: trigger.fired,
                level: trigger.level,
                last_fire_us: trigger.last_fire_t_us,
                action_kind: trigger.action.kind().to_string(),
                cooldown_ms: (trigger.cooldown_us / 1_000).min(u64::from(u32::MAX)) as u32,
                max_fires: trigger.max_fires,
                pre_frames: trigger.pre_frames.map(|n| n.min(u32::MAX as usize) as u32),
                post_frames: trigger.post_frames,
                send_frame,
                send_data,
                someip_method,
                someip_client,
                someip_session,
                someip_type,
                someip_values,
                someip_transport,
                someip_host,
                someip_port,
                ..Default::default()
            };
            match &trigger.action {
                TriggerAction::SetSysVar { key, value } => {
                    view.sys_var_key.clone_from(key);
                    view.sys_var_value = *value;
                }
                TriggerAction::RunScript { name, value } => {
                    view.script_name.clone_from(name);
                    view.script_value = *value;
                }
                _ => {}
            }
            match &trigger.cond {
                TriggerCond::SignalCross {
                    ch,
                    id,
                    ext,
                    signal,
                    threshold,
                    rising,
                } => {
                    view.ch = *ch;
                    view.id = *id;
                    view.ext = *ext;
                    view.signal = signal.clone();
                    view.threshold = *threshold;
                    view.rising = *rising;
                }
                TriggerCond::IdPresent { ch, id } | TriggerCond::CycleTimeout { ch, id } => {
                    view.ch = *ch;
                    view.id = *id;
                }
                TriggerCond::ErrorFrame { ch } => view.ch = *ch,
                TriggerCond::FrFramePresent { bus, slot } => {
                    view.bus = *bus;
                    view.slot = *slot;
                }
                TriggerCond::FrSignalCross {
                    bus,
                    slot,
                    signal,
                    threshold,
                    rising,
                } => {
                    view.bus = *bus;
                    view.slot = *slot;
                    view.signal = signal.clone();
                    view.threshold = *threshold;
                    view.rising = *rising;
                }
                TriggerCond::SysVar {
                    key,
                    threshold,
                    rising,
                } => {
                    view.var_key = key.clone();
                    view.threshold = *threshold;
                    view.rising = *rising;
                }
                TriggerCond::DerivedSignal {
                    id,
                    threshold,
                    rising,
                } => {
                    view.derived_id = id.clone();
                    view.signal = app
                        .derived_defs
                        .iter()
                        .find(|def| &def.id == id)
                        .map(|def| def.name.clone())
                        .unwrap_or_else(|| id.clone());
                    view.threshold = *threshold;
                    view.rising = *rising;
                }
                TriggerCond::PayloadMatch {
                    ch,
                    id,
                    ext,
                    offset: at,
                    mask: bits,
                    value: wanted,
                } => {
                    view.ch = *ch;
                    view.id = *id;
                    view.ext = *ext;
                    view.offset = *at;
                    view.mask = *bits;
                    view.value = *wanted;
                }
                TriggerCond::EthFlow {
                    bus,
                    ethertype,
                    vlan,
                } => {
                    view.bus = *bus;
                    view.id = u32::from(*ethertype);
                    view.vlan = *vlan;
                }
                TriggerCond::FrTimeout { bus, slot } => {
                    view.bus = *bus;
                    view.slot = *slot;
                }
                TriggerCond::E2eIssue { ch, id, ext } => {
                    view.ch = *ch;
                    view.id = *id;
                    view.ext = *ext;
                }
                TriggerCond::SomeIp {
                    bus,
                    service,
                    method,
                    message_type,
                } => {
                    view.bus = *bus;
                    view.id = u32::from(*service);
                    view.cond_someip_method = *method;
                    view.cond_someip_type = *message_type;
                }
            }
            if let TriggerAction::Send { ch, id } = trigger.action {
                view.send_ch = ch;
                view.send_id = id;
            }
            if let TriggerAction::SendFr { bus, slot } = trigger.action {
                view.send_ch = bus;
                view.send_id = u32::from(slot);
            }
            view
        })
        .collect();

    let replay_map = {
        use crate::replay_map::LogBusKind;
        let map = &snap.replay_map;
        WebReplayMap {
            can: replay_rows(&snap.replay_census_can, map, LogBusKind::Can, |raw| {
                app.replay_bus(LogBusKind::Can, raw)
                    .filter(|bus| (*bus as usize) < snap.channels.len())
            }),
            fr: replay_rows(&snap.replay_census_fr, map, LogBusKind::Fr, |raw| {
                app.replay_bus(LogBusKind::Fr, raw)
            }),
            eth: replay_rows(&snap.replay_census_eth, map, LogBusKind::Eth, |raw| {
                app.replay_bus(LogBusKind::Eth, raw)
            }),
        }
    };

    let bus_stats: Vec<WebBusStats> = snap
        .bus_loads
        .iter()
        .enumerate()
        .filter_map(|(index, load)| {
            let channel = snap.channels.get(index)?;
            Some(WebBusStats {
                name: channel.name.clone(),
                kbps: channel.bitrate_kbps,
                load_now: load.load(),
                load_history: load.history().map(|(_, value)| value).collect(),
                rows: bus_stat_rows(load),
            })
        })
        .collect();

    let spec_rows: Vec<WebSpecRow> = app
        .spec_rows()
        .into_iter()
        .map(|row| WebSpecRow {
            name: row.name,
            addr: row.addr,
            bus: row.bus,
            rule: row.kind.label().to_string(),
            declared: crate::spec::qty(row.kind, row.declared),
            measured: crate::spec::qty(row.kind, row.measured),
            count: row.count,
            first: format!("{:.3} s", row.first_t_us as f64 / 1e6),
            last: format!("{:.3} s", row.last_t_us as f64 / 1e6),
        })
        .collect();
    let spec_periodic: usize = snap
        .channels
        .iter()
        .filter_map(|channel| channel.dbc.as_ref())
        .map(|db| {
            db.messages
                .values()
                .filter(|message| message.cycle_us.is_some_and(|cycle| cycle > 0))
                .count()
        })
        .sum();

    let sysvars: Vec<WebSysVar> = snap
        .sysvars
        .iter()
        .map(|v| WebSysVar {
            namespace: v.def.namespace.clone(),
            name: v.def.name.clone(),
            value: v.value,
            init: v.def.init,
            min: v.def.min,
            max: v.def.max,
            unit: v.def.unit.clone(),
            comment: v.def.comment.clone(),
            label_sets: v.def.display_label_sets(),
            band_set_id: v.def.band_set_id.clone(),
        })
        .collect();

    // The value table mirrors the first Data window model. Missing values
    // keep the row but print dashes, so a freshly added signal does not
    // flicker in and out before its first frame.
    let data: Vec<WebDataRow> = app
        .data_windows
        .first()
        .map(|window| {
            window
                .signals
                .iter()
                .map(|signal| {
                    let RowAddress {
                        kind,
                        ch,
                        id,
                        ext,
                        frame,
                    } = row_address(app, &signal.key);
                    let name = signal.key.name().to_string();
                    let sub = signal_sub(app, &signal.key);
                    let (value, unit, raw, min, avg, max, frac) = match sub {
                        // An empty subscription has min = inf / max = -inf;
                        // a fresh row shows dashes until its first frame.
                        Some(sub) if sub.min <= sub.max => {
                            let frac = match app.declared_range(&signal.key) {
                                Some((lo, hi)) if hi > lo => {
                                    ((sub.latest - lo) / (hi - lo)).clamp(0.0, 1.0)
                                }
                                _ if sub.max > sub.min => {
                                    ((sub.latest - sub.min) / (sub.max - sub.min)).clamp(0.0, 1.0)
                                }
                                _ => 0.0,
                            };
                            (
                                sub.label.clone().unwrap_or_else(|| {
                                    crate::dbc::fmt_decoded(&sub.type_tag, sub.latest)
                                }),
                                sub.unit.clone(),
                                if kind == "derived" {
                                    "-".to_string()
                                } else {
                                    sub.last_raw.to_string()
                                },
                                crate::dbc::fmt_decoded(&sub.type_tag, sub.min),
                                crate::dbc::fmt_decoded(&sub.type_tag, sub.avg),
                                crate::dbc::fmt_decoded(&sub.type_tag, sub.max),
                                frac,
                            )
                        }
                        _ => (
                            "-".to_string(),
                            String::new(),
                            "-".to_string(),
                            "-".to_string(),
                            "-".to_string(),
                            "-".to_string(),
                            0.0,
                        ),
                    };
                    WebDataRow {
                        kind: kind.to_string(),
                        ch,
                        id,
                        ext,
                        frame,
                        name,
                        key: series_key(&signal.key),
                        bus: signal_bus(app, &signal.key),
                        path: signal_path(app, &signal.key).unwrap_or_default(),
                        value,
                        unit,
                        raw,
                        min,
                        avg,
                        max,
                        frac,
                        visible: signal.visible,
                    }
                })
                .collect()
        })
        .unwrap_or_default();

    let state_window_s = app
        .state_trackers
        .first()
        .map(|window| window.time_window_s)
        .unwrap_or(20.0);
    let state_rows: Vec<WebStateRow> = app
        .state_trackers
        .first()
        .map(|window| {
            let tw = window.time_window_s.max(0.1);
            let right_s = app.plot_now_s();
            let left_s = (right_s - tw).max(0.0);
            let lo = (left_s * 1e6) as u64;
            let hi = (right_s * 1e6) as u64;
            window
                .signals
                .iter()
                .map(|signal| {
                    let RowAddress {
                        kind,
                        ch,
                        id,
                        ext,
                        frame,
                    } = row_address(app, &signal.key);
                    let name = signal.key.name().to_string();
                    // A derived channel carries its bands on the definition
                    // (that is what the editor persists), not per window.
                    let derived_rule = match &signal.key {
                        SigKey::Derived { id, .. } if !window.rules.contains_key(&signal.key) => {
                            app.derived_rule(id)
                        }
                        _ => None,
                    };
                    let rule = window.rules.get(&signal.key).or(derived_rule.as_ref());
                    let (current, bands) = match signal_sub(app, &signal.key) {
                        Some(sub) => {
                            let held = sub.history.at(lo);
                            let samples: Vec<(u64, f64)> =
                                sub.history.range(lo, hi).copied().collect();
                            match rule {
                                Some(rule) => state_bands_custom(held, &samples, lo, hi, rule),
                                None => state_bands_from(held, &samples, lo, hi),
                            }
                        }
                        None => ("-".to_string(), Vec::new()),
                    };
                    WebStateRow {
                        kind: kind.to_string(),
                        ch,
                        id,
                        ext,
                        frame,
                        name,
                        key: series_key(&signal.key),
                        bus: signal_bus(app, &signal.key),
                        current,
                        bands,
                        rule: rule.map(|rule| WebStateRule {
                            bands: rule.bands.clone(),
                            band_set_id: rule.band_set_id.clone(),
                        }),
                    }
                })
                .collect()
        })
        .unwrap_or_default();

    let scripts: Vec<WebScriptInfo> = snap
        .nodes
        .iter()
        .map(|node| WebScriptInfo {
            id: node.id,
            name: node.name.clone(),
            channel: node.channel,
            attached_bus: node.attached.as_ref().map(|(bus, _)| *bus),
            attached_node: node.attached.as_ref().map(|(_, name)| name.clone()),
            enabled: node.enabled,
            running: node.running,
            errored: node.errored,
            file_path: node.file_path.clone().unwrap_or_default(),
            log_len: node.log.len(),
            last_log: node.log.last().cloned().unwrap_or_default(),
            panel_actions: node.panel_actions.clone(),
        })
        .collect();

    let channels: Vec<WebChannel> = snap
        .channels
        .iter()
        .map(|ch| WebChannel {
            name: ch.name.clone(),
            bitrate_kbps: ch.bitrate_kbps,
            fd_data_kbps: ch.fd_data_kbps,
        })
        .collect();
    let bus_name = |bus: u8| {
        channels
            .get(bus as usize)
            .map(|c| c.name.clone())
            .unwrap_or_else(|| format!("CAN{}", bus + 1))
    };
    let db = |bus: u8| snap.channels.get(bus as usize).and_then(|c| c.dbc.as_ref());

    let monitor: Vec<WebMonitorRow> = app
        .monitor_view
        .rows
        .iter()
        .map(|row| {
            let RowAddress {
                kind,
                ch,
                id,
                ext,
                frame,
            } = row_address(app, &row.key);
            let (value, has_value, unit) = signal_sub(app, &row.key)
                .map(|sub| (sub.latest, sub.min <= sub.max, sub.unit.clone()))
                .unwrap_or((0.0, false, String::new()));
            WebMonitorRow {
                kind: kind.to_string(),
                ch,
                id,
                ext,
                frame,
                name: row.key.name().to_string(),
                key: series_key(&row.key),
                label: row.label.clone(),
                bus: signal_bus(app, &row.key),
                path: signal_path(app, &row.key).unwrap_or_default(),
                value,
                has_value,
                unit,
                digits: row.digits,
                bands: row
                    .band_set_id
                    .as_ref()
                    .and_then(|id| app.band_sets.iter().find(|set| &set.id == id))
                    .map(|set| set.bands.clone())
                    .unwrap_or_else(|| row.bands.clone()),
                band_set_id: row.band_set_id.clone(),
                enum_label: signal_sub(app, &row.key).and_then(|sub| sub.label.clone()),
            }
        })
        .collect();

    let mut messages: Vec<WebMessage> = snap
        .aggs
        .iter()
        .map(|m| {
            let addr = if m.extended {
                format!("0x{:08X}", m.id)
            } else {
                format!("0x{:03X}", m.id)
            };
            let key = format!("can:{}:{}:{}", m.channel, m.id, m.extended);
            let channel_db = db(m.channel);
            let model_frame = snap.model_frames.get(&(m.channel, m.id, m.extended));
            let signal_count = if m.flags.contains(crate::can::frame::FrameFlags::ERROR)
                || m.flags.contains(crate::can::frame::FrameFlags::RTR)
            {
                0
            } else {
                model_frame
                    .map(|frame| frame.signal_count)
                    .unwrap_or_else(|| {
                        channel_db
                            .and_then(|d| d.messages.get(&(m.id, m.extended)))
                            .map(|message| message.signals.len())
                            .unwrap_or(0)
                    })
            };
            let signals = if app.expanded_messages.contains(&key) {
                snap.expanded_signals
                    .get(&key)
                    .map(|rows| expanded_signal_dto(rows))
                    .unwrap_or_default()
            } else {
                Vec::new()
            };
            WebMessage {
                key,
                kind: "can".to_string(),
                addr,
                bus: m.channel,
                bus_name: bus_name(m.channel),
                id: m.id,
                extended: m.extended,
                name: model_frame
                    .map(|frame| frame.name.clone())
                    .or_else(|| {
                        channel_db
                            .and_then(|d| d.message_name(m.id))
                            .map(str::to_string)
                    })
                    .unwrap_or_default(),
                count: m.count,
                cycle_ms: m.cycle_us / 1000.0,
                len: m.len,
                data: hex(m.payload()),
                signal_count,
                signals,
                e2e: snap.e2e.get(&(m.channel, m.id, m.extended)).map(e2e_dto),
            }
        })
        .collect();
    for m in &snap.fr_aggs {
        let name = App::fr_frame_name(m).to_string();
        let occupant = match &m.occupant {
            crate::aggregate::FrOccupant::Frame(index) => format!("f{index}"),
            crate::aggregate::FrOccupant::Logged(name) => format!("l:{name}"),
            crate::aggregate::FrOccupant::Unknown => "u".to_string(),
        };
        let key = format!("fr:{}:{}:{}", m.bus, m.slot, occupant);
        let frame_ix = app.fr_described_frame(m);
        let frame_db = app.fr_db(m.bus);
        let signal_count = snap
            .fr_model_frames
            .get(&(m.bus, m.slot))
            .map(|frame| frame.signal_count)
            .unwrap_or_else(|| {
                frame_ix
                    .and_then(|ix| frame_db.map(|db| db.signal_names(ix).len()))
                    .unwrap_or(0)
            });
        let signals = if app.expanded_messages.contains(&key) {
            snap.expanded_signals
                .get(&key)
                .map(|rows| expanded_signal_dto(rows))
                .unwrap_or_default()
        } else {
            Vec::new()
        };
        messages.push(WebMessage {
            key,
            kind: "fr".to_string(),
            addr: format!("slot {}", m.slot),
            bus: m.bus,
            bus_name: app.fr_bus_name(m.bus),
            id: m.slot as u32,
            extended: false,
            name: if name.is_empty() {
                format!("slot {}", m.slot)
            } else {
                name
            },
            count: m.count,
            cycle_ms: m.cycle_us / 1000.0,
            len: m.payload.len().min(u8::MAX as usize) as u8,
            data: hex(&m.payload),
            signal_count,
            signals,
            e2e: snap.fr_e2e.get(&(m.bus, m.slot)).map(e2e_dto),
        });
    }
    for a in &snap.eth_aggs {
        let vlan = a
            .key
            .vlan
            .map(|vlan| format!("VLAN {vlan}  "))
            .unwrap_or_default();
        let key = eth_message_key(&a.key);
        let flow = a.key.flow();
        let (signal_count, signals) = app
            .eth_buses
            .get(&a.key.bus)
            .map(|config| {
                let count = match config.match_flow(&flow, app.eth_length_includes_header) {
                    crate::eth::db::EthMatch::Unique(index) => config
                        .frames
                        .get(index)
                        .map(|frame| {
                            snap.eth_model_frames
                                .get(&frame.name)
                                .map(|model| model.signal_count)
                                .unwrap_or_else(|| {
                                    frame.pdus.iter().map(|pdu| pdu.signals.len()).sum()
                                })
                        })
                        .unwrap_or(0),
                    _ => 0,
                };
                let signals = if app.expanded_messages.contains(&key) {
                    snap.expanded_signals
                        .get(&key)
                        .map(|rows| expanded_signal_dto(rows))
                        .unwrap_or_default()
                } else {
                    Vec::new()
                };
                (count, signals)
            })
            .unwrap_or_default();
        messages.push(WebMessage {
            key,
            kind: "eth".to_string(),
            addr: format!("0x{:04X}", a.key.ethertype),
            bus: a.key.bus,
            bus_name: app.eth_bus_label(a.key.bus),
            id: a.key.ethertype as u32,
            extended: false,
            name: format!("{vlan}{}", app.eth_flow_name(&a.key)),
            count: a.count,
            cycle_ms: a.cycle_us / 1000.0,
            len: a.key.len.min(u8::MAX as usize) as u8,
            data: String::new(),
            signal_count,
            signals,
            e2e: app
                .eth_buses
                .get(&a.key.bus)
                .and_then(|config| {
                    let flow = a.key.flow();
                    match config.match_flow(&flow, app.eth_length_includes_header) {
                        crate::eth::db::EthMatch::Unique(index) => {
                            Some(config.frames[index].name.clone())
                        }
                        _ => None,
                    }
                })
                .and_then(|frame| snap.eth_e2e.get(&frame).map(e2e_dto)),
        });
    }
    messages.sort_by_key(|m| {
        let order = match m.kind.as_str() {
            "can" => 0,
            "fr" => 1,
            _ => 2,
        };
        (order, m.bus, m.id)
    });

    let write: Vec<WebLine> = snap
        .write
        .iter()
        .rev()
        .take(6)
        .rev()
        .map(|line| WebLine {
            wall_us: line.wall_us,
            kind: match line.kind {
                crate::bus::WriteKind::Script => "script",
                crate::bus::WriteKind::Info => "info",
                crate::bus::WriteKind::Warning => "warn",
                crate::bus::WriteKind::Error => "error",
            },
            text: line.text.clone(),
        })
        .collect();

    WebState {
        measuring: snap.measuring,
        trace_paused: snap.trace_paused,
        recording: snap.recording,
        record_file: snap.last_record.clone(),
        record_eth_file: snap.last_eth_record.clone(),
        record_filter: app.record_filter_text.clone(),
        markers: snap
            .markers
            .iter()
            .rev()
            .take(500)
            .rev()
            .map(|t_us| *t_us as f64 / 1e6)
            .collect(),
        mode: match snap.mode {
            Mode::Virtual => "Simulation".to_string(),
            Mode::Replay => "Replay".to_string(),
        },
        frames: snap.frame_counter,
        fps,
        trace_len: snap.trace.len() + snap.fr_trace.len() + snap.eth_trace.len(),
        sim_time_us: snap.sim_t_us,
        status: app.status.clone(),
        project: app
            .project_path
            .as_ref()
            .and_then(|path| path.file_name())
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_default(),
        project_path: app
            .project_path
            .as_ref()
            .map(|path| path.to_string_lossy().into_owned())
            .unwrap_or_default(),
        dirty: app.is_dirty(),
        log: std::path::Path::new(&app.log_path)
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_default(),
        log_info: app.log_info.clone().unwrap_or_default(),
        run_mode: match snap.run_mode {
            Mode::Virtual => "Simulation".to_string(),
            Mode::Replay => "Replay".to_string(),
        },
        replay_pos_us: snap.replay.map(|(pos, _)| pos),
        replay_total_us: snap.replay.map(|(_, total)| total),
        replay_speed: if app.replay_speed.is_finite() {
            app.replay_speed
        } else {
            1e9
        },
        recent_projects: app.recent_projects.clone(),
        recent_logs: app.recent_log.clone(),
        replay_folders: app.replay_folders.clone(),
        replay_files: app.replay_files.clone(),
        replay_recordings: app
            .replay_scan
            .iter()
            .map(|file| WebReplaySource {
                path: file.path.clone(),
                name: file.name.clone(),
                folder: file.folder.clone(),
                size: file.size,
                modified_ms: file.modified_ms,
            })
            .collect(),
        workspace: app.web_workspace.clone(),
        panel_pages: app.panel_pages.clone(),
        someip: snap
            .someip
            .iter()
            .map(|event| {
                let mut event = event.clone();
                if event.sd.is_none() && event.decoded.is_empty() {
                    match app.someip_services.service(event.service) {
                        None => {
                            event.decode_error =
                                Some(format!("no ARXML service 0x{:04X}", event.service));
                        }
                        Some(service) => {
                            let fields = service
                                .events
                                .iter()
                                .find(|entry| entry.id == event.method)
                                .map(|entry| entry.fields.as_slice())
                                .or_else(|| {
                                    service
                                        .methods
                                        .iter()
                                        .find(|entry| entry.id == event.method)
                                        .map(|entry| {
                                            if event.message_type == 0x80 {
                                                entry.output.as_slice()
                                            } else {
                                                entry.input.as_slice()
                                            }
                                        })
                                });
                            match fields {
                                None => {
                                    event.decode_error = Some(format!(
                                        "no ARXML method/event 0x{:04X}",
                                        event.method
                                    ));
                                }
                                Some(fields) => match crate::eth::arxml_service::decode_values(
                                    fields,
                                    &event.payload,
                                ) {
                                    Some(values) => event.decoded = values,
                                    None => {
                                        event.decode_error =
                                            Some("payload shorter than ARXML layout".to_string());
                                    }
                                },
                            }
                        }
                    }
                }
                event
            })
            .collect(),
        someip_subscriptions: snap.someip_subscriptions.clone(),
        someip_issues: app
            .someip_services
            .services
            .iter()
            .flat_map(|service| service.issues.iter().cloned())
            .collect(),
        someip_services: app
            .someip_services
            .services
            .iter()
            .flat_map(|service| {
                let buses = if service.buses.is_empty() {
                    vec![0]
                } else {
                    service.buses.clone()
                };
                buses.into_iter().map(|bus| WebSomeIpService {
                    id: service.id,
                    name: service.name.clone(),
                    bus,
                    bus_name: app.eth_bus_label(bus),
                    major: service.major,
                    source: app
                        .eth_sources
                        .get(&bus)
                        .and_then(|sources| sources.first())
                        .map(|source| source.path().unwrap_or("").to_string())
                        .unwrap_or_default(),
                    methods: service
                        .methods
                        .iter()
                        .map(|method| WebSomeIpMember {
                            id: method.id,
                            name: method.name.clone(),
                            fields: method
                                .input
                                .iter()
                                .chain(method.output.iter())
                                .flat_map(|field| someip_signal_names(&field.name, &field.ty))
                                .collect(),
                            field_defs: method
                                .input
                                .iter()
                                .chain(method.output.iter())
                                .flat_map(|field| someip_field_defs(&field.name, field))
                                .collect(),
                            input_field_defs: method
                                .input
                                .iter()
                                .flat_map(|field| someip_field_defs(&field.name, field))
                                .collect(),
                            output_field_defs: method
                                .output
                                .iter()
                                .flat_map(|field| someip_field_defs(&field.name, field))
                                .collect(),
                        })
                        .collect(),
                    events: service
                        .events
                        .iter()
                        .map(|event| WebSomeIpMember {
                            id: event.id,
                            name: event.name.clone(),
                            fields: event
                                .fields
                                .iter()
                                .flat_map(|field| someip_signal_names(&field.name, &field.ty))
                                .collect(),
                            field_defs: event
                                .fields
                                .iter()
                                .flat_map(|field| someip_field_defs(&field.name, field))
                                .collect(),
                            input_field_defs: event
                                .fields
                                .iter()
                                .flat_map(|field| someip_field_defs(&field.name, field))
                                .collect(),
                            output_field_defs: event
                                .fields
                                .iter()
                                .flat_map(|field| someip_field_defs(&field.name, field))
                                .collect(),
                        })
                        .collect(),
                    instances: service
                        .instances
                        .iter()
                        .map(|instance| WebSomeIpInstance {
                            id: instance.instance,
                            major: instance.major,
                            minor: instance.minor,
                            eventgroups: instance.eventgroups.clone(),
                            endpoint: instance.endpoint.clone(),
                        })
                        .collect(),
                })
            })
            .collect(),
        workspace_rev: app.workspace_rev,
        reference: app.reference.clone(),
        reference_rev: app.reference_rev,
        derived: app
            .derived_defs
            .iter()
            .map(|def| WebDerivedDef {
                id: def.id.clone(),
                name: def.name.clone(),
                unit: def.unit.clone(),
                expression: def.expression.clone(),
                band_set_id: def.band_set_id.clone(),
                bands: def.state_bands(),
                sources: def.sources.clone(),
                error: app
                    .snap
                    .derived_errors
                    .get(&def.id)
                    .cloned()
                    .or_else(|| crate::derived::parse(&def.expression).err()),
            })
            .collect(),
        replay_map,
        buses,
        arxml_scan: app.arxml_scan.as_ref().map(|scan| WebArxmlScan {
            path: scan.path.clone(),
            clusters: scan
                .clusters
                .iter()
                .map(|(kind, name)| WebArxmlCluster {
                    kind: kind.clone(),
                    name: name.clone(),
                })
                .collect(),
        }),
        real_bus: snap.real_bus,
        hw_refresh: snap.hw_refresh,
        tx,
        fr_tx,
        blocks,
        triggers,
        trigger_fires: snap
            .trigger_fires
            .iter()
            .map(|fire| WebTriggerFire {
                index: fire.index,
                at_us: fire.at_us,
                text: fire.text.clone(),
                action: fire.action.clone(),
                result: fire.result.clone(),
                ok: fire.ok,
            })
            .collect(),
        trigger_limits: WebTriggerLimits {
            pre_frames: app.limits.pre_frames,
            post_frames: app.limits.post_frames,
            marker_cap: app.limits.marker_cap,
        },
        sysvars,
        band_sets: app.band_sets.clone(),
        monitor,
        monitor_columns: app.monitor_view.columns,
        data,
        state_rows,
        state_window_s,
        scripts,
        stats,
        bus_stats,
        spec_rows,
        spec_tolerance: app.spec_tol_pct,
        spec_grace: app.spec_grace,
        spec_periodic,
        channels,
        messages,
        write,
        series: series.iter().cloned().collect(),
    }
}

fn hex(bytes: &[u8]) -> String {
    use std::fmt::Write as _;
    let mut out = String::with_capacity(bytes.len() * 3);
    for (i, byte) in bytes.iter().enumerate() {
        if i > 0 {
            out.push(' ');
        }
        let _ = write!(out, "{byte:02X}");
    }
    out
}

