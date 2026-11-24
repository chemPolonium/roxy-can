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
                WebCommand::SetScriptSource { id, source } => {
                    app.send(BusCommand::SetNodeSource { id, source });
                }
                WebCommand::SetScriptEnabled { id, on } => {
                    app.send(BusCommand::SetNodeEnabled { id, on });
                }
                WebCommand::RemoveScript { id } => {
                    app.send(BusCommand::RemoveNode { id });
                }
                WebCommand::SaveScriptFile { path, source } => {
                    if path.trim().is_empty() {
                        app.fail("script save needs a path");
                    } else {
                        match std::fs::write(&path, source) {
                            Ok(()) => app.status = format!("script saved: {path}"),
                            Err(error) => app.fail(format!("script save failed: {error}")),
                        }
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
                    } else if let Some(id) = band_set_id
                        && let Some(row) = app
                            .monitor_view
                            .rows
                            .iter_mut()
                            .find(|row| row.key.text() == key)
                    {
                        row.band_set_id = if id.is_empty() { None } else { Some(id) };
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

/// Incremental cache of the chosen signals. Unlike a Graphics window this is
/// a session-level selection: a project load starts from its Graphics rows
/// and the Signals panel adds session-only picks. The frontend asks for a seed
/// and then only for samples newer than its last timestamp.
#[derive(Default)]
struct SignalStore {
    epoch: u64,
    primed: bool,
    selected: Vec<SelectedSignal>,
    buffers: Vec<SignalBuffer>,
    last_t: Vec<u64>,
    /// Fingerprint of the derived definitions appended after the selected
    /// signals; a change rebuilds the lane list (and its epoch).
    derived_meta: String,
}

impl SignalStore {
    fn is_selected(&self, key: &SigKey) -> bool {
        self.selected.iter().any(|signal| &signal.key == key)
    }

    fn is_read(&self, key: &SigKey) -> bool {
        self.selected
            .iter()
            .any(|signal| &signal.key == key && signal.read)
    }

    /// Every subscribed key: plotted lanes and read-only signals alike.
    fn keys(&self) -> Vec<SigKey> {
        self.selected
            .iter()
            .map(|signal| signal.key.clone())
            .collect()
    }

    /// Plotted lanes only, in lane order. Read-only signals never draw.
    fn chart_keys(&self) -> Vec<SigKey> {
        self.selected
            .iter()
            .filter(|signal| !signal.read)
            .map(|signal| signal.key.clone())
            .collect()
    }

    /// Adds a selection without subscribing; the command handler owns the
    /// bus subscription so this stays a pure cache operation.
    #[cfg(test)]
    fn select(&mut self, app: &App, ch: u8, id: u32, ext: bool, name: &str) {
        self.select_key(app, app.can_key(ch, id, ext, name.to_string()));
    }

    fn select_key(&mut self, app: &App, key: SigKey) {
        self.select_key_mode(app, key, false);
    }

    /// Selects a signal as a plotted lane (`read = false`) or a cursor-only
    /// read signal. Changing the mode keeps the entry and its history; only
    /// the lane/readout placement changes.
    fn select_key_mode(&mut self, app: &App, key: SigKey, read: bool) {
        let Some(key) = app.bind_key(key) else {
            return;
        };
        if let Some(pos) = self.selected.iter().position(|signal| signal.key == key) {
            if self.selected[pos].read == read {
                return;
            }
            let mut signal = self.selected.remove(pos);
            signal.read = read;
            if !read {
                // A signal coming back to the chart gets a lane colour that
                // is not already in use.
                signal.color = self.free_chart_color();
            }
            let at = if read {
                self.selected.len()
            } else {
                self.selected
                    .iter()
                    .position(|signal| signal.read)
                    .unwrap_or(self.selected.len())
            };
            self.selected.insert(at, signal);
            self.primed = false;
            return;
        }
        let name = match &key {
            SigKey::Can { name, .. }
            | SigKey::Fr { name, .. }
            | SigKey::Eth { name, .. }
            | SigKey::Derived { name, .. } => name.clone(),
        };
        let bus = signal_bus(app, &key);
        let unit = match &key {
            SigKey::Can { id, ext, name, .. } => app
                .key_bus(&key)
                .map(|ch| signal_unit(app, ch, *id, *ext, name))
                .unwrap_or_default(),
            SigKey::Derived { id, .. } => app
                .derived_defs
                .iter()
                .find(|def| &def.id == id)
                .map(|def| def.unit.clone())
                .unwrap_or_default(),
            // FR/Eth units arrive with the first subscription publish.
            SigKey::Fr { .. } | SigKey::Eth { .. } => String::new(),
        };
        let color = if read { 0 } else { self.free_chart_color() };
        let signal = SelectedSignal {
            key,
            name,
            bus,
            unit,
            color,
            read,
        };
        if read {
            self.selected.push(signal);
        } else {
            let at = self
                .selected
                .iter()
                .position(|signal| signal.read)
                .unwrap_or(self.selected.len());
            self.selected.insert(at, signal);
        }
        self.primed = false;
    }

    fn free_chart_color(&self) -> usize {
        (0..SIGNAL_PALETTE)
            .find(|color| {
                !self
                    .selected
                    .iter()
                    .any(|signal| !signal.read && signal.color == *color)
            })
            .unwrap_or(self.selected.len() % SIGNAL_PALETTE)
    }

    fn unselect(&mut self, key: &SigKey) {
        if let Some(pos) = self.selected.iter().position(|signal| &signal.key == key) {
            self.selected.remove(pos);
        }
        self.primed = false;
    }

    /// Reorders one lane. Colors travel with the signal; the next sync
    /// rebuilds the per-lane buffers and bumps the epoch so the frontend
    /// replaces its lane list rather than merging against stale indexes.
    fn move_selected(&mut self, key: &SigKey, to: usize) -> bool {
        let Some(from) = self.selected.iter().position(|signal| &signal.key == key) else {
            return false;
        };
        if self.selected[from].read {
            return false;
        }
        let charted = self.selected.iter().filter(|signal| !signal.read).count();
        let to = to.min(charted.saturating_sub(1));
        if from == to {
            return false;
        }
        let signal = self.selected.remove(from);
        self.selected.insert(to, signal);
        self.primed = false;
        true
    }

    /// The removed channel's cluster takes its selections with it; the
    /// channels that stay keep their keys, so nothing is renumbered here.
    fn remap_after_remove(&mut self, removed: Option<crate::model::ClusterId>) {
        self.selected.retain(
            |signal| !matches!(removed, Some(cluster) if signal.key.cluster() == Some(cluster)),
        );
        self.primed = false;
    }

    /// A loaded database can replace the signals under a selection. Drop the
    /// ones it no longer declares and return their keys for unsubscription.
    #[cfg(test)]
    fn prune_missing(&mut self, app: &App) -> Vec<SigKey> {
        let mut dropped = Vec::new();
        self.selected.retain(|signal| {
            let exists = match &signal.key {
                SigKey::Can { id, ext, name, .. } => app
                    .key_bus(&signal.key)
                    .and_then(|ch| app.snap.channels.get(ch as usize))
                    .and_then(|channel| channel.dbc.as_ref())
                    .and_then(|db| db.messages.get(&(*id, *ext)))
                    .is_some_and(|message| {
                        message
                            .signals
                            .iter()
                            .any(|candidate| &candidate.name == name)
                    }),
                SigKey::Derived { id, .. } => app.derived_defs.iter().any(|def| &def.id == id),
                _ => true,
            };
            if !exists {
                dropped.push(signal.key.clone());
            }
            exists
        });
        if !dropped.is_empty() {
            self.primed = false;
        }
        dropped
    }

    /// Drops selected signals and buffers; used when the bus set changes.
    fn clear_selection(&mut self) {
        self.selected.clear();
        self.primed = false;
    }

    /// Keeps the selection but forgets sampled history; used when a new log
    /// starts on the same project.
    fn reset(&mut self) {
        self.primed = false;
    }

    fn sync(&mut self, app: &App) {
        // A derived rename replaces its series key; keep the lane/read choice
        // on the same definition id.
        let defs: std::collections::HashMap<&str, &crate::derived::DerivedDef> = app
            .derived_defs
            .iter()
            .map(|def| (def.id.as_str(), def))
            .collect();
        self.selected.retain(|signal| match &signal.key {
            SigKey::Derived { id, .. } => defs.contains_key(id.as_str()),
            _ => true,
        });
        for signal in &mut self.selected {
            if let SigKey::Derived { id, name } = &mut signal.key
                && let Some(def) = defs.get(id.as_str())
            {
                *name = def.name.clone();
                signal.name = def.name.clone();
                signal.unit = def.unit.clone();
            }
        }
        for signal in &mut self.selected {
            signal.bus = signal_bus(app, &signal.key);
            if let Some(sub) = signal_sub(app, &signal.key) {
                signal.unit = sub.unit.clone();
            }
        }
        // Computed channels are ordinary selectable series: definitions only
        // make them available, Plot/Read decides whether they appear.
        let signals = self.selected.clone();
        let derived_meta = app
            .derived_defs
            .iter()
            .map(|def| format!("{}|{}|{}", def.id, def.name, def.unit))
            .collect::<Vec<_>>()
            .join(";");
        let mut reset = !self.primed || derived_meta != self.derived_meta;
        self.derived_meta = derived_meta;
        if self.primed {
            for (i, signal) in signals.iter().enumerate() {
                let Some(sub) = signal_sub(app, &signal.key) else {
                    continue;
                };
                let last = sub.history.iter().last().map(|(t, _)| *t).unwrap_or(0);
                if self.last_t.get(i).copied().unwrap_or(0) > last {
                    reset = true;
                }
            }
        }
        if reset {
            self.primed = true;
            self.epoch = self.epoch.wrapping_add(1).max(1);
            self.buffers.clear();
            self.last_t = vec![0; signals.len()];
            for (i, signal) in signals.iter().enumerate() {
                let sub = signal_sub(app, &signal.key);
                let mut meta = signal_meta(signal);
                if let Some(sub) = sub {
                    meta.label = sub.label.clone();
                }
                let mut points = VecDeque::new();
                if let Some(sub) = sub {
                    let all: Vec<(u64, f64)> = sub.history.iter().copied().collect();
                    let start = all.len().saturating_sub(SIGNAL_CAP);
                    points.extend(all[start..].iter().copied());
                    if let Some((t, _)) = points.back() {
                        self.last_t[i] = *t;
                    }
                }
                self.buffers.push(SignalBuffer { meta, points });
            }
            return;
        }

        for (i, signal) in signals.iter().enumerate() {
            let Some(sub) = signal_sub(app, &signal.key) else {
                continue;
            };
            let Some(buffer) = self.buffers.get_mut(i) else {
                continue;
            };
            buffer.meta = signal_meta(signal);
            buffer.meta.label = sub.label.clone();
            let last = self.last_t.get(i).copied().unwrap_or(0);
            for &(t, value) in sub.history.iter() {
                if t > last {
                    buffer.points.push_back((t, value));
                }
            }
            while buffer.points.len() > SIGNAL_CAP {
                buffer.points.pop_front();
            }
            if let Some((t, _)) = buffer.points.back() {
                self.last_t[i] = *t;
            }
        }
    }

    fn max_t(&self) -> u64 {
        self.last_t.iter().copied().max().unwrap_or(0)
    }

    fn earliest_t(&self) -> u64 {
        self.buffers
            .iter()
            .filter_map(|buffer| buffer.points.front().map(|(t, _)| *t))
            .min()
            .unwrap_or(self.max_t())
    }

    /// Includes selected signals with no samples yet; the lane has to exist
    /// before the first frame arrives.
    fn seed(&self) -> Vec<SignalSeries> {
        self.buffers
            .iter()
            .map(|buffer| Self::dto(buffer, 0))
            .collect()
    }

    fn after(&self, since: u64) -> Vec<SignalSeries> {
        self.buffers
            .iter()
            .filter(|buffer| buffer.points.back().is_some_and(|(t, _)| *t > since))
            .map(|buffer| Self::dto(buffer, since))
            .collect()
    }

    fn dto(buffer: &SignalBuffer, since: u64) -> SignalSeries {
        let mut series = buffer.meta.clone();
        series.points = buffer
            .points
            .iter()
            .filter(|(t, _)| *t > since)
            .map(|(t, value)| [*t as f64 / 1e6, *value])
            .collect();
        series
    }
}

#[cfg(test)]
fn key_id(key: &SigKey) -> u32 {
    match key {
        SigKey::Can { id, .. } => *id,
        SigKey::Fr { slot, .. } => *slot as u32,
        SigKey::Eth { .. } | SigKey::Derived { .. } => 0,
    }
}

fn bus_name(app: &App, ch: u8) -> String {
    app.snap
        .channels
        .get(ch as usize)
        .map(|channel| channel.name.clone())
        .unwrap_or_else(|| format!("CAN{}", ch + 1))
}

fn signal_unit(app: &App, ch: u8, id: u32, ext: bool, name: &str) -> String {
    app.snap
        .channels
        .get(ch as usize)
        .and_then(|channel| channel.dbc.as_ref())
        .and_then(|db| db.messages.get(&(id, ext)))
        .and_then(|message| message.signals.iter().find(|signal| signal.name == name))
        .map(|signal| signal.unit.clone())
        .unwrap_or_default()
}

fn signal_sub<'a>(app: &'a App, key: &SigKey) -> Option<&'a SubView> {
    app.snap.subs.iter().find(|sub| &sub.key == key)
}

