use super::*;
use crate::model::ProjectV2;
use crate::test_assets::TestBusLoad;
// The tests rely on the parent's imports through `use super::*`; the
// ones app.rs itself no longer needs are imported directly here.
use crate::bus::MAX_TX_CATCHUP;
use crate::can::frame::{FrameFlags, MAX_CAN_FD_LEN};
use crate::generator::TxMsg;
use crate::log::AscWriter;
use crate::observe::SigKey;
use crate::observe::Subscription;
use crate::observe::YMode;
use crate::sim::ValueSrc;
use crate::source::FrameSource;
use crate::trigger::{Trigger, TriggerAction, TriggerCond};

#[test]
fn record_survives_start_and_writes_frames() {
    let mut app = App::headless();
    let path = std::env::temp_dir().join("roxy_can_record_test.asc");
    app.record_path_buf = path.to_string_lossy().to_string();
    app.toggle_record();
    assert!(app.recorder.recording);
    assert!(
        !app.tx_list.is_empty(),
        "DBC messages pre-populate the generator"
    );
    for tx in &mut app.tx_list {
        tx.active = true;
        tx.cycle_us = 10_000;
    }
    open_all_gates(&mut app);
    open_all_gates(&mut app);
    app.start_virtual();
    assert!(
        app.recorder.recording,
        "Start must not clear the Record checkbox"
    );
    for _ in 0..12 {
        std::thread::sleep(std::time::Duration::from_millis(11));
        app.update();
    }
    app.stop();
    let actual = app.recorder.last_record.clone();
    assert!(actual.ends_with(".asc"), "no record file: {actual}");
    assert!(
        actual.contains("roxy_can_record_test_"),
        "generated name should keep the user base: {actual}"
    );
    let content = std::fs::read_to_string(&actual).unwrap();
    let frames = crate::log::asc::parse_asc(&content);
    assert!(frames.len() >= 10, "expected frames, got {}", frames.len());
    if let Some(dir) = std::path::Path::new(&actual).parent()
        && let Ok(rd) = std::fs::read_dir(dir)
    {
        for e in rd.flatten() {
            let n = e.file_name().to_string_lossy().to_string();
            if n.starts_with("roxy_can_record_test") {
                std::fs::remove_file(e.path()).ok();
            }
        }
    }
}

/// The record filter gates only the FILE: a whitelist of ids keeps the
/// recorded ASC to those frames, while trace and aggregates still see
/// the whole bus.
#[test]
fn the_record_filter_limits_the_file_but_not_the_bus() {
    let mut app = App::headless();
    let path = std::env::temp_dir().join("roxy_can_record_filtered.asc");
    app.record_path_buf = path.to_string_lossy().to_string();
    app.set_record_filter(crate::recorder::RecordFilter::can(vec![(0x100, false)]));
    app.toggle_record();
    // motbus on CAN2: activate one entry whose id is NOT in the whitelist.
    app.add_tx(1, 0x999);
    for tx in &mut app.tx_list {
        tx.active = true;
        tx.cycle_us = 10_000;
    }
    open_all_gates(&mut app);
    app.start_virtual();
    for _ in 0..12 {
        std::thread::sleep(std::time::Duration::from_millis(11));
        app.update();
    }
    app.stop();
    let actual = app.recorder.last_record.clone();
    let content = std::fs::read_to_string(&actual).unwrap();
    let frames = crate::log::asc::parse_asc(&content);
    assert!(
        frames.len() >= 5,
        "the whitelisted id flowed: got {} frame(s)",
        frames.len()
    );
    assert!(
        frames.iter().all(|f| f.id == 0x100),
        "nothing outside the whitelist landed in the file"
    );
    // The bus itself saw the filtered-out traffic: aggregates count it.
    let stray = app
        .snap
        .aggs
        .iter()
        .find(|a| a.channel == 1 && a.id == 0x999)
        .expect("the filtered-out id still reached the bus");
    assert!(stray.count >= 1, "aggregates must not be filtered");
    std::fs::remove_file(&actual).ok();
}

/// The record filter can name a FlexRay slot, and it still gates only the
/// file: three arrivals, one of them on the whitelist, and the Trace ring keeps
/// all three. The CAN side of the same recording is untouched by a slot-only
/// whitelist -- the filter lists what to keep, it does not mute the other bus.
#[test]
fn a_record_filter_can_keep_only_some_flexray_slots() {
    use crate::hw::vector::flexray::FrFrame;
    let mut app = App::headless();
    let path = std::env::temp_dir().join("roxy_can_record_fr_filtered.asc");
    app.record_path_buf = path.to_string_lossy().to_string();
    app.set_record_filter(crate::recorder::RecordFilter {
        can: vec![],
        fr: vec![(0, 13)],
    });
    app.toggle_record();
    let q0 = app.hw.attach_fr_mock(0, 5);
    let q1 = app.hw.attach_fr_mock(1, 6);
    app.start_virtual();
    let frame = |slot: u16| FrFrame {
        slot,
        cycle: 0,
        payload: vec![0x2A],
        header_crc: 0,
        flags: 0,
    };
    {
        let mut g = q0.lock().expect("mock lock");
        g.push_back(frame(13));
        g.push_back(frame(14));
    }
    q1.lock().expect("mock lock").push_back(frame(13));
    for t in (10_000u64..=60_000).step_by(10_000) {
        app.advance_clock(t);
        app.tick(t);
    }
    app.stop();
    let actual = app.recorder.last_record.clone();
    let text = std::fs::read_to_string(&actual).expect("the recording exists");
    let (_, rows) = crate::log::asc::parse_asc_full(&text);
    assert_eq!(
        rows.iter().map(|r| (r.bus, r.slot)).collect::<Vec<_>>(),
        vec![(0, 13)],
        "only the whitelisted cluster+slot landed in the file"
    );
    // The two filtered-out arrivals are still in the session: the ring saw them,
    // and so did the per-frame tallies.
    assert_eq!(
        app.snap.fr_trace.len(),
        3,
        "the filter gates the file, never the trace"
    );
    assert_eq!(app.snap.fr_aggs.len(), 3, "one tally per arrival");
    std::fs::remove_file(&actual).ok();
}

/// The filter holds for trigger-started recordings too: the pre-trigger
/// context the trigger drains into the file only ever held whitelisted
/// frames, so a filtered recording stays clean end to end.
#[test]
fn a_trigger_started_recording_follows_the_record_filter() {
    let mut app = App::headless();
    let path = std::env::temp_dir().join("roxy_can_trig_filtered.asc");
    app.record_path_buf = path.to_string_lossy().to_string();
    app.set_record_filter(crate::recorder::RecordFilter::can(vec![(0x100, false)]));
    app.toggle_record();
    app.triggers.push(Trigger::new(
        TriggerCond::IdPresent { ch: 0, id: 0x100 },
        TriggerAction::StartRecording,
    ));
    app.add_tx(0, 0x999);
    for tx in &mut app.tx_list {
        tx.active = true;
        tx.cycle_us = 10_000;
    }
    open_all_gates(&mut app);
    app.start_virtual();
    for _ in 0..12 {
        std::thread::sleep(std::time::Duration::from_millis(11));
        app.update();
    }
    app.stop();

    let actual = app.recorder.last_record.clone();
    let content = std::fs::read_to_string(&actual).unwrap();
    let frames = crate::log::asc::parse_asc(&content);
    assert!(
        frames.len() >= 5,
        "whitelisted frames flowed: {}",
        frames.len()
    );
    assert!(
        frames.iter().all(|f| f.id == 0x100),
        "the pre-trigger context and live tail stay filtered"
    );
    if let Some(dir) = std::path::Path::new(&actual).parent()
        && let Ok(rd) = std::fs::read_dir(dir)
    {
        for e in rd.flatten() {
            let n = e.file_name().to_string_lossy().to_string();
            if n.starts_with("roxy_can_trig_filtered") {
                std::fs::remove_file(e.path()).ok();
            }
        }
    }
}

#[test]
fn replay_after_recorded_simulation_creates_no_second_file() {
    let mut app = App::headless();
    let path = std::env::temp_dir().join("roxy_can_replay_rec_test.asc");
    app.record_path_buf = path.to_string_lossy().to_string();
    app.toggle_record();
    for tx in &mut app.tx_list {
        tx.active = true;
        tx.cycle_us = 10_000;
    }
    open_all_gates(&mut app);
    app.start_virtual();
    for _ in 0..12 {
        std::thread::sleep(std::time::Duration::from_millis(11));
        app.update();
    }
    app.stop();
    let first = app.recorder.last_record.clone();
    assert!(!first.is_empty(), "simulation should have recorded a file");
    app.log_path = first.clone();
    app.replay();
    assert!(!app.recorder.recording, "replay must drop the Record state");
    assert_eq!(
        app.recorder.last_record, first,
        "replay must not open a second record file"
    );
    app.stop();
    std::fs::remove_file(&first).ok();
}

/// The toolbar combo, not the draft's typed extension, picks the record
/// backend: `toggle_record` stamps the selected extension onto the stem
/// it forwards to the recorder.
#[test]
fn the_format_combo_picks_the_record_extension() {
    let mut app = App::headless();
    let base = std::env::temp_dir()
        .join("roxy_can_combo_fmt")
        .to_string_lossy()
        .to_string();
    // A draft typed as .asc forwards .blf when the combo says BLF.
    app.record_path_buf = format!("{base}.asc");
    app.record_format = 1;
    app.toggle_record();
    assert!(app.recorder.recording);
    assert_eq!(
        app.recorder.record_path,
        format!("{base}.blf"),
        "combo BLF must win over the typed .asc"
    );
    app.toggle_record();
    // A bare stem forwards .asc with the combo back at its default.
    app.record_path_buf = base.clone();
    app.record_format = 0;
    app.toggle_record();
    assert_eq!(
        app.recorder.record_path,
        format!("{base}.asc"),
        "combo ASC must stamp .asc"
    );
    app.toggle_record();
}

/// The recording's default home follows the project: the toolbar input
/// holds only the file name ("record"), and arming resolves it into the
/// project's Record/ folder. A draft the user typed themselves is left
/// alone, and an absolute path is honored exactly as typed.
#[test]
fn a_project_saves_schema_v2_and_reopens_the_same_cluster_ids() {
    let dir = std::env::temp_dir().join("roxy_can_v2_project");
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("v2.rxproj");
    let mut app = App::headless();
    app.send(crate::bus::BusCommand::AddChannel);
    app.settle();
    let ids = app.snap.cluster_ids.clone();
    assert!(
        !ids.can.is_empty(),
        "the default workspace has a CAN cluster"
    );
    let mut map = crate::replay_map::ReplayMap::default();
    map.set(
        crate::replay_map::LogBusKind::Can,
        3,
        Some(crate::model::ClusterId(ids.can[1])),
    );
    app.send(crate::bus::BusCommand::SetReplayMap(map));
    app.add_tx(1, 0x321);
    app.settle();
    let row = app
        .snap
        .tx
        .iter()
        .position(|t| t.channel == 1 && t.id == 0x321)
        .expect("the new generator row");
    assert!(app.set_tx_hex(row, "11 22 33"), "hex accepted");
    app.set_tx_active(row, true);
    app.triggers.push(crate::trigger::Trigger::new(
        crate::trigger::TriggerCond::SignalCross {
            ch: 1,
            id: 0x321,
            ext: false,
            signal: "EngSpeed".into(),
            threshold: 12.5,
            rising: true,
        },
        crate::trigger::TriggerAction::Send { ch: 1, id: 0x321 },
    ));
    app.trace_windows[0]
        .manual
        .insert(crate::workspace::Pick::Can { ch: 1, id: 0x321 });
    app.refresh_snapshot();
    app.settle();
    assert!(app.save_project(Some(path.clone())), "save writes the file");

    let text = std::fs::read_to_string(&path).unwrap();
    let value: serde_json::Value = serde_json::from_str(&text).unwrap();
    assert_eq!(value["version"], 2);
    assert!(!app.trace_windows.is_empty(), "the bootstrap opens windows");
    assert_eq!(
        value["traceWindows"].as_array().map(Vec::len),
        Some(app.trace_windows.len()),
        "windows are a typed v2 section"
    );
    assert!(value.get("config").is_none(), "there is no compat blob");
    assert!(
        value["traceWindows"][0]["manual"]
            .as_array()
            .is_some_and(|rows| rows.iter().any(|row| row[0] == 1 && row[1] == 0x321)),
        "window picks keep the bus index: {}",
        value["traceWindows"][0]
    );
    assert!(value["clusters"].as_array().unwrap().len() >= ids.can.len());
    assert_eq!(value["clusters"][0]["id"], ids.can[0]);
    assert!(
        value.get("config").is_none(),
        "the project has no compatibility blob"
    );
    assert!(
        value["replay"]
            .as_array()
            .is_some_and(|rows| rows.iter().any(|row| {
                row["kind"] == "can" && row["fileChannel"] == 3 && row["cluster"] == ids.can[1]
            })),
        "replay mapping is written with cluster ids: {}",
        value["replay"]
    );
    let saved = value["generators"]
        .as_array()
        .unwrap()
        .iter()
        .find(|row| row["kind"] == "can" && row["id"] == 0x321)
        .unwrap_or_else(|| panic!("generator row saved: {}", value["generators"]));
    assert_eq!(
        saved["cluster"], ids.can[1],
        "generator rows use the bus's cluster id"
    );
    assert_eq!(saved["dataText"], "11 22 33");
    assert_eq!(saved["active"], true);
    let rule = value["triggers"]
        .as_array()
        .unwrap()
        .iter()
        .find(|row| row["signal"] == "EngSpeed")
        .unwrap_or_else(|| panic!("trigger row saved: {}", value["triggers"]));
    assert_eq!(
        rule["cluster"], ids.can[1],
        "the condition names its cluster: {rule}"
    );
    assert_eq!(
        rule["actionCluster"], ids.can[1],
        "the reaction send names its cluster"
    );
    assert_eq!(rule["threshold"], 12.5);
    assert_eq!(rule["kind"], 0);

    let mut restored = App::headless();
    restored.open_project_path(&path);
    assert_eq!(
        restored.trace_windows.len(),
        app.trace_windows.len(),
        "windows come back from the typed section"
    );
    assert!(
        restored.trace_windows[0]
            .manual
            .contains(&crate::workspace::Pick::Can { ch: 1, id: 0x321 }),
        "the cluster-id pick maps back to its bus"
    );
    assert_eq!(restored.snap.cluster_ids, ids);
    assert_eq!(
        restored.snap.tx.len(),
        app.snap.tx.len(),
        "the v2 generator rows reopen one for one"
    );
    let reopened = restored
        .snap
        .tx
        .iter()
        .find(|t| t.channel == 1 && t.id == 0x321)
        .expect("the v2 generator row reopens");
    assert_eq!(reopened.data_text, "11 22 33");
    assert!(reopened.active);
    let rule = restored
        .snap
        .triggers
        .iter()
        .find(|t| matches!(&t.cond, crate::trigger::TriggerCond::SignalCross { signal, .. } if signal == "EngSpeed"))
        .expect("the v2 trigger reopens");
    assert_eq!(
        rule.cond,
        crate::trigger::TriggerCond::SignalCross {
            ch: 1,
            id: 0x321,
            ext: false,
            signal: "EngSpeed".into(),
            threshold: 12.5,
            rising: true,
        },
        "the cluster id is translated back to its bus"
    );
    assert_eq!(
        rule.action,
        crate::trigger::TriggerAction::Send { ch: 1, id: 0x321 }
    );
    assert_eq!(restored.snap.channel_count, app.snap.channel_count);
    assert_eq!(
        restored
            .snap
            .replay_map
            .target(crate::replay_map::LogBusKind::Can, 3),
        crate::replay_map::ReplayTarget::Cluster(crate::model::ClusterId(ids.can[1])),
        "the v2 replay mapping survives a reopen"
    );
}

