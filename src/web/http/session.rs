/// The Vite bundle lives inside the binary in release builds, so the exe can
/// be copied anywhere on its own.
#[derive(RustEmbed)]
#[folder = "web/dist"]
struct Assets;

/// Hashed assets never change under the same name; `index.html` must be
/// revalidated so an updated binary's new bundle is picked up.
async fn static_file(uri: Uri) -> Response {
    let requested = uri.path().trim_start_matches('/');
    let path = if requested.is_empty() {
        "index.html"
    } else {
        requested
    };
    let (served, file) = match Assets::get(path) {
        Some(file) => (path, file),
        None if !requested.contains('.') => match Assets::get("index.html") {
            Some(file) => ("index.html", file),
            None => return StatusCode::NOT_FOUND.into_response(),
        },
        None => return StatusCode::NOT_FOUND.into_response(),
    };
    let mime = mime_guess::from_path(served).first_or_octet_stream();
    let cache = if served == "index.html" {
        "no-cache"
    } else {
        "public, max-age=31536000, immutable"
    };
    (
        StatusCode::OK,
        [(CONTENT_TYPE, mime.as_ref()), (CACHE_CONTROL, cache)],
        file.data.into_owned(),
    )
        .into_response()
}

/// A double-click has no terminal to copy the URL from, so opening the browser
/// is how it reaches the UI. `--web` and `ROXY_WEB_NO_OPEN=1` keep scripted
/// runs silent.
fn open_browser(url: &str) -> std::io::Result<()> {
    #[cfg(target_os = "windows")]
    let mut command = {
        let mut command = std::process::Command::new("cmd");
        command.args(["/C", "start", "", url]);
        command
    };
    #[cfg(target_os = "macos")]
    let mut command = {
        let mut command = std::process::Command::new("open");
        command.arg(url);
        command
    };
    #[cfg(all(unix, not(target_os = "macos")))]
    let mut command = {
        let mut command = std::process::Command::new("xdg-open");
        command.arg(url);
        command
    };
    command.spawn().map(|_| ())
}

/// A bare launch opens the UI in the default browser, but only from an
/// ordinary interactive console: `--web` runs, elevated processes and
/// captured stdout (agent runs, redirected logs) print the address instead.
fn should_open_browser(open: bool, elevated: bool, interactive: bool) -> bool {
    open && !elevated && interactive
}

/// Whether this process holds an elevated token. Opening a browser from an
/// elevated process can start an elevated browser instance, and the user's
/// next normal launch then blocks on the browser's elevation-mismatch prompt
/// (Edge on Windows). A bare launch skips the auto-open in that case.
#[cfg(target_os = "windows")]
fn process_is_elevated() -> bool {
    use std::ffi::c_void;
    use windows_sys::Win32::Foundation::{CloseHandle, HANDLE};
    use windows_sys::Win32::Security::{
        GetTokenInformation, TOKEN_ELEVATION, TOKEN_QUERY, TokenElevation,
    };
    use windows_sys::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};

    let mut token: HANDLE = 0;
    // SAFETY: GetCurrentProcess returns a valid pseudo handle and the token
    // handle is closed before returning.
    unsafe {
        if OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) == 0 {
            return false;
        }
        let mut elevation = TOKEN_ELEVATION { TokenIsElevated: 0 };
        let mut size = 0u32;
        let ok = GetTokenInformation(
            token,
            TokenElevation,
            &mut elevation as *mut TOKEN_ELEVATION as *mut c_void,
            std::mem::size_of::<TOKEN_ELEVATION>() as u32,
            &mut size,
        );
        CloseHandle(token);
        ok != 0 && elevation.TokenIsElevated != 0
    }
}

#[cfg(not(target_os = "windows"))]
fn process_is_elevated() -> bool {
    false
}

fn reset_visual_stores(
    trace: &Arc<Mutex<TraceStore>>,
    series: &mut VecDeque<WebSeriesPoint>,
    rate_window: &mut VecDeque<(Instant, u64)>,
) {
    if let Ok(mut store) = trace.lock() {
        store.reset();
    }
    series.clear();
    rate_window.clear();
}

/// Replaces the live session with a project file and rebuilds every derived
/// view (trace, chart selection, log state) the same way for the session
/// picker and the generated sample.
fn open_project_session(
    app: &mut App,
    path: &std::path::Path,
    signals: &Arc<Mutex<SignalStore>>,
    trace: &Arc<Mutex<TraceStore>>,
    series: &mut VecDeque<WebSeriesPoint>,
    rate_window: &mut VecDeque<(Instant, u64)>,
) {
    if app.snap.measuring {
        app.stop();
    }
    let keys = signals.lock().map(|store| store.keys()).unwrap_or_default();
    for key in keys {
        app.send(BusCommand::Unsubscribe { key });
    }
    if let Ok(mut store) = signals.lock() {
        store.clear_selection();
    }
    app.send(BusCommand::ClearTrace);
    app.send(BusCommand::ClearAggregates);
    app.clear_message_expanded();
    app.open_project_path(path);
    // Same as the bootstrap load: applying the live layout is part of
    // opening the project, not an unsaved edit.
    app.mark_clean();
    if let Ok(mut store) = signals.lock() {
        seed_chart_selection(app, &mut store);
    }
    app.log_path.clear();
    app.log_info = None;
    app.send(BusCommand::SetRunMode(Mode::Virtual));
    reset_visual_stores(trace, series, rate_window);
}