/// Resolves the address every observer command carries. CAN keys use a
/// message id; FlexRay keys use a slot in the same `id` field with the
/// cluster in `ch`; Ethernet names its frame. One command shape covers the
/// three bus kinds.
fn command_key(
    app: &crate::bus::BusCore,
    kind: &str,
    ch: u8,
    id: u32,
    ext: bool,
    frame: Option<String>,
    name: String,
) -> Option<SigKey> {
    match kind {
        "can" => Some(app.can_key(ch, id, ext, name)),
        "fr" => u16::try_from(id)
            .ok()
            .map(|slot| app.fr_key(ch, slot, name)),
        "eth" => frame.map(|frame| app.eth_key(ch, frame, name)),
        "derived" => frame.map(|id| SigKey::derived(id, name)),
        _ => None,
    }
}

struct RowAddress {
    kind: &'static str,
    ch: u8,
    id: u32,
    ext: bool,
    frame: Option<String>,
}

/// Observer rows carry one address shape for every bus kind. FlexRay puts
/// its cluster in `ch` and its slot in `id`; Ethernet keeps `ch` and names
/// its frame.
fn row_address(app: &crate::bus::BusCore, key: &SigKey) -> RowAddress {
    match key {
        SigKey::Can { id, ext, .. } => RowAddress {
            kind: "can",
            ch: app.key_bus(key).unwrap_or(0),
            id: *id,
            ext: *ext,
            frame: None,
        },
        SigKey::Fr { slot, .. } => RowAddress {
            kind: "fr",
            ch: app.key_bus(key).unwrap_or(0),
            id: u32::from(*slot),
            ext: false,
            frame: None,
        },
        SigKey::Eth { frame, .. } => RowAddress {
            kind: "eth",
            ch: app.key_bus(key).unwrap_or(0),
            id: 0,
            ext: false,
            frame: Some(frame.clone()),
        },
        SigKey::Derived { id, .. } => RowAddress {
            kind: "derived",
            ch: 0,
            id: 0,
            ext: false,
            frame: Some(id.clone()),
        },
    }
}