/// An ARXML CAN cluster must reach the core as the document's unified model,
/// not as a DBC-shaped table re-derived from it: only the document carries
/// structure the table cannot express. The model also has to survive a
/// reopen, where `SetClusterIds` runs after the file load and rebuilds the
/// adapter models around the restored ids.
#[test]
fn arxml_can_import_reaches_the_unified_model_and_survives_reopen() {
    let dir = std::env::temp_dir().join("roxy_can_arxml_model");
    std::fs::create_dir_all(&dir).unwrap();
    let arxml = dir.join("two.arxml");
    std::fs::write(&arxml, crate::test_assets::CAN_ARXML_SAMPLE).unwrap();
    let mut app = App::headless();
    app.add_empty_channel();
    app.settle();
    let bus = (app.snap.channel_count - 1) as u8;
    app.send(crate::bus::BusCommand::SetChannelConfig {
        ch: bus,
        name: Some("CanA".to_string()),
        dbc_path: None,
        bitrate_kbps: None,
        fd_data_kbps: None,
        node_roles: None,
    });
    app.load_can_arxml_on_bus(arxml.to_string_lossy().as_ref(), "CanA", bus)
        .expect("CanA binds to the new CAN bus");
    app.settle();
    assert_eq!(
        app.snap.channels.get(bus as usize).map(|c| c.name.as_str()),
        Some("CanA"),
        "the chosen cluster becomes the bus"
    );
    let model = app.model_comms[bus as usize]
        .as_ref()
        .expect("the cluster's model");
    let cluster = model.clusters.values().next().expect("one cluster");
    assert!(
        matches!(
            cluster.sources.first(),
            Some(crate::model::ClusterSource::Arxml { .. })
        ),
        "the model is read from the document, not the adapter: {:?}",
        cluster.sources
    );
    assert_eq!(
        cluster.id,
        crate::model::ClusterId(app.snap.cluster_ids.can[bus as usize]),
        "the document model is rebound to the channel's global id"
    );
    assert!(
        model.frames.values().any(|f| f.name == "A"),
        "the document's frame is in the model"
    );

    let path = dir.join("model.rxproj");
    assert!(app.save_project(Some(path.clone())), "save writes the file");
    let mut restored = App::headless();
    restored.open_project_path(&path);
    restored.settle();
    let bus = restored
        .snap
        .channels
        .iter()
        .position(|c| c.name == "CanA")
        .expect("CanA is still a channel");
    let model = restored.model_comms[bus]
        .as_ref()
        .expect("the ARXML model after reopen");
    let cluster = model.clusters.values().next().expect("one cluster");
    assert!(
        matches!(
            cluster.sources.first(),
            Some(crate::model::ClusterSource::Arxml { .. })
        ),
        "the reopen restores the document model, not the adapter's: {:?}",
        cluster.sources
    );
    assert_eq!(cluster.id.0, restored.snap.cluster_ids.can[bus]);
}

/// One physical bus may be described by several suppliers' files. The second
/// ARXML cluster joins the first bus instead of becoming its own channel, and
/// the merged document model carries both frames.
#[test]
fn two_arxml_clusters_share_one_can_bus_and_survive_reopen() {
    let dir = std::env::temp_dir().join("roxy_can_multi_can");
    std::fs::create_dir_all(&dir).unwrap();
    let arxml = dir.join("two.arxml");
    std::fs::write(&arxml, crate::test_assets::CAN_ARXML_SAMPLE).unwrap();
    let path = arxml.to_string_lossy().into_owned();

    let mut app = App::headless();
    app.add_empty_channel();
    app.settle();
    let bus = (app.snap.channel_count - 1) as u8;
    app.send(crate::bus::BusCommand::SetChannelConfig {
        ch: bus,
        name: Some("CanA".to_string()),
        dbc_path: None,
        bitrate_kbps: None,
        fd_data_kbps: None,
        node_roles: None,
    });
    app.load_can_arxml_on_bus(&path, "CanA", bus).unwrap();
    app.load_can_arxml_on_bus(&path, "CanB", bus).unwrap();
    app.settle();
    assert_eq!(app.snap.channel_count, 3, "the second cluster reused bus 2");
    assert_eq!(app.can_arxml.get(&bus).map(Vec::len), Some(2));
    let model = app.model_comms[bus as usize]
        .as_ref()
        .expect("the merged model");
    assert!(
        model.frames.values().any(|frame| frame.name == "A")
            && model.frames.values().any(|frame| frame.name == "B"),
        "both documents' frames are decoded on the same bus"
    );
    let cluster = model.clusters.values().next().expect("one cluster");
    assert_eq!(
        cluster.sources.len(),
        2,
        "the bus lists both descriptions: {:?}",
        cluster.sources
    );

    let project = dir.join("multi.rxproj");
    assert!(app.save_project(Some(project.clone())));
    let mut restored = App::headless();
    restored.open_project_path(&project);
    restored.settle();
    let bus = restored
        .snap
        .channels
        .iter()
        .position(|channel| channel.name == "CanA")
        .expect("CanA is still a channel");
    assert_eq!(
        restored.can_arxml.get(&(bus as u8)).map(Vec::len),
        Some(2),
        "both descriptions are saved and rebound on open"
    );
    let model = restored.model_comms[bus]
        .as_ref()
        .expect("the merged model after reopen");
    assert!(
        model.frames.values().any(|frame| frame.name == "A")
            && model.frames.values().any(|frame| frame.name == "B"),
        "both frames survive the project round trip"
    );
}

/// Acceptance with the bundled cantools/canmatrix corpus: one CAN bus takes
/// the DBC starter plus cantools' `system-4.2.arxml` cluster, and one FlexRay
/// bus takes both FIBEX exports. Both remain merged after a project round
/// trip, which is the multi-supplier shape the bus-first binding exists for.
#[test]
fn cantools_corpus_binds_several_descriptions_to_each_bus() {
    let arxml = crate::test_assets::path("system-4.2.arxml");
    let fibex_a = crate::test_assets::path("PowerTrain_v2.xml");
    let fibex_b = crate::test_assets::path("PowerTrain2_v2.xml");

    let mut app = App::headless();
    app.load_can_arxml_on_bus(arxml, "Cluster0", 0)
        .expect("cantools CAN cluster binds to CAN1");
    app.settle();
    let model = app.model_comms[0].as_ref().expect("the merged CAN model");
    assert!(
        model
            .frames
            .values()
            .any(|frame| frame.name == "EngineStatus"),
        "the DBC starter is still decoded on the bus"
    );
    let message1 = model
        .frames
        .values()
        .find(|frame| frame.name == "Message1")
        .expect("the cantools frame is merged in");
    assert!(
        !model.signals_in_frame(message1).is_empty(),
        "the cantools frame keeps its signals: {:?}",
        message1.name
    );
    let cluster = model.clusters.values().next().expect("one bus cluster");
    assert!(
        cluster.sources.iter().any(|source| matches!(
            source,
            crate::model::ClusterSource::Dbc { path } if path == "builtin:sample.dbc"
        )) && cluster.sources.iter().any(|source| matches!(
            source,
            crate::model::ClusterSource::Arxml { path, .. } if path == arxml
        )),
        "the bus lists both the DBC and the ARXML: {:?}",
        cluster.sources
    );

    let fr_bus = app.add_flexray_bus().expect("a FlexRay bus");
    app.add_fr_description(fr_bus, crate::channel::description_source(fibex_a, None))
        .expect("the first FIBEX binds");
    let one_file = app
        .fr_db(fr_bus)
        .expect("the first description")
        .frames
        .len();
    app.add_fr_description(fr_bus, crate::channel::description_source(fibex_b, None))
        .expect("the second FIBEX shares the schedule");
    let merged_frames = app.fr_db(fr_bus).expect("the merged schedule").frames.len();
    assert!(
        merged_frames >= one_file,
        "merging never loses the first file's frames ({one_file} -> {merged_frames})"
    );
    assert_eq!(app.fr_sources.get(&fr_bus).map(Vec::len), Some(2));

    let dir = std::env::temp_dir().join("roxy_can_cantools_corpus");
    std::fs::create_dir_all(&dir).unwrap();
    let project = dir.join("corpus.rxproj");
    assert!(app.save_project(Some(project.clone())));
    let mut restored = App::headless();
    restored.open_project_path(&project);
    restored.settle();
    let can = restored
        .snap
        .channels
        .iter()
        .position(|channel| channel.name == "CAN1")
        .expect("CAN1 survives");
    let model = restored.model_comms[can].as_ref().expect("the CAN model");
    assert!(
        model.frames.values().any(|frame| frame.name == "Message1")
            && model
                .frames
                .values()
                .any(|frame| frame.name == "EngineStatus"),
        "both descriptions are rebound on open"
    );
    assert_eq!(
        restored.fr_sources.get(&fr_bus).map(Vec::len),
        Some(2),
        "both FIBEX files are rebound on open"
    );
    assert_eq!(
        restored.fr_db(fr_bus).map(|db| db.frames.len()),
        Some(merged_frames),
        "the merged FlexRay schedule survives the round trip"
    );
}

/// Ethernet matrices merge per bus; a renamed second file adds a second
/// cluster and its frame rather than replacing the first.
#[test]
fn a_second_ethernet_matrix_joins_the_same_bus() {
    let dir = std::env::temp_dir().join("roxy_can_multi_eth");
    std::fs::create_dir_all(&dir).unwrap();
    let first = dir.join("eth1.arxml");
    std::fs::write(&first, crate::test_assets::ETH_ARXML_SAMPLE).unwrap();
    let second = dir.join("eth2.arxml");
    let text = crate::test_assets::ETH_ARXML_SAMPLE
        .replace("EthCluster", "EthCluster2")
        .replace("EthFrame", "EthFrame2")
        .replace("Pdu", "Pdu2")
        .replace("Sig", "Sig2")
        .replace("Cm", "Cm2");
    std::fs::write(&second, text).unwrap();

    let mut app = App::headless();
    let bus = app.add_eth_bus().expect("an Ethernet bus index");
    app.add_eth_description(
        bus,
        crate::channel::description_source(&first.to_string_lossy(), Some("EthCluster")),
    )
    .unwrap();
    app.add_eth_description(
        bus,
        crate::channel::description_source(&second.to_string_lossy(), Some("EthCluster2")),
    )
    .unwrap();
    app.settle();
    assert_eq!(app.eth_sources.get(&bus).map(Vec::len), Some(2));
    let db = app.eth_db(bus).expect("the merged matrix");
    assert_eq!(db.clusters.len(), 2, "both clusters are on the bus");
    assert_eq!(db.frames.len(), 2, "and both frames decode there");
}

