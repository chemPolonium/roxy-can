fn pct_text(v: f64) -> String {
    format!("{:.2}", v * 100.0)
}

fn opt_pct_text(v: Option<f64>) -> String {
    v.map(pct_text).unwrap_or_else(|| "-".to_string())
}

fn num_text(v: f64) -> String {
    format!("{v:.0}")
}

fn opt_num_text(v: Option<f64>) -> String {
    v.map(num_text).unwrap_or_else(|| "-".to_string())
}

fn us_ms_text(us: u64) -> String {
    format!("{:.3}", us as f64 / 1000.0)
}

fn opt_ms_text(us: Option<u64>) -> String {
    us.map(us_ms_text).unwrap_or_else(|| "-".to_string())
}

fn opt_ms_f_text(us: Option<f64>) -> String {
    us.map(|v| format!("{:.3}", v / 1000.0))
        .unwrap_or_else(|| "-".to_string())
}

fn opt_num_u_text(v: Option<u64>) -> String {
    v.map(|x| x.to_string()).unwrap_or_else(|| "-".to_string())
}

fn class_stat_rows(
    rows: &mut Vec<WebBusStatRow>,
    load: &crate::load::BusLoad,
    label: &str,
    class: crate::load::FrameClass,
) {
    let (min, max, avg) = load.rate_stats(class);
    rows.push(WebBusStatRow {
        label: format!("{label} [n/s]"),
        current: num_text(load.class_rate(class)),
        min: opt_num_text(min),
        max: opt_num_text(max),
        avg: opt_num_text(avg),
    });
    rows.push(WebBusStatRow {
        label: format!("{label} [total]"),
        current: load.class_total(class).to_string(),
        min: "-".to_string(),
        max: "-".to_string(),
        avg: "-".to_string(),
    });
}

/// The CAN section of Bus Statistics, one row per statistic. The reference's
/// hardware-only rows have no meaning here.
fn bus_stat_rows(load: &crate::load::BusLoad) -> Vec<WebBusStatRow> {
    use crate::load::FrameClass;
    let mut rows = Vec::new();
    let (min, max, avg) = load.load_stats();
    rows.push(WebBusStatRow {
        label: "Busload [%]".to_string(),
        current: pct_text(load.load()),
        min: opt_pct_text(min),
        max: opt_pct_text(max),
        avg: opt_pct_text(avg),
    });
    let (last, min, max, avg) = load.send_dist_us();
    rows.push(WebBusStatRow {
        label: "Min. Send Dist. [ms]".to_string(),
        current: opt_ms_text(last),
        min: opt_ms_text(min),
        max: opt_ms_text(max),
        avg: opt_ms_f_text(avg),
    });
    rows.push(WebBusStatRow {
        label: "Bursts [total]".to_string(),
        current: load.bursts_total().to_string(),
        min: "-".to_string(),
        max: "-".to_string(),
        avg: "-".to_string(),
    });
    let (last, min, max, avg) = load.burst_time_us();
    rows.push(WebBusStatRow {
        label: "Burst Time [ms]".to_string(),
        current: us_ms_text(last),
        min: opt_ms_text(min),
        max: opt_ms_text(max),
        avg: opt_ms_f_text(avg),
    });
    let (cur, min, max, avg) = load.frames_per_burst();
    rows.push(WebBusStatRow {
        label: "Frames per Burst".to_string(),
        current: cur.to_string(),
        min: opt_num_u_text(min),
        max: opt_num_u_text(max),
        avg: opt_num_text(avg),
    });
    class_stat_rows(&mut rows, load, "Std. Data", FrameClass::StdData);
    class_stat_rows(&mut rows, load, "Ext. Data", FrameClass::ExtData);
    class_stat_rows(&mut rows, load, "Std. Remote", FrameClass::StdRemote);
    class_stat_rows(&mut rows, load, "Ext. Remote", FrameClass::ExtRemote);
    let (min, max, avg) = load.rate_stats(FrameClass::Error);
    rows.push(WebBusStatRow {
        label: "Errorframes [n/s]".to_string(),
        current: num_text(load.class_rate(FrameClass::Error)),
        min: opt_num_text(min),
        max: opt_num_text(max),
        avg: opt_num_text(avg),
    });
    rows.push(WebBusStatRow {
        label: "Errorframes [total]".to_string(),
        current: load.errors.to_string(),
        min: "-".to_string(),
        max: "-".to_string(),
        avg: "-".to_string(),
    });
    rows.push(WebBusStatRow {
        label: "Chip State".to_string(),
        current: "Simulated".to_string(),
        min: "-".to_string(),
        max: "-".to_string(),
        avg: "-".to_string(),
    });
    rows
}

/// Turns visible samples into at most one band per pixel column (360) using
/// explicit cut points: the band index is the state, its name the label.
fn state_bands_custom(
    held: Option<f64>,
    samples: &[(u64, f64)],
    lo: u64,
    hi: u64,
    rule: &crate::observe::StateRule,
) -> (String, Vec<WebStateBand>) {
    const COLS: usize = 360;
    let span = hi.saturating_sub(lo).max(1);
    let mut values: Vec<Option<f64>> = vec![None; COLS];
    for &(t, v) in samples {
        let idx =
            (((t.saturating_sub(lo)) as u128 * COLS as u128 / span as u128) as usize).min(COLS - 1);
        values[idx] = Some(v);
    }
    let mut last = held;
    for slot in &mut values {
        if let Some(v) = *slot {
            last = Some(v);
        } else {
            *slot = last;
        }
    }
    let label_of = |v: f64| {
        let band = rule.band(v);
        rule.label(band)
    };
    let mut bands = Vec::new();
    let mut open: Option<(usize, usize)> = None;
    for (i, slot) in values.iter().enumerate() {
        let band = slot.map(|v| rule.band(v));
        match open {
            Some((open_band, _)) if Some(open_band) == band => {}
            Some((open_band, start)) => {
                bands.push(WebStateBand {
                    start: start as f64 / COLS as f64,
                    end: i as f64 / COLS as f64,
                    label: rule.label(open_band),
                    color: rule.color(open_band),
                });
                open = band.map(|band| (band, i));
            }
            None => open = band.map(|band| (band, i)),
        }
    }
    if let Some((open_band, start)) = open {
        bands.push(WebStateBand {
            start: start as f64 / COLS as f64,
            end: 1.0,
            label: rule.label(open_band),
            color: rule.color(open_band),
        });
    }
    let current = last.map(label_of).unwrap_or_else(|| "-".to_string());
    (current, bands)
}