pub(crate) fn series_key(key: &SigKey) -> String {
    key.text()
}

fn signal_bus(app: &App, key: &SigKey) -> String {
    match key {
        SigKey::Can { .. } => app
            .key_bus(key)
            .map(|ch| bus_name(app, ch))
            .unwrap_or_default(),
        SigKey::Fr { .. } => app
            .key_bus(key)
            .map(|bus| app.fr_bus_name(bus))
            .unwrap_or_default(),
        SigKey::Eth { .. } => app
            .key_bus(key)
            .map(|bus| app.eth_bus_label(bus))
            .unwrap_or_default(),
        SigKey::Derived { .. } => "Derived".to_string(),
    }
}

/// `Frame / PDU / Signal`, skipping a PDU whose name repeats the frame (DBC
/// imports name them the same). Rows can then tell apart same-named ARXML
/// signals that live in different frames or PDUs.
fn signal_path(app: &App, key: &SigKey) -> Option<String> {
    let bus = app.key_bus(key)?;
    match key {
        SigKey::Can {
            cluster,
            id,
            ext,
            name,
        } => {
            let comm = app.model_comms.get(bus as usize)?.as_ref()?;
            let frame = comm
                .frame_by_can_id(*cluster, *id)
                .into_iter()
                .find(|frame| {
                    matches!(
                        frame.address,
                        crate::model::FrameAddress::Can {
                            extended,
                            ..
                        } if extended == *ext
                    )
                })?;
            let signal = comm.signal_by_name(frame, name).into_iter().next()?;
            Some(model_signal_path(comm, frame, signal))
        }
        SigKey::Fr {
            cluster,
            slot,
            name,
        } => {
            let comm = &app.fr_models.get(&bus)?.comm;
            for frame in comm.frame_by_slot(*cluster, *slot) {
                if let Some(signal) = comm.signal_by_name(frame, name).into_iter().next() {
                    return Some(model_signal_path(comm, frame, signal));
                }
            }
            None
        }
        SigKey::Eth {
            cluster,
            frame,
            name,
        } => {
            let comm = &app.eth_models.get(&bus)?.comm;
            let frame = comm.frame_by_name(*cluster, frame)?;
            let signal = comm.signal_by_name(frame, name).into_iter().next()?;
            Some(model_signal_path(comm, frame, signal))
        }
        SigKey::Derived { .. } => None,
    }
}