#[test]
fn loading_a_project_points_the_record_draft_at_its_record_folder() {
    let dir = std::env::temp_dir().join("roxy_can_record_home");
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("net.rxproj");
    let mut app = App::headless();
    assert!(app.save_project(Some(path.clone())), "save writes the file");
    assert_eq!(
        app.record_path_buf, "record",
        "the draft shows the bare default name"
    );

    // Arming resolves the name into the project's Record/ folder.
    app.toggle_record();
    assert!(app.recorder.recording);
    let expected = std::path::Path::new(&dir)
        .join("Record")
        .join("record.asc")
        .to_string_lossy()
        .replace('\\', "/");
    let sent = app.recorder.record_path.replace('\\', "/");
    assert_eq!(sent, expected, "relative name lands in Record/");
    app.toggle_record();

    // A fresh load from disk resets the draft to the default name too.
    let mut app = App::headless();
    app.open_project_path(&path);
    assert_eq!(
        app.record_path_buf, "record",
        "draft retargeted on load: {}",
        app.record_path_buf
    );

    // A draft the user typed survives a Save As of the same workspace.
    app.record_path_buf = "D:/mylogs/custom".to_string();
    assert!(app.save_project(Some(path.clone())));
    assert_eq!(
        app.record_path_buf, "D:/mylogs/custom",
        "Save As must not clobber a custom draft"
    );
    app.toggle_record();
    assert_eq!(
        app.recorder.record_path, "D:/mylogs/custom.asc",
        "absolute paths are honored as typed"
    );
    app.toggle_record();
    std::fs::remove_file(&path).ok();
}

#[test]
fn loading_log_does_not_start_replay() {
    let mut app = App::headless();
    let path = std::env::temp_dir().join("roxy_can_load_asc_test.asc");
    app.record_path_buf = path.to_string_lossy().to_string();
    app.toggle_record();
    for tx in &mut app.tx_list {
        tx.active = true;
        tx.cycle_us = 10_000;
    }
    open_all_gates(&mut app);
    app.start_virtual();
    for _ in 0..12 {
        std::thread::sleep(std::time::Duration::from_millis(11));
        app.update();
    }
    app.stop();
    let first = app.recorder.last_record.clone();
    app.load_log(&first);
    assert!(!app.measuring, "loading must not start playback");
    assert!(app.log_info.is_some(), "load should cache a stream summary");
    app.replay();
    assert!(app.measuring, "replay starts on demand");
    assert!(matches!(app.mode, Mode::Replay));
    app.stop();
    std::fs::remove_file(&first).ok();
}

/// Replay navigation is forward-only and reuses the normal step: a step
/// delivers the frames the step covers, play-to resumes and pauses at the
/// target, and a target behind the playhead is refused instead of silently
/// rewinding.
#[test]
fn replay_step_and_play_to_advance_forward() {
    use std::time::{Duration, Instant};
    let path = std::env::temp_dir().join(format!("roxy_replay_nav_{}.asc", std::process::id()));
    std::fs::write(
        &path,
        "base hex  timestamps absolute\n\
         0.000000 Start of measurement\n\
         0.000000  1  100  Rx  d  8  01 02 03 04 05 06 07 08\n\
         0.100000  1  100  Rx  d  8  11 12 13 14 15 16 17 18\n\
         0.200000  1  100  Rx  d  8  21 22 23 24 25 26 27 28\n",
    )
    .unwrap();
    let mut app = App::headless();
    app.load_log(&path.to_string_lossy());
    app.replay();
    app.update();
    assert!(app.measuring);
    let before_step = app.snap.frame_counter;

    app.send(crate::bus::BusCommand::SetTracePaused(true));
    app.update();
    app.send(crate::bus::BusCommand::ReplayStep { delta_us: 150_000 });
    let stepped = app.replay_position().expect("a replay timeline").0;
    assert!(stepped >= 0.150, "step moved the playhead to {stepped}s");
    assert!(
        app.snap.frame_counter > before_step,
        "the 100 ms frame was delivered by the step"
    );

    app.send(crate::bus::BusCommand::ReplayPlayTo { target_us: 200_000 });
    assert!(!app.snap.trace_paused, "play-to resumes the replay");
    let deadline = Instant::now() + Duration::from_secs(2);
    while Instant::now() < deadline && !app.snap.trace_paused {
        std::thread::sleep(Duration::from_millis(5));
        app.update();
    }
    assert!(
        app.snap.trace_paused,
        "play-to pauses at the target; status {}",
        app.status
    );
    assert!(app.replay_position().expect("timeline").0 >= 0.200);

    app.send(crate::bus::BusCommand::ReplayPlayTo { target_us: 100_000 });
    assert!(
        app.status.contains("behind"),
        "a backward target is refused: {}",
        app.status
    );
    app.stop();
    drop(app);
    std::fs::remove_file(&path).ok();
}

#[test]
fn loading_blf_does_not_start_replay() {
    let mut app = App::headless();
    let path = std::env::temp_dir().join("roxy_can_load_blf_test.blf");
    let bytes = crate::log::blf::tests::minimal_file();
    std::fs::write(&path, &bytes).unwrap();
    app.load_log(&path.to_string_lossy());
    assert!(
        app.log_info.is_some(),
        "BLF load should cache a stream summary, got status {:?}",
        app.status
    );
    assert!(!app.measuring, "loading must not start playback");
    app.replay();
    assert!(app.measuring, "replay starts on demand");
    assert!(matches!(app.mode, Mode::Replay));
    app.stop();
    std::fs::remove_file(&path).ok();
}

#[test]
fn open_dropped_reports_unsupported_for_mf4() {
    let mut app = App::headless();
    app.open_dropped(std::path::Path::new("/tmp/does-not-exist.mf4"));
    assert!(
        app.status.contains("unsupported format: MF4"),
        "MF4 should surface a clear reason, got {:?}",
        app.status
    );
}

#[test]
fn aggregates_frames_per_message_id() {
    let mut app = App::headless();
    let tx = app
        .tx_list
        .iter_mut()
        .find(|t| t.id == 0x100)
        .expect("EngineStatus pre-populated in generator");
    tx.active = true;
    tx.cycle_us = 10_000;
    // 角色开关允许发送：0x100 属 EngineECU。
    app.set_node_role(0, "EngineECU", NodeRole::Simulated);
    app.start_virtual();
    for _ in 0..8 {
        std::thread::sleep(std::time::Duration::from_millis(12));
        app.update();
    }
    let agg = app
        .aggs
        .get(&(
            crate::model::ClusterId(app.snap.cluster_ids.can[0]),
            0x100,
            false,
        ))
        .expect("EngineStatus aggregated on CAN1");
    assert!(agg.count >= 5, "expected several frames, got {}", agg.count);
    assert!(
        (agg.cycle_us / 1000.0 - 12.0).abs() < 8.0,
        "cycle should track the update cadence, got {}ms",
        agg.cycle_us / 1000.0
    );
    assert!(agg.min_us > 0.0, "min cycle should be recorded");
    assert!(agg.max_us >= agg.min_us, "max cycle >= min cycle");
    app.stop();
}

/// The observer Clear buttons mean "empty the view": the trace ring and
/// the per-message counters blank out while the run keeps going, and the
/// next arrivals re-populate both. (They used to reset the window's
/// filter settings instead, which read as "does nothing".)
#[test]
fn clearing_the_trace_and_message_counters_empties_the_views() {
    let mut app = App::headless();
    app.add_tx(0, 0x777);
    let tx = app.tx_list.last_mut().unwrap();
    tx.cycle_us = 10_000;
    tx.active = true;
    // 0x777 is not in the database: unassigned, no role gate applies.
    app.start_virtual();
    run_sim(&mut app, 5, 10_000);
    assert!(app.trace.len() > 0, "frames on the trace before the clear");
    assert!(
        app.aggs.contains_key(&(
            crate::model::ClusterId(app.snap.cluster_ids.can[0]),
            0x777,
            false
        )),
        "a counter exists before the clear"
    );

    app.send(crate::bus::BusCommand::ClearTrace);
    app.send(crate::bus::BusCommand::ClearAggregates);
    assert_eq!(app.trace.len(), 0, "the trace view is blank");
    assert!(app.aggs.is_empty(), "the counters are blank");

    // Still measuring: the next cycles repopulate both views (the clock
    // keeps advancing -- a manual clear happens mid-run).
    for i in 6..=10u64 {
        app.sim_t_us = i * 10_000;
        app.tick(app.sim_t_us);
    }
    assert!(app.trace.len() > 0, "frames arrive after the clear");
    assert!(
        app.aggs.contains_key(&(
            crate::model::ClusterId(app.snap.cluster_ids.can[0]),
            0x777,
            false
        )),
        "counters resume after the clear"
    );
    app.stop();
}

/// Drives `ticks` steps of the loop, each `step_us` of simulation time
/// apart. `tick` reads `sim_t_us` directly, so no wall clock is involved.
fn run_sim(app: &mut App, ticks: u32, step_us: u64) {
    for i in 1..=ticks {
        app.sim_t_us = u64::from(i) * step_us;
        app.tick(app.sim_t_us);
    }
}

fn slots_of(app: &App, id: u32) -> Vec<u64> {
    app.trace
        .iter()
        .filter(|f| f.id == id)
        .map(|f| f.t_us)
        .collect()
}

#[test]
fn generator_frames_are_spaced_exactly_one_cycle() {
    let mut app = App::headless();
    app.add_tx(0, 0x777);
    let tx = app.tx_list.last_mut().unwrap();
    tx.cycle_us = 20_000;
    tx.active = true;
    app.start_virtual();
    // Ticks land on multiples of 7 ms, which the 20 ms cycle never lines up
    // with: slots must still come out on exact 20 ms boundaries.
    run_sim(&mut app, 12, 7_000);
    assert_eq!(
        slots_of(&app, 0x777),
        vec![0, 20_000, 40_000, 60_000, 80_000]
    );
    let agg = app
        .aggs
        .get(&(
            crate::model::ClusterId(app.snap.cluster_ids.can[0]),
            0x777,
            false,
        ))
        .expect("aggregate");
    assert_eq!((agg.min_us, agg.max_us), (20_000.0, 20_000.0));
    app.stop();
}

#[test]
fn generator_brs_is_fd_only_and_reaches_the_wire() {
    let mut app = App::headless();
    app.add_tx(0, 0x77B);
    let brs = |app: &App| {
        app.snap
            .tx
            .iter()
            .find(|t| t.channel == 0 && t.id == 0x77B)
            .map(|t| (t.fd, t.brs))
            .expect("generator row")
    };
    app.send(crate::bus::BusCommand::SetEntryFd {
        ch: 0,
        id: 0x77B,
        fd: true,
    });
    app.send(crate::bus::BusCommand::SetEntryBrs {
        ch: 0,
        id: 0x77B,
        brs: true,
    });
    assert_eq!(brs(&app), (true, true));
    app.send(crate::bus::BusCommand::SetEntryFd {
        ch: 0,
        id: 0x77B,
        fd: false,
    });
    assert_eq!(brs(&app), (false, false), "dropping FD clears BRS");
    app.send(crate::bus::BusCommand::SetEntryBrs {
        ch: 0,
        id: 0x77B,
        brs: true,
    });
    assert_eq!(brs(&app), (false, false), "BRS requires FD");

    app.send(crate::bus::BusCommand::SetEntryFd {
        ch: 0,
        id: 0x77B,
        fd: true,
    });
    app.send(crate::bus::BusCommand::SetEntryBrs {
        ch: 0,
        id: 0x77B,
        brs: true,
    });
    {
        let tx = app.tx_list.last_mut().expect("generator row");
        tx.cycle_us = 20_000;
        tx.active = true;
        tx.len = 64;
    }
    app.start_virtual();
    run_sim(&mut app, 2, 20_000);
    let frame = app
        .trace
        .iter()
        .find(|f| f.id == 0x77B)
        .expect("emitted frame");
    assert!(frame.is_fd() && frame.brs());
    app.stop();
}

#[test]
fn on_stop_keeps_aggregated_test_results_and_clears_them_on_restart() {
    let mut app = App::headless();
    app.send(crate::bus::BusCommand::AddNode {
        name: "checks".to_string(),
        channel: 0,
        attached: None,
    });
    app.settle();
    let id = app.snap.nodes[0].id;
    app.send(crate::bus::BusCommand::SetNodeSource {
        id,
        source: "on start { test(\"started\", true); }\n\
                 on timer 10 { test(\"tick\", now() < 1.0); test(\"range\", 5 < 3); }\n\
                 on stop { test(\"finished\", now() > 0.0); }"
            .to_string(),
    });
    app.settle();
    app.start_virtual();
    app.settle();
    for t in 1..=30 {
        app.advance_clock(t * 1_000);
        app.tick(t * 1_000);
    }
    app.stop();
    app.settle();

    let node = &app.snap.nodes[0];
    assert!(
        !node.tests.is_empty(),
        "no tests recorded; log={:?} issue={:?}",
        node.log,
        node.issue
    );
    let test = |name: &str| {
        node.tests
            .iter()
            .find(|test| test.name == name)
            .unwrap_or_else(|| panic!("missing test {name}"))
    };
    assert_eq!((test("started").runs, test("started").failures), (1, 0));
    assert_eq!(test("tick").runs, 2, "one per due timer beat");
    assert_eq!(test("tick").failures, 0);
    assert_eq!((test("range").runs, test("range").failures), (2, 2));
    assert_eq!((test("finished").runs, test("finished").failures), (1, 0));
    assert!(test("finished").time_s > 0.0, "the stop event has a clock");
    assert!(
        node.log
            .iter()
            .any(|line| line == "[tests] 3/4 passed; 1 failed: range"),
        "the run ends with one verdict line: {:?}",
        node.log
    );
    assert!(
        app.write_log
            .iter()
            .any(|line| line.text.contains("3/4 passed; 1 failed: range")),
        "the verdict reaches Write"
    );

    app.start_virtual();
    app.settle();
    assert!(
        app.snap.nodes[0].tests.is_empty(),
        "a new run does not inherit the previous verdict"
    );
    app.stop();
}