fn parse_id_filter(text: &str) -> Vec<(u32, bool)> {
    text.split([',', ' ', ';'])
        .filter(|part| !part.is_empty())
        .map(|raw| {
            let token = raw.trim();
            match token.strip_suffix(['x', 'X']) {
                Some(hex) => (u32::from_str_radix(hex, 16).unwrap_or(0), true),
                None => (u32::from_str_radix(token, 16).unwrap_or(0), false),
            }
        })
        .filter(|&(id, _)| id != 0)
        .collect()
}

/// Record-filter parser for the web form: CAN hex ids (an `x` suffix marks
/// extended) and `FRn:slot` entries, comma/space separated.
fn parse_record_filter(text: &str) -> crate::recorder::RecordFilter {
    let mut filter = crate::recorder::RecordFilter::default();
    for raw in text.split([',', ' ', ';']) {
        let token = raw.trim();
        if token.is_empty() {
            continue;
        }
        match token
            .strip_prefix(['f', 'F'])
            .and_then(|rest| rest.strip_prefix(['r', 'R']))
        {
            Some(rest) => {
                let Some((bus, slot)) = rest.split_once(':') else {
                    continue;
                };
                if let (Ok(bus), Ok(slot)) = (bus.trim().parse::<u8>(), slot.trim().parse::<u16>())
                {
                    filter.fr.push((bus, slot));
                }
            }
            None => filter.can.extend(parse_id_filter(token)),
        }
    }
    filter
}

fn trigger_kind(cond: &TriggerCond) -> &'static str {
    match cond {
        TriggerCond::SignalCross { .. } => "signal",
        TriggerCond::IdPresent { .. } => "id",
        TriggerCond::ErrorFrame { .. } => "error",
        TriggerCond::CycleTimeout { .. } => "timeout",
        TriggerCond::FrFramePresent { .. } => "fr_frame",
        TriggerCond::FrSignalCross { .. } => "fr_signal",
        TriggerCond::SysVar { .. } => "sysvar",
        TriggerCond::DerivedSignal { .. } => "derived",
        TriggerCond::PayloadMatch { .. } => "payload",
        TriggerCond::EthFlow { .. } => "eth_flow",
        TriggerCond::FrTimeout { .. } => "fr_timeout",
        TriggerCond::E2eIssue { .. } => "e2e",
        TriggerCond::SomeIp { .. } => "someip",
    }
}

fn parse_trigger_kind(value: &str) -> Option<TriggerKind> {
    match value {
        "signal" => Some(TriggerKind::Signal),
        "id" => Some(TriggerKind::Id),
        "error" => Some(TriggerKind::ErrorFrame),
        "fr_frame" => Some(TriggerKind::FrFrame),
        "fr_signal" => Some(TriggerKind::FrSignal),
        "timeout" => Some(TriggerKind::Timeout),
        "sysvar" => Some(TriggerKind::SysVar),
        "derived" => Some(TriggerKind::Derived),
        "payload" => Some(TriggerKind::Payload),
        "eth_flow" => Some(TriggerKind::EthFlow),
        "fr_timeout" => Some(TriggerKind::FrTimeout),
        "e2e" => Some(TriggerKind::E2e),
        "someip" => Some(TriggerKind::SomeIp),
        _ => None,
    }
}

fn parse_driver(value: &str) -> Option<HwDriver> {
    match value {
        "Vector" => Some(HwDriver::Vector),
        "Kvaser" => Some(HwDriver::Kvaser),
        _ => None,
    }
}

fn flag_value(args: &[String], flag: &str) -> Option<String> {
    args.iter()
        .position(|arg| arg == flag)
        .and_then(|i| args.get(i + 1))
        .cloned()
}

/// Bring up the requested session: a saved project, a replay log, or an
/// empty workspace. A project without a log opens stopped; a log starts
/// playback; no flags starts with no buses and no measurement.
fn prepare_session(app: &mut App, project: Option<&str>, replay: Option<&str>) {
    if let Some(path) = project {
        app.open_project_path(std::path::Path::new(path));
    }
    if let Some(path) = replay {
        app.load_log(path);
        if !app.log_path.is_empty() {
            app.send(BusCommand::SetRunMode(Mode::Replay));
            app.start_selected();
        }
        if project.is_some() {
            app.mark_clean();
        }
        return;
    }
    if project.is_some() {
        // Loading applies the project's live layout, so a fresh open has
        // nothing unsaved.
        app.mark_clean();
    }
}

#[derive(Clone)]
struct ApiState {
    shared: Arc<Mutex<WebState>>,
    trace: Arc<Mutex<TraceStore>>,
    signals: Arc<Mutex<SignalStore>>,
    catalog: Arc<Mutex<WebCatalog>>,
    hardware: Arc<Mutex<HardwareCache>>,
    scripts: Arc<Mutex<Vec<WebScript>>>,
    commands: Sender<WebCommand>,
}

async fn catalog_handler(State(api): State<ApiState>) -> Json<WebCatalog> {
    Json(
        api.catalog
            .lock()
            .map(|catalog| catalog.clone())
            .unwrap_or_default(),
    )
}