/// Validates a system-variable definition coming from the web form. The core
/// applies the same rules; the host is the boundary, so the web path checks
/// them again instead of trusting the page.
#[allow(clippy::too_many_arguments)]
fn checked_sysvar(
    namespace: String,
    name: String,
    init: f64,
    min: Option<f64>,
    max: Option<f64>,
    unit: String,
    comment: String,
    label_sets: Vec<crate::bus::SysVarLabelSet>,
) -> Result<crate::bus::SysVarDef, String> {
    let namespace = namespace.trim().to_string();
    let name = name.trim().to_string();
    if namespace.is_empty() || name.is_empty() {
        return Err("namespace and name must not be empty".into());
    }
    if namespace.contains("::") || name.contains("::") {
        return Err("namespace and name must not contain \"::\"".into());
    }
    if let (Some(lo), Some(hi)) = (min, max)
        && lo > hi
    {
        return Err(format!("lower bound {lo} is greater than upper bound {hi}"));
    }
    let label_sets = checked_label_sets(label_sets)?;
    Ok(crate::bus::SysVarDef {
        namespace,
        name,
        init,
        min,
        max,
        unit: unit.trim().to_string(),
        comment: comment.trim().to_string(),
        label_sets,
        band_set_id: None,
        labels: Vec::new(),
    })
}

fn checked_label_sets(
    sets: Vec<crate::bus::SysVarLabelSet>,
) -> Result<Vec<crate::bus::SysVarLabelSet>, String> {
    if sets.len() > 8 {
        return Err("at most 8 label sets are allowed".into());
    }
    let mut seen_ids = std::collections::HashSet::new();
    let mut seen_names = std::collections::HashSet::new();
    let mut defaults = 0usize;
    let mut result = Vec::with_capacity(sets.len());
    for mut set in sets {
        set.id = set.id.trim().to_string();
        set.name = set.name.trim().to_string();
        if set.id.is_empty() || set.id.chars().count() > 64 {
            return Err("label set id must be 1–64 characters".into());
        }
        if set.name.is_empty() || set.name.chars().count() > 40 {
            return Err("label set name must be 1–40 characters".into());
        }
        if !seen_ids.insert(set.id.clone()) {
            return Err(format!("duplicate label set id \"{}\"", set.id));
        }
        if !seen_names.insert(set.name.clone()) {
            return Err(format!("duplicate label set name \"{}\"", set.name));
        }
        if set.bands.len() > 64 {
            return Err("at most 64 bands are allowed per set".into());
        }
        let mut bands = Vec::with_capacity(set.bands.len());
        for mut band in set.bands {
            if let Some(upper) = band.upper
                && !upper.is_finite()
            {
                return Err("band bounds must be finite numbers".into());
            }
            band.text = band.text.trim().to_string();
            if band.text.chars().count() > 80 {
                return Err("band text is limited to 80 characters".into());
            }
            if let Some(color) = band.color
                && color > 63
            {
                return Err("band color slot must be 0–63".into());
            }
            bands.push(band);
        }
        if bands.is_empty() {
            return Err(format!(
                "label set \"{}\" needs at least one band",
                set.name
            ));
        }
        if bands.last().is_some_and(|band| band.upper.is_some()) {
            return Err(format!(
                "label set \"{}\" needs an open-ended final band",
                set.name
            ));
        }
        bands.sort_by(|a, b| match (a.upper, b.upper) {
            (Some(left), Some(right)) => left
                .total_cmp(&right)
                .then_with(|| a.inclusive.cmp(&b.inclusive)),
            (Some(_), None) => std::cmp::Ordering::Less,
            (None, Some(_)) => std::cmp::Ordering::Greater,
            (None, None) => std::cmp::Ordering::Equal,
        });
        for pair in bands.windows(2) {
            if pair[0].upper == pair[1].upper && pair[0].inclusive == pair[1].inclusive {
                return Err(format!("duplicate band bound in set \"{}\"", set.name));
            }
        }
        let open = bands.iter().filter(|band| band.upper.is_none()).count();
        if open != 1 {
            return Err(format!(
                "label set \"{}\" needs exactly one open-ended band",
                set.name
            ));
        }
        set.bands = bands;
        if set.is_default {
            defaults += 1;
        }
        result.push(set);
    }
    if defaults > 1 {
        return Err("only one label set can be the default".into());
    }
    if defaults == 0
        && let Some(first) = result.first_mut()
    {
        first.is_default = true;
    }
    Ok(result)
}