fn model_signal_path(
    comm: &crate::model::Comm,
    frame: &crate::model::Frame,
    signal: &crate::model::Signal,
) -> String {
    let mut parts = Vec::with_capacity(3);
    if !frame.name.is_empty() {
        parts.push(frame.name.clone());
    }
    if let Some(pdu) = comm.pdus.get(&signal.pdu)
        && !pdu.name.is_empty()
        && pdu.name != frame.name
    {
        parts.push(pdu.name.clone());
    }
    parts.push(signal.name.clone());
    parts.join(" / ")
}

fn signal_meta(signal: &SelectedSignal) -> SignalSeries {
    SignalSeries {
        key: series_key(&signal.key),
        name: signal.name.clone(),
        bus: signal.bus.clone(),
        unit: signal.unit.clone(),
        color: signal.color,
        label: None,
        read: signal.read,
        points: Vec::new(),
    }
}

/// Build the signal tree for the picker: every CAN channel with a DBC plus
/// every loaded FlexRay cluster. Keys are the same strings the series
/// endpoint reports, so selection state round-trips without translation.
#[derive(Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
struct WebCatalog {
    channels: Vec<WebCatalogChannel>,
}

#[derive(Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
struct WebCatalogChannel {
    kind: String,
    index: u8,
    name: String,
    /// Cluster names the description carries, so the editor can offer the
    /// named FlexRay/Ethernet spellings (`FR.PowerTrain...`).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    aliases: Vec<String>,
    messages: Vec<WebCatalogMessage>,
}