async fn hardware_handler(State(api): State<ApiState>) -> Json<WebHardware> {
    let generation = api.shared.lock().map(|state| state.hw_refresh).unwrap_or(0);
    if let Ok(cache) = api.hardware.lock()
        && cache.generation == generation
        && let Some(data) = &cache.data
    {
        return Json(data.clone());
    }
    let probed = tokio::task::spawn_blocking(|| {
        (
            crate::hw::enumerate_all(),
            crate::hw::vector::enumerate_flexray(),
        )
    })
    .await
    .unwrap_or_else(|error| {
        (
            Err(format!("hardware probe failed: {error}")),
            Err(format!("hardware probe failed: {error}")),
        )
    });
    let (can, fr) = probed;
    let (channels, error) = match can {
        Ok(channels) => (
            channels
                .into_iter()
                .map(|channel| WebHwChannel {
                    driver: channel.driver.word().to_string(),
                    index: channel.index,
                    name: channel.name,
                    detail: channel.detail,
                })
                .collect(),
            String::new(),
        ),
        Err(error) => (Vec::new(), error),
    };
    let (fr_channels, fr_error) = match fr {
        Ok(list) => {
            let mut counts = std::collections::BTreeMap::new();
            for channel in &list {
                *counts.entry(channel.name.clone()).or_insert(0u32) += 1;
            }
            (
                list.into_iter()
                    .map(|channel| WebHwChannel {
                        driver: "Vector".to_string(),
                        index: channel.index,
                        name: channel.name.clone(),
                        detail: (counts.get(&channel.name).copied().unwrap_or(0) > 1)
                            .then(|| format!("dev {}", channel.hw_index)),
                    })
                    .collect(),
                String::new(),
            )
        }
        Err(error) => (Vec::new(), error),
    };
    let (eth_channels, eth_error) = match crate::hw::Hardware::eth_interfaces() {
        Ok(list) => (
            list.into_iter()
                .map(|iface| WebEthChannel {
                    name: if iface.description.is_empty() {
                        iface.device.clone()
                    } else {
                        format!("{} · {}", iface.device, iface.description)
                    },
                    device: iface.device,
                })
                .collect(),
            String::new(),
        ),
        Err(error) => (Vec::new(), error),
    };
    let data = WebHardware {
        channels,
        fr_channels,
        eth_channels,
        error,
        fr_error,
        eth_error,
    };
    if let Ok(mut cache) = api.hardware.lock() {
        cache.generation = generation;
        cache.data = Some(data.clone());
    }
    Json(data)
}