/// Applies the editable parts of a Monitor row (not its signal). The row is
/// UI/project state, so the web edit lands in place; only adding/removing a
/// signal touches the bus.
fn edit_monitor_row(
    app: &mut App,
    key: &str,
    digits: Option<usize>,
    bands: Option<Vec<crate::band::Band>>,
) -> Result<(), String> {
    let Some(row) = app
        .monitor_view
        .rows
        .iter_mut()
        .find(|row| row.key.text() == key)
    else {
        return Err("monitor row not found".to_string());
    };
    if let Some(bands) = bands {
        if bands.is_empty() {
            return Err("monitor needs at least one band".to_string());
        }
        for band in &bands {
            if band.upper.is_some_and(|upper| !upper.is_finite()) {
                return Err("monitor bands must be finite".to_string());
            }
            if band.color.is_some_and(|color| color > 63) {
                return Err("monitor color slot must be 0–63".to_string());
            }
        }
        if bands.last().is_some_and(|band| band.upper.is_some()) {
            return Err("monitor needs an open-ended final band".to_string());
        }
        for pair in bands.windows(2) {
            match (pair[0].upper, pair[1].upper) {
                (Some(left), Some(right)) if right < left => {
                    return Err("monitor bands must be ordered".to_string());
                }
                (Some(left), Some(right))
                    if left == right && pair[0].inclusive == pair[1].inclusive =>
                {
                    return Err("monitor bands need unique bounds".to_string());
                }
                (Some(left), Some(right)) if left == right && pair[0].inclusive => {
                    return Err("at one bound, < must come before ≤".to_string());
                }
                _ => {}
            }
        }
        row.bands = bands;
    }
    if let Some(digits) = digits {
        row.digits = digits.min(3);
    }
    Ok(())
}

/// Six-significant-digit state identity, the same quantization the State
/// Tracker uses: tiny numeric jitter does not split a band.
fn quantize_state(v: f64) -> f64 {
    if v == 0.0 || !v.is_finite() {
        return v;
    }
    let mag = v.abs().log10().floor() as i32;
    let digits = (5 - mag).clamp(0, 9);
    let p = 10f64.powi(digits);
    (v * p).round() / p
}

fn state_label(v: f64) -> String {
    format!("{v}")
}

/// Turns visible samples into at most one band per pixel column (360). Each
/// column keeps the last value seen in it, and empty columns carry the last
/// state forward, so a signal that updates once a second still paints its
/// band across the whole window.
fn state_bands_from(
    held: Option<f64>,
    samples: &[(u64, f64)],
    lo: u64,
    hi: u64,
) -> (String, Vec<WebStateBand>) {
    const COLS: usize = 360;
    let span = hi.saturating_sub(lo).max(1);
    let mut values: Vec<Option<f64>> = vec![None; COLS];
    for &(t, v) in samples {
        let idx =
            (((t.saturating_sub(lo)) as u128 * COLS as u128 / span as u128) as usize).min(COLS - 1);
        values[idx] = Some(quantize_state(v));
    }
    let mut last = held.map(quantize_state);
    for slot in &mut values {
        if let Some(v) = *slot {
            last = Some(v);
        } else {
            *slot = last;
        }
    }
    let mut bands = Vec::new();
    let mut open: Option<(f64, usize)> = None;
    for (i, slot) in values.iter().enumerate() {
        let value = *slot;
        match open {
            Some((open_value, _)) if Some(open_value) == value => {}
            Some((open_value, start)) => {
                bands.push(WebStateBand {
                    start: start as f64 / COLS as f64,
                    end: i as f64 / COLS as f64,
                    label: state_label(open_value),
                    color: 0,
                });
                open = value.map(|v| (v, i));
            }
            None => open = value.map(|v| (v, i)),
        }
    }
    if let Some((value, start)) = open {
        bands.push(WebStateBand {
            start: start as f64 / COLS as f64,
            end: 1.0,
            label: state_label(value),
            color: 0,
        });
    }
    // Few states get distinct palette slots in first-seen order; an analog
    // sweep (more states than the palette) is drawn as one neutral band by
    // the frontend instead of a rainbow of one-pixel stripes.
    let mut seen: Vec<&str> = Vec::new();
    for band in &mut bands {
        let slot = seen
            .iter()
            .position(|label| *label == band.label)
            .unwrap_or_else(|| {
                seen.push(&band.label);
                seen.len() - 1
            });
        band.color = slot % 8;
    }
    let current = last.map(state_label).unwrap_or_else(|| "-".to_string());
    (current, bands)
}