#[test]
fn a_tick_exactly_on_a_slot_does_not_drop_a_cycle() {
    let mut app = App::headless();
    app.add_tx(0, 0x779);
    let tx = app.tx_list.last_mut().unwrap();
    tx.cycle_us = 20_000;
    tx.active = true;
    app.start_virtual();
    // Ticks land precisely on slot boundaries. A slot is due when the
    // clock reaches it, so the tick sitting on 120 ms owns that slot too --
    // nothing is skipped, and nothing waits a tick past its own stamp.
    run_sim(&mut app, 6, 20_000);
    assert_eq!(
        slots_of(&app, 0x779),
        vec![0, 20_000, 40_000, 60_000, 80_000, 100_000, 120_000],
        "one frame per cycle, none skipped at the boundary"
    );
    app.stop();
}

#[test]
fn a_stalled_ui_backfills_the_slots_it_missed() {
    let mut app = App::headless();
    app.add_tx(0, 0x778);
    let tx = app.tx_list.last_mut().unwrap();
    tx.cycle_us = 20_000;
    tx.active = true;
    app.start_virtual();
    app.sim_t_us = 0;
    app.tick(0);
    // Twelve cycles' worth of stall: every missed slot still goes out at
    // its own stamp. Skipping the backlog used to punch a hole into the
    // bus's own timeline, and at fine Graphics strides that hole read as
    // the curve being eaten while the plot slid on.
    app.sim_t_us = 250_000;
    app.tick(250_000);
    assert_eq!(
        slots_of(&app, 0x778),
        (0..13u64).map(|i| i * 20_000).collect::<Vec<_>>(),
        "the backlog was emitted, not skipped"
    );
    assert_eq!(
        app.tx_list.last().unwrap().next_t_us,
        260_000,
        "the schedule resumes past the stall"
    );
    app.stop();
}

#[test]
fn a_stall_backfill_is_bounded_per_tick() {
    let mut app = App::headless();
    app.add_tx(0, 0x778);
    let tx = app.tx_list.last_mut().unwrap();
    tx.cycle_us = 1_000;
    tx.active = true;
    app.start_virtual();
    app.sim_t_us = 0;
    app.tick(0);
    // A 100 s freeze at a 1 ms cycle owes 100 000 slots: one tick takes a
    // bounded bite and the rest streams over the following ticks.
    app.sim_t_us = 100_000_000;
    app.tick(app.sim_t_us);
    assert_eq!(
        slots_of(&app, 0x778).len() as u32,
        MAX_TX_CATCHUP + 1,
        "slot 0 plus the bounded burst"
    );
    assert!(
        app.tx_list.last().unwrap().next_t_us <= app.sim_t_us,
        "still behind, so the next tick keeps streaming"
    );
    app.tick(app.sim_t_us);
    assert_eq!(
        slots_of(&app, 0x778).len() as u32,
        2 * MAX_TX_CATCHUP + 1,
        "each tick takes another bounded bite"
    );
    app.stop();
}

#[test]
fn a_ui_stall_never_punches_a_hole_into_the_sample_timeline() {
    let mut app = App::headless();
    let key = {
        let db = app.channel_dbc(0).expect("sample DBC loaded");
        let id = db.order[0].0;
        SigKey::can_in(
            crate::model::ClusterId(1),
            id,
            false,
            db.messages[&(id, false)].signals[0].name.clone(),
        )
    };
    app.subscribe(key.clone());
    for tx in &mut app.tx_list {
        tx.active = true;
        tx.cycle_us = 10_000;
    }
    open_all_gates(&mut app);
    app.start_virtual();
    app.graphics[0].opened = true;
    app.graphics[0].time_window_s = 1.0;
    // Two seconds of ordinary ticks, then the clock jumps a 0.4 s UI
    // freeze in one step, then ordinary ticks resume. Whatever the stall,
    // the sampled timeline must stay on its 10 ms grid.
    for _ in 0..120 {
        app.sim_t_us += 16_667;
        app.tick(app.sim_t_us);
    }
    app.sim_t_us += 400_000;
    app.tick(app.sim_t_us);
    for _ in 0..60 {
        app.sim_t_us += 16_667;
        app.tick(app.sim_t_us);
    }
    let stamps: Vec<u64> = app.subs[&key].history.iter().map(|&(t, _)| t).collect();
    assert!(
        stamps.len() > 200,
        "100 Hz for ~3.4 s, got {}",
        stamps.len()
    );
    for (a, b) in stamps.iter().zip(stamps.iter().skip(1)) {
        assert!(
            b - a <= 20_000,
            "a {} us gap at {:.3}s -- the stall ate the samples",
            b - a,
            *a as f64 / 1e6
        );
    }
    app.stop();
}

#[test]
fn a_pause_freezes_the_simulation_clock_and_its_phase() {
    let mut app = App::headless();
    app.start_virtual();
    app.update();
    let before = app.sim_t_us;
    // Age the wall clock by 400 ms the way a real pause would, then check
    // the simulation clock neither moves during the pause nor absorbs the
    // paused span afterwards.
    app.trace_paused = true;
    app.refresh_snapshot();
    app.t0 = Instant::now() - std::time::Duration::from_millis(400);
    app.update();
    assert_eq!(app.sim_t_us, before, "a paused clock must not advance");
    app.trace_paused = false;
    app.refresh_snapshot();
    app.update();
    assert!(
        app.sim_t_us - before < 50_000,
        "resuming absorbed the paused span: sim advanced {} us",
        app.sim_t_us - before
    );
    app.stop();
}

/// A 16-byte message with one signal in the classic area and one starting at
/// byte 9, so payload widening is testable without an FD asset.
const WIDE_DBC: &str = r#"VERSION "roxy-can test database"

NS_ :

BU_: ECU

BO_ 768 WideMsg: 16 ECU
 SG_ NearSig : 0|16@1+ (1,0) [0|65535] "" ECU
 SG_ FarSig : 72|16@1+ (1,0) [0|65535] "" ECU
"#;

/// Channel 0 on [`WIDE_DBC`] with one active, source-driven `WideMsg`. The
/// default period is one second, so a slot `t` microseconds into the run
/// carries `(t as f64 / 1e6) * hi`.
fn driven_app(signal: &str, kind: crate::sim::SrcKind, lo: f64, hi: f64) -> App {
    let mut app = App::headless();
    app.set_can_table_for_test(
        0,
        Some(std::sync::Arc::new(
            crate::dbc::load_dbc_str(WIDE_DBC).expect("wide dbc parses"),
        )),
    );
    // The generator encodes through the unified model, so the fixture has to
    // build the model the direct table assignment skipped.
    app.rebuild_can_model(0);
    app.add_tx(0, 0x300);
    // 测试 DBC 的发送节点 ECU 需要角色开关允许发送。
    app.set_node_role(0, "ECU", NodeRole::Simulated);
    let tx = app.tx_list.last_mut().expect("tx entry added");
    tx.cycle_us = 20_000;
    tx.active = true;
    tx.srcs.push(ValueSrc::new(signal, kind, lo, hi));
    app.start_virtual();
    app
}

fn emitted(app: &App, id: u32) -> Vec<CanFrame> {
    app.trace.iter().filter(|f| f.id == id).copied().collect()
}

fn raw_at(f: &CanFrame, start_bit: u64) -> u64 {
    crate::decode::extract_raw(&f.data, start_bit, 16, false)
}

#[test]
fn a_driven_signal_carries_the_value_of_its_own_timestamp() {
    // Ticks land off the 20 ms slot grid on purpose, so a payload read at
    // the tick instead of at the stamp it carries would come out wrong.
    let mut app = driven_app("NearSig", crate::sim::SrcKind::Ramp, 0.0, 1000.0);
    run_sim(&mut app, 12, 7_000);
    let frames = emitted(&app, 0x300);
    let slots: Vec<u64> = frames.iter().map(|f| f.t_us).collect();
    assert_eq!(slots, vec![0, 20_000, 40_000, 60_000, 80_000]);
    let vals: Vec<u64> = frames.iter().map(|f| raw_at(f, 0)).collect();
    assert_eq!(
        vals,
        vec![0, 20, 40, 60, 80],
        "each frame must hold the ramp value at its own stamp"
    );
    app.stop();
}

#[test]
fn the_wall_clock_cannot_move_a_generated_value() {
    let value_at = |now_us: u64| {
        let mut app = driven_app("NearSig", crate::sim::SrcKind::Ramp, 0.0, 1000.0);
        app.tx_list.last_mut().unwrap().next_t_us = 40_000;
        app.sim_t_us = 45_000;
        app.tick(now_us);
        raw_at(&emitted(&app, 0x300)[0], 0)
    };
    assert_eq!(
        value_at(45_000),
        value_at(9_999_999),
        "payloads must depend on simulation time only"
    );
    assert_eq!(value_at(9_999_999), 40, "the value at the slot stamped");
}

#[test]
fn driving_a_signal_leaves_the_base_payload_alone() {
    let mut app = driven_app("NearSig", crate::sim::SrcKind::Sine, 0.0, 1000.0);
    let base = app.tx_list.last().unwrap().data;
    run_sim(&mut app, 30, 7_000);
    assert!(
        emitted(&app, 0x300).iter().any(|f| raw_at(f, 0) != 0),
        "the source should have moved something by now"
    );
    assert_eq!(
        app.tx_list.last().unwrap().data,
        base,
        "a waveform sample must not become the saved base payload"
    );
    app.stop();
}

#[test]
fn a_driven_signal_past_byte_8_widens_the_frame() {
    let mut app = driven_app("FarSig", crate::sim::SrcKind::Ramp, 0.0, 1000.0);
    let i = app.tx_list.len() - 1;
    assert!(app.set_tx_hex(i, "00 01 02 03 04 05 06 07"));
    app.tx_list[i].next_t_us = 70_000;
    app.sim_t_us = 70_000;
    app.tick(70_000);
    let f = emitted(&app, 0x300).pop().expect("one frame");
    // Bits 72..88 need 11 bytes; 11 is not a legal FD length, so the frame
    // goes out at 12 with the FD flag set.
    assert_eq!(f.len, 12, "widened to the next legal FD length");
    assert!(f.flags.contains(FrameFlags::FD), "widening implies FD");
    assert_eq!(raw_at(&f, 72), 70, "the driven bytes are really there");
    assert_eq!(raw_at(&f, 0), 0x0100, "the base bytes still come through");
    assert_eq!(
        app.tx_list[i].len, 8,
        "only the emitted frame grows, not the base"
    );
    app.stop();
}

#[test]
fn pin_signal_stops_only_that_signal() {
    let mut app = driven_app("NearSig", crate::sim::SrcKind::Ramp, 0.0, 1000.0);
    let i = app.tx_list.len() - 1;
    app.set_source(
        i,
        ValueSrc::new("FarSig", crate::sim::SrcKind::Sine, 0.0, 100.0),
    );
    assert_eq!(app.tx_list[i].srcs.len(), 2);
    assert!(app.pin_signal(i, "NearSig", 250.0));
    let names: Vec<&str> = app.tx_list[i]
        .srcs
        .iter()
        .map(|s| s.name.as_str())
        .collect();
    assert_eq!(names, ["FarSig"], "pinning must not stop the other source");
    assert_eq!(
        crate::decode::extract_raw(&app.tx_list[i].data, 0, 16, false),
        250,
        "pinned into the base"
    );
    assert_eq!(
        app.tx_list[i].data_text,
        "FA 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00"
    );
    assert!(
        !app.pin_signal(i, "NoSuchSignal", 1.0),
        "unknown signal refused"
    );
    app.stop();
}

#[test]
fn a_hex_edit_keeps_the_sources_running() {
    let mut app = driven_app("NearSig", crate::sim::SrcKind::Ramp, 0.0, 1000.0);
    let i = app.tx_list.len() - 1;
    assert!(!app.set_tx_hex(i, "0 zz"), "non-hex must not apply");
    assert_eq!(app.tx_list[i].len, 16, "the rejected edit changed nothing");
    assert!(app.set_tx_hex(i, "11 22 33"));
    assert_eq!(app.tx_list[i].len, 3);
    assert_eq!(app.tx_list[i].data_text, "11 22 33", "text stays canonical");
    assert_eq!(
        app.tx_list[i].srcs.len(),
        1,
        "fixing one byte must not throw away the stimulus setup"
    );
    app.stop();
}