#[derive(Default, Deserialize)]
struct FsListQuery {
    path: Option<String>,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct FsRoot {
    name: String,
    path: String,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct FsEntry {
    name: String,
    path: String,
    dir: bool,
    size: u64,
    modified_ms: u64,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct FsList {
    path: String,
    parent: Option<String>,
    roots: Vec<FsRoot>,
    entries: Vec<FsEntry>,
    truncated: bool,
}

fn clean_path(path: &std::path::Path) -> String {
    let text = path.to_string_lossy();
    text.strip_prefix("\\\\?\\").unwrap_or(&text).to_string()
}

fn fs_roots() -> Vec<FsRoot> {
    let mut roots = Vec::new();
    let mut seen = std::collections::HashSet::new();
    let mut push = |name: &str, path: std::path::PathBuf| {
        let path = clean_path(&path);
        if seen.insert(path.to_ascii_lowercase()) {
            roots.push(FsRoot {
                name: name.to_string(),
                path,
            });
        }
    };
    if let Some(home) = std::env::var_os("USERPROFILE").or_else(|| std::env::var_os("HOME")) {
        push("Home", std::path::PathBuf::from(home));
    }
    if let Ok(cwd) = std::env::current_dir() {
        push("Working folder", cwd);
    }
    #[cfg(windows)]
    {
        for letter in b'C'..=b'Z' {
            let root = format!("{}:\\", letter as char);
            if std::path::Path::new(&root).exists() {
                push(
                    &format!("{}:", letter as char),
                    std::path::PathBuf::from(root),
                );
            }
        }
    }
    #[cfg(not(windows))]
    {
        push("/", std::path::PathBuf::from("/"));
    }
    roots
}

fn list_directory(requested: Option<&str>) -> Result<FsList, String> {
    let requested = requested
        .map(str::trim)
        .filter(|path| !path.is_empty())
        .map(std::path::PathBuf::from)
        .or_else(|| {
            std::env::var_os("USERPROFILE")
                .or_else(|| std::env::var_os("HOME"))
                .map(std::path::PathBuf::from)
        })
        .or_else(|| std::env::current_dir().ok())
        .ok_or_else(|| "no folder to list".to_string())?;
    let current = std::fs::canonicalize(&requested)
        .map_err(|error| format!("cannot open {}: {error}", clean_path(&requested)))?;
    if !current.is_dir() {
        return Err(format!("not a folder: {}", clean_path(&current)));
    }
    let mut entries = Vec::new();
    let mut truncated = false;
    if let Ok(read) = std::fs::read_dir(&current) {
        for entry in read.flatten() {
            if entries.len() >= 2000 {
                truncated = true;
                break;
            }
            let Ok(meta) = entry.metadata() else {
                continue;
            };
            let name = entry.file_name().to_string_lossy().into_owned();
            entries.push(FsEntry {
                name,
                path: clean_path(&entry.path()),
                dir: meta.is_dir(),
                size: if meta.is_file() { meta.len() } else { 0 },
                modified_ms: meta
                    .modified()
                    .ok()
                    .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
                    .map(|duration| duration.as_millis() as u64)
                    .unwrap_or(0),
            });
        }
    }
    entries.sort_by(|a, b| {
        b.dir.cmp(&a.dir).then_with(|| {
            a.name
                .to_ascii_lowercase()
                .cmp(&b.name.to_ascii_lowercase())
        })
    });
    Ok(FsList {
        path: clean_path(&current),
        parent: current.parent().map(clean_path),
        roots: fs_roots(),
        entries,
        truncated,
    })
}

async fn fs_list_handler(
    Query(query): Query<FsListQuery>,
) -> Result<Json<FsList>, (StatusCode, String)> {
    list_directory(query.path.as_deref())
        .map(Json)
        .map_err(|error| (StatusCode::BAD_REQUEST, error))
}

#[derive(Deserialize)]
struct CreateFolderRequest {
    path: String,
    name: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct NewFolder {
    path: String,
    name: String,
}

fn valid_folder_name(name: &str) -> Result<&str, String> {
    let name = name.trim();
    if name.is_empty() {
        return Err("folder name is empty".to_string());
    }
    if name == "." || name == ".." || name.ends_with(' ') || name.ends_with('.') {
        return Err("folder name is not valid".to_string());
    }
    if name.chars().any(|ch| {
        ch.is_control() || matches!(ch, '/' | '\\' | '<' | '>' | ':' | '"' | '|' | '?' | '*')
    }) {
        return Err("folder name contains an invalid character".to_string());
    }
    Ok(name)
}

fn create_folder(parent: &str, name: &str) -> Result<NewFolder, String> {
    let name = valid_folder_name(name)?;
    let parent = std::fs::canonicalize(parent).map_err(|error| {
        format!(
            "cannot open {}: {error}",
            clean_path(std::path::Path::new(parent))
        )
    })?;
    if !parent.is_dir() {
        return Err(format!("not a folder: {}", clean_path(&parent)));
    }
    let path = parent.join(name);
    if path.exists() {
        return Err(format!("folder already exists: {}", clean_path(&path)));
    }
    std::fs::create_dir(&path).map_err(|error| format!("cannot create folder: {error}"))?;
    Ok(NewFolder {
        path: clean_path(&path),
        name: name.to_string(),
    })
}

async fn create_folder_handler(
    Json(request): Json<CreateFolderRequest>,
) -> Result<Json<NewFolder>, (StatusCode, String)> {
    create_folder(&request.path, &request.name)
        .map(Json)
        .map_err(|error| (StatusCode::BAD_REQUEST, error))
}

#[derive(Deserialize)]
struct RenameFolderRequest {
    path: String,
    name: String,
}

#[derive(Deserialize)]
struct DeleteFolderRequest {
    path: String,
}

fn rename_folder(path: &str, name: &str) -> Result<NewFolder, String> {
    let name = valid_folder_name(name)?;
    let path =
        std::fs::canonicalize(path).map_err(|error| format!("cannot open {path}: {error}"))?;
    if !path.is_dir() {
        return Err(format!("not a folder: {}", clean_path(&path)));
    }
    let parent = path
        .parent()
        .ok_or_else(|| "cannot rename a root folder".to_string())?;
    let target = parent.join(name);
    if target == path {
        return Ok(NewFolder {
            path: clean_path(&path),
            name: name.to_string(),
        });
    }
    if target.exists() {
        return Err(format!("folder already exists: {}", clean_path(&target)));
    }
    std::fs::rename(&path, &target).map_err(|error| format!("cannot rename folder: {error}"))?;
    Ok(NewFolder {
        path: clean_path(&target),
        name: name.to_string(),
    })
}

fn delete_folder(path: &str) -> Result<(), String> {
    let path =
        std::fs::canonicalize(path).map_err(|error| format!("cannot open {path}: {error}"))?;
    if !path.is_dir() {
        return Err(format!("not a folder: {}", clean_path(&path)));
    }
    if path.parent().is_none() {
        return Err("cannot delete a root folder".to_string());
    }
    match std::fs::remove_dir(&path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::DirectoryNotEmpty => {
            Err("folder is not empty; remove its contents first".to_string())
        }
        Err(error) => Err(format!("cannot delete folder: {error}")),
    }
}

async fn rename_folder_handler(
    Json(request): Json<RenameFolderRequest>,
) -> Result<Json<NewFolder>, (StatusCode, String)> {
    rename_folder(&request.path, &request.name)
        .map(Json)
        .map_err(|error| (StatusCode::BAD_REQUEST, error))
}

async fn delete_folder_handler(
    Json(request): Json<DeleteFolderRequest>,
) -> Result<StatusCode, (StatusCode, String)> {
    delete_folder(&request.path)
        .map(|()| StatusCode::NO_CONTENT)
        .map_err(|error| (StatusCode::BAD_REQUEST, error))
}

#[derive(Default, Deserialize)]
struct TraceQuery {
    epoch: Option<u64>,
    seq: Option<u64>,
    signals_epoch: Option<u64>,
    signals_since: Option<u64>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct WebResponse {
    #[serde(flatten)]
    state: WebState,
    trace_epoch: u64,
    trace_seq: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    trace_seed: Option<Vec<WebFrame>>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    trace_delta: Vec<WebFrame>,
    signals_epoch: u64,
    signals_max_t: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    signals_seed: Option<Vec<SignalSeries>>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    signals_delta: Vec<SignalSeries>,
}

async fn state(State(api): State<ApiState>, Query(query): Query<TraceQuery>) -> Json<WebResponse> {
    let snapshot = api.shared.lock().map(|s| s.clone()).unwrap_or_default();
    let (epoch, seq, seed, delta) = {
        let store = api.trace.lock().unwrap_or_else(|p| p.into_inner());
        let epoch = store.epoch;
        let seq = store.next_seq;
        let first = store.first_seq();
        let synced = query.epoch == Some(epoch)
            && query
                .seq
                .is_some_and(|client| client >= first && client <= seq);
        if synced {
            (epoch, seq, None, store.after(query.seq.unwrap_or(seq)))
        } else {
            (epoch, seq, Some(store.seed()), Vec::new())
        }
    };
    let (signals_epoch, signals_max_t, signals_seed, signals_delta) = {
        let store = api.signals.lock().unwrap_or_else(|p| p.into_inner());
        let epoch = store.epoch;
        let max_t = store.max_t();
        let first = store.earliest_t();
        let synced = query.signals_epoch == Some(epoch)
            && query
                .signals_since
                .is_some_and(|client| client >= first && client <= max_t);
        if synced {
            (
                epoch,
                max_t,
                None,
                store.after(query.signals_since.unwrap_or(max_t)),
            )
        } else {
            (epoch, max_t, Some(store.seed()), Vec::new())
        }
    };
    Json(WebResponse {
        state: snapshot,
        trace_epoch: epoch,
        trace_seq: seq,
        trace_seed: seed,
        trace_delta: delta,
        signals_epoch,
        signals_max_t,
        signals_seed,
        signals_delta,
    })
}

#[derive(Deserialize)]
struct UploadQuery {
    name: Option<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct UploadReply {
    path: String,
    name: String,
    size: usize,
}

#[derive(Deserialize)]
struct DownloadQuery {
    name: Option<String>,
}

/// Files the browser exchanges with the host live in the per-user data
/// folder, not the temp folder: a project saved after picking a DBC must
/// still find that DBC after the next reboot.
fn web_data_dir(sub: &str) -> std::path::PathBuf {
    let root = std::env::var_os("LOCALAPPDATA")
        .map(std::path::PathBuf::from)
        .or_else(|| std::env::var_os("XDG_DATA_HOME").map(std::path::PathBuf::from))
        .or_else(|| {
            std::env::var_os("HOME")
                .map(|home| std::path::PathBuf::from(home).join(".local").join("share"))
        })
        .unwrap_or_else(std::env::temp_dir);
    root.join("roxy-can").join(sub)
}

fn upload_dir() -> std::path::PathBuf {
    web_data_dir("web-uploads")
}

fn download_dir() -> std::path::PathBuf {
    web_data_dir("web-downloads")
}

fn to_download_dir(file: &str) -> std::path::PathBuf {
    download_dir().join(upload_name(file))
}

/// A bare file name from the export or save dialog is written to the
/// download folder so the browser can fetch it back. Paths with a directory
/// part are the user's own host paths and stay untouched.
fn download_file_path(path: &str) -> String {
    let trimmed = path.trim();
    if std::path::Path::new(trimmed).components().count() != 1 {
        return trimmed.to_string();
    }
    let _ = std::fs::create_dir_all(download_dir());
    to_download_dir(trimmed).to_string_lossy().into_owned()
}

/// Serves only files in the export folder; the name is sanitized the same
/// way the export path was, so a caller cannot walk out of that folder.
async fn download(Query(query): Query<DownloadQuery>) -> Result<Response, (StatusCode, String)> {
    let name = upload_name(query.name.as_deref().unwrap_or(""));
    let path = to_download_dir(&name);
    let read = path.clone();
    let bytes = tokio::task::spawn_blocking(move || std::fs::read(&read))
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("download task: {e}"),
            )
        })?
        .map_err(|_| (StatusCode::NOT_FOUND, "export is not ready".to_string()))?;
    let mut response = bytes.into_response();
    response.headers_mut().insert(
        CONTENT_TYPE,
        HeaderValue::from_static("application/octet-stream"),
    );
    let disposition =
        HeaderValue::from_str(&format!("attachment; filename=\"{name}\"")).map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("download name: {e}"),
            )
        })?;
    response
        .headers_mut()
        .insert(CONTENT_DISPOSITION, disposition);
    Ok(response)
}

