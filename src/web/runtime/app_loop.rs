/// Bind or clear a Monitor row's BandSet. `None` and an empty id both mean
/// "no bands", so Apply from the Settings dialog can actually unbind.
fn bind_monitor_band_set(app: &mut App, key: &str, band_set_id: Option<String>) {
    let binding = band_set_id.filter(|id| !id.is_empty());
    if let Some(row) = app
        .monitor_view
        .rows
        .iter_mut()
        .find(|row| row.key.text() == key)
    {
        row.band_set_id = binding;
    }
}

fn app_loop(
    inbox: Receiver<WebCommand>,
    stores: SharedStores,
    project: Option<String>,
    replay: Option<String>,
    profile: Option<String>,
) {
    let SharedStores {
        shared,
        trace,
        signals,
        catalog,
        scripts,
    } = stores;
    let mut app = if project.is_none() && replay.is_none() {
        App::blank()
    } else {
        App::headless()
    };
    prepare_session(&mut app, project.as_deref(), replay.as_deref());
    if let (Some(project), Some(profile)) = (&project, &profile) {
        let dir = std::path::Path::new(project)
            .parent()
            .unwrap_or_else(|| std::path::Path::new("."));
        match crate::profile::apply_profile(&mut app, dir, profile) {
            Ok(summary) => app.status = format!("{}; profile: {summary}", app.status),
            Err(error) => app.fail(format!("profile failed: {error}")),
        }
    }
    if let Ok(mut store) = signals.lock() {
        seed_chart_selection(&mut app, &mut store);
    }
    // The web plot is independent of observer zoom. Ask the core for the
    // 5 ms stress stride directly instead of mutating project config.
    app.plot_stride_us = Some(5_000);
    let mut catalog_sig = String::new();
    refresh_catalog(&app, &catalog, &mut catalog_sig);
    let mut series: VecDeque<WebSeriesPoint> = VecDeque::with_capacity(SERIES_POINTS);
    let mut rate_window: VecDeque<(Instant, u64)> = VecDeque::new();
    let mut last_publish = Instant::now() - PUBLISH_EVERY;
    let mut scripts_sig = u64::MAX;

    loop {
        while let Ok(cmd) = inbox.try_recv() {
            match cmd {
                WebCommand::Start => {
                    if !app.snap.measuring {
                        if app.snap.run_mode == Mode::Replay && app.log_path.is_empty() {
                            app.fail("replay needs a log; start with --replay <path>");
                        } else {
                            app.start_selected();
                            rate_window.clear();
                        }
                    }
                }
                WebCommand::Stop => {
                    if app.snap.measuring {
                        app.stop();
                        rate_window.clear();
                    }
                }
                WebCommand::Pause { on } => {
                    app.send(BusCommand::SetTracePaused(on));
                    // The rate window spans wall time; do not let a pause or
                    // resume turn the frozen interval into a false sample.
                    rate_window.clear();
                }
                WebCommand::ReplayStep { delta_us } => {
                    app.send(BusCommand::ReplayStep { delta_us });
                }
                WebCommand::ReplayPlayTo { target_us } => {
                    app.send(BusCommand::ReplayPlayTo { target_us });
                }
                WebCommand::ReplayPlayFor { duration_us } => {
                    app.send(BusCommand::ReplayPlayFor { duration_us });
                }
                WebCommand::SetReplaySpeed { speed } => {
                    let speed = if speed.is_finite() && speed > 0.0 {
                        speed
                    } else {
                        1.0
                    };
                    app.replay_speed = speed;
                    app.send(BusCommand::SetReplaySpeed(speed));
                }
                WebCommand::SetReplayChannel { kind, raw, target } => {
                    let Some(kind) = crate::replay_map::LogBusKind::parse(&kind) else {
                        app.fail(format!("unknown replay bus kind: {kind}"));
                        continue;
                    };
                    if let Err(message) = apply_replay_channel(&mut app, kind, raw, target) {
                        app.fail(message);
                    }
                }
                WebCommand::RecordToggle { name, format } => {
                    if let Some(name) = name {
                        app.record_path_buf = name;
                    }
                    if let Some(format) = format {
                        app.record_format = match format {
                            1 => 1,
                            2 => 2,
                            _ => 0,
                        };
                    }
                    app.toggle_record();
                }
                WebCommand::SetRecordSettings { name, format } => {
                    app.record_path_buf = name;
                    app.record_format = match format {
                        1 => 1,
                        2 => 2,
                        _ => 0,
                    };
                    app.sync_record_path();
                }
                WebCommand::SetRecordFilter { text } => {
                    app.record_filter_text = text.clone();
                    app.send(BusCommand::SetRecordFilter {
                        filter: parse_record_filter(&text),
                    });
                }
                WebCommand::ClearTrace => {
                    app.send(BusCommand::ClearTrace);
                }
                WebCommand::ClearMessages => {
                    app.clear_messages();
                }
                WebCommand::SetAllMessagesExpanded { on } => {
                    app.set_all_messages_expanded(on);
                }
                WebCommand::SetMessageExpanded { key, on } => {
                    app.set_message_expanded(&key, on);
                }
                WebCommand::RunPanelAction { name, value } => {
                    let name = name.trim().to_string();
                    if name.is_empty() {
                        app.fail("[panel] action name is empty");
                    } else if name.chars().count() > 80 {
                        app.fail("[panel] action name is too long");
                    } else if value.is_some_and(|value| !value.is_finite()) {
                        app.fail("[panel] action value must be a number");
                    } else {
                        app.send(BusCommand::PanelAction { name, value });
                    }
                }
                WebCommand::PanelWriteSignal { source, value } => {
                    match app.resolve_key_text(source.trim()) {
                        Some(key)
                            if value.is_finite()
                                && matches!(
                                    key,
                                    crate::observe::SigKey::Can { .. }
                                        | crate::observe::SigKey::Fr { .. }
                                        | crate::observe::SigKey::Eth { .. }
                                ) =>
                        {
                            app.send(BusCommand::PanelWriteSignal { key, value });
                        }
                        Some(crate::observe::SigKey::Derived { .. }) => {
                            app.fail("[panel] derived signals are read-only");
                        }
                        Some(_) => {
                            app.fail("[panel] setpoint value must be a number");
                        }
                        None => {
                            app.fail("[panel] setpoint source must be a signal key");
                        }
                    }
                }
                WebCommand::PanelSendFrame {
                    channel,
                    id,
                    extended,
                    data,
                    physical,
                    signals,
                } => {
                    app.send(BusCommand::PanelSendFrame {
                        channel,
                        id,
                        extended,
                        data,
                        physical,
                        signals,
                    });
                }
                WebCommand::PanelSendFrFrame {
                    bus,
                    slot,
                    data,
                    physical,
                    signals,
                } => {
                    app.send(BusCommand::PanelSendFrFrame {
                        bus,
                        slot,
                        data,
                        physical,
                        signals,
                    });
                }
                WebCommand::PanelSendSomeIp {
                    bus,
                    service,
                    method,
                    client,
                    session,
                    message_type,
                    data,
                    values,
                    texts,
                    transport,
                    host,
                    port,
                } => {
                    app.send(BusCommand::PanelSendSomeIp {
                        bus,
                        service,
                        method,
                        client,
                        session,
                        message_type,
                        data,
                        values,
                        texts,
                        transport,
                        host,
                        port,
                    });
                }
                WebCommand::PanelSendEthFrame {
                    bus,
                    frame,
                    data,
                    physical,
                    signals,
                } => {
                    app.send(BusCommand::PanelSendEthFrame {
                        bus,
                        frame,
                        data,
                        physical,
                        signals,
                    });
                }
                WebCommand::OpenProject { path } => {
                    open_project_session(
                        &mut app,
                        std::path::Path::new(&path),
                        &signals,
                        &trace,
                        &mut series,
                        &mut rate_window,
                    );
                }
                WebCommand::CreateSampleProject { parent } => {
                    let parent = std::path::PathBuf::from(parent);
                    match write_sample_project(&parent) {
                        Ok(path) => {
                            open_project_session(
                                &mut app,
                                &path,
                                &signals,
                                &trace,
                                &mut series,
                                &mut rate_window,
                            );
                            app.status = format!("sample project created: {}", path.display());
                        }
                        Err(e) => app.fail(format!("sample project failed: {e}")),
                    }
                }
                WebCommand::SetRunMode { mode } => {
                    let target = match mode.to_ascii_lowercase().as_str() {
                        "simulation" | "virtual" => Some(Mode::Virtual),
                        "replay" => Some(Mode::Replay),
                        _ => None,
                    };
                    match target {
                        Some(target) => {
                            if app.snap.measuring {
                                app.stop();
                            }
                            app.send(BusCommand::SetRunMode(target));
                            app.status = match target {
                                Mode::Virtual => "simulation mode".to_string(),
                                Mode::Replay if app.log_path.is_empty() => {
                                    "replay mode · load a log".to_string()
                                }
                                Mode::Replay => "replay mode".to_string(),
                            };
                        }
                        None => app.fail(format!("unknown run mode: {mode}")),
                    }
                }
                WebCommand::OpenLog { path, play } => {
                    if app.snap.measuring {
                        app.stop();
                    }
                    app.load_log(&path);
                    if app.log_path == path && !app.status.contains("failed") {
                        app.clear_message_expanded();
                        let keys = signals.lock().map(|store| store.keys()).unwrap_or_default();
                        for key in &keys {
                            app.subscribe(key.clone());
                        }
                        if let Ok(mut store) = signals.lock() {
                            store.reset();
                        }
                        app.send(BusCommand::ClearTrace);
                        app.send(BusCommand::ClearAggregates);
                        app.send(BusCommand::SetRunMode(Mode::Replay));
                        reset_visual_stores(&trace, &mut series, &mut rate_window);
                        if play {
                            app.start_selected();
                        }
                    }
                }
                WebCommand::AddReplayFolder { path } => match app.add_replay_folder(path.clone()) {
                    Ok(count) => {
                        app.status = format!("replay folder added: {path} · {count} recording(s)");
                    }
                    Err(e) => app.fail(e),
                },
                WebCommand::AddReplayFile { path } => match app.add_replay_file(path.clone()) {
                    Ok(count) => {
                        app.status = format!("replay file added: {path} · {count} recording(s)");
                    }
                    Err(e) => app.fail(e),
                },
                WebCommand::RemoveReplaySource { path, folder } => {
                    app.remove_replay_source(&path, folder);
                    app.status = "replay source removed".to_string();
                }
                WebCommand::RefreshReplaySources => {
                    app.refresh_replay_sources();
                    app.status = format!(
                        "replay list refreshed · {} recording(s)",
                        app.replay_scan.len()
                    );
                }
                WebCommand::SelectSignal {
                    ch,
                    id,
                    ext,
                    name,
                    read,
                } => {
                    let key = app.can_key(ch, id, ext, name);
                    if let Ok(mut store) = signals.lock() {
                        set_signal_mode(&mut app, &mut store, key, read);
                    }
                }
                WebCommand::UnselectSignal { ch, id, ext, name } => {
                    let key = app.can_key(ch, id, ext, name);
                    let selected = signals
                        .lock()
                        .map(|store| store.is_selected(&key))
                        .unwrap_or(false);
                    if selected && let Ok(mut store) = signals.lock() {
                        store.unselect(&key);
                        app.set_session_chart_keep(store.keys());
                        chart_unselect(&mut app, &key);
                    }
                }
                WebCommand::SelectFrSignal {
                    bus,
                    slot,
                    name,
                    read,
                } => {
                    let key = app.fr_key(bus, slot, name);
                    if let Ok(mut store) = signals.lock() {
                        set_signal_mode(&mut app, &mut store, key, read);
                    }
                }
                WebCommand::UnselectFrSignal { bus, slot, name } => {
                    let key = app.fr_key(bus, slot, name);
                    let selected = signals
                        .lock()
                        .map(|store| store.is_selected(&key))
                        .unwrap_or(false);
                    if selected && let Ok(mut store) = signals.lock() {
                        store.unselect(&key);
                        app.set_session_chart_keep(store.keys());
                        chart_unselect(&mut app, &key);
                    }
                }
                WebCommand::SelectEthSignal {
                    bus,
                    frame,
                    name,
                    read,
                } => {
                    let key = app.eth_key(bus, frame, name);
                    if let Ok(mut store) = signals.lock() {
                        set_signal_mode(&mut app, &mut store, key, read);
                    }
                }
                WebCommand::UnselectEthSignal { bus, frame, name } => {
                    let key = app.eth_key(bus, frame, name);
                    let selected = signals
                        .lock()
                        .map(|store| store.is_selected(&key))
                        .unwrap_or(false);
                    if selected && let Ok(mut store) = signals.lock() {
                        store.unselect(&key);
                        app.set_session_chart_keep(store.keys());
                        chart_unselect(&mut app, &key);
                    }
                }
                WebCommand::SelectDerivedSignal { id, name, read } => {
                    let key = SigKey::derived(id, name);
                    if let Ok(mut store) = signals.lock() {
                        set_signal_mode(&mut app, &mut store, key, read);
                    }
                }
                WebCommand::UnselectDerivedSignal { id, name } => {
                    let key = SigKey::derived(id, name);
                    let selected = signals
                        .lock()
                        .map(|store| store.is_selected(&key))
                        .unwrap_or(false);
                    if selected && let Ok(mut store) = signals.lock() {
                        store.unselect(&key);
                        app.set_session_chart_keep(store.keys());
                        chart_unselect(&mut app, &key);
                    }
                }
                WebCommand::MoveSignal { key, to } => {
                    let target = signals.lock().ok().and_then(|store| {
                        store
                            .chart_keys()
                            .into_iter()
                            .find(|candidate| series_key(candidate) == key)
                    });
                    if let Some(target) = target
                        && let Ok(mut store) = signals.lock()
                    {
                        chart_move(&mut app, &mut store, &target, to);
                    }
                }
                WebCommand::ChartSignal { key, action } => {
                    let selected = signals.lock().ok().and_then(|store| {
                        store
                            .keys()
                            .into_iter()
                            .find(|candidate| series_key(candidate) == key)
                    });
                    // Derived lanes are not part of the selection list; a key
                    // shaped `derived:<id>` names one straight from the defs.
                    let target = selected.or_else(|| {
                        app.derived_defs
                            .iter()
                            .map(|def| SigKey::derived(&def.id, &def.name))
                            .find(|candidate| series_key(candidate) == key)
                    });
                    if let Some(target) = target
                        && let Ok(mut store) = signals.lock()
                        && !chart_signal_action(&mut app, &mut store, &target, &action)
                    {
                        app.fail(format!("unknown chart signal action: {action}"));
                    }
                }
                WebCommand::AttachHardware { bus, driver, index } => {
                    let Some(driver) = parse_driver(&driver) else {
                        app.fail(format!("unknown hardware driver: {driver}"));
                        continue;
                    };
                    let Some(channel) = app.snap.channels.get(bus as usize) else {
                        app.fail(format!("bus {bus} does not exist"));
                        continue;
                    };
                    let (arb, data) = (channel.bitrate_kbps, channel.fd_data_kbps);
                    app.hw_pick.insert(bus, (driver, index));
                    app.set_hardware_channel(bus, driver, index, arb, Some(data));
                }
                WebCommand::DetachHardware { bus } => {
                    app.detach_hardware(bus);
                }
                WebCommand::AttachEthPort { bus, device } => {
                    app.send(BusCommand::AttachEthPort { bus, device });
                }
                WebCommand::DetachEthPort { bus } => {
                    app.send(BusCommand::DetachEthPort { bus });
                }
                WebCommand::AttachFrHardware { bus, index } => {
                    app.attach_fr_watch_on(bus, index);
                }
                WebCommand::DetachFrHardware { bus } => {
                    app.detach_flexray_watch(bus);
                }
                WebCommand::RefreshHardware => {
                    app.send(BusCommand::RefreshHardware);
                }
                WebCommand::SetBusMode { real } => {
                    app.send(BusCommand::SetBusMode { real });
                }
                WebCommand::AddBus { kind } => {
                    if app.snap.measuring {
                        app.stop();
                    }
                    match kind.as_str() {
                        "flexray" | "fr" => {
                            app.add_flexray_bus();
                        }
                        "ethernet" | "eth" => {
                            app.add_eth_bus();
                        }
                        _ => app.add_empty_channel(),
                    }
                }
                WebCommand::RemoveBus { ch } => {
                    if app.snap.measuring {
                        app.stop();
                    }
                    let removed = app
                        .snap
                        .cluster_ids
                        .can
                        .get(ch as usize)
                        .copied()
                        .map(crate::model::ClusterId);
                    app.remove_channel(ch as usize);
                    if let Ok(mut store) = signals.lock() {
                        store.remap_after_remove(removed);
                    }
                    if let Ok(mut store) = trace.lock() {
                        store.reset();
                    }
                    series.clear();
                    rate_window.clear();
                }
                WebCommand::RenameBus { ch, name } => {
                    app.send(BusCommand::SetChannelConfig {
                        ch,
                        name: Some(name),
                        dbc_path: None,
                        bitrate_kbps: None,
                        fd_data_kbps: None,
                        node_roles: None,
                    });
                }
                WebCommand::SetBusRates {
                    ch,
                    bitrate_kbps,
                    fd_data_kbps,
                } => {
                    app.send(BusCommand::SetChannelConfig {
                        ch,
                        name: None,
                        dbc_path: None,
                        bitrate_kbps: Some(bitrate_kbps),
                        fd_data_kbps: Some(fd_data_kbps),
                        node_roles: None,
                    });
                }
                WebCommand::ScanArxml { path } => {
                    app.scan_arxml(&path);
                }
                WebCommand::AddBusDescription {
                    bus,
                    kind,
                    path,
                    cluster,
                } => {
                    if app.snap.measuring {
                        app.stop();
                    }
                    let cluster = (!cluster.trim().is_empty()).then_some(cluster.as_str());
                    match kind.as_str() {
                        "can" if path.to_ascii_lowercase().ends_with(".dbc") => {
                            app.add_can_dbc(bus, &path);
                        }
                        "can" => {
                            if let Some(cluster) = cluster
                                && let Err(e) = app.load_can_arxml_on_bus(&path, cluster, bus)
                            {
                                app.fail(format!("CAN{}: {e}", bus + 1));
                            }
                        }
                        "flexray" | "fr" => {
                            let source = crate::channel::description_source(&path, cluster);
                            if let Err(e) = app.add_fr_description(bus, source) {
                                app.fail(format!("FR{bus}: {e}"));
                            }
                        }
                        "ethernet" | "eth" => {
                            let source = crate::channel::description_source(&path, cluster);
                            if let Err(e) = app.add_eth_description(bus, source) {
                                app.fail(format!("ETH{bus}: {e}"));
                            }
                        }
                        other => app.fail(format!("unknown bus kind: {other}")),
                    }
                }
                WebCommand::RemoveBusDescription {
                    bus,
                    kind,
                    path,
                    cluster,
                } => {
                    if app.snap.measuring {
                        app.stop();
                    }
                    match kind.as_str() {
                        "can" if path.to_ascii_lowercase().ends_with(".dbc") => {
                            app.remove_can_dbc(bus, &path);
                        }
                        "can" => app.remove_can_arxml(bus, &path, &cluster),
                        "flexray" | "fr" => app.remove_fr_description(bus, &path, &cluster),
                        "ethernet" | "eth" => app.remove_eth_description(bus, &path, &cluster),
                        other => app.fail(format!("unknown bus kind: {other}")),
                    }
                }
                WebCommand::SaveProject { path, panels } => {
                    app.panel_pages = panels;
                    let target = path
                        .filter(|value| !value.trim().is_empty())
                        .map(|value| std::path::PathBuf::from(download_file_path(&value)))
                        .or_else(|| app.project_path.clone());
                    match target {
                        Some(target) => {
                            app.save_project(Some(target));
                        }
                        None => {
                            app.fail("save needs a project path");
                        }
                    }
                }
                WebCommand::AddScript {
                    name,
                    channel,
                    attached_bus,
                    attached_node,
                } => {
                    let attached = match (attached_bus, attached_node) {
                        (Some(bus), Some(node)) => Some((bus, node)),
                        _ => None,
                    };
                    app.send(BusCommand::AddNode {
                        name,
                        channel,
                        attached,
                    });
                }
                WebCommand::SetScriptName { id, name } => {
                    app.send(BusCommand::SetNodeName { id, name });
                }
                WebCommand::SetScriptEnabled { id, on } => {
                    app.send(BusCommand::SetNodeEnabled { id, on });
                }
                WebCommand::RemoveScript { id } => {
                    app.send(BusCommand::RemoveNode { id });
                }
                WebCommand::LoadScript { id, path } => {
                    if path.trim().is_empty() {
                        app.fail("script load needs a path");
                        continue;
                    }
                    match std::fs::read_to_string(&path) {
                        Ok(source) => {
                            app.status = format!("script loaded: {path}");
                            app.send(BusCommand::SetNodeFile {
                                id,
                                path,
                                file_source: source,
                            });
                        }
                        Err(error) => app.fail(format!("script load failed: {error}")),
                    }
                }
                WebCommand::SaveScript { id, source } => {
                    let path = app
                        .snap
                        .nodes
                        .iter()
                        .find(|node| node.id == id)
                        .and_then(|node| node.file_path.clone());
                    let Some(path) = path.filter(|path| !path.trim().is_empty()) else {
                        app.fail("script has no file; use Save As");
                        continue;
                    };
                    match std::fs::write(&path, &source) {
                        Ok(()) => {
                            app.status = format!("script saved: {path}");
                            app.send(crate::bus::BusCommand::SetNodeFile {
                                id,
                                path,
                                file_source: source,
                            });
                        }
                        Err(error) => app.fail(format!("script save failed: {error}")),
                    }
                }
                WebCommand::SaveScriptAs { id, path, source } => {
                    if path.trim().is_empty() {
                        app.fail("script save needs a path");
                        continue;
                    }
                    match std::fs::write(&path, &source) {
                        Ok(()) => {
                            app.status = format!("script saved: {path}");
                            app.send(BusCommand::SetNodeFile {
                                id,
                                path,
                                file_source: source,
                            });
                        }
                        Err(error) => app.fail(format!("script save failed: {error}")),
                    }
                }
                WebCommand::CheckScript { id, source } => {
                    app.send(BusCommand::CheckNodeSource { id, source });
                }
                WebCommand::ApplyScript { id, source } => {
                    let linked = app
                        .snap
                        .nodes
                        .iter()
                        .find(|node| node.id == id)
                        .is_some_and(|node| {
                            node.file_path.as_deref().is_some_and(|path| !path.trim().is_empty())
                        });
                    if linked {
                        app.send(BusCommand::SetNodeSource { id, source });
                    } else {
                        app.fail("script has no .rxcan file; use Save As before Apply");
                    }
                }
                WebCommand::SetSpecTolerance { value } => {
                    app.spec_tol_pct = value.min(100);
                }
                WebCommand::SetSpecGrace { value } => {
                    app.spec_grace = value.clamp(1, 20);
                }
                WebCommand::ClearSpec => {
                    app.send(BusCommand::ClearSpec);
                }
                WebCommand::Export {
                    kind,
                    path,
                    from_us,
                    to_us,
                } => {
                    let range = match (from_us, to_us) {
                        (Some(from), Some(to)) => Some((from, to)),
                        _ => None,
                    };
                    let target = download_file_path(&path);
                    if kind == "signals" {
                        let keys = signals.lock().map(|store| store.keys()).unwrap_or_default();
                        app.export_signals_keys_csv(&keys, &target, range);
                    } else {
                        export_file(&mut app, &kind, &target, range);
                    }
                }
                WebCommand::SetSysVar {
                    namespace,
                    name,
                    value,
                } => {
                    app.send(BusCommand::SetSysVar {
                        namespace,
                        name,
                        value,
                    });
                }
                WebCommand::DefineSysVar {
                    namespace,
                    name,
                    init,
                    min,
                    max,
                    unit,
                    comment,
                    label_sets,
                    band_set_id,
                    old_namespace,
                    old_name,
                } => {
                    match checked_sysvar(namespace, name, init, min, max, unit, comment, label_sets)
                    {
                        Ok(mut def) => {
                            def.band_set_id = band_set_id;
                            // A rename deletes the old key first: leaving it
                            // would keep a phantom variable scripts could write.
                            if let (Some(old_ns), Some(old_name)) = (old_namespace, old_name)
                                && (old_ns != def.namespace || old_name != def.name)
                            {
                                app.send(BusCommand::DeleteSysVar {
                                    namespace: old_ns,
                                    name: old_name,
                                });
                            }
                            app.send(BusCommand::DefineSysVar(def));
                        }
                        Err(error) => app.fail(format!("sysvar: {error}")),
                    }
                }
                WebCommand::DeleteSysVar { namespace, name } => {
                    app.send(BusCommand::DeleteSysVar { namespace, name });
                }
                WebCommand::DefineBandSet {
                    id,
                    name,
                    bands,
                    old_id,
                } => match checked_band_set(id, name, bands) {
                    Ok(set) => {
                        if let Some(old) = old_id
                            && old != set.id
                        {
                            app.band_sets.retain(|entry| entry.id != old);
                        }
                        app.band_sets.retain(|entry| entry.id != set.id);
                        for rule in app
                            .state_trackers
                            .iter_mut()
                            .flat_map(|window| window.rules.values_mut())
                        {
                            if rule.band_set_id.as_deref() == Some(set.id.as_str()) {
                                rule.bands = set.bands.clone();
                            }
                        }
                        for def in &mut app.derived_defs {
                            if def.band_set_id.as_deref() == Some(set.id.as_str()) {
                                def.bands = set.bands.clone();
                            }
                        }
                        app.band_sets.push(set);
                    }
                    Err(error) => app.fail(format!("bands: {error}")),
                },
                WebCommand::DeleteBandSet { id } => {
                    let unbound: Vec<(String, String)> = app
                        .snap
                        .sysvars
                        .iter()
                        .filter(|def| def.def.band_set_id.as_deref() == Some(id.as_str()))
                        .map(|def| (def.def.namespace.clone(), def.def.name.clone()))
                        .collect();
                    for (namespace, name) in unbound {
                        app.send(BusCommand::SetSysVarBandSet {
                            namespace,
                            name,
                            band_set_id: None,
                        });
                    }
                    app.band_sets.retain(|set| set.id != id);
                    for row in &mut app.monitor_view.rows {
                        if row.band_set_id.as_deref() == Some(id.as_str()) {
                            row.band_set_id = None;
                            row.bands.clear();
                        }
                    }
                    for window in &mut app.state_trackers {
                        for rule in window.rules.values_mut() {
                            if rule.band_set_id.as_deref() == Some(id.as_str()) {
                                rule.band_set_id = None;
                                rule.bands.clear();
                            }
                        }
                    }
                    for def in &mut app.derived_defs {
                        if def.band_set_id.as_deref() == Some(id.as_str()) {
                            def.band_set_id = None;
                            def.bands.clear();
                        }
                    }
                }
                WebCommand::AddMonitor {
                    kind,
                    ch,
                    id,
                    ext,
                    frame,
                    name,
                } => {
                    let Some(key) = command_key(&app, &kind, ch, id, ext, frame, name) else {
                        app.fail(format!("unknown signal kind: {kind}"));
                        continue;
                    };
                    app.set_win_signal(crate::workspace::PopupTarget::Monitor, key, true);
                }
                WebCommand::RemoveMonitor {
                    kind,
                    ch,
                    id,
                    ext,
                    frame,
                    name,
                } => {
                    let Some(key) = command_key(&app, &kind, ch, id, ext, frame, name) else {
                        app.fail(format!("unknown signal kind: {kind}"));
                        continue;
                    };
                    sync_chart_keep(&mut app, &signals);
                    app.set_win_signal(crate::workspace::PopupTarget::Monitor, key, false);
                }
                WebCommand::ClearMonitor => {
                    let keys: Vec<SigKey> = app
                        .monitor_view
                        .rows
                        .iter()
                        .map(|row| row.key.clone())
                        .collect();
                    sync_chart_keep(&mut app, &signals);
                    for key in keys {
                        app.set_win_signal(crate::workspace::PopupTarget::Monitor, key, false);
                    }
                }
                WebCommand::ConfigureMonitor {
                    key,
                    digits,
                    bands,
                    band_set_id,
                } => {
                    if let Err(error) = edit_monitor_row(&mut app, &key, digits, bands) {
                        app.fail(error);
                    } else {
                        bind_monitor_band_set(&mut app, &key, band_set_id);
                    }
                }
                WebCommand::SetMonitorColumns { columns } => {
                    if !(1..=2).contains(&columns) {
                        app.fail("monitor columns must be 1 or 2");
                    } else {
                        app.set_monitor_columns(columns);
                    }
                }
                WebCommand::MoveMonitorRow { key, to } => {
                    if let Some(key) = SigKey::from_text(&key) {
                        app.move_monitor_row(&key, to);
                    }
                }
                WebCommand::AddData {
                    kind,
                    ch,
                    id,
                    ext,
                    frame,
                    name,
                } => {
                    let Some(key) = command_key(&app, &kind, ch, id, ext, frame, name) else {
                        app.fail(format!("unknown signal kind: {kind}"));
                        continue;
                    };
                    if app.data_windows.is_empty() {
                        app.new_data_window();
                    }
                    app.set_win_signal(crate::workspace::PopupTarget::Data(0), key, true);
                }
                WebCommand::RemoveData {
                    kind,
                    ch,
                    id,
                    ext,
                    frame,
                    name,
                } => {
                    let Some(key) = command_key(&app, &kind, ch, id, ext, frame, name) else {
                        app.fail(format!("unknown signal kind: {kind}"));
                        continue;
                    };
                    sync_chart_keep(&mut app, &signals);
                    app.set_win_signal(crate::workspace::PopupTarget::Data(0), key, false);
                }
                WebCommand::ClearData => {
                    let keys: Vec<SigKey> = app
                        .data_windows
                        .first()
                        .map(|window| {
                            window
                                .signals
                                .iter()
                                .map(|signal| signal.key.clone())
                                .collect()
                        })
                        .unwrap_or_default();
                    sync_chart_keep(&mut app, &signals);
                    for key in keys {
                        app.set_win_signal(crate::workspace::PopupTarget::Data(0), key, false);
                    }
                }
                WebCommand::MoveDataRow { key, to } => {
                    if let Some(key) = SigKey::from_text(&key) {
                        app.move_data_row(&key, to);
                    }
                }
                WebCommand::SetDataVisible {
                    kind,
                    ch,
                    id,
                    ext,
                    frame,
                    name,
                    visible,
                } => {
                    let Some(key) = command_key(&app, &kind, ch, id, ext, frame, name) else {
                        app.fail(format!("unknown signal kind: {kind}"));
                        continue;
                    };
                    if let Some(signal) = app
                        .data_windows
                        .first_mut()
                        .and_then(|window| window.signals.iter_mut().find(|row| row.key == key))
                    {
                        signal.visible = visible;
                    }
                }
                WebCommand::AddState {
                    kind,
                    ch,
                    id,
                    ext,
                    frame,
                    name,
                } => {
                    let Some(key) = command_key(&app, &kind, ch, id, ext, frame, name) else {
                        app.fail(format!("unknown signal kind: {kind}"));
                        continue;
                    };
                    if app.state_trackers.is_empty() {
                        app.new_state_window();
                    }
                    app.set_win_signal(crate::workspace::PopupTarget::State(0), key, true);
                }
                WebCommand::RemoveState {
                    kind,
                    ch,
                    id,
                    ext,
                    frame,
                    name,
                } => {
                    let Some(key) = command_key(&app, &kind, ch, id, ext, frame, name) else {
                        app.fail(format!("unknown signal kind: {kind}"));
                        continue;
                    };
                    sync_chart_keep(&mut app, &signals);
                    app.set_win_signal(crate::workspace::PopupTarget::State(0), key, false);
                }
                WebCommand::ClearState => {
                    let keys: Vec<SigKey> = app
                        .state_trackers
                        .first()
                        .map(|window| {
                            window
                                .signals
                                .iter()
                                .map(|signal| signal.key.clone())
                                .collect()
                        })
                        .unwrap_or_default();
                    sync_chart_keep(&mut app, &signals);
                    for key in keys {
                        app.set_win_signal(crate::workspace::PopupTarget::State(0), key, false);
                    }
                }
                WebCommand::SetStateWindow { seconds } => {
                    if let Some(window) = app.state_trackers.first_mut() {
                        window.time_window_s = seconds.clamp(0.1, 3600.0);
                    }
                }
                WebCommand::SetStateRule {
                    kind,
                    ch,
                    id,
                    ext,
                    frame,
                    name,
                    bands,
                    band_set_id,
                } => {
                    let Some(key) = command_key(&app, &kind, ch, id, ext, frame, name) else {
                        app.fail(format!("unknown signal kind: {kind}"));
                        continue;
                    };
                    set_state_rule(&mut app, &key, bands, band_set_id);
                }
                WebCommand::ClearStateRule {
                    kind,
                    ch,
                    id,
                    ext,
                    frame,
                    name,
                } => {
                    let Some(key) = command_key(&app, &kind, ch, id, ext, frame, name) else {
                        app.fail(format!("unknown signal kind: {kind}"));
                        continue;
                    };
                    clear_state_rule(&mut app, &key);
                }
                WebCommand::SetEntryActive { ch, id, on } => {
                    app.send(BusCommand::SetEntryActive { ch, id, on });
                }
                WebCommand::SetEntryCycle { ch, id, cycle_us } => {
                    app.send(BusCommand::SetEntryCycle { ch, id, cycle_us });
                }
                WebCommand::SetEntryFd { ch, id, fd } => {
                    app.send(BusCommand::SetEntryFd { ch, id, fd });
                }
                WebCommand::SetEntryBrs { ch, id, brs } => {
                    app.send(BusCommand::SetEntryBrs { ch, id, brs });
                }
                WebCommand::SetEntryHex { ch, id, text } => {
                    app.send(BusCommand::SetEntryHex { ch, id, text });
                }
                WebCommand::SetEntrySource { ch, id, src } => {
                    let Some(kind) = crate::sim::KINDS
                        .iter()
                        .copied()
                        .find(|kind| kind.label().eq_ignore_ascii_case(&src.kind))
                    else {
                        app.fail(format!("unknown source kind: {}", src.kind));
                        continue;
                    };
                    let name = src.name.trim().to_string();
                    if name.is_empty() {
                        app.fail("source needs a signal name");
                        continue;
                    }
                    app.send(BusCommand::SetEntrySource {
                        ch,
                        id,
                        src: crate::sim::ValueSrc {
                            name,
                            kind,
                            lo: src.lo,
                            hi: src.hi,
                            period_us: src.period_us,
                            phase_us: src.phase_us,
                            seq: src.seq,
                            seed: src.seed,
                            redraw_us: src.redraw_us,
                        },
                    });
                }
                WebCommand::ClearEntrySource { ch, id, name } => {
                    app.send(BusCommand::ClearEntrySource { ch, id, name });
                }
                WebCommand::SetNodeRole {
                    bus,
                    node,
                    simulated,
                } => {
                    app.send(BusCommand::SetNodeRole {
                        ch: bus,
                        node,
                        role: if simulated {
                            crate::app::NodeRole::Simulated
                        } else {
                            crate::app::NodeRole::Absent
                        },
                    });
                }
                WebCommand::RemoveFrBus { bus } => {
                    if app.snap.measuring {
                        app.stop();
                    }
                    app.remove_fr_bus(bus);
                }
                WebCommand::RemoveEthBus { bus } => {
                    if app.snap.measuring {
                        app.stop();
                    }
                    app.remove_eth_bus(bus);
                }
                WebCommand::AddReplayBlock {
                    channel,
                    name,
                    path,
                    node,
                } => {
                    let node = node.filter(|value| !value.trim().is_empty());
                    let name = if name.trim().is_empty() {
                        format!("Block {}", app.snap.blocks.len() + 1)
                    } else {
                        name
                    };
                    let attached = node.clone().map(|node| (channel, node));
                    app.add_replay_block(channel, name, path, node, attached);
                }
                WebCommand::SetReplayBlock {
                    id,
                    channel,
                    name,
                    path,
                    node,
                    ids,
                } => {
                    let node = node.filter(|value| !value.trim().is_empty());
                    let name = if name.trim().is_empty() {
                        app.snap
                            .blocks
                            .iter()
                            .find(|block| block.id == id)
                            .map(|block| block.name.clone())
                            .unwrap_or_else(|| "Block".to_string())
                    } else {
                        name
                    };
                    app.set_replay_block(id, channel, name, path, node, ids);
                }
                WebCommand::SetReplayBlockEnabled { id, on } => {
                    app.set_replay_block_enabled(id, on);
                }
                WebCommand::RemoveReplayBlock { id } => {
                    app.remove_replay_block(id);
                }
                WebCommand::DuplicateReplayBlock { id } => {
                    app.duplicate_replay_block(id);
                }
                WebCommand::AddFrEntry { bus, slot } => {
                    app.add_fr_tx(bus, slot);
                }
                WebCommand::RemoveFrEntry { bus, slot } => {
                    app.send(BusCommand::RemoveFrEntry { bus, slot });
                }
                WebCommand::SetFrEntryActive { bus, slot, on } => {
                    app.send(BusCommand::SetFrEntryActive { bus, slot, on });
                }
                WebCommand::SetFrEntryCycle {
                    bus,
                    slot,
                    cycle_us,
                } => {
                    app.send(BusCommand::SetFrEntryCycle {
                        bus,
                        slot,
                        cycle_us,
                    });
                }
                WebCommand::SetFrEntryHex { bus, slot, text } => {
                    app.send(BusCommand::SetFrEntryHex { bus, slot, text });
                }
                WebCommand::AddEntry { ch, id } => {
                    app.send(BusCommand::AddEntry { ch, id });
                }
                WebCommand::RemoveEntry { ch, id } => {
                    app.send(BusCommand::RemoveEntry { ch, id });
                }
                WebCommand::AddTrigger {
                    kind,
                    ch,
                    id,
                    bus,
                    slot,
                    signal_key,
                } => {
                    // Chart lanes hand over the canonical signal key; the
                    // backend owns the cluster -> bus resolution.
                    let mut cond: Option<TriggerCond> = None;
                    if let Some(text) = signal_key.as_deref() {
                        match crate::observe::SigKey::from_text(text) {
                            Some(key) => match app.key_bus(&key) {
                                Some(bus_index) => {
                                    cond = match key {
                                        crate::observe::SigKey::Can { id, ext, name, .. } => {
                                            Some(TriggerCond::SignalCross {
                                                ch: bus_index,
                                                id,
                                                ext,
                                                signal: name,
                                                threshold: 0.0,
                                                rising: true,
                                            })
                                        }
                                        crate::observe::SigKey::Fr { slot, name, .. } => {
                                            Some(TriggerCond::FrSignalCross {
                                                bus: bus_index,
                                                slot,
                                                signal: name,
                                                threshold: 0.0,
                                                rising: true,
                                            })
                                        }
                                        crate::observe::SigKey::Eth { .. } => {
                                            app.fail(
                                                "Ethernet signals have no trigger condition yet",
                                            );
                                            None
                                        }
                                        crate::observe::SigKey::Derived { .. } => None,
                                    };
                                }
                                None => {
                                    app.fail("that signal's bus is not in this project");
                                }
                            },
                            None => {
                                app.fail(format!("unrecognised signal key: {text}"));
                            }
                        }
                    }
                    if cond.is_none() {
                        match kind.as_str() {
                            "id" => {
                                cond = Some(TriggerCond::IdPresent {
                                    ch: ch.unwrap_or(0),
                                    id: id.unwrap_or(0x100),
                                });
                            }
                            "fr_frame" => {
                                cond = Some(TriggerCond::FrFramePresent {
                                    bus: bus.unwrap_or(0),
                                    slot: slot.unwrap_or(1),
                                });
                            }
                            _ => {}
                        }
                    }
                    match cond {
                        Some(cond) => app.apply_trigger_cmds(vec![TriggerCmd::AddCond(cond)]),
                        None => {
                            if let Some(kind) = parse_trigger_kind(&kind) {
                                app.apply_trigger_cmds(vec![TriggerCmd::Add(kind)]);
                            }
                        }
                    }
                }
                WebCommand::SetTriggerEnabled { index, on } => {
                    app.apply_trigger_cmds(vec![TriggerCmd::SetEnabled { index, on }]);
                }
                WebCommand::RemoveTrigger { index } => {
                    app.apply_trigger_cmds(vec![TriggerCmd::Remove(index)]);
                }
                WebCommand::DuplicateTrigger { index } => {
                    app.apply_trigger_cmds(vec![TriggerCmd::Duplicate(index)]);
                }
                WebCommand::RearmTriggers => {
                    app.apply_trigger_cmds(vec![TriggerCmd::Rearm]);
                }
                WebCommand::RearmTrigger { index } => {
                    app.apply_trigger_cmds(vec![TriggerCmd::RearmOne(index)]);
                }
                WebCommand::EditTrigger {
                    index,
                    kind,
                    ch,
                    id,
                    ext,
                    signal,
                    threshold,
                    rising,
                    bus,
                    slot,
                    key,
                    derived_id,
                    action,
                    send_ch,
                    send_id,
                    sys_var_key,
                    sys_var_value,
                    script_name,
                    script_value,
                    send_frame,
                    send_data,
                    someip_method,
                    someip_client,
                    someip_session,
                    someip_type,
                    cond_someip_method,
                    cond_someip_type,
                    someip_values,
                    someip_transport,
                    someip_host,
                    someip_port,
                    offset,
                    mask,
                    value,
                    vlan,
                    cooldown_ms,
                    max_fires,
                    pre_frames,
                    post_frames,
                } => {
                    let cond = match kind.as_str() {
                        "signal" => TriggerCond::SignalCross {
                            ch,
                            id,
                            ext,
                            signal,
                            threshold,
                            rising,
                        },
                        "id" => TriggerCond::IdPresent { ch, id },
                        "error" => TriggerCond::ErrorFrame { ch },
                        "timeout" => TriggerCond::CycleTimeout { ch, id },
                        "fr_frame" => TriggerCond::FrFramePresent { bus, slot },
                        "fr_signal" => TriggerCond::FrSignalCross {
                            bus,
                            slot,
                            signal,
                            threshold,
                            rising,
                        },
                        "sysvar" => TriggerCond::SysVar {
                            key,
                            threshold,
                            rising,
                        },
                        "derived" => TriggerCond::DerivedSignal {
                            id: derived_id,
                            threshold,
                            rising,
                        },
                        "payload" => TriggerCond::PayloadMatch {
                            ch,
                            id,
                            ext,
                            offset,
                            mask,
                            value,
                        },
                        "eth_flow" => TriggerCond::EthFlow {
                            bus,
                            ethertype: id.min(u32::from(u16::MAX)) as u16,
                            vlan,
                        },
                        "fr_timeout" => TriggerCond::FrTimeout { bus, slot },
                        "e2e" => TriggerCond::E2eIssue { ch, id, ext },
                        "someip" => TriggerCond::SomeIp {
                            bus,
                            service: id.min(u32::from(u16::MAX)) as u16,
                            method: cond_someip_method,
                            message_type: cond_someip_type,
                        },
                        _ => {
                            app.fail(format!("unknown trigger kind: {kind}"));
                            continue;
                        }
                    };
                    let action = match action.as_str() {
                        "start_recording" => TriggerAction::StartRecording,
                        "stop_recording" => TriggerAction::StopRecording,
                        "send" => TriggerAction::Send {
                            ch: send_ch,
                            id: send_id,
                        },
                        "send_fr" => TriggerAction::SendFr {
                            bus: send_ch,
                            slot: send_id.min(u32::from(u16::MAX)) as u16,
                        },
                        "send_eth" => {
                            let Some(data) = crate::generator::parse_hex_bytes(&send_data) else {
                                app.fail("Ethernet payload must be hex bytes");
                                return;
                            };
                            TriggerAction::SendEth {
                                bus: send_ch,
                                frame: send_frame,
                                data,
                            }
                        }
                        "send_someip" => {
                            let data = if someip_values.is_empty() {
                                let Some(data) = crate::generator::parse_hex_bytes(&send_data)
                                else {
                                    app.fail("SOME/IP payload must be hex bytes");
                                    return;
                                };
                                data
                            } else {
                                Vec::new()
                            };
                            TriggerAction::SendSomeIp {
                                bus: send_ch,
                                service: send_id.min(u32::from(u16::MAX)) as u16,
                                method: someip_method,
                                client: someip_client,
                                session: someip_session,
                                message_type: someip_type,
                                data,
                                values: someip_values,
                                transport: someip_transport,
                                host: someip_host,
                                port: someip_port,
                            }
                        }
                        "clear_trace" => TriggerAction::ClearTrace,
                        "insert_marker" => TriggerAction::InsertMarker,
                        "set_sysvar" => TriggerAction::SetSysVar {
                            key: sys_var_key,
                            value: sys_var_value.unwrap_or(0.0),
                        },
                        "script" => TriggerAction::RunScript {
                            name: script_name,
                            value: script_value,
                        },
                        _ => {
                            app.fail(format!("unknown trigger action: {action}"));
                            continue;
                        }
                    };
                    if index < app.snap.triggers.len() {
                        app.apply_trigger_cmds(vec![TriggerCmd::Edit {
                            index,
                            cond,
                            action,
                            cooldown_us: u64::from(cooldown_ms) * 1_000,
                            max_fires,
                            pre_frames: pre_frames.map(|n| n as usize),
                            post_frames,
                        }]);
                    }
                }
                WebCommand::SetTriggerLimits {
                    pre_frames,
                    post_frames,
                    marker_cap,
                } => {
                    app.apply_trigger_cmds(vec![
                        TriggerCmd::SetPreFrames(pre_frames),
                        TriggerCmd::SetPostFrames(post_frames),
                        TriggerCmd::SetMarkerCap(marker_cap),
                    ]);
                }
                WebCommand::SetWebWorkspace { rev, workspace } => {
                    set_web_workspace(&mut app, rev, workspace);
                }
                WebCommand::LoadReference { path } => {
                    let keys = signals.lock().map(|store| store.keys()).unwrap_or_default();
                    match crate::reference::load(&path, &keys, &app) {
                        Ok(run) => {
                            let count = run.signals.len();
                            app.status =
                                format!("reference loaded: {} · {count} signals", run.name);
                            app.reference = Some(run);
                            app.reference_rev = app.reference_rev.wrapping_add(1);
                        }
                        Err(error) => app.fail(error),
                    }
                }
                WebCommand::ClearReference => {
                    app.reference = None;
                    app.reference_rev = app.reference_rev.wrapping_add(1);
                }
                WebCommand::SetDerived { defs } => {
                    let count = defs.len();
                    app.set_derived_defs(defs);
                    app.status = if count == 1 {
                        "1 derived channel".to_string()
                    } else {
                        format!("{count} derived channels")
                    };
                }
                WebCommand::SimulateBusNodes { ch } => {
                    let nodes = app
                        .snap
                        .channels
                        .get(ch as usize)
                        .and_then(|channel| channel.dbc.as_ref())
                        .map(|db| db.nodes.clone())
                        .unwrap_or_default();
                    for node in nodes {
                        app.set_node_role(ch, &node, NodeRole::Simulated);
                    }
                }
            }
        }
        app.update();
        if last_publish.elapsed() >= PUBLISH_EVERY {
            let now = Instant::now();
            let mut fps = 0.0;
            // A stopped or paused measurement has no rate to report. Keep the
            // chart at its last point instead of growing a tail of zeros.
            if app.snap.measuring && !app.snap.trace_paused {
                rate_window.push_back((now, app.snap.frame_counter));
                while rate_window.len() > 2
                    && now.duration_since(rate_window[0].0) > Duration::from_secs(1)
                {
                    rate_window.pop_front();
                }
                let (started, frames_then) = rate_window[0];
                let elapsed = now.duration_since(started).as_secs_f64();
                // A one-second window reads as the bus rate; the per-lap EMA
                // swings with the generator's batch arrivals and makes the
                // chart look like noise.
                // A one-second window needs a moment to fill; feeding the
                // frontend the per-lap EMA before that put a 500 f/s spike at
                // the left edge and squashed the whole chart.
                // Skip the fill-in period instead of plotting synthetic zeros:
                // the chart should continue from its last real rate.
                if elapsed >= 0.5 {
                    fps = app.snap.frame_counter.saturating_sub(frames_then) as f64 / elapsed;
                    let bus_count = app.snap.channels.len();
                    let mut bus_rates: Vec<f64> = app
                        .snap
                        .bus_loads
                        .iter()
                        .map(|load| load.frame_rate())
                        .collect();
                    let mut bus_loads: Vec<f64> = app
                        .snap
                        .bus_loads
                        .iter()
                        .map(|load| load.load() * 100.0)
                        .collect();
                    if bus_rates.len() < bus_count {
                        bus_rates.resize(bus_count, 0.0);
                    }
                    if bus_loads.len() < bus_count {
                        bus_loads.resize(bus_count, 0.0);
                    }
                    series.push_back(WebSeriesPoint {
                        total: fps,
                        bus_rates,
                        bus_loads,
                    });
                    if series.len() > SERIES_POINTS {
                        series.pop_front();
                    }
                }
            }
            if let Ok(mut store) = trace.lock() {
                store.sync(&app);
            }
            if let Ok(mut store) = signals.lock() {
                store.sync(&app);
            }
            refresh_catalog(&app, &catalog, &mut catalog_sig);
            let next = build_state(&app, &series, fps);
            if let Ok(mut slot) = shared.lock() {
                *slot = next;
            }
            let sig = scripts_signature(&app);
            if sig != scripts_sig
                && let Ok(mut slot) = scripts.lock()
            {
                *slot = scripts_snapshot(&app);
                scripts_sig = sig;
            }
            last_publish = now;
        }
        std::thread::sleep(TICK_EVERY);
    }
}