#[test]
fn set_source_replaces_by_name() {
    let mut app = driven_app("NearSig", crate::sim::SrcKind::Ramp, 0.0, 1000.0);
    let i = app.tx_list.len() - 1;
    app.set_source(
        i,
        ValueSrc::new("NearSig", crate::sim::SrcKind::Sine, 0.0, 50.0),
    );
    assert_eq!(app.tx_list[i].srcs.len(), 1, "the same name is one source");
    assert_eq!(app.tx_list[i].srcs[0].hi, 50.0, "and the later one wins");
    app.clear_source(i, "NearSig");
    assert!(app.tx_list[i].srcs.is_empty());
    app.stop();
}

/// A DBC that declares 0 ms means event-triggered; one that declares
/// nothing at all must still get the invented fallback period.
const CYCLE_TEST_DBC: &str = r#"VERSION "roxy-can cycle test"

NS_ :

BU_: ECU

BO_ 4096 EventMsg: 8 ECU
 SG_ S : 0|8@1+ (1,0) [0|0] "" ECU

BO_ 4097 DefaultedMsg: 8 ECU
 SG_ S : 0|8@1+ (1,0) [0|0] "" ECU

BA_DEF_ BO_  "GenMsgCycleTime" INT 0 10000;
BA_DEF_DEF_  "GenMsgCycleTime" 77;
BA_ "GenMsgCycleTime" BO_ 4096 0;
"#;

#[test]
fn new_generator_entries_inherit_the_declared_cycle() {
    let app = App::headless();
    let cycle = |ch: u8, id: u32| {
        app.tx_list
            .iter()
            .find(|t| t.channel == ch && t.id == id)
            .map(|t| t.cycle_us)
    };
    // The built-in motbus DBC declares these two explicitly...
    assert_eq!(cycle(1, 0x64), Some(133_000), "EngineData 133ms");
    assert_eq!(cycle(1, 0xC9), Some(50_000), "ABSdata 50ms");
    // ...and its BA_DEF_DEF_ puts 100ms on the rest.
    assert_eq!(cycle(1, 0xC7), Some(100_000), "declared default");
    // sample.dbc declares nothing, so the fallback is what shows up.
    assert_eq!(cycle(0, 0x100), Some(100_000), "no declaration -> fallback");
}

#[test]
fn an_event_triggered_message_is_never_auto_sent() {
    let mut app = App::headless();
    app.set_can_table_for_test(
        0,
        Some(std::sync::Arc::new(
            crate::dbc::load_dbc_str(CYCLE_TEST_DBC).unwrap(),
        )),
    );
    // Generator metadata comes from the model, so the fixture has to build it
    // after assigning the table directly.
    app.rebuild_can_model(0);
    app.tx_list.retain(|t| t.channel != 0);
    app.add_tx(0, 4096);
    app.add_tx(0, 4097);
    let i = app.tx_list.len() - 2;
    assert_eq!(
        app.tx_list[i].cycle_us, 0,
        "an explicit 0 is not 'undeclared'"
    );
    assert_eq!(
        app.tx_list[i + 1].cycle_us,
        77_000,
        "the default still applies"
    );
    for t in &mut app.tx_list {
        t.active = true;
    }
    open_all_gates(&mut app);
    app.start_virtual();
    run_sim(&mut app, 20, 10_000);
    assert!(
        slots_of(&app, 4096).is_empty(),
        "event-triggered means no timer"
    );
    assert_eq!(
        slots_of(&app, 4097),
        vec![0, 77_000, 154_000],
        "the declared 77ms period is what runs"
    );
    app.stop();
}

#[test]
fn simulated_node_state_follows_the_bus_it_lives_on() {
    let mut app = App::headless();
    app.set_role_for_test(0, "EngineECU", NodeRole::Simulated);
    app.set_role_for_test(1, "ABS", NodeRole::Simulated);
    app.remove_channel(0);
    assert_eq!(
        app.can_role_of(0, "ABS"),
        NodeRole::Simulated,
        "the survivor keeps its own nodes instead of inheriting the deleted bus's"
    );
    assert_eq!(
        app.can_role_of(0, "EngineECU"),
        NodeRole::Absent,
        "the removed bus's declarations go with it"
    );
}

/// What one bus is actually putting on the wire right now.
fn active_ids(app: &App, ch: u8) -> Vec<u32> {
    let mut ids: Vec<u32> = app
        .tx_list
        .iter()
        .filter(|t| t.channel == ch && t.active)
        .map(|t| t.id)
        .collect();
    ids.sort_unstable();
    ids
}

/// 测试助手（新语义）：声明节点角色为模拟，并启用它名下全部条目。
/// 角色本身不再触碰条目开关——需要旧行为（角色切了就发送）的测试
/// 用这个助手显式补上“启用”一步。
fn simulate_node_and_enable(app: &mut App, ch: u8, node: &str) {
    app.set_node_role(ch, node, NodeRole::Simulated);
    let ids: Vec<u32> = app
        .tx_list
        .iter()
        .filter(|t| t.channel == ch && t.node == node)
        .map(|t| t.id)
        .collect();
    for id in ids {
        app.send(crate::bus::BusCommand::SetEntryActive { ch, id, on: true });
    }
    app.settle();
}

/// 测试助手：把所有总线的所有 DBC 节点角色设为模拟（打开全部开关）。
/// 手动武装（tx.active = true）的测试需要它，否则开关拦截发送。
fn open_all_gates(app: &mut App) {
    for ch in 0..app.snap.channel_count as u8 {
        app.simulate_all_nodes(ch);
    }
    app.settle();
}

fn entry_of(app: &App, ch: u8, id: u32) -> &TxMsg {
    app.tx_list
        .iter()
        .find(|t| t.channel == ch && t.id == id)
        .expect("entry exists")
}

/// 模拟角色补建条目但不发送；逐条启用（SetEntryActive）后只有启用的
/// 条目开始发送，且不影响其他总线。
#[test]
fn ticking_a_node_activates_only_its_own_messages() {
    let mut app = App::headless();
    app.set_node_role(1, "ABS", NodeRole::Simulated);
    // The built-in motbus DBC: ABS owns these three, nobody else.
    assert_eq!(
        app.tx_list
            .iter()
            .filter(|t| t.channel == 1 && t.node == "ABS")
            .count(),
        3
    );
    simulate_node_and_enable(&mut app, 1, "ABS");
    assert_eq!(active_ids(&app, 1), [199, 200, 201]);
    assert!(active_ids(&app, 0).is_empty(), "the other bus untouched");
    assert!(
        app.tx_list
            .iter()
            .filter(|t| t.channel == 1 && t.active)
            .all(|t| t.next_t_us == 0),
        "a ticked node starts on the next tick, not one period later"
    );
    assert_eq!(app.node_role(1, "ABS"), NodeRole::Simulated);
    assert_eq!(
        app.node_role(1, "GearBox"),
        NodeRole::Absent,
        "not a side effect"
    );
}

#[test]
fn ticking_a_node_creates_the_entries_it_lacks() {
    let mut app = App::headless();
    app.tx_list.clear();
    app.set_node_role(1, "ABS", NodeRole::Simulated);
    assert_eq!(
        app.tx_list.iter().filter(|t| t.channel == 1).count(),
        3,
        "the generator refills from the DBC"
    );
    assert_eq!(app.tx_list.len(), 3, "and only this node's messages");
    assert_eq!(entry_of(&app, 1, 201).name, "ABSdata");
    assert_eq!(entry_of(&app, 1, 201).cycle_us, 50_000);
}

/// 新模型：角色 = 总开关，条目开/关 = 自定义。模拟创建条目但不改写
/// 任何开关；条目的启停只经由 SetEntryActive。
#[test]
fn simulating_a_node_creates_entries_but_leaves_them_off() {
    let mut app = App::headless();
    app.tx_list.clear();
    app.set_node_role(1, "ABS", NodeRole::Simulated);
    // 条目已补建（inactive），角色开关已打开——但没有任何一条在发送。
    assert_eq!(app.tx_list.iter().filter(|t| t.channel == 1).count(), 3);
    assert!(
        active_ids(&app, 1).is_empty(),
        "模拟不自动发送：条目开关是用户自定义"
    );

    // 逐条启用后按各自周期发送。
    let ids: Vec<u32> = app
        .tx_list
        .iter()
        .filter(|t| t.channel == 1)
        .map(|t| t.id)
        .collect();
    for id in ids {
        app.send(crate::bus::BusCommand::SetEntryActive {
            ch: 1,
            id,
            on: true,
        });
    }
    app.settle();
    assert_eq!(active_ids(&app, 1), [199, 200, 201]);
    assert_eq!(entry_of(&app, 1, 201).name, "ABSdata");
}

#[test]
fn ticking_a_node_never_overwrites_a_tuned_cycle() {
    let mut app = App::headless();
    let tuned = entry_of(&app, 1, 201).cycle_us;
    assert_eq!(tuned, 50_000, "what the DBC declares");
    let i = app
        .tx_list
        .iter()
        .position(|t| t.channel == 1 && t.id == 201)
        .unwrap();
    app.tx_list[i].cycle_us = 250_000;
    app.set_node_role(1, "ABS", NodeRole::Simulated);
    assert_eq!(
        app.tx_list[i].cycle_us, 250_000,
        "a period someone dialed in outlives the click"
    );
    app.send(crate::bus::BusCommand::SetEntryActive {
        ch: 1,
        id: 201,
        on: true,
    });
    assert!(app.tx_list[i].active, "and the entry is switched on");
}

#[test]
fn unticking_a_node_keeps_its_entries_and_their_stimulus() {
    let mut app = App::headless();
    let i = app
        .tx_list
        .iter()
        .position(|t| t.channel == 1 && t.id == 201)
        .unwrap();
    app.set_source(
        i,
        ValueSrc::new("CarSpeed", crate::sim::SrcKind::Ramp, 0.0, 300.0),
    );
    let before = app.tx_list.len();

    // 逐条目开关是自定义：关掉后条目、激励、周期原样保留。
    app.send(crate::bus::BusCommand::SetEntryActive {
        ch: 1,
        id: 201,
        on: false,
    });
    assert!(active_ids(&app, 1).is_empty(), "stopped sending");
    assert_eq!(app.tx_list.len(), before, "entries survive");
    assert_eq!(app.tx_list[i].srcs.len(), 1, "with the waveform attached");
    assert_eq!(app.tx_list[i].cycle_us, 50_000, "and the declared period");

    app.send(crate::bus::BusCommand::SetEntryActive {
        ch: 1,
        id: 201,
        on: true,
    });
    assert_eq!(
        app.tx_list[i].srcs.len(),
        1,
        "ticking it back on does not rebuild the entry"
    );
    assert!(app.tx_list[i].active);
}

/// 角色开关关闭（节点离线/监听）不影响条目开关：条目仍是"启用"，
/// 只是开关不允许它发送；切回模拟即恢复。
#[test]
fn node_gate_does_not_touch_entry_switches() {
    let mut app = App::headless();
    app.set_node_role(1, "ABS", NodeRole::Simulated);
    let ids: Vec<u32> = app
        .tx_list
        .iter()
        .filter(|t| t.channel == 1)
        .map(|t| t.id)
        .collect();
    for id in ids {
        app.send(crate::bus::BusCommand::SetEntryActive {
            ch: 1,
            id,
            on: true,
        });
    }
    app.settle();

    // 切到离线：开关关闭（条目开/关保留），再切回模拟：原样恢复发送。
    app.set_node_role(1, "ABS", NodeRole::Absent);
    app.settle();
    for t in app.snap.tx.iter().filter(|t| t.channel == 1) {
        assert!(!t.gate_open, "offline closes the gate: {}", t.id);
        assert!(t.active, "switches stay as customized: {}", t.id);
    }

    app.set_node_role(1, "ABS", NodeRole::Simulated);
    app.settle();
    for t in app
        .snap
        .tx
        .iter()
        .filter(|t| t.channel == 1 && t.node == "ABS")
    {
        assert!(t.gate_open, "gate reopens: {}", t.id);
    }
}

/// 开关模型下，DBC 消失不影响“角色开关关闭即停发”：生成器用 base
/// 字节就能发送，DBC 只影响解码与补建。
#[test]
fn the_role_gate_silences_a_node_even_after_its_dbc_is_gone() {
    let mut app = App::headless();
    app.set_node_role(1, "ABS", NodeRole::Simulated);
    let ids: Vec<u32> = app
        .tx_list
        .iter()
        .filter(|t| t.channel == 1 && t.node == "ABS")
        .map(|t| t.id)
        .collect();
    for id in &ids {
        app.send(crate::bus::BusCommand::SetEntryActive {
            ch: 1,
            id: *id,
            on: true,
        });
    }
    app.settle();
    assert_eq!(active_ids(&app, 1).len(), 3);

    app.set_can_table_for_test(1, None);
    app.refresh_snapshot();
    app.set_node_role(1, "ABS", NodeRole::Absent);
    app.settle();
    // 开关语义：开关原样保留（自定义不丢），但角色离线即不发送。
    // 只检查 ABS 名下的三条（其余节点的条目本就是关）。
    for id in &ids {
        let t = app
            .tx_list
            .iter()
            .find(|t| t.channel == 1 && t.id == *id)
            .expect("entry survives");
        assert!(t.active, "switch preserved: {id}");
    }
    assert_eq!(app.node_role(1, "ABS"), NodeRole::Absent);
}