/// Serves the file the recorder finished last. The path comes from the
/// core's own snapshot, never from the query, and a live recording is
/// refused until its writer has closed the trailer.
async fn recording_download(
    State(api): State<ApiState>,
    Query(query): Query<std::collections::HashMap<String, String>>,
) -> Result<Response, (StatusCode, String)> {
    let want_eth = query.get("eth").is_some_and(|value| value == "1");
    let (path, eth_path, recording) = api
        .shared
        .lock()
        .map(|state| {
            (
                state.record_file.clone(),
                state.record_eth_file.clone(),
                state.recording,
            )
        })
        .unwrap_or_default();
    let path = if want_eth && !eth_path.is_empty() {
        eth_path
    } else {
        path
    };
    if recording {
        return Err((
            StatusCode::CONFLICT,
            "stop recording before downloading".to_string(),
        ));
    }
    if path.is_empty() {
        return Err((StatusCode::NOT_FOUND, "no recording yet".to_string()));
    }
    let read = path.clone();
    let bytes = tokio::task::spawn_blocking(move || std::fs::read(&read))
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("recording task: {e}"),
            )
        })?
        .map_err(|e| (StatusCode::NOT_FOUND, format!("recording: {e}")))?;
    let file = std::path::Path::new(&path)
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("recording.asc");
    let name = upload_name(file);
    let mut response = bytes.into_response();
    response.headers_mut().insert(
        CONTENT_TYPE,
        HeaderValue::from_static("application/octet-stream"),
    );
    let disposition =
        HeaderValue::from_str(&format!("attachment; filename=\"{name}\"")).map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("download name: {e}"),
            )
        })?;
    response
        .headers_mut()
        .insert(CONTENT_DISPOSITION, disposition);
    Ok(response)
}

async fn discard_download(
    Query(query): Query<DownloadQuery>,
) -> Result<StatusCode, (StatusCode, String)> {
    let name = upload_name(query.name.as_deref().unwrap_or(""));
    let path = to_download_dir(&name);
    tokio::task::spawn_blocking(move || match std::fs::remove_file(&path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error),
    })
    .await
    .map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("discard task: {e}"),
        )
    })?
    .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, format!("discard: {e}")))?;
    Ok(StatusCode::NO_CONTENT)
}

fn upload_name(raw: &str) -> String {
    let base = std::path::Path::new(raw)
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("");
    let mut cleaned: String = base
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-') {
                c
            } else {
                '_'
            }
        })
        .collect();
    cleaned.truncate(120);
    if cleaned.is_empty() || cleaned.chars().all(|c| c == '.') {
        "upload".to_string()
    } else {
        cleaned
    }
}