#[derive(Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
struct WebCatalogMessage {
    id: u32,
    extended: bool,
    name: String,
    /// Ethernet frames are addressed by name, not by their catalog index.
    #[serde(skip_serializing_if = "Option::is_none")]
    frame: Option<String>,
    /// `CM_ BO_` description; omitted when the database has none.
    #[serde(skip_serializing_if = "String::is_empty")]
    comment: String,
    /// CAN FD, either declared by `VFrameFormat` or implied by a long DLC.
    fd: bool,
    /// Ethernet payload bytes after the project's L2-header convention.
    #[serde(skip_serializing_if = "Option::is_none")]
    length: Option<u32>,
    signals: Vec<WebCatalogSignal>,
}

#[derive(Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
struct WebCatalogSignal {
    key: String,
    name: String,
    unit: String,
    min: f64,
    max: f64,
    /// Declared start value in physical units; the physical send mode shows
    /// it as the placeholder for an untouched field.
    #[serde(skip_serializing_if = "Option::is_none")]
    initial: Option<f64>,
    /// `CM_ SG_` description; omitted when the database has none.
    #[serde(skip_serializing_if = "String::is_empty")]
    comment: String,
    /// The I-SIGNAL-GROUP this signal belongs to, when one names it.
    #[serde(skip_serializing_if = "Option::is_none")]
    group: Option<String>,
}