#[test]
fn a_receive_only_node_can_still_be_simulated() {
    let mut app = App::headless();
    app.set_node_role(1, "DashBoard", NodeRole::Simulated);
    assert!(active_ids(&app, 1).is_empty(), "it has no messages to send");
    assert_eq!(
        app.node_role(1, "DashBoard"),
        NodeRole::Simulated,
        "the intent is remembered anyway"
    );
}

/// The zero-cost role: a monitor is present and listening, but the bus
/// carries nothing from it -- behaviourally the same as absent today,
/// and a different declaration for the hardware phase to honour.
#[test]
fn a_monitoring_node_declares_presence_without_traffic() {
    let mut app = App::headless();
    // 模拟并启用全部条目，然后切到监听：条目开关原样保留（自定义不
    // 被角色切换改写），开关关闭即停发。
    app.set_node_role(1, "ABS", NodeRole::Simulated);
    let ids: Vec<u32> = app
        .tx_list
        .iter()
        .filter(|t| t.channel == 1 && t.node == "ABS")
        .map(|t| t.id)
        .collect();
    for id in &ids {
        app.send(crate::bus::BusCommand::SetEntryActive {
            ch: 1,
            id: *id,
            on: true,
        });
    }
    app.settle();
    assert_eq!(active_ids(&app, 1).len(), 3);

    app.set_node_role(1, "ABS", NodeRole::Absent);
    assert_eq!(app.node_role(1, "ABS"), NodeRole::Absent);
    assert!(
        [199, 200, 201].iter().all(|id| app
            .tx_list
            .iter()
            .any(|t| t.channel == 1 && t.id == *id && t.active)),
        "its entries survive, ready for a return to Simulated"
    );
    assert!(
        !app.can_node_roles(1).contains_key("GearBox"),
        "absent stays the stored default: no entry, no traffic"
    );
}

/// The restore chip compares a row against this, so it has to stay the
/// database's own opinion even after the row disagrees with it.
#[test]
fn the_declared_cycle_survives_a_hand_tuned_row() {
    let mut app = App::headless();
    assert_eq!(app.dbc_cycle_us(1, 0xC9), Some(50_000), "ABSdata");
    assert_eq!(
        app.dbc_cycle_us(0, 0x100),
        None,
        "sample.dbc declares nothing"
    );
    assert_eq!(app.dbc_cycle_us(1, 0x5AA), None, "no such message");
    let i = app
        .tx_list
        .iter()
        .position(|t| t.channel == 1 && t.id == 0xC9)
        .unwrap();
    app.tx_list[i].cycle_us = 250_000;
    assert_eq!(
        app.dbc_cycle_us(1, 0xC9),
        Some(50_000),
        "not whatever the row currently says"
    );

    app.set_can_table_for_test(
        0,
        Some(std::sync::Arc::new(
            crate::dbc::load_dbc_str(CYCLE_TEST_DBC).unwrap(),
        )),
    );
    app.refresh_snapshot();
    assert_eq!(
        app.dbc_cycle_us(0, 4096),
        Some(0),
        "a declared 0 is event-triggered, not undeclared"
    );
}

#[test]
fn the_cycle_box_accepts_only_whole_milliseconds_in_range() {
    assert_eq!(cycle_from_ms_text("100"), Some(100_000));
    assert_eq!(cycle_from_ms_text("  133 "), Some(133_000));
    assert_eq!(cycle_from_ms_text("0"), Some(0), "0 is event-triggered");
    assert_eq!(
        cycle_from_ms_text("60000"),
        Some(60_000_000),
        "top of the range"
    );
    assert_eq!(cycle_from_ms_text(""), None, "half-deleted text");
    assert_eq!(cycle_from_ms_text("1.5"), None, "no sub-millisecond step");
    assert_eq!(cycle_from_ms_text("-1"), None);
    assert_eq!(cycle_from_ms_text("60001"), None, "past the ceiling");
    assert_eq!(cycle_from_ms_text("abc"), None);
}

/// The parser writes `""` for a transmitter the DBC never assigned, and
/// that matches every unassigned message at once.
const NO_OWNER_DBC: &str = r#"VERSION "roxy-can orphan test"

NS_ :

BU_: ECU

BO_ 4096 Orphan: 8 Vector__XXX
 SG_ S : 0|8@1+ (1,0) [0|0] "" ECU

"#;

#[test]
fn a_node_with_no_name_simulates_nothing() {
    let mut app = App::headless();
    app.set_can_table_for_test(
        0,
        Some(std::sync::Arc::new(
            crate::dbc::load_dbc_str(NO_OWNER_DBC).unwrap(),
        )),
    );
    app.tx_list.retain(|t| t.channel != 0);
    app.add_tx(0, 4096);
    assert_eq!(entry_of(&app, 0, 4096).node, "", "unassigned");

    app.set_node_role(0, "", NodeRole::Simulated);
    assert!(
        app.can_node_roles(0).is_empty(),
        "not even recorded as a tick"
    );
    assert!(
        active_ids(&app, 0).is_empty(),
        "an empty name must not adopt every message without an owner"
    );
}

#[test]
fn tx_generator_emits_frames() {
    let mut app = App::headless();
    app.add_tx(0, 0x777);
    let tx = app.tx_list.last_mut().expect("tx entry added");
    assert_eq!(tx.id, 0x777);
    assert_eq!(tx.channel, 0);
    assert!(!tx.active, "new entries start inactive");
    tx.cycle_us = 20_000;
    tx.active = true;
    app.start_virtual();
    app.update();
    assert!(
        app.trace
            .iter()
            .any(|f| f.id == 0x777 && matches!(f.dir, Direction::Tx)),
        "expected a Tx frame from the generator"
    );
    assert!(
        app.aggs.contains_key(&(
            crate::model::ClusterId(app.snap.cluster_ids.can[0]),
            0x777,
            false
        )),
        "generator frames aggregate"
    );
    app.stop();
}

#[test]
fn a_reference_log_decodes_the_selected_can_signals() {
    let app = App::headless();
    let db = app.channel_dbc(0).expect("sample DBC loaded");
    let id = db.order[0].0;
    let name = db.messages[&(id, false)].signals[0].name.clone();
    let key = SigKey::can_in(crate::model::ClusterId(1), id, false, name);
    let path = std::env::temp_dir().join(format!("roxy_reference_{}.asc", std::process::id()));
    let mut writer = AscWriter::new(&path.to_string_lossy()).unwrap();
    for i in 0..6u8 {
        let mut frame = CanFrame {
            t_us: u64::from(i) * 100_000,
            channel: 0,
            id,
            extended: false,
            len: 8,
            data: [0; MAX_CAN_FD_LEN],
            dir: Direction::Rx,
            flags: FrameFlags::NONE,
        };
        frame.data = [i + 1; MAX_CAN_FD_LEN];
        writer.write(&frame).unwrap();
    }
    writer.finish().ok();

    let run = crate::reference::load(&path.to_string_lossy(), std::slice::from_ref(&key), &app)
        .expect("reference decodes");
    assert_eq!(run.signals.len(), 1);
    assert_eq!(run.signals[0].key, crate::web::series_key(&key));
    assert!(
        run.signals[0].points.len() >= 4,
        "the reference samples the log"
    );
    std::fs::remove_file(&path).ok();
}

#[test]
fn derived_channels_sample_their_subscribed_sources() {
    let mut app = App::headless();
    let a = SigKey::can_in(crate::model::ClusterId(1), 0x100, false, "A");
    let b = SigKey::can_in(crate::model::ClusterId(1), 0x101, false, "B");
    app.subscribe_signal(a.clone());
    app.subscribe_signal(b.clone());
    for i in 0..5u64 {
        let t = i * 100_000;
        app.subs.get_mut(&a).unwrap().push_sample(t, i as f64, 1);
        app.subs.get_mut(&b).unwrap().push_sample(t, 10.0, 1);
    }
    app.set_derived_defs(vec![crate::derived::DerivedDef {
        id: "d1".into(),
        name: "Sum".into(),
        unit: "V".into(),
        expression: "[A] + [B]".into(),
        band_set_id: None,
        bands: Vec::new(),
        cuts: vec![12.0],
        names: vec!["low".into(), "high".into()],
        sources: Vec::new(),
    }]);
    let key = SigKey::derived("d1", "Sum");
    let sub = app.subs.get(&key).expect("the derived lane is subscribed");
    let points: Vec<(u64, f64)> = sub.history.iter().copied().collect();
    assert_eq!(points.len(), 5, "every source instant is sampled");
    assert_eq!(points[0], (0, 10.0));
    assert_eq!(points[4], (400_000, 14.0));
    assert_eq!(sub.unit, "V");
    assert!(
        app.derived_rule("d1").is_some(),
        "bands travel with the def"
    );
    assert_eq!(app.derived[0].error, None);
}

#[test]
fn a_derived_channel_can_use_an_earlier_derived_channel() {
    let mut app = App::headless();
    let a = SigKey::can_in(crate::model::ClusterId(1), 0x100, false, "A");
    app.subscribe_signal(a.clone());
    for i in 0..3u64 {
        app.subs
            .get_mut(&a)
            .unwrap()
            .push_sample(i * 100_000, (i + 1) as f64, 1);
    }
    app.set_derived_defs(vec![
        crate::derived::DerivedDef {
            id: "sum".into(),
            name: "Sum".into(),
            unit: String::new(),
            expression: "[A] + 1".into(),
            ..Default::default()
        },
        crate::derived::DerivedDef {
            id: "twice".into(),
            name: "Twice".into(),
            unit: String::new(),
            expression: "[Sum] * 2".into(),
            ..Default::default()
        },
    ]);
    let twice = app
        .subs
        .get(&SigKey::derived("twice", "Twice"))
        .expect("the second lane samples after the first");
    let values: Vec<f64> = twice.history.iter().map(|(_, value)| *value).collect();
    assert_eq!(values, vec![4.0, 6.0, 8.0]);
}

#[test]
fn a_derived_channel_waits_for_a_source_that_appears_later() {
    let mut app = App::headless();
    app.set_derived_defs(vec![crate::derived::DerivedDef {
        id: "d".into(),
        name: "Late".into(),
        unit: String::new(),
        expression: "[C] * 2".into(),
        ..Default::default()
    }]);
    assert!(
        app.derived[0].error.is_some(),
        "no subscription can resolve the reference yet"
    );
    let c = SigKey::can_in(crate::model::ClusterId(1), 0x102, false, "C");
    app.send(crate::bus::BusCommand::Subscribe { key: c.clone() });
    for i in 0..2u64 {
        app.subs
            .get_mut(&c)
            .unwrap()
            .push_sample(i * 100_000, (i + 3) as f64, 1);
    }
    app.send(crate::bus::BusCommand::Subscribe { key: c });
    let sub = app
        .subs
        .get(&SigKey::derived("d", "Late"))
        .expect("the lane samples once its source exists");
    let values: Vec<f64> = sub.history.iter().map(|(_, value)| *value).collect();
    assert_eq!(values, vec![6.0, 8.0]);
    assert_eq!(app.derived[0].error, None);
}

#[test]
fn a_derived_trigger_follows_the_core_sampled_value() {
    let mut app = App::headless();
    let a = SigKey::can_in(crate::model::ClusterId(1), 0x100, false, "A");
    app.subscribe_signal(a.clone());
    app.subs.get_mut(&a).unwrap().push_sample(0, 1.0, 1);
    app.set_derived_defs(vec![crate::derived::DerivedDef {
        id: "d".into(),
        name: "Sum".into(),
        expression: "[A] + 0".into(),
        ..Default::default()
    }]);
    let key = SigKey::derived("d", "Sum");
    {
        let sub = app
            .subs
            .get_mut(&key)
            .expect("the lane subscribed with its definition");
        sub.latest = 5.0;
        sub.push_sample(1_000_000, 5.0, 1);
    }
    app.send(crate::bus::BusCommand::AddTrigger {
        cond: crate::trigger::TriggerCond::DerivedSignal {
            id: "d".into(),
            threshold: 3.0,
            rising: true,
        },
        action: crate::trigger::TriggerAction::InsertMarker,
        enabled: true,
    });
    let mut status = String::new();
    app.eval_timeout_triggers(2_000_000, 0, &mut status);
    assert_eq!(app.triggers[0].fired, 1, "the first sweep crosses up");
    assert!(app.markers.contains(&2_000_000), "the edge left a marker");

    {
        let sub = app.subs.get_mut(&key).unwrap();
        sub.latest = 1.0;
        sub.push_sample(3_000_000, 1.0, 1);
    }
    app.eval_timeout_triggers(4_000_000, 0, &mut status);
    assert_eq!(app.triggers[0].fired, 1, "a level below the cut is quiet");
    {
        let sub = app.subs.get_mut(&key).unwrap();
        sub.latest = 4.0;
        sub.push_sample(5_000_000, 4.0, 1);
    }
    app.eval_timeout_triggers(6_000_000, 0, &mut status);
    assert_eq!(app.triggers[0].fired, 2, "a new crossing fires again");
}