/// Saves a browser-picked file into the OS temp folder and returns the host
/// path the rest of the API can load. Only the file name is trusted: any
/// directory part is dropped and the stamped prefix keeps two uploads from
/// overwriting each other.
async fn upload(
    Query(query): Query<UploadQuery>,
    body: Bytes,
) -> Result<Json<UploadReply>, (StatusCode, String)> {
    let dir = upload_dir();
    std::fs::create_dir_all(&dir).map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("upload folder: {e}"),
        )
    })?;
    let name = upload_name(query.name.as_deref().unwrap_or("upload"));
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let path = dir.join(format!("{stamp}_{name}"));
    let size = body.len();
    let write = path.clone();
    tokio::task::spawn_blocking(move || std::fs::write(&write, &body))
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("upload task: {e}"),
            )
        })?
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("upload write: {e}"),
            )
        })?;
    Ok(Json(UploadReply {
        path: path.to_string_lossy().into_owned(),
        name,
        size,
    }))
}

fn default_true() -> bool {
    true
}

fn default_mask() -> u8 {
    0xFF
}

fn default_someip_port() -> u16 {
    30490
}

/// Where one log channel should replay. `"same"` removes the table entry
/// instead of pinning today's bus, so a project with no deliberate mapping
/// stays portable; `"drop"` is an explicit no-replay that survives a reopen.
#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
enum ReplayChannelChoice {
    Word(String),
    Bus(u8),
}