fn channel_signature(app: &App) -> String {
    let parts: Vec<String> = app
        .snap
        .channels
        .iter()
        .map(|channel| {
            let table = channel
                .dbc
                .as_ref()
                .map(|db| Arc::as_ptr(db) as usize)
                .unwrap_or(0);
            format!("{}:{}:{table}", channel.name, channel.dbc_paths.join(","))
        })
        .enumerate()
        .map(|(index, text)| {
            let model = app
                .model_comms
                .get(index)
                .and_then(Option::as_ref)
                .map(|comm| Arc::as_ptr(comm) as usize)
                .unwrap_or(0);
            format!("{text}:{model}")
        })
        .collect();
    let fr: Vec<String> = app
        .fr_buses
        .iter()
        .map(|(bus, cfg)| {
            format!(
                "fr{bus}:{}:{}",
                app.fr_description_path(*bus).unwrap_or_default(),
                Arc::as_ptr(cfg) as usize
            )
        })
        .collect();
    let eth: Vec<String> = app
        .eth_buses
        .iter()
        .map(|(bus, cfg)| {
            format!(
                "eth{bus}:{}:{}",
                app.eth_description_path(*bus).unwrap_or_default(),
                Arc::as_ptr(cfg) as usize
            )
        })
        .collect();
    let someip: Vec<String> = app
        .someip_services
        .services
        .iter()
        .map(|service| {
            format!(
                "someip{}:{}:{}:{}",
                service.id,
                service.methods.len(),
                service.events.len(),
                service.instances.len()
            )
        })
        .collect();
    parts
        .into_iter()
        .chain(fr)
        .chain(eth)
        .chain(someip)
        .collect::<Vec<_>>()
        .join(";")
}

fn refresh_catalog(app: &App, catalog: &Arc<Mutex<WebCatalog>>, signature: &mut String) {
    let next = channel_signature(app);
    if next == *signature {
        return;
    }
    *signature = next;
    if let Ok(mut slot) = catalog.lock() {
        *slot = build_catalog(app);
    }
}

fn catalog_signal(
    key: String,
    signal: &crate::model::Signal,
    group: Option<String>,
) -> WebCatalogSignal {
    WebCatalogSignal {
        key,
        name: signal.name.clone(),
        unit: signal.unit.clone(),
        min: signal.min.unwrap_or(0.0),
        max: signal.max.unwrap_or(0.0),
        initial: signal.initial_physical(),
        comment: signal.comment.clone(),
        group,
    }
}

/// The name of the I-SIGNAL-GROUP a signal belongs to, from the model.
fn signal_group_name(comm: &crate::model::Comm, signal: &crate::model::Signal) -> Option<String> {
    signal
        .group
        .and_then(|group| comm.groups.get(&group))
        .map(|group| group.name.clone())
}