/// The chart is a subscription owner even for lanes that never entered the
/// project's Graphics window (the demo's eight lanes). Clearing Data must not
/// unmount the live curve; only the last owner going away may unsubscribe.
#[test]
fn someip_trigger_filters_method_and_type() {
    let cond = crate::trigger::TriggerCond::SomeIp {
        bus: 0,
        service: 0x1234,
        method: Some(0x8001),
        message_type: Some(0x02),
    };
    assert!(cond.someip_matches(0, 0x1234, 0x8001, 0x02));
    assert!(!cond.someip_matches(1, 0x1234, 0x8001, 0x02), "wrong bus");
    assert!(
        !cond.someip_matches(0, 0x1235, 0x8001, 0x02),
        "wrong service"
    );
    assert!(
        !cond.someip_matches(0, 0x1234, 0x8002, 0x02),
        "wrong method"
    );
    assert!(!cond.someip_matches(0, 0x1234, 0x8001, 0x00), "wrong type");
    assert!(
        crate::trigger::TriggerCond::SomeIp {
            bus: 0,
            service: 0x1234,
            method: None,
            message_type: None,
        }
        .someip_matches(0, 0x1234, 0x9999, 0x80)
    );
}

/// Panel sends can encode named ARXML fields instead of requiring hex.
#[test]
fn panel_someip_send_encodes_arxml_fields() {
    let mut app = quiet_app();
    app.ensure_eth_bus(0);
    app.measuring = true;
    app.someip_services
        .services
        .push(crate::eth::arxml_service::ServiceDef {
            name: "SpeedService".to_string(),
            id: 0x1234,
            major: 1,
            methods: Vec::new(),
            events: vec![crate::eth::arxml_service::ServiceEvent {
                name: "SpeedEvent".to_string(),
                id: 0x8001,
                fields: vec![crate::eth::arxml_service::ServiceField {
                    name: "speed".to_string(),
                    ty: crate::eth::arxml_service::ServiceType::U16,
                    props: Default::default(),
                }],
            }],
            instances: Vec::new(),
            buses: vec![0],
            issues: Vec::new(),
        });
    app.send(crate::bus::BusCommand::SetSomeIpServices(
        app.someip_services.clone(),
    ));
    app.send(crate::bus::BusCommand::PanelSendSomeIp {
        bus: 0,
        service: 0x1234,
        method: 0x8001,
        client: 1,
        session: 0,
        message_type: 0x02,
        data: Vec::new(),
        values: std::collections::BTreeMap::from([("speed".to_string(), 258.0)]),
        texts: Default::default(),
        transport: "udp".to_string(),
        host: String::new(),
        port: 30490,
    });
    app.refresh_snapshot();
    let event = app.snap.someip.last().expect("panel frame was injected");
    assert_eq!(event.payload, [0x01, 0x02]);
    assert_eq!(event.session, 1, "session 0 means auto");
    app.send(crate::bus::BusCommand::PanelSendSomeIp {
        bus: 0,
        service: 0x1234,
        method: 0x8001,
        client: 1,
        session: 0,
        message_type: 0x02,
        data: Vec::new(),
        values: std::collections::BTreeMap::new(),
        texts: Default::default(),
        transport: "udp".to_string(),
        host: String::new(),
        port: 30490,
    });
    app.refresh_snapshot();
    assert_eq!(app.snap.someip.last().unwrap().session, 2);
}

/// A Panel request is correlated with the response session; the response is
/// reported in Write, and an unanswered request times out.
#[test]
fn someip_request_response_is_correlated() {
    let mut app = quiet_app();
    app.ensure_eth_bus(0);
    app.measuring = true;
    app.someip_services
        .services
        .push(crate::eth::arxml_service::ServiceDef {
            name: "SpeedService".to_string(),
            id: 0x1234,
            major: 1,
            methods: vec![crate::eth::arxml_service::ServiceMethod {
                name: "GetSpeed".to_string(),
                id: 0x0001,
                input: Vec::new(),
                output: vec![crate::eth::arxml_service::ServiceField {
                    name: "speed".to_string(),
                    ty: crate::eth::arxml_service::ServiceType::U16,
                    props: Default::default(),
                }],
            }],
            events: Vec::new(),
            instances: Vec::new(),
            buses: vec![0],
            issues: Vec::new(),
        });
    app.send(crate::bus::BusCommand::SetSomeIpServices(
        app.someip_services.clone(),
    ));
    app.send(crate::bus::BusCommand::PanelSendSomeIp {
        bus: 0,
        service: 0x1234,
        method: 0x0001,
        client: 1,
        session: 0,
        message_type: 0x00,
        data: Vec::new(),
        values: Default::default(),
        texts: Default::default(),
        transport: "udp".to_string(),
        host: String::new(),
        port: 30490,
    });
    app.send(crate::bus::BusCommand::PanelSendSomeIp {
        bus: 0,
        service: 0x1234,
        method: 0x0001,
        client: 1,
        session: 1,
        message_type: 0x80,
        data: vec![0, 7],
        values: Default::default(),
        texts: Default::default(),
        transport: "udp".to_string(),
        host: String::new(),
        port: 30490,
    });
    app.refresh_snapshot();
    assert!(
        app.snap
            .write
            .iter()
            .any(|line| line.text.contains("SOME/IP response 0x1234.0x0001")),
        "response is reported after matching the request session"
    );
}

/// A Button-sent message must survive the sampler even when a generator
/// refreshed the same signal only a few milliseconds earlier.
#[test]
fn fast_generator_does_not_swallow_button_samples() {
    let mut app = quiet_app();
    let key = SigKey::can_in(crate::model::ClusterId(1), 0x100, false, "Value");
    app.subscribe_signal(key.clone());
    let sub = app.subs.get_mut(&key).unwrap();
    sub.push_sample(1_000_000, 10.0, 50_000);
    sub.push_sample(1_005_000, 42.0, 50_000);
    let values: Vec<f64> = sub.history.iter().map(|(_, value)| *value).collect();
    assert_eq!(values, [10.0, 42.0]);
}

#[test]
fn clearing_an_observer_keeps_a_chart_owned_subscription() {
    let mut app = App::headless();
    let key = SigKey::can_in(crate::model::ClusterId(1), 0x100, false, "A");
    app.subscribe_signal(key.clone());
    app.set_win_signal(crate::workspace::PopupTarget::Data(0), key.clone(), true);
    app.set_session_chart_keep([key.clone()]);
    app.set_win_signal(crate::workspace::PopupTarget::Data(0), key.clone(), false);
    assert!(
        app.subs.contains_key(&key),
        "the chart still owns the subscription"
    );

    app.set_session_chart_keep(Vec::<SigKey>::new());
    app.set_win_signal(crate::workspace::PopupTarget::Data(0), key.clone(), true);
    app.set_win_signal(crate::workspace::PopupTarget::Data(0), key.clone(), false);
    assert!(
        !app.subs.contains_key(&key),
        "no chart lane and no observer left: prune unsubscribes"
    );
}

/// A derived lane's subscription belongs to its definition, so removing the
/// observer row (or a chart copy) must leave the lane sampling.
#[test]
fn removing_a_derived_observer_row_keeps_its_lane_subscribed() {
    let mut app = App::headless();
    app.set_derived_defs(vec![crate::derived::DerivedDef {
        id: "d".into(),
        name: "Sum".into(),
        expression: "[A] + 0".into(),
        ..Default::default()
    }]);
    let key = SigKey::derived("d", "Sum");
    app.set_win_signal(crate::workspace::PopupTarget::Data(0), key.clone(), true);
    app.set_session_chart_keep(Vec::<SigKey>::new());
    app.set_win_signal(crate::workspace::PopupTarget::Data(0), key.clone(), false);
    assert!(
        app.subs.contains_key(&key),
        "the definition keeps the lane subscribed"
    );
}

/// The editor pins the exact lane behind each reference. A later observer
/// that watches a same-named signal on another bus must not turn the pinned
/// lane ambiguous and silently stop it.
#[test]
fn pinned_derived_sources_ignore_same_named_observers() {
    let mut app = App::headless();
    let a = SigKey::can_in(crate::model::ClusterId(1), 0x100, false, "A");
    app.subscribe_signal(a.clone());
    app.subs.get_mut(&a).unwrap().push_sample(0, 1.0, 1);
    let def = crate::derived::DerivedDef {
        id: "d".into(),
        name: "Sum".into(),
        expression: "[A] + 1".into(),
        sources: vec![crate::derived::DerivedSource {
            name: "A".into(),
            key: a.text(),
        }],
        ..Default::default()
    };
    app.set_derived_defs(vec![def.clone()]);
    // A second subscription with the same signal name, as Monitor/Data can
    // introduce, would make name-only resolution ambiguous.
    app.subscribe_signal(SigKey::can_in(
        crate::model::ClusterId(2),
        0x200,
        false,
        "A",
    ));
    app.subs.get_mut(&a).unwrap().push_sample(100_000, 2.0, 1);
    app.set_derived_defs(vec![def]);
    assert_eq!(app.derived[0].error, None, "the pinned key still resolves");
    let values: Vec<f64> = app
        .subs
        .get(&SigKey::derived("d", "Sum"))
        .unwrap()
        .history
        .iter()
        .map(|(_, value)| *value)
        .collect();
    assert_eq!(values, vec![2.0, 3.0]);
}

/// An unresolved reference is reported to the editor instead of leaving the
/// lane empty with no explanation.
#[test]
fn an_unresolved_derived_reference_reaches_the_snapshot() {
    let mut app = App::headless();
    app.set_derived_defs(vec![crate::derived::DerivedDef {
        id: "d".into(),
        name: "Broken".into(),
        expression: "[Missing] + 1".into(),
        ..Default::default()
    }]);
    assert!(
        app.snap.derived_errors.contains_key("d"),
        "the core's reason rides the snapshot"
    );
}

/// A rename changes the key (the name is part of it), so observer rows and
/// their per-key State maps have to move to the new key with it.
#[test]
fn renaming_a_derived_channel_remaps_observer_keys() {
    let mut app = App::headless();
    let def = |name: &str| crate::derived::DerivedDef {
        id: "d".into(),
        name: name.into(),
        expression: "[A] + 0".into(),
        ..Default::default()
    };
    app.set_derived_defs(vec![def("Old")]);
    let old = SigKey::derived("d", "Old");
    let new = SigKey::derived("d", "New");
    for target in [
        crate::workspace::PopupTarget::Monitor,
        crate::workspace::PopupTarget::Data(0),
        crate::workspace::PopupTarget::State(0),
    ] {
        app.set_win_signal(target, old.clone(), true);
    }
    app.set_derived_defs(vec![def("New")]);
    assert!(
        app.monitor_view
            .rows
            .iter()
            .any(|row| row.key == new && row.label == "New"),
        "Monitor row and label follow the definition"
    );
    assert!(
        app.data_windows[0]
            .signals
            .iter()
            .any(|signal| signal.key == new)
    );
    assert!(
        app.state_trackers[0]
            .signals
            .iter()
            .any(|signal| signal.key == new)
    );
    assert!(app.subs.contains_key(&new));
    assert!(!app.subs.contains_key(&old));
}

/// Deleting a derived channel takes its threshold rules with it; a rule with
/// no lane can never fire and must not linger in the project.
#[test]
fn deleting_a_derived_channel_drops_its_trigger_rules() {
    let mut app = App::headless();
    app.set_derived_defs(vec![crate::derived::DerivedDef {
        id: "d".into(),
        name: "Sum".into(),
        expression: "[A] + 0".into(),
        ..Default::default()
    }]);
    app.send(crate::bus::BusCommand::AddTrigger {
        cond: crate::trigger::TriggerCond::DerivedSignal {
            id: "d".into(),
            threshold: 3.0,
            rising: true,
        },
        action: crate::trigger::TriggerAction::InsertMarker,
        enabled: true,
    });
    assert_eq!(app.triggers.len(), 1);
    app.set_derived_defs(Vec::new());
    assert!(
        app.triggers.is_empty(),
        "the rule referenced a lane that no longer exists"
    );
}

/// A script may read a derived lane at start, before the first sample. The
/// value reads nil and the node stays healthy instead of being marked errored.
#[test]
fn a_script_can_read_derived_at_start() {
    let mut app = App::headless();
    app.send(crate::bus::BusCommand::AddNode {
        name: "reader".to_string(),
        channel: 0,
        attached: None,
    });
    app.settle();
    let id = app.snap.nodes[0].id;
    app.send(crate::bus::BusCommand::SetNodeSource {
        id,
        source: "on start { print(defined(derived(\"Late\")), derived(\"Late\")); }".to_string(),
    });
    app.settle();
    app.start_virtual();
    app.settle();
    let node = app.snap.nodes.first().expect("node present");
    assert!(
        !node.errored,
        "missing first sample is not fatal: {:?}",
        node.log
    );
    assert!(
        node.log.iter().any(|line| line == "false nil"),
        "the lane reads nil until sampled: {:?}",
        node.log
    );
    app.stop();
}