#[derive(Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
#[allow(clippy::large_enum_variant)]
enum WebCommand {
    Start,
    Stop,
    Pause {
        on: bool,
    },
    ReplayStep {
        delta_us: u64,
    },
    ReplayPlayTo {
        target_us: u64,
    },
    ReplayPlayFor {
        duration_us: u64,
    },
    SetReplaySpeed {
        speed: f64,
    },
    SetReplayChannel {
        kind: String,
        raw: u8,
        target: ReplayChannelChoice,
    },
    RecordToggle {
        name: Option<String>,
        format: Option<u8>,
    },
    SetRecordSettings {
        name: String,
        format: u8,
    },
    SetRecordFilter {
        text: String,
    },
    ClearTrace,
    /// Reset the Messages counters without dropping the Trace ring.
    ClearMessages,
    /// Open or close every Messages row's signal children.
    SetAllMessagesExpanded {
        on: bool,
    },
    /// Open or close one Messages row's signal children.
    SetMessageExpanded {
        key: String,
        on: bool,
    },
    /// Fire `on panel_action` handlers by name on the next measurement step.
    /// `value` is handed to a handler that declares a parameter.
    RunPanelAction {
        name: String,
        #[serde(default)]
        value: Option<f64>,
    },
    /// Write one DBC signal from a Panel setpoint. `source` is the observer
    /// key `ch:id:signal` the catalog hands out.
    PanelWriteSignal {
        source: String,
        value: f64,
    },
    /// Send one raw CAN frame once from a Panel button. `physical` encodes
    /// `signals` against the catalog message instead; `data` is ignored then.
    PanelSendFrame {
        channel: u8,
        id: u32,
        extended: bool,
        data: Vec<u8>,
        #[serde(default)]
        physical: bool,
        #[serde(default)]
        signals: std::collections::BTreeMap<String, f64>,
    },
    PanelSendFrFrame {
        bus: u8,
        slot: u16,
        data: Vec<u8>,
        #[serde(default)]
        physical: bool,
        #[serde(default)]
        signals: std::collections::BTreeMap<String, f64>,
    },
    PanelSendEthFrame {
        bus: u8,
        frame: String,
        data: Vec<u8>,
        #[serde(default)]
        physical: bool,
        #[serde(default)]
        signals: std::collections::BTreeMap<String, f64>,
    },
    PanelSendSomeIp {
        bus: u8,
        service: u16,
        method: u16,
        client: u16,
        session: u16,
        message_type: u8,
        data: Vec<u8>,
        #[serde(default)]
        values: std::collections::BTreeMap<String, f64>,
        #[serde(default)]
        texts: std::collections::BTreeMap<String, String>,
        #[serde(default)]
        transport: String,
        #[serde(default)]
        host: String,
        #[serde(default = "default_someip_port")]
        port: u16,
    },
    OpenProject {
        path: String,
    },
    CreateSampleProject {
        parent: String,
    },
    SetRunMode {
        mode: String,
    },
    OpenLog {
        path: String,
        /// Load and start playback; false parks the log at its beginning.
        #[serde(default = "default_true")]
        play: bool,
    },
    AddReplayFolder {
        path: String,
    },
    AddReplayFile {
        path: String,
    },
    RemoveReplaySource {
        path: String,
        folder: bool,
    },
    RefreshReplaySources,
    SelectSignal {
        ch: u8,
        id: u32,
        ext: bool,
        name: String,
        /// Read-only selection: subscribe and show at cursors, no lane.
        #[serde(default)]
        read: bool,
    },
    UnselectSignal {
        ch: u8,
        id: u32,
        ext: bool,
        name: String,
    },
    SelectFrSignal {
        bus: u8,
        slot: u16,
        name: String,
        #[serde(default)]
        read: bool,
    },
    UnselectFrSignal {
        bus: u8,
        slot: u16,
        name: String,
    },
    SelectEthSignal {
        bus: u8,
        frame: String,
        name: String,
        #[serde(default)]
        read: bool,
    },
    UnselectEthSignal {
        bus: u8,
        frame: String,
        name: String,
    },
    SelectDerivedSignal {
        id: String,
        name: String,
        #[serde(default)]
        read: bool,
    },
    UnselectDerivedSignal {
        id: String,
        name: String,
    },
    MoveSignal {
        key: String,
        to: usize,
    },
    ChartSignal {
        key: String,
        action: String,
    },
    AttachHardware {
        bus: u8,
        driver: String,
        index: i32,
    },
    DetachHardware {
        bus: u8,
    },
    AttachEthPort {
        bus: u8,
        device: String,
    },
    DetachEthPort {
        bus: u8,
    },
    AttachFrHardware {
        bus: u8,
        index: i32,
    },
    DetachFrHardware {
        bus: u8,
    },
    RefreshHardware,
    SetBusMode {
        real: bool,
    },
    AddBus {
        #[serde(default)]
        kind: String,
    },
    RemoveBus {
        ch: u8,
    },
    RenameBus {
        ch: u8,
        name: String,
    },
    SetBusRates {
        ch: u8,
        bitrate_kbps: u32,
        fd_data_kbps: u32,
    },
    SetNodeRole {
        bus: u8,
        node: String,
        simulated: bool,
    },
    RemoveFrBus {
        bus: u8,
    },
    RemoveEthBus {
        bus: u8,
    },
    /// Lists the clusters of an uploaded ARXML so the UI can ask which one
    /// belongs to the bus the user picked.
    ScanArxml {
        path: String,
    },
    /// Binds one description to a bus. `kind` is `can`, `flexray` or
    /// `ethernet`; `path` is a DBC/ARXML/FIBEX file. A DBC is `kind: can`
    /// with a `.dbc` path.
    AddBusDescription {
        bus: u8,
        kind: String,
        path: String,
        #[serde(default)]
        cluster: String,
    },
    RemoveBusDescription {
        bus: u8,
        kind: String,
        path: String,
        #[serde(default)]
        cluster: String,
    },
    SaveProject {
        path: Option<String>,
        /// Panel pages to store inside the project, in `.rxpanel` format.
        #[serde(default)]
        panels: Vec<serde_json::Value>,
    },
    AddScript {
        name: String,
        channel: u8,
        attached_bus: Option<u8>,
        attached_node: Option<String>,
    },
    SetScriptName {
        id: u64,
        name: String,
    },
    SetScriptEnabled {
        id: u64,
        on: bool,
    },
    RemoveScript {
        id: u64,
    },
    LoadScript {
        id: u64,
        path: String,
    },
    SaveScript {
        id: u64,
        source: String,
    },
    SaveScriptAs {
        id: u64,
        path: String,
        source: String,
    },
    CheckScript {
        id: u64,
        source: String,
    },
    ApplyScript {
        id: u64,
        source: String,
    },
    SetSpecTolerance {
        value: u64,
    },
    SetSpecGrace {
        value: u64,
    },
    ClearSpec,
    Export {
        kind: String,
        path: String,
        /// Optional trace interval, in microseconds; ignored by non-trace kinds.
        #[serde(default)]
        from_us: Option<u64>,
        #[serde(default)]
        to_us: Option<u64>,
    },
    SetSysVar {
        namespace: String,
        name: String,
        value: f64,
    },
    DefineSysVar {
        namespace: String,
        name: String,
        init: f64,
        min: Option<f64>,
        max: Option<f64>,
        unit: String,
        comment: String,
        #[serde(default)]
        label_sets: Vec<crate::bus::SysVarLabelSet>,
        #[serde(default)]
        band_set_id: Option<String>,
        old_namespace: Option<String>,
        old_name: Option<String>,
    },
    DeleteSysVar {
        namespace: String,
        name: String,
    },
    DefineBandSet {
        id: String,
        name: String,
        bands: Vec<crate::band::Band>,
        old_id: Option<String>,
    },
    DeleteBandSet {
        id: String,
    },
    AddMonitor {
        kind: String,
        ch: u8,
        id: u32,
        ext: bool,
        frame: Option<String>,
        name: String,
    },
    RemoveMonitor {
        kind: String,
        ch: u8,
        id: u32,
        ext: bool,
        frame: Option<String>,
        name: String,
    },
    ClearMonitor,
    /// Edit one Monitor row: decimals and optional value bands. `key` is the
    /// canonical `sig:` key; omitted fields keep their current value.
    ConfigureMonitor {
        key: String,
        digits: Option<usize>,
        bands: Option<Vec<crate::band::Band>>,
        #[serde(default)]
        band_set_id: Option<String>,
    },
    /// One or two Monitor rows per line.
    SetMonitorColumns {
        columns: u8,
    },
    /// Reorder Monitor rows: the `sig:` key moves to slot `to` (index in the
    /// post-removal list, clamped by the core).
    MoveMonitorRow {
        key: String,
        to: usize,
    },
    AddData {
        kind: String,
        ch: u8,
        id: u32,
        ext: bool,
        frame: Option<String>,
        name: String,
    },
    RemoveData {
        kind: String,
        ch: u8,
        id: u32,
        ext: bool,
        frame: Option<String>,
        name: String,
    },
    ClearData,
    /// Reorder the first Data window's rows, same semantics as the Monitor
    /// move.
    MoveDataRow {
        key: String,
        to: usize,
    },
    SetDataVisible {
        kind: String,
        ch: u8,
        id: u32,
        ext: bool,
        frame: Option<String>,
        name: String,
        visible: bool,
    },
    AddState {
        kind: String,
        ch: u8,
        id: u32,
        ext: bool,
        frame: Option<String>,
        name: String,
    },
    RemoveState {
        kind: String,
        ch: u8,
        id: u32,
        ext: bool,
        frame: Option<String>,
        name: String,
    },
    ClearState,
    SetStateWindow {
        seconds: f64,
    },
    SetStateRule {
        kind: String,
        ch: u8,
        id: u32,
        ext: bool,
        frame: Option<String>,
        name: String,
        bands: Vec<crate::band::Band>,
        #[serde(default)]
        band_set_id: Option<String>,
    },
    ClearStateRule {
        kind: String,
        ch: u8,
        id: u32,
        ext: bool,
        frame: Option<String>,
        name: String,
    },
    SetEntryActive {
        ch: u8,
        id: u32,
        on: bool,
    },
    SetEntryCycle {
        ch: u8,
        id: u32,
        cycle_us: u64,
    },
    SetEntryFd {
        ch: u8,
        id: u32,
        fd: bool,
    },
    SetEntryBrs {
        ch: u8,
        id: u32,
        brs: bool,
    },
    SetEntryHex {
        ch: u8,
        id: u32,
        text: String,
    },
    SetEntrySource {
        ch: u8,
        id: u32,
        src: WebSrc,
    },
    AddReplayBlock {
        channel: u8,
        name: String,
        path: String,
        node: Option<String>,
    },
    SetReplayBlock {
        id: u64,
        channel: u8,
        name: String,
        path: String,
        node: Option<String>,
        ids: Vec<(u32, bool)>,
    },
    SetReplayBlockEnabled {
        id: u64,
        on: bool,
    },
    RemoveReplayBlock {
        id: u64,
    },
    /// Clone replay block `id` as a new disabled "… copy".
    DuplicateReplayBlock {
        id: u64,
    },
    AddFrEntry {
        bus: u8,
        slot: u16,
    },
    RemoveFrEntry {
        bus: u8,
        slot: u16,
    },
    SetFrEntryActive {
        bus: u8,
        slot: u16,
        on: bool,
    },
    SetFrEntryCycle {
        bus: u8,
        slot: u16,
        cycle_us: u64,
    },
    SetFrEntryHex {
        bus: u8,
        slot: u16,
        text: String,
    },
    ClearEntrySource {
        ch: u8,
        id: u32,
        name: String,
    },
    AddEntry {
        ch: u8,
        id: u32,
    },
    RemoveEntry {
        ch: u8,
        id: u32,
    },
    SimulateBusNodes {
        ch: u8,
    },
    AddTrigger {
        kind: String,
        /// Optional seed fields from the Trace/chart context menus.
        #[serde(default)]
        ch: Option<u8>,
        #[serde(default)]
        id: Option<u32>,
        #[serde(default)]
        bus: Option<u8>,
        #[serde(default)]
        slot: Option<u16>,
        /// `SigKey::text()` for a chart-lane signal; the backend resolves
        /// the cluster to a bus index.
        #[serde(default)]
        signal_key: Option<String>,
    },
    SetTriggerEnabled {
        index: usize,
        on: bool,
    },
    RemoveTrigger {
        index: usize,
    },
    /// Clone rule `index` onto the end; the copy starts disarmed.
    DuplicateTrigger {
        index: usize,
    },
    RearmTriggers,
    RearmTrigger {
        index: usize,
    },
    EditTrigger {
        index: usize,
        kind: String,
        ch: u8,
        id: u32,
        ext: bool,
        signal: String,
        threshold: f64,
        rising: bool,
        bus: u8,
        slot: u16,
        key: String,
        derived_id: String,
        action: String,
        send_ch: u8,
        send_id: u32,
        #[serde(default)]
        sys_var_key: String,
        #[serde(default)]
        sys_var_value: Option<f64>,
        #[serde(default)]
        script_name: String,
        #[serde(default)]
        script_value: Option<f64>,
        #[serde(default)]
        send_frame: String,
        #[serde(default)]
        send_data: String,
        #[serde(default)]
        someip_method: u16,
        #[serde(default)]
        someip_client: u16,
        #[serde(default)]
        someip_session: u16,
        #[serde(default)]
        someip_type: u8,
        /// SOME/IP condition filters; absent = wildcard.
        #[serde(default)]
        cond_someip_method: Option<u16>,
        #[serde(default)]
        cond_someip_type: Option<u8>,
        #[serde(default)]
        someip_values: std::collections::BTreeMap<String, f64>,
        #[serde(default)]
        someip_transport: String,
        #[serde(default)]
        someip_host: String,
        #[serde(default = "default_someip_port")]
        someip_port: u16,
        #[serde(default)]
        offset: u8,
        #[serde(default = "default_mask")]
        mask: u8,
        #[serde(default)]
        value: u8,
        #[serde(default)]
        vlan: Option<u16>,
        #[serde(default)]
        cooldown_ms: u32,
        #[serde(default)]
        max_fires: u32,
        #[serde(default)]
        pre_frames: Option<u32>,
        #[serde(default)]
        post_frames: Option<u32>,
    },
    SetTriggerLimits {
        pre_frames: usize,
        post_frames: u32,
        marker_cap: usize,
    },
    /// Adopt the browser's workspace for the current project revision. A
    /// late debounce from the previous project carries its old revision and
    /// is dropped instead of overwriting the newly loaded file.
    SetWebWorkspace {
        rev: u64,
        workspace: serde_json::Value,
    },
    /// Decode another ASC/BLF with the current descriptions and keep its
    /// selected CAN, FlexRay and Ethernet signals as a comparison baseline.
    LoadReference {
        path: String,
    },
    ClearReference,
    /// Replace the derived-channel definitions. The core samples the curves;
    /// the snapshot carries the definitions back to the editor.
    SetDerived {
        defs: Vec<crate::derived::DerivedDef>,
    },
}

async fn command(State(api): State<ApiState>, Json(cmd): Json<WebCommand>) -> StatusCode {
    match api.commands.send(cmd) {
        Ok(()) => StatusCode::ACCEPTED,
        Err(_) => StatusCode::SERVICE_UNAVAILABLE,
    }
}

/// Owns the App, applies UI commands, and publishes the web snapshot. The
/// manual drive is what the headless CLI already uses: no window, no
/// mailbox race, commands take effect on the next lap.
/// The caches the core thread publishes into and the HTTP handlers read.
/// Grouped so `app_loop` keeps a small argument list.
struct SharedStores {
    shared: Arc<Mutex<WebState>>,
    trace: Arc<Mutex<TraceStore>>,
    signals: Arc<Mutex<SignalStore>>,
    catalog: Arc<Mutex<WebCatalog>>,
    scripts: Arc<Mutex<Vec<WebScript>>>,
}