/// The messages one CAN channel's model declares, in id order. Names,
/// comments, signals and ranges all come from the model.
fn can_catalog_messages(comm: &crate::model::Comm) -> Vec<WebCatalogMessage> {
    let cluster = comm
        .clusters
        .keys()
        .next()
        .copied()
        .unwrap_or(crate::model::ClusterId(0));
    let mut out: Vec<WebCatalogMessage> = comm
        .frames
        .values()
        .filter_map(|frame| {
            let crate::model::FrameAddress::Can { id, extended, fd } = frame.address else {
                return None;
            };
            Some(WebCatalogMessage {
                id,
                extended,
                name: frame.name.clone(),
                frame: None,
                comment: frame.comment.clone(),
                fd,
                length: None,
                signals: comm
                    .signals_in_frame(frame)
                    .into_iter()
                    .map(|signal| {
                        catalog_signal(
                            series_key(&SigKey::can_in(cluster, id, extended, &signal.name)),
                            signal,
                            signal_group_name(comm, signal),
                        )
                    })
                    .collect(),
            })
        })
        .collect();
    out.sort_by_key(|message| (message.id, message.extended));
    out
}

fn fr_catalog_messages(comm: &crate::model::Comm) -> Vec<WebCatalogMessage> {
    let cluster = comm
        .clusters
        .keys()
        .next()
        .copied()
        .unwrap_or(crate::model::ClusterId(0));
    let mut out: Vec<WebCatalogMessage> = comm
        .frames
        .values()
        .filter_map(|frame| {
            let crate::model::FrameAddress::FlexRay { slot, .. } = frame.address else {
                return None;
            };
            Some(WebCatalogMessage {
                id: u32::from(slot),
                extended: false,
                name: frame.name.clone(),
                frame: None,
                comment: String::new(),
                fd: false,
                length: None,
                signals: comm
                    .signals_in_frame(frame)
                    .into_iter()
                    .map(|signal| {
                        catalog_signal(
                            series_key(&SigKey::fr_in(cluster, slot, &signal.name)),
                            signal,
                            signal_group_name(comm, signal),
                        )
                    })
                    .collect(),
            })
        })
        .collect();
    out.sort_by_key(|message| message.id);
    out
}

fn eth_catalog_messages(
    comm: &crate::model::Comm,
    length_includes_header: bool,
) -> Vec<WebCatalogMessage> {
    let cluster = comm
        .clusters
        .keys()
        .next()
        .copied()
        .unwrap_or(crate::model::ClusterId(0));
    comm.frames
        .values()
        .filter(|frame| matches!(frame.address, crate::model::FrameAddress::Ethernet { .. }))
        .enumerate()
        .map(|(index, frame)| WebCatalogMessage {
            id: index as u32,
            extended: false,
            name: frame.name.clone(),
            frame: Some(frame.name.clone()),
            comment: String::new(),
            fd: false,
            length: Some(if length_includes_header {
                frame.length.saturating_sub(14)
            } else {
                frame.length
            }),
            signals: comm
                .signals_in_frame(frame)
                .into_iter()
                .map(|signal| {
                    catalog_signal(
                        series_key(&SigKey::eth_in(cluster, &frame.name, &signal.name)),
                        signal,
                        signal_group_name(comm, signal),
                    )
                })
                .collect(),
        })
        .collect()
}

fn catalog_aliases(comm: &crate::model::Comm) -> Vec<String> {
    let mut names: Vec<String> = comm
        .clusters
        .values()
        .map(|cluster| cluster.name.clone())
        .filter(|name| !name.is_empty())
        .collect();
    names.dedup();
    names
}

fn someip_signal_names(prefix: &str, ty: &crate::eth::arxml_service::ServiceType) -> Vec<String> {
    use crate::eth::arxml_service::ServiceType;
    match ty {
        ServiceType::Record(fields) => fields
            .iter()
            .flat_map(|field| someip_signal_names(&format!("{prefix}.{}", field.name), &field.ty))
            .collect(),
        ServiceType::Array { element, count } => (0..*count)
            .flat_map(|index| someip_signal_names(&format!("{prefix}[{index}]"), element))
            .collect(),
        _ => vec![prefix.to_string()],
    }
}