#[test]
fn a_derived_trigger_fires_inside_a_real_step() {
    let mut app = App::headless();
    let a = SigKey::can_in(crate::model::ClusterId(1), 0x100, false, "A");
    app.subscribe_signal(a.clone());
    app.set_derived_defs(vec![crate::derived::DerivedDef {
        id: "d".into(),
        name: "Sum".into(),
        expression: "[A] + 0".into(),
        ..Default::default()
    }]);
    app.send(crate::bus::BusCommand::AddTrigger {
        cond: crate::trigger::TriggerCond::DerivedSignal {
            id: "d".into(),
            threshold: 3.0,
            rising: true,
        },
        action: crate::trigger::TriggerAction::InsertMarker,
        enabled: true,
    });
    app.start_virtual();
    app.subs.get_mut(&a).unwrap().push_sample(1_000, 5.0, 1);
    app.advance_clock(2_000);
    app.tick(2_000);
    assert_eq!(
        app.triggers[0].fired, 1,
        "the step evaluates the lane and then the rule"
    );
    app.stop();
}

#[test]
fn aligned_signal_csv_writes_one_row_per_time_step() {
    let mut app = App::headless();
    let db = app.channel_dbc(0).expect("sample DBC loaded");
    let id = db.order[0].0;
    let name = db.messages[&(id, false)].signals[0].name.clone();
    let key = SigKey::can_in(crate::model::ClusterId(1), id, false, name);
    app.subscribe(key.clone());
    app.graphics[0].signals.push(GfxSignal {
        key: key.clone(),
        visible: true,
        y_mode: YMode::Auto,
    });
    for tx in &mut app.tx_list {
        tx.active = true;
        tx.cycle_us = 10_000;
    }
    open_all_gates(&mut app);
    app.start_virtual();
    for _ in 0..10 {
        std::thread::sleep(std::time::Duration::from_millis(11));
        app.update();
    }
    app.stop();
    let sub = app
        .snap
        .subs
        .iter()
        .find(|sub| sub.key == key)
        .expect("signal subscribed");
    let points: Vec<(u64, f64)> = sub.history.iter().copied().collect();
    assert!(points.len() >= 2, "history sampled");
    let from = points.first().unwrap().0;
    let to = points.last().unwrap().0;
    assert!(to > from + 20_000, "range spans several samples");

    let path =
        std::env::temp_dir().join(format!("roxy_aligned_signals_{}.csv", std::process::id()));
    app.export_signals_csv(0, &path.to_string_lossy(), Some((from, to)));
    let content = std::fs::read_to_string(&path).unwrap();
    let mut lines = content.lines();
    let header = lines.next().expect("header");
    assert!(header.starts_with("time_s,"), "time column leads");
    let rows: Vec<&str> = lines.collect();
    assert!(rows.len() >= 3, "the grid has several rows");
    let mut previous = 0.0;
    for (index, row) in rows.iter().enumerate() {
        let fields: Vec<&str> = row.split(',').collect();
        assert_eq!(fields.len(), 2, "one time plus one signal column");
        let time: f64 = fields[0].parse().expect("numeric time");
        if index > 0 {
            assert!(time > previous, "time is strictly increasing");
        }
        previous = time;
        assert!(!fields[1].is_empty(), "sample-and-hold fills the value");
    }
    assert!(
        previous <= to as f64 / 1e6,
        "the grid stops at the requested end"
    );
    std::fs::remove_file(&path).ok();
}

#[test]
fn trace_export_honors_an_absolute_range() {
    let mut app = App::headless();
    for tx in &mut app.tx_list {
        tx.active = true;
        tx.cycle_us = 10_000;
    }
    open_all_gates(&mut app);
    app.start_virtual();
    for _ in 0..10 {
        std::thread::sleep(std::time::Duration::from_millis(11));
        app.update();
    }
    app.stop();
    let mut times: Vec<u64> = app.snap.trace.iter().map(|frame| frame.t_us).collect();
    times.sort_unstable();
    assert!(times.len() >= 4, "the fixture produced frames");
    let from = times[times.len() / 4];
    let to = times[times.len() * 3 / 4];
    assert!(from < to, "the test picks a real interval");

    let path =
        std::env::temp_dir().join(format!("roxy_can_range_export_{}.asc", std::process::id()));
    app.export_trace_range(0, &path.to_string_lossy(), from, to);
    let content = std::fs::read_to_string(&path).unwrap();
    let frames = crate::log::asc::parse_asc(&content);
    assert!(!frames.is_empty(), "the range still holds frames");
    assert!(
        frames
            .iter()
            .all(|frame| frame.t_us >= from && frame.t_us <= to),
        "nothing outside the A/B cursors is exported"
    );
    std::fs::remove_file(&path).ok();
}

#[test]
fn export_trace_writes_parseable_asc() {
    let mut app = App::headless();
    for tx in &mut app.tx_list {
        tx.active = true;
        tx.cycle_us = 10_000;
    }
    open_all_gates(&mut app);
    app.start_virtual();
    for _ in 0..6 {
        std::thread::sleep(std::time::Duration::from_millis(11));
        app.update();
    }
    let n = app.trace.len();
    assert!(n > 0, "expected captured frames");
    let path = std::env::temp_dir().join("roxy_can_export_test.asc");
    let path_str = path.to_string_lossy().to_string();
    app.export_trace(0, &path_str);
    let content = std::fs::read_to_string(&path).unwrap();
    let frames = crate::log::asc::parse_asc(&content);
    assert_eq!(frames.len(), n, "exported frame count mismatch");
    std::fs::remove_file(&path).ok();
    app.stop();
}

/// A mixed log: one CAN frame and one FlexRay row per 2 ms step, the row a
/// millisecond after its frame. Both sides therefore have `rows` arrivals at a
/// 2 ms cadence, and the file's line order interleaves the two buses. The
/// FlexRay slot is the caller's, so a test can put the traffic where its own
/// description says a frame belongs.
fn write_mixed_fr_asc(name: &str, rows: u64, slot: u16) -> std::path::PathBuf {
    use crate::trace::FrRow;
    let path = std::env::temp_dir().join(format!("{name}_{}.asc", std::process::id()));
    let mut w = AscWriter::new(&path.to_string_lossy()).unwrap();
    for i in 0..rows {
        let mut f = CanFrame {
            t_us: i * 2_000,
            channel: 0,
            id: 0x100,
            extended: false,
            len: 2,
            data: [0; MAX_CAN_FD_LEN],
            dir: Direction::Rx,
            flags: FrameFlags::NONE,
        };
        f.data[0] = i as u8;
        w.write(&f).unwrap();
        w.write_fr(&FrRow {
            bus: 0,
            t_us: i * 2_000 + 1_000,
            ab: 0,
            slot,
            cycle: i as u8,
            payload: vec![0xF0, i as u8],
            header_crc: 0,
            flags: 0,
            name: None,
        })
        .unwrap();
    }
    w.finish().unwrap();
    path
}

/// Replays `path` on the manual drive until the core has published it.
fn replay_to_the_end(app: &mut App, path: &std::path::Path) {
    app.load_log(&path.to_string_lossy());
    app.replay();
    for _ in 0..8 {
        std::thread::sleep(std::time::Duration::from_millis(11));
        app.update();
    }
    app.stop();
}

/// The trace export is not CAN-only: the FlexRay rows in the ring cross into
/// the ASC file too, and the file stays in time order, so an exported mixed
/// run is still a log rather than two logs glued together.
#[test]
fn the_trace_export_carries_flexray_rows() {
    let mut app = App::headless();
    let src = write_mixed_fr_asc("roxy_can_export_fr_in", 4, 13);
    replay_to_the_end(&mut app, &src);
    assert_eq!(app.snap.fr_trace.len(), 4, "the replay fed the FR ring");

    let out = std::env::temp_dir().join("roxy_can_export_fr_out.asc");
    app.export_trace(0, &out.to_string_lossy());
    std::fs::remove_file(&src).ok();
    let content = std::fs::read_to_string(&out).unwrap();
    std::fs::remove_file(&out).ok();
    let (can, fr) = crate::log::asc::parse_asc_full(&content);
    assert_eq!(can.len(), 4, "the CAN side still crosses");
    assert_eq!(fr.len(), 4, "so does FlexRay");
    assert_eq!(fr[2].slot, 13);
    assert_eq!(fr[2].payload, vec![0xF0, 2]);
    assert_eq!(fr[2].t_us, 5_000);
    assert_eq!(content.matches("Fr RMSG").count(), 4);
}

/// The Message Statistics table carries FlexRay slots with the same columns it
/// computes for CAN -- count, the min/avg/max of the observed intervals and
/// the share of the run -- and the same scope rule: pin the window to a CAN
/// bus and the slots leave, because a slot is not a CAN id.
#[test]
fn the_statistics_window_lists_flexray_slots() {
    let mut app = App::headless();
    let src = write_mixed_fr_asc("roxy_can_stats_fr_in", 4, 13);
    replay_to_the_end(&mut app, &src);
    std::fs::remove_file(&src).ok();

    app.sync_stats_text(0);
    let rows = &app.stats_windows[0].text_rows;
    let fr = rows.iter().find(|r| r.bus == "FR0").expect("a FlexRay row");
    assert_eq!(fr.label, "slot 13");
    assert_eq!(fr.count, "4");
    // Four arrivals 2 ms apart: every interval is the same, so min, avg and
    // max all read 2.00 ms.
    assert_eq!(
        (fr.min.as_str(), fr.avg.as_str(), fr.max.as_str()),
        ("2.00", "2.00", "2.00")
    );
    assert_eq!(fr.len, "2");
    assert_eq!(fr.share, "50.0%", "half the run's frames are FlexRay");
    assert!(
        app.stats_windows[0]
            .text_header
            .starts_with(&format!("{} messages", rows.len())),
        "the header counts the rows it shows: {:?}",
        app.stats_windows[0].text_header
    );

    let csv = std::env::temp_dir().join("roxy_can_stats_fr.csv");
    app.export_stats_csv(0, &csv.to_string_lossy());
    let text = std::fs::read_to_string(&csv).unwrap();
    std::fs::remove_file(&csv).ok();
    assert!(
        text.lines().any(|l| l.starts_with("FR0,13,")),
        "the CSV carries the slot: {text}"
    );

    // A bus-scoped window is a CAN scope.
    app.stats_windows[0].scope = crate::workspace::SigScope::Bus(0);
    app.text_fresh = true;
    app.sync_stats_text(0);
    assert!(
        app.stats_windows[0]
            .text_rows
            .iter()
            .all(|r| r.bus != "FR0"),
        "FlexRay leaves a CAN-bus window"
    );
}

/// The report carries its own premises -- which databases, the tolerance
/// and grace in effect, how many messages declared a period -- plus every
/// latched row. Without the header, the same table means something else
/// at a different threshold setting and cannot be re-checked.
#[test]
fn the_spec_report_export_includes_its_premises_and_rows() {
    let mut app = App::headless();
    app.start_virtual();
    app.tx_list.retain(|t| t.channel != 0);
    receive(
        &mut app,
        1_000,
        vec![rx_frame(1_000, 0x777, 8, FrameFlags::NONE)],
    );
    assert!(
        app.spec
            .rows
            .contains_key(&(0, 0x777, false, crate::spec::Kind::Unknown))
    );
    app.spec_tol_pct = 5;
    app.spec_grace = 4;

    let path = std::env::temp_dir().join("roxy_can_spec_report.csv");
    app.export_spec_csv(path.to_string_lossy().as_ref());
    let content = std::fs::read_to_string(&path).unwrap();

    assert!(
        content.contains("# database,CAN1,builtin:sample.dbc"),
        "which database each bus carried"
    );
    assert!(content.contains("# tolerance,+/-5%"));
    assert!(content.contains("# grace,4x declared period"));
    assert!(
        content.contains("# periodic messages declared,"),
        "how many messages had a period to break"
    );
    assert!(
        content.contains("CAN1,777,not in database,Unknown id"),
        "the latched row itself: {content}"
    );
    std::fs::remove_file(&path).ok();
    app.stop();
}

#[test]
fn two_channels_aggregate_separately() {
    let mut app = App::headless();
    assert_eq!(app.can_count(), 2);
    for ch in 0..app.can_count() {
        assert!(
            app.channel_dbc(ch as u8).is_some(),
            "CAN{} should load its DBC",
            ch + 1
        );
    }
    // 角色开关：两条总线的节点全部模拟，条目才会发送。
    open_all_gates(&mut app);
    assert!(
        app.tx_list.iter().any(|t| t.channel == 0 && t.id == 0x100)
            && app.tx_list.iter().any(|t| t.channel == 1 && t.id == 0xC8),
        "generator pre-populated on both buses"
    );
    for tx in &mut app.tx_list {
        if (tx.channel == 0 && tx.id == 0x100) || (tx.channel == 1 && tx.id == 0xC8) {
            tx.active = true;
            tx.cycle_us = 10_000;
        }
    }
    app.start_virtual();
    for _ in 0..6 {
        std::thread::sleep(std::time::Duration::from_millis(11));
        app.update();
    }
    let a = app
        .aggs
        .get(&(
            crate::model::ClusterId(app.snap.cluster_ids.can[0]),
            0x100,
            false,
        ))
        .expect("CAN1 aggregate");
    let b = app
        .aggs
        .get(&(
            crate::model::ClusterId(app.snap.cluster_ids.can[1]),
            0xC8,
            false,
        ))
        .expect("CAN2 aggregate");
    assert!(a.count >= 3, "CAN1 frames: {}", a.count);
    assert!(b.count >= 3, "CAN2 frames: {}", b.count);
    assert!(app.trace.iter().any(|f| f.channel == 1 && f.id == 0xC8));
    app.stop();
}