/// The editable leaves of one SOME/IP field, with the type facts the Panel
/// needs to render the right input. It mirrors `someip_signal_names`, but
/// keeps enum labels and scalar/text kinds.
fn someip_field_defs(
    prefix: &str,
    field: &crate::eth::arxml_service::ServiceField,
) -> Vec<WebSomeIpField> {
    let mut out = Vec::new();
    someip_collect_field_defs(prefix, &field.ty, &field.props, &mut out);
    out
}

fn someip_collect_field_defs(
    prefix: &str,
    ty: &crate::eth::arxml_service::ServiceType,
    props: &crate::eth::arxml_service::SomeIpProps,
    out: &mut Vec<WebSomeIpField>,
) {
    use crate::eth::arxml_service::ServiceType;
    let numeric = |name: &str, props: &crate::eth::arxml_service::SomeIpProps| WebSomeIpField {
        name: name.to_string(),
        kind: "number".to_string(),
        unit: props.unit.clone(),
        min: props.min.as_deref().and_then(|value| value.parse().ok()),
        max: props.max.as_deref().and_then(|value| value.parse().ok()),
        labels: Vec::new(),
    };
    match ty {
        ServiceType::Record(fields) => {
            for field in fields {
                someip_collect_field_defs(
                    &format!("{prefix}.{}", field.name),
                    &field.ty,
                    &field.props,
                    out,
                );
            }
        }
        ServiceType::Array { element, count } => {
            for index in 0..*count {
                someip_collect_field_defs(&format!("{prefix}[{index}]"), element, props, out);
            }
        }
        ServiceType::Enum { labels, .. } => {
            let mut field = numeric(prefix, props);
            field.labels = labels.clone();
            out.push(field);
        }
        ServiceType::String => out.push(WebSomeIpField {
            name: prefix.to_string(),
            kind: "text".to_string(),
            unit: props.unit.clone(),
            min: None,
            max: None,
            labels: Vec::new(),
        }),
        ServiceType::Union(variants) => {
            out.push(numeric(&format!("{prefix}.$selector"), props));
            for variant in variants {
                someip_collect_field_defs(
                    &format!("{prefix}.{}", variant.name),
                    &variant.ty,
                    &variant.props,
                    out,
                );
            }
        }
        _ => out.push(numeric(prefix, props)),
    }
}

fn someip_catalog_messages(app: &App, bus: u8) -> Vec<WebCatalogMessage> {
    let mut out = Vec::new();
    for service in &app.someip_services.services {
        if !service.buses.is_empty() && !service.buses.contains(&bus) {
            continue;
        }
        let mut add =
            |name: &str, method: u16, fields: &[crate::eth::arxml_service::ServiceField]| {
                if fields.is_empty() {
                    return;
                }
                let frame = format!("SOMEIP 0x{:04X}/0x{:04X}", service.id, method);
                let signals = fields
                    .iter()
                    .filter(|field| !field.name.is_empty())
                    .flat_map(|field| {
                        someip_signal_names(&field.name, &field.ty)
                            .into_iter()
                            .map(|signal| WebCatalogSignal {
                                key: series_key(&app.eth_key(bus, frame.clone(), signal.clone())),
                                name: signal,
                                unit: field.props.unit.clone(),
                                min: field
                                    .props
                                    .min
                                    .as_deref()
                                    .and_then(|value| value.parse().ok())
                                    .unwrap_or(0.0),
                                max: field
                                    .props
                                    .max
                                    .as_deref()
                                    .and_then(|value| value.parse().ok())
                                    .unwrap_or(0.0),
                                initial: None,
                                comment: String::new(),
                                group: None,
                            })
                    })
                    .collect();
                out.push(WebCatalogMessage {
                    id: (u32::from(service.id) << 16) | u32::from(method),
                    extended: true,
                    name: name.to_string(),
                    frame: Some(frame),
                    comment: format!("SOME/IP service 0x{:04X}", service.id),
                    fd: false,
                    length: None,
                    signals,
                });
            };
        for event in &service.events {
            add(&event.name, event.id, &event.fields);
        }
        for method in &service.methods {
            let mut fields = method.input.clone();
            fields.extend(method.output.iter().cloned());
            add(&method.name, method.id, &fields);
        }
    }
    out
}

