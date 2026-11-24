#[test]
fn a_script_node_round_trips_through_a_project() {
    let mut app = App::headless();
    app.send(crate::bus::BusCommand::AddNode {
        name: "gen".into(),
        channel: 0,
        attached: Some((0, "EngineECU".to_string())),
    });
    let id = app.snap.nodes[0].id;
    let source = "on timer 100 { print(1); }";
    app.send(crate::bus::BusCommand::SetNodeSource {
        id,
        source: source.to_string(),
    });
    let path = std::env::temp_dir().join("roxy_can_node.rxproj");
    assert!(app.save_project(Some(path.clone())));

    let mut restored = App::headless();
    restored.open_project_path(&path);
    assert_eq!(restored.snap.nodes.len(), 1);
    let node = &restored.snap.nodes[0];
    assert_eq!(node.name, "gen");
    assert_eq!(node.channel, 0);
    assert_eq!(node.source, source);
    assert!(node.enabled, "enabled defaults ride the round trip");
    // The binding must survive the restore: a lost binding would flag a
    // perfectly bound script as an orphan for adoption.
    assert_eq!(
        node.attached,
        Some((0, "EngineECU".to_string())),
        "the node binding rides the round trip"
    );
    std::fs::remove_file(&path).ok();
}

#[test]
fn the_text_gate_fires_on_its_cadence() {
    let mut app = quiet_app();
    app.text_rate_hz = 10;
    // Long since the last refresh: the next frame re-renders text...
    app.last_text_refresh = std::time::Instant::now() - std::time::Duration::from_millis(200);
    app.update();
    assert!(app.text_fresh, "past the period: text re-renders");
    // ...and the frame right after it does not.
    app.update();
    assert!(!app.text_fresh, "within the period: text holds");

    // Rate 0 means follow the frame rate: every frame is a text frame.
    app.text_rate_hz = 0;
    app.update();
    assert!(app.text_fresh, "unthrottled re-renders every frame");
}

#[test]
fn data_values_hold_still_until_the_text_gate_fires() {
    let mut app = quiet_app();
    let key = SigKey::can_in(crate::model::ClusterId(1), 0x100, false, "EngineSpeed");
    app.subscribe(key.clone());
    if app.data_windows.is_empty() {
        app.new_data_window();
    }
    app.data_windows[0].signals.push(GfxSignal {
        key: key.clone(),
        visible: true,
        y_mode: YMode::Auto,
    });

    feed_rpm(&mut app, &[(10_000, 100.0)]);
    app.text_fresh = true;
    app.sync_data_text(0);
    assert_eq!(
        app.data_windows[0].text_cache[0][0], "100",
        "first snapshot"
    );

    // New traffic arrives, but the gate has not fired: the drawn text must
    // hold at the last snapshot instead of flickering with every frame.
    feed_rpm(&mut app, &[(200_000, 300.0)]);
    app.text_fresh = false;
    app.sync_data_text(0);
    assert_eq!(
        app.data_windows[0].text_cache[0][0], "100",
        "stale by design between text frames"
    );

    // The gate fires and the snapshot catches up -- while the bar, fed
    // straight from `latest`, never waited.
    app.text_fresh = true;
    app.sync_data_text(0);
    assert_eq!(app.data_windows[0].text_cache[0][0], "300");
    assert_eq!(app.subs[&key].latest, 300.0);
}

#[test]
fn the_text_rate_round_trips_through_a_project() {
    let mut app = App::headless();
    app.text_rate_hz = 5;
    let path = std::env::temp_dir().join("roxy_can_textrate.rxproj");
    assert!(app.save_project(Some(path.clone())));

    let mut restored = App::headless();
    restored.open_project_path(&path);
    assert_eq!(restored.text_rate_hz, 5);
    std::fs::remove_file(&path).ok();
}

#[test]
fn stats_and_message_rows_hold_still_until_the_text_gate_fires() {
    let mut app = quiet_app();
    if app.msg_windows.is_empty() {
        app.new_msg_window();
    }
    if app.stats_windows.is_empty() {
        app.new_stats_window();
    }
    feed_rpm(&mut app, &[(10_000, 100.0), (30_000, 110.0)]);

    app.text_fresh = true;
    app.sync_stats_text(0);
    app.sync_msg_text(0);
    assert_eq!(
        app.stats_windows[0].text_rows[0].count, "2",
        "first snapshot"
    );
    assert_eq!(app.msg_windows[0].text_rows[0].count, "2");
    assert!(
        app.stats_windows[0]
            .text_header
            .starts_with("1 messages, 2 frames"),
        "the header counts rows and frames: {}",
        app.stats_windows[0].text_header
    );

    // More traffic, gate closed: both tables keep drawing the old snapshot.
    feed_rpm(&mut app, &[(50_000, 120.0)]);
    app.text_fresh = false;
    app.sync_stats_text(0);
    app.sync_msg_text(0);
    assert_eq!(
        app.stats_windows[0].text_rows[0].count, "2",
        "stale by design between text frames"
    );
    assert_eq!(app.msg_windows[0].text_rows[0].count, "2");

    // The gate fires and both tables catch up in one step.
    app.text_fresh = true;
    app.sync_stats_text(0);
    app.sync_msg_text(0);
    assert_eq!(app.stats_windows[0].text_rows[0].count, "3");
    assert_eq!(app.msg_windows[0].text_rows[0].count, "3");
    assert_eq!(app.msg_windows[0].text_header, "1 messages");
    app.stop();
}

#[test]
fn the_status_counters_hold_still_until_the_text_gate_fires() {
    let mut app = quiet_app();
    feed_rpm(&mut app, &[(10_000, 100.0)]);

    app.text_fresh = true;
    app.sync_status_text();
    let held = app.status_counters.clone();
    assert!(held.contains("frames:"), "the snapshot was built: {held}");

    feed_rpm(&mut app, &[(30_000, 110.0)]);
    app.text_fresh = false;
    app.sync_status_text();
    assert_eq!(app.status_counters, held, "stale between text frames");

    app.text_fresh = true;
    app.sync_status_text();
    assert_ne!(app.status_counters, held, "the counters caught up");
    app.stop();
}

#[test]
fn graphics_legends_hold_still_until_the_text_gate_fires() {
    let (mut app, _key) = gfx_app(YMode::Auto);
    feed_rpm(&mut app, &[(10_000, 100.0)]);

    app.text_fresh = true;
    app.sync_gfx_legend(0);
    let held = app.graphics[0].legend[0].clone();
    assert!(
        held.starts_with("EngineSpeed = 100"),
        "first snapshot: {held}"
    );

    feed_rpm(&mut app, &[(200_000, 300.0)]);
    app.text_fresh = false;
    app.sync_gfx_legend(0);
    assert_eq!(
        app.graphics[0].legend,
        vec![held],
        "stale between text frames"
    );

    app.text_fresh = true;
    app.sync_gfx_legend(0);
    assert!(
        app.graphics[0].legend[0].starts_with("EngineSpeed = 300"),
        "caught up: {}",
        app.graphics[0].legend[0]
    );
    app.stop();
}

#[test]
fn a_signal_value_condition_filters_the_trace() {
    let mut app = quiet_app();
    // Filter text of the `Signal>value` form is a signal-value condition:
    // only frames whose decoded EngineSpeed passes it match.
    app.trace_windows[0].filter = "EngineSpeed>=200".to_string();
    feed_rpm(&mut app, &[(10_000, 100.0), (30_000, 300.0)]);

    app.text_fresh = true;
    app.sync_trace_rows(0);
    assert_eq!(app.trace_windows[0].rows.len(), 1, "only the >=200 frame");
    assert_eq!(app.trace_windows[0].rows[0].t_us(), 30_000);
    app.stop();
}

#[test]
fn trace_rows_reveal_in_batches_on_the_text_gate() {
    let mut app = quiet_app();
    if app.trace_windows.is_empty() {
        app.new_trace_window();
    }
    feed_rpm(&mut app, &[(10_000, 100.0), (30_000, 110.0)]);

    app.text_fresh = true;
    app.sync_trace_rows(0);
    assert_eq!(app.trace_windows[0].rows.len(), 2, "first reveal");
    assert_eq!(
        app.trace_revealed(&app.trace_windows[0]).count(),
        2,
        "both rows are on screen"
    );

    // More traffic, gate closed: the fresh tail stays hidden while the
    // drawn rows hold still.
    feed_rpm(&mut app, &[(50_000, 120.0)]);
    app.text_fresh = false;
    app.sync_trace_rows(0);
    assert_eq!(app.trace.len(), 3, "the frame did arrive");
    assert_eq!(
        app.trace_revealed(&app.trace_windows[0]).count(),
        2,
        "stale by design between text frames"
    );

    // The gate fires and the row appears in one step.
    app.text_fresh = true;
    app.sync_trace_rows(0);
    assert_eq!(app.trace_revealed(&app.trace_windows[0]).count(), 3);
    assert_eq!(app.trace_windows[0].rows.len(), 3);
    app.stop();
}

/// A window's hand-picked set now speaks both numbering spaces. Picking a
/// FlexRay slot admits that slot's rows and no others, and the two spaces share
/// arbitrary integers -- `(0, 5)` is CAN channel 0's id 5 *and* cluster 0's slot
/// 5 -- so a pick must not read as the other kind.
#[test]
fn a_hand_picked_flexray_slot_admits_only_that_slot() {
    use crate::workspace::Pick;
    let manual: std::collections::HashSet<Pick> =
        [Pick::Fr { bus: 0, slot: 5 }, Pick::Can { ch: 1, id: 5 }]
            .into_iter()
            .collect();
    assert!(App::scope_match_fr(SigScope::Manual, &manual, 0, 5));
    assert!(
        !App::scope_match_fr(SigScope::Manual, &manual, 1, 5),
        "another cluster's slot 5 is a different pick"
    );
    assert!(!App::scope_match_fr(SigScope::Manual, &manual, 0, 6));
    assert!(App::scope_match(SigScope::Manual, &manual, 1, 5));
    assert!(
        !App::scope_match(SigScope::Manual, &manual, 0, 5),
        "the FlexRay pick must not admit CAN channel 0's id 5"
    );
    // The other scopes behave as before.
    assert!(App::scope_match_fr(SigScope::All, &manual, 3, 99));
    assert!(App::scope_match_fr(SigScope::FrBus(0), &manual, 0, 6));
    assert!(
        !App::scope_match_fr(SigScope::Bus(1), &manual, 0, 5),
        "a CAN channel scope still restricts the table to CAN"
    );
    assert!(!App::scope_match(SigScope::FrBus(0), &manual, 0, 5));
}

/// The shape of a Trace window's cache as the table would draw it: the order,
/// which ring each row came from, its address, and for a decoded FlexRay signal
/// row the signal and value text it prints -- one entry per *table* row, so a
/// frame row with three children contributes four. Two caches with the same
/// shape drew the same table.
fn cache_shape(app: &App) -> Vec<(u64, u8, u32, String)> {
    let mut out = Vec::new();
    for r in app.trace_windows[0].rows.iter() {
        match r {
            TraceRow::Can(f) => out.push((f.t_us, 0, f.id, String::new())),
            TraceRow::Fr(f, kids) => {
                out.push((f.t_us, 1, u32::from(f.slot), String::new()));
                for child in 0..*kids {
                    // Resolved the way the window resolves it, so the deferred
                    // decoding is what this compares, not just the count.
                    let (signal, value) = app.fr_child_cell(f, child).unwrap_or_default();
                    out.push((f.t_us, 2, u32::from(f.slot), format!("{signal}={value}")));
                }
            }
            TraceRow::Eth(f) => out.push((f.t_us, 3, u32::from(f.ethertype), app.eth_row_name(f))),
        }
    }
    out
}

/// A steady run only prepends to the Trace row cache. Getting that wrong shows
/// up as duplicated or missing rows, so the check is against the thing it
/// replaces: extending the cache batch after batch has to end with exactly the
/// list one walk of the whole ring gives -- here with FlexRay frames expanded
/// into their decoded signal children, the shape where a stale row is easiest
/// to lose.
#[test]
fn the_trace_row_cache_extends_in_place_without_losing_rows() {
    use std::sync::Arc;
    let arxml = crate::test_assets::path("PowerTrain.arxml");
    let mut app = quiet_app();
    app.tx_list.retain(|t| t.channel != 0);
    let Ok(bytes) = std::fs::read(arxml) else {
        panic!(
            "{arxml} missing -- it is tracked in the repository, so a broken checkout must fail, not skip"
        );
    };
    let db = crate::fr_db::FrDb::parse(&crate::dbc::text_from_bytes(bytes)).expect("parses");
    // Slots whose frame actually declares signals, with the cycle that frame is
    // scheduled in: those rows are the ones that grow children under.
    let drive: Vec<(u16, u8)> = db
        .frames
        .iter()
        .enumerate()
        .filter(|(ix, _)| !db.decode(*ix, &[0u8; 48]).is_empty())
        .map(|(_, f)| (f.triggering.slot_id as u16, f.triggering.base_cycle as u8))
        .take(3)
        .collect();
    assert!(
        !drive.is_empty(),
        "the asset has no static frame with signals to expand"
    );
    app.fr_buses.insert(0, Arc::new(db));
    app.push_fr_db_to_core();
    let q = app.hw.attach_fr_mock(0, 5);
    app.start_virtual();
    app.trace_windows[0].fr_expand = true;
    for (round, (slot, cycle)) in drive.iter().enumerate() {
        use crate::hw::vector::flexray::FrFrame;
        q.lock().expect("mock lock").push_back(FrFrame {
            slot: *slot,
            cycle: *cycle,
            // Long enough for the signals the selection above counted: a
            // payload that does not reach a signal decodes to nothing.
            payload: vec![0; 48],
            header_crc: 0,
            flags: 0,
        });
        let t = (round as u64 + 1) * 20_000;
        app.advance_clock(t);
        app.tick(t);
        app.refresh_snapshot();
        app.text_fresh = true;
        app.sync_trace_rows(0);
    }
    let extended = cache_shape(&app);
    assert!(
        !app.trace_windows[0].rows.is_empty(),
        "the cache is not empty, or this proves nothing"
    );
    assert!(
        extended.len() > app.trace_windows[0].rows.len(),
        "the drawn table has more rows than the list has entries, because a \
         FlexRay frame row carries its children: {extended:?}"
    );
    assert!(
        extended.iter().any(|(_, kind, ..)| *kind == 2),
        "FlexRay children are in the cache: {extended:?}"
    );
    assert!(
        extended.windows(2).all(|w| w[0].0 >= w[1].0),
        "newest first: {extended:?}"
    );
    // The same traffic, walked once from scratch.
    app.trace_windows[0].rows_build = None;
    app.text_fresh = true;
    app.sync_trace_rows(0);
    assert_eq!(
        cache_shape(&app),
        extended,
        "batch by batch and all at once have to agree"
    );
    app.stop();
}

/// What must throw the incremental cache away: an edited filter (rows that no
/// longer match have to leave), a column sort (a sorted cache is no longer
/// time-ordered, so there is nothing to prepend to), and a cleared trace (the
/// rows under the build point are gone from the ring). Each case ends with the
/// cache describing the ring as it is now.
#[test]
fn an_edited_filter_or_a_cleared_trace_rebuilds_the_row_cache() {
    let mut app = quiet_app();
    for round in 0..3u64 {
        feed_rpm(&mut app, &[(round * 30_000 + 10_000, 100.0)]);
    }
    app.text_fresh = true;
    app.sync_trace_rows(0);
    assert_eq!(app.trace_windows[0].rows.len(), 3);

    // The filter text narrows, so every row is tested again: the cache shrinks
    // to what matches instead of keeping the old rows plus the new ones.
    app.trace_windows[0].filter = "no-such-frame-name".to_string();
    feed_rpm(&mut app, &[(120_000, 110.0)]);
    app.text_fresh = true;
    app.sync_trace_rows(0);
    assert!(
        app.trace_windows[0].rows.is_empty(),
        "a filter edit rebuilds: {:?}",
        cache_shape(&app)
    );
    // Back to no filter: the whole ring is in again, in one refresh.
    app.trace_windows[0].filter.clear();
    app.text_fresh = true;
    app.sync_trace_rows(0);
    assert_eq!(
        app.trace_windows[0].rows.len(),
        4,
        "and the rebuild finds the whole ring"
    );

    // A sorted cache is no longer time-ordered, so a refresh has to rebuild it
    // rather than prepend: turned oldest-first here, where prepending the new
    // row would leave a list that runs backwards right after the newest row.
    app.trace_windows[0].rows.make_contiguous().reverse();
    app.trace_windows[0].rows_sorted = true;
    feed_rpm(&mut app, &[(150_000, 120.0)]);
    app.text_fresh = true;
    app.sync_trace_rows(0);
    let shape = cache_shape(&app);
    assert_eq!(shape.len(), 5);
    assert!(
        shape.windows(2).all(|w| w[0].0 >= w[1].0),
        "back in time order: {shape:?}"
    );
    assert!(
        !app.trace_windows[0].rows_sorted,
        "the flag is consumed by the rebuild"
    );

    // A cleared trace: the ring is empty, so the cache has to be too.
    app.send(crate::bus::BusCommand::ClearTrace);
    app.text_fresh = true;
    app.sync_trace_rows(0);
    assert!(
        app.trace_windows[0].rows.is_empty(),
        "clearing the trace empties the cache: {:?}",
        cache_shape(&app)
    );
    app.stop();
}

/// A one-slot FlexRay description whose frame declares exactly these signals,
/// on a cluster whose static slot lasts `slot_wire_us` microseconds (0 = the
/// description declares no timing at all). Enough to pin how many child rows a
/// frame row owns, which name each one prints, and what the cluster's occupancy
/// is charged.
fn one_slot_db(signals: &[&str], slot_wire_us: f64) -> std::sync::Arc<crate::fr_db::FrDb> {
    use crate::fr_db::{
        FrChannel, FrClusterParams, FrDb, FrFrameDb, FrPdu, FrSignal, FrTriggering,
    };
    let sig = |name: &str| FrSignal {
        name: name.into(),
        start_bit: 0,
        length_bits: 8,
        big_endian: false,
        signed: false,
        factor: 1.0,
        offset: 0.0,
        min: 0.0,
        max: 255.0,
        unit: String::new(),
        comment: String::new(),
        value_descriptions: vec![],
    };
    FrDb::assemble(
        FrClusterParams {
            // One macrotick of `slot_wire_us`, so the product is the figure the
            // load is charged per arrival.
            static_slot_duration: 1,
            macrotick_duration_us: slot_wire_us,
            ..Default::default()
        },
        vec![],
        vec![FrPdu {
            name: "P".into(),
            length: 1,
            dynamic: false,
            comment: String::new(),
            signals: signals.iter().map(|s| sig(s)).collect(),
        }],
        vec![FrFrameDb {
            name: "F".into(),
            length: 1,
            payload_preamble: false,
            triggering: FrTriggering {
                channel: FrChannel::Both,
                slot_id: 5,
                base_cycle: 0,
                cycle_repetition: 1,
                startup: false,
            },
            pdus: vec![("P".into(), 0u32)],
            comment: String::new(),
        }],
    )
    .into()
}

/// A FlexRay cluster's load is the same kind of number as a CAN channel's: the
/// share of the rolling window the traffic occupied, with a FlexRay frame's
/// occupancy being its static slot (`gstaticSlot` × `gmacrotick`) rather than
/// its length. The description supplies the timing; nothing here is invented.
#[test]
fn a_flexray_cluster_reports_its_static_segment_occupancy() {
    use crate::hw::vector::flexray::FrFrame;
    let mut app = quiet_app();
    app.tx_list.retain(|t| t.channel != 0);
    app.fr_buses.insert(0, one_slot_db(&["One"], 40.0));
    app.push_fr_db_to_core();
    let q = app.hw.attach_fr_mock(0, 5);
    app.start_virtual();
    for i in 0..25u64 {
        q.lock().expect("mock lock").push_back(FrFrame {
            slot: 5,
            cycle: 0,
            payload: vec![0x2A],
            header_crc: 0,
            flags: 0,
        });
        let t = (i + 1) * 20_000;
        app.advance_clock(t);
        app.tick(t);
        app.refresh_snapshot();
    }
    let load = app.snap.fr_loads.get(&0).expect("bus 0 carried traffic");
    assert_eq!(load.frames, 25);
    assert!(
        (load.load() - 25.0 * 40.0 / 1e6).abs() < 1e-12,
        "25 slots of 40 us inside a one-second window: {}",
        load.load()
    );
    assert_eq!(app.fr_slot_wire_us(0), Some(40.0));
    app.stop();
}

/// A cluster watched with a description that declares no slot timing counts
/// frames and claims no load, which is what keeps the Statistics row an honest
/// "-" instead of a 0 %.
#[test]
fn an_undescribed_flexray_cluster_counts_frames_but_no_occupancy() {
    use crate::hw::vector::flexray::FrFrame;
    let mut app = quiet_app();
    app.tx_list.retain(|t| t.channel != 0);
    app.fr_buses.insert(0, one_slot_db(&["One"], 0.0));
    app.push_fr_db_to_core();
    let q = app.hw.attach_fr_mock(0, 5);
    app.start_virtual();
    q.lock().expect("mock lock").push_back(FrFrame {
        slot: 5,
        cycle: 0,
        payload: vec![0x2A],
        header_crc: 0,
        flags: 0,
    });
    app.advance_clock(20_000);
    app.tick(20_000);
    app.refresh_snapshot();
    assert_eq!(app.fr_slot_wire_us(0), None, "no timing to divide by");
    let load = app.snap.fr_loads.get(&0).expect("bus 0 carried traffic");
    assert_eq!(load.frames, 1, "the arrival still counts");
    assert_eq!(load.load(), 0.0);
    app.stop();
}

/// A script reads a FlexRay signal the same way it reads a CAN one. End to end
/// on purpose -- a frame off the FlexRay watch, then a node timer that asks for
/// its value -- because the read is worth nothing if the arrival never reaches
/// it: the value has to travel the ingest → per-frame tally → decode → host
/// input path, and the slot number has to mean the frame that actually arrived.
#[test]
fn a_node_script_reads_a_flexray_signal() {
    use crate::hw::vector::flexray::FrFrame;
    let mut app = quiet_app();
    app.tx_list.retain(|t| t.channel != 0);
    app.fr_buses.insert(0, one_slot_db(&["One"], 40.0));
    app.push_fr_db_to_core();
    // The node lives on a CAN channel and names the cluster by index: a FlexRay
    // bus is not a channel a node could belong to.
    app.send(crate::bus::BusCommand::AddNode {
        name: "reader".to_string(),
        channel: 0,
        attached: None,
    });
    app.settle();
    let id = app.snap.nodes[0].id;
    app.send(crate::bus::BusCommand::SetNodeSource {
        id,
        source: "on timer 100 { emit_value(\"FromFr\", fr_sig(0, 5, \"One\")); }".to_string(),
    });
    app.settle();
    let q = app.hw.attach_fr_mock(0, 5);
    app.start_virtual();
    q.lock().expect("mock lock").push_back(FrFrame {
        slot: 5,
        cycle: 0,
        payload: vec![0x2A],
        header_crc: 0,
        flags: 0,
    });
    app.advance_clock(20_000);
    app.tick(20_000);
    // Past the timer's 100 ms, so the handler runs with the frame already in.
    for t in (30_000..=200_000).step_by(10_000) {
        app.advance_clock(t);
        app.tick(t);
    }
    app.refresh_snapshot();
    let node = app
        .snap
        .nodes
        .iter()
        .find(|n| n.id == id)
        .expect("the node");
    assert!(!node.errored, "the read did not fail: {:?}", node.log);
    let key = SigKey::can_in(
        crate::model::ClusterId(1),
        crate::app::EMITTED_ID_BASE | id as u32,
        false,
        "FromFr".to_string(),
    );
    let sub = app.subs.get(&key).expect("the derived stream");
    assert_eq!(
        sub.latest, 42.0,
        "0x2A in slot 5's one 8-bit signal, decoded through its frame"
    );
    app.stop();
}

/// A FlexRay arrival wakes a running node's `on fr slot` handler, and the
/// reaction lands on the node's CAN channel in the same step. The read inside
/// the handler is the point of the ordering: `fr_sig` must see the value this
/// very frame carried, so the ingest runs first and the dispatch after -- had
/// the dispatch come first, this script would error on an unseen signal.
#[test]
fn a_flexray_arrival_drives_a_script_reaction() {
    use crate::hw::vector::flexray::FrFrame;
    let mut app = quiet_app();
    app.tx_list.retain(|t| t.channel != 0);
    app.fr_buses.insert(0, one_slot_db(&["One"], 40.0));
    app.push_fr_db_to_core();
    app.send(crate::bus::BusCommand::AddNode {
        name: "gw".to_string(),
        channel: 0,
        attached: None,
    });
    app.settle();
    let id = app.snap.nodes[0].id;
    app.send(crate::bus::BusCommand::SetNodeSource {
        id,
        source: "on fr slot 5 { send(0x320, fr_sig(0, 5, \"One\")); }".to_string(),
    });
    app.settle();
    let q = app.hw.attach_fr_mock(0, 5);
    app.start_virtual();
    q.lock().expect("mock lock").push_back(FrFrame {
        slot: 5,
        cycle: 3,
        payload: vec![0x2A],
        header_crc: 0,
        flags: 0,
    });
    app.advance_clock(20_000);
    app.tick(20_000);
    app.refresh_snapshot();
    let node = app
        .snap
        .nodes
        .iter()
        .find(|n| n.id == id)
        .expect("the node");
    assert!(
        !node.errored,
        "the handler read the value its own arrival carried: {:?}",
        node.log
    );
    let agg = app
        .aggs
        .get(&(
            crate::model::ClusterId(app.snap.cluster_ids.can[0]),
            0x320,
            false,
        ))
        .expect("the reaction frame reached the CAN bus");
    assert_eq!(
        agg.data[0], 42,
        "the decoded physical value, truncated to a byte"
    );
    assert_eq!(agg.count, 1, "one arrival, one reaction");
    app.stop();
}

/// A script's `fr_send` puts a frame into the session through the same funnel a
/// generator slot uses -- so Trace, the Messages tally and everything else that
/// reads arrivals see it -- and it lands as soon as that step's generator block
/// runs (a FlexRay arrival is drained before it, so the same step; a CAN-driven or
/// timer-driven handler waits for the next one). Either way **one round per
/// step** is the bound a script that answers its own frame runs into, instead of a
/// loop inside a single step.
#[test]
fn a_script_frame_enters_the_session_like_any_arrival() {
    use crate::hw::vector::flexray::FrFrame;
    let mut app = quiet_app();
    app.tx_list.retain(|t| t.channel != 0);
    app.fr_buses.insert(0, one_slot_db(&["One"], 40.0));
    app.push_fr_db_to_core();
    app.send(crate::bus::BusCommand::AddNode {
        name: "gw".to_string(),
        channel: 0,
        attached: None,
    });
    app.settle();
    let id = app.snap.nodes[0].id;
    // Slot 6 is not in this description at all: with no declared width, the
    // frame goes out exactly as long as the script wrote it.
    app.send(crate::bus::BusCommand::SetNodeSource {
        id,
        source: "on fr slot 5 { fr_send(0, 6, 0x11, 0x22); }".to_string(),
    });
    app.settle();
    let rows = |app: &App| -> Vec<(u8, u16, usize)> {
        (0..app.snap.fr_trace.len())
            .filter_map(|i| app.snap.fr_trace.get(i))
            .map(|r| (r.bus, r.slot, r.payload.len()))
            .collect()
    };
    let q = app.hw.attach_fr_mock(0, 5);
    q.lock().expect("mock lock").push_back(FrFrame {
        slot: 5,
        cycle: 0,
        payload: vec![0x2A],
        header_crc: 0,
        flags: 0,
    });
    app.advance_clock(20_000);
    app.tick(20_000);
    app.refresh_snapshot();
    assert_eq!(
        rows(&app),
        [(0, 5, 1), (0, 6, 2)],
        "the FlexRay watch is drained before the generator block, so the script's \
         frame joins this same step"
    );
    // A second step adds nothing: the row the script sent wakes handlers (slot 6
    // here, which this script does not listen to), and whatever they queue waits
    // for the next step's ingest -- one round per step is the bound a script that
    // answers its own frame runs into, instead of a loop inside one step.
    app.advance_clock(40_000);
    app.tick(40_000);
    app.refresh_snapshot();
    assert_eq!(rows(&app), [(0, 5, 1), (0, 6, 2)], "no cascade");
    let payload = (0..app.snap.fr_trace.len())
        .filter_map(|i| app.snap.fr_trace.get(i))
        .find(|r| r.slot == 6)
        .map(|r| r.payload.clone())
        .expect("the script row");
    assert_eq!(payload, vec![0x11, 0x22], "the bytes as typed");
    assert!(
        app.fr_aggs
            .values()
            .any(|a| a.bus == 0 && a.slot == 6 && a.count == 1),
        "and it is tallied in Messages like any arrival"
    );
    let node = app
        .snap
        .nodes
        .iter()
        .find(|n| n.id == id)
        .expect("the node");
    assert!(!node.errored, "the handler ran clean: {:?}", node.log);
    app.stop();
}

/// A FlexRay slot is as wide as the schedule says, and a script cannot overfill
/// it: a payload longer than the declared width is **dropped whole** with the
/// reason in that node's log, rather than truncated into a frame nobody asked
/// for. Silently sending different bytes than the script wrote is the failure
/// this refuses.
#[test]
fn a_script_frame_wider_than_its_slot_is_dropped_with_the_reason() {
    use crate::hw::vector::flexray::FrFrame;
    let mut app = quiet_app();
    app.tx_list.retain(|t| t.channel != 0);
    app.fr_buses.insert(0, one_slot_db(&["One"], 40.0));
    app.push_fr_db_to_core();
    app.send(crate::bus::BusCommand::AddNode {
        name: "wide".to_string(),
        channel: 0,
        attached: None,
    });
    app.settle();
    let id = app.snap.nodes[0].id;
    // Slot 5 declares one byte; the handler offers three.
    app.send(crate::bus::BusCommand::SetNodeSource {
        id,
        source: "on fr slot 5 { fr_send(0, 5, 1, 2, 3); }".to_string(),
    });
    app.settle();
    let q = app.hw.attach_fr_mock(0, 5);
    q.lock().expect("mock lock").push_back(FrFrame {
        slot: 5,
        cycle: 0,
        payload: vec![0x2A],
        header_crc: 0,
        flags: 0,
    });
    for t in [20_000u64, 40_000, 60_000] {
        app.advance_clock(t);
        app.tick(t);
    }
    app.refresh_snapshot();
    let lengths = (0..app.snap.fr_trace.len())
        .filter_map(|i| app.snap.fr_trace.get(i))
        .map(|r| r.payload.len())
        .collect::<Vec<_>>();
    assert!(
        !lengths.contains(&3),
        "no three-byte frame was ever admitted: {lengths:?}"
    );
    let node = app
        .snap
        .nodes
        .iter()
        .find(|n| n.id == id)
        .expect("the node");
    assert!(
        node.log
            .iter()
            .any(|l| l.contains("fr_send") && l.contains("declares 1 bytes")),
        "the reason is in the node's log: {:?}",
        node.log
    );
    assert!(
        !node.errored,
        "a dropped frame is advice, not a fault: {:?}",
        node.log
    );
    app.stop();
}

/// A script cannot invent a bus: `fr_send` to a cluster with neither a
/// description nor a watch is dropped with the reason in that node's log, rather
/// than conjuring a Buses row that claims "日志里有这路流量" for traffic nobody
/// received. A slot the description does not schedule is a different thing -- the
/// 路 exists, so the frame goes out with the bytes as typed.
#[test]
fn a_script_frame_for_an_unconfigured_bus_is_refused() {
    use crate::hw::vector::flexray::FrFrame;
    let mut app = quiet_app();
    app.tx_list.retain(|t| t.channel != 0);
    app.fr_buses.insert(0, one_slot_db(&["One"], 40.0));
    app.push_fr_db_to_core();
    app.send(crate::bus::BusCommand::AddNode {
        name: "typo".to_string(),
        channel: 0,
        attached: None,
    });
    app.settle();
    let id = app.snap.nodes[0].id;
    app.send(crate::bus::BusCommand::SetNodeSource {
        id,
        source: "on fr slot 5 { fr_send(7, 5, 1); fr_send(0, 6, 2); }".to_string(),
    });
    app.settle();
    let q = app.hw.attach_fr_mock(0, 5);
    q.lock().expect("mock lock").push_back(FrFrame {
        slot: 5,
        cycle: 0,
        payload: vec![0x2A],
        header_crc: 0,
        flags: 0,
    });
    for n in 1..=3u64 {
        app.advance_clock(n * 20_000);
        app.tick(n * 20_000);
    }
    app.refresh_snapshot();
    let buses = (0..app.snap.fr_trace.len())
        .filter_map(|i| app.snap.fr_trace.get(i))
        .map(|r| r.bus)
        .collect::<Vec<_>>();
    assert!(!buses.contains(&7), "FR7 was never invented: {buses:?}");
    assert!(buses.contains(&0), "the configured 路 still carries rows");
    assert_eq!(
        app.fr_bus_rows(),
        [0],
        "and the Buses table lists only the 路 that exists"
    );
    let node = app
        .snap
        .nodes
        .iter()
        .find(|n| n.id == id)
        .expect("the node");
    assert!(
        node.log
            .iter()
            .any(|l| l.contains("fr_send") && l.contains("FR7 is not a configured bus")),
        "the typo is reported where the script writes: {:?}",
        node.log
    );
    assert!(!node.errored, "{:?}", node.log);
    app.stop();
}
/// panel: it encodes one signal into the generator entry that owns the slot,
/// through the cluster description -- and `fr_send` to a described slot fills
/// that slot with what the description declares, not with what felt convenient.
/// The write lands in the entry, so an entry that is Off changes what it will
/// carry and still sends nothing.
#[test]
fn a_script_writes_a_flexray_signal_into_its_entry() {
    let arxml = crate::test_assets::path("PowerTrain.arxml");
    let mut app = quiet_app();
    app.tx_list.retain(|t| t.channel != 0);
    let Ok(bytes) = std::fs::read(arxml) else {
        panic!(
            "{arxml} missing -- it is tracked in the repository, so a broken checkout must fail, not skip"
        );
    };
    let parsed = crate::fr_db::FrDb::parse(&crate::dbc::text_from_bytes(bytes)).expect("parses");
    let (slot, sig, declared_len) = (0..parsed.frames.len())
        .find_map(|ix| {
            parsed.frame_sender(ix)?;
            let f = parsed.frame_index(ix)?;
            let sig = parsed.edit_signals(ix).into_iter().find(|s| s.size >= 4)?;
            Some((f.triggering.slot_id as u16, sig, f.length as usize))
        })
        .expect("the asset binds a described slot to a sending ECU");
    let sig_name = sig.name.clone();
    // Named in raw code words and converted through the signal's own coding, so
    // the value written is exactly the value read back whatever the factor is --
    // and wide enough that the raw number is representable at all.
    let want = crate::decode::to_physical(5, sig.size, sig.signed, sig.factor, sig.offset);
    let db = std::sync::Arc::new(parsed);
    app.fr_buses.insert(0, std::sync::Arc::clone(&db));
    app.push_fr_db_to_core();
    app.add_fr_tx(0, slot);
    app.settle();
    let before = app.snap.fr_tx[0].data_text.clone();
    app.send(crate::bus::BusCommand::AddNode {
        name: "ecu".to_string(),
        channel: 0,
        attached: None,
    });
    app.settle();
    let id = app.snap.nodes[0].id;
    app.send(crate::bus::BusCommand::SetNodeSource {
        id,
        source: format!(
            "on timer 10 {{ set_fr_sig(0, {slot}, {sig_name:?}, {want}); fr_send(0, {slot}, \
             0x00); }}"
        ),
    });
    app.settle();
    for n in 1..=6u64 {
        app.advance_clock(n * 10_000);
        app.tick(n * 10_000);
    }
    app.refresh_snapshot();
    let node = app
        .snap
        .nodes
        .iter()
        .find(|n| n.id == id)
        .expect("the node");
    assert!(!node.errored, "the handler ran clean: {:?}", node.log);
    assert!(
        node.log.iter().all(|l| !l.contains("set_fr_sig]")),
        "nothing was refused: {:?}",
        node.log
    );
    assert_ne!(
        app.snap.fr_tx[0].data_text, before,
        "the entry's base payload changed"
    );
    let ix = db.frame_ix_of_slot(slot).expect("described");
    let got = db
        .decode_signals(ix, &app.fr_tx_list[0].data)
        .into_iter()
        .find(|d| d.name == sig_name)
        .expect("the signal is in this frame");
    assert_eq!(got.phys, want, "the value the script wrote reads back");
    // The same script's `fr_send` fills this slot with one byte typed into a
    // frame the description says is `declared_len` wide.
    let row = (0..app.snap.fr_trace.len())
        .filter_map(|i| app.snap.fr_trace.get(i))
        .find(|r| r.slot == slot)
        .expect("the script's frame for this slot");
    assert_eq!(
        row.payload.len(),
        declared_len,
        "the slot keeps the width its schedule declares"
    );
    app.stop();
}

/// A script can only write a slot this tool is going to fill: `set_fr_sig` on a
/// slot with no generator entry, or on a 路 with no cluster description, drops
/// the write and says which fact is missing -- the value it computed is not sent
/// as half a frame, and nothing silently lands in a slot nobody owns.
#[test]
fn a_flexray_signal_write_without_its_entry_is_refused() {
    let mut app = quiet_app();
    app.tx_list.retain(|t| t.channel != 0);
    app.fr_buses.insert(0, one_slot_db(&["One"], 40.0));
    app.push_fr_db_to_core();
    app.send(crate::bus::BusCommand::AddNode {
        name: "ghost".to_string(),
        channel: 0,
        attached: None,
    });
    app.settle();
    let id = app.snap.nodes[0].id;
    app.send(crate::bus::BusCommand::SetNodeSource {
        id,
        source: "on timer 10 { set_fr_sig(0, 5, \"One\", 7); set_fr_sig(7, 5, \"One\", 7); }"
            .to_string(),
    });
    app.settle();
    for n in 1..=4u64 {
        app.advance_clock(n * 10_000);
        app.tick(n * 10_000);
    }
    app.refresh_snapshot();
    let node = app
        .snap
        .nodes
        .iter()
        .find(|n| n.id == id)
        .expect("the node");
    let lines: Vec<&String> = node
        .log
        .iter()
        .filter(|l| l.contains("set_fr_sig]"))
        .collect();
    assert!(
        lines
            .iter()
            .any(|l| l.contains("FR0 slot 5 has no transmit entry")),
        "the slot nobody fills: {lines:?}"
    );
    assert!(
        lines
            .iter()
            .any(|l| l.contains("FR7 has no cluster description")),
        "the 路 that does not exist: {lines:?}"
    );
    assert!(
        !lines.iter().any(|l| l.contains("does not declare signal")),
        "it never got as far as the coding: {lines:?}"
    );
    assert!(!node.errored, "{:?}", node.log);
    assert!(app.fr_tx_list.is_empty(), "no entry was invented either");
    app.stop();
}

/// Replaying a FlexRay log drives the `on fr slot` handlers off the file's own
/// timeline: one run per arrival, in the log's order, with that arrival's slot,
/// cycle and payload as the frame context. This is the shape a user actually
/// tests -- replay a recording and watch a simulated ECU answer it -- and the
/// replay path is a separate call site from the live watch, so it earns its own
/// check.
#[test]
fn replaying_a_flexray_log_drives_the_slot_handlers() {
    let mut app = quiet_app();
    app.tx_list.retain(|t| t.channel != 0);
    app.send(crate::bus::BusCommand::AddNode {
        name: "resp".to_string(),
        channel: 0,
        attached: None,
    });
    app.settle();
    let id = app.snap.nodes[0].id;
    app.send(crate::bus::BusCommand::SetNodeSource {
        id,
        source: "on fr 0 slot 13 { print(frame_id(), fr_cycle(), frame_byte(1)); send(0x320, fr_cycle()); }"
            .to_string(),
    });
    app.settle();
    let src = write_mixed_fr_asc("roxy_can_fr_handler", 4, 13);
    replay_to_the_end(&mut app, &src);
    let node = app
        .snap
        .nodes
        .iter()
        .find(|n| n.id == id)
        .expect("the node");
    assert!(!node.errored, "the handler ran clean: {:?}", node.log);
    assert_eq!(
        node.log,
        ["13 0 0", "13 1 1", "13 2 2", "13 3 3"],
        "one run per arrival, each with its own cycle and payload byte"
    );
    let agg = app
        .aggs
        .get(&(
            crate::model::ClusterId(app.snap.cluster_ids.can[0]),
            0x320,
            false,
        ))
        .expect("the reactions reached the CAN bus");
    assert_eq!(agg.count, 4, "one CAN frame per FlexRay arrival");
    assert_eq!(agg.data[0], 3, "the last one carried the last cycle");
}

/// The FlexRay half of the interactive generator. An entry fills one slot on
/// the period that slot's own schedule declares, and the frame it puts there
/// goes through the whole receive path -- the tally, the frame count, the
/// payload the views decode -- like a frame that arrived on a watched port.
/// Pinning a signal has to survive that trip: the value written in is the value
/// read back out of the bytes that actually went out.
#[test]
fn a_generated_flexray_slot_fills_the_session_on_its_schedule() {
    let arxml = crate::test_assets::path("PowerTrain.arxml");
    let mut app = quiet_app();
    app.tx_list.retain(|t| t.channel != 0);
    let db = crate::fr_db::FrDb::parse(&read_fr_asset(arxml)).expect("parses");
    // The first slot the ECU bindings give a sender to *and* that carries a
    // signal: what a generator entry can exist for at all. The period comes from
    // the asset rather than from the code under test, so the assertion below
    // means "as declared" and not "as computed".
    let (slot, ecu, sig, declared_us) = (0..db.frames.len())
        .find_map(|ix| {
            let ecu = db.frame_sender(ix).map(str::to_string)?;
            let f = db.frame_index(ix)?;
            // Wide enough that the raw numbers below are all representable.
            let sig = db.edit_signals(ix).into_iter().find(|s| s.size >= 4)?;
            let rep = f.triggering.cycle_repetition.max(1) as u64;
            Some((
                f.triggering.slot_id as u16,
                ecu,
                sig,
                (db.params.cycle_time_ms * rep as f64 * 1_000.0).round() as u64,
            ))
        })
        .expect("the asset binds a slot with a wide enough signal to a sending ECU");
    let sig_name = sig.name.clone();
    // Values named in raw code words and converted with the signal's own
    // encoding, so every one of them is exactly representable whatever the
    // coding's factor, offset, sign or declared range says.
    let phys =
        |raw: u64| crate::decode::to_physical(raw, sig.size, sig.signed, sig.factor, sig.offset);
    let (lo, hi, want) = (phys(0), phys(10), phys(5));
    let one_lsb = (phys(1) - phys(0)).abs() * 1.5;
    assert!(declared_us > 0, "the cluster declares a cycle time");
    let db = std::sync::Arc::new(db);
    app.fr_buses.insert(0, std::sync::Arc::clone(&db));
    app.push_fr_db_to_core();
    app.settle();

    app.add_fr_tx(0, slot);
    app.settle();
    assert_eq!(app.snap.fr_tx.len(), 1, "one entry for the slot");
    assert_eq!(
        app.snap.fr_tx[0].node, ecu,
        "the entry belongs to the ECU the description names as the sender"
    );
    assert_eq!(
        app.snap.fr_tx[0].cycle_us, declared_us,
        "the entry starts on the schedule's own period"
    );
    assert!(!app.snap.fr_tx[0].undescribed, "this slot is described");

    app.start_virtual();
    app.set_fr_tx_active(0, slot, true);
    app.settle();
    let before = app.snap.frame_counter;
    for n in 1..=3u64 {
        app.advance_clock(declared_us * n);
        app.tick(declared_us * n);
    }
    app.refresh_snapshot();
    let agg = app
        .fr_aggs
        .values()
        .find(|a| a.bus == 0 && a.slot == slot)
        .expect("the generated frames are tallied like any arrival");
    assert!(
        agg.count >= 3,
        "three periods passed, the slot filled each time: {}",
        agg.count
    );
    assert_eq!(
        app.snap.frame_counter - before,
        agg.count,
        "a generated frame counts as a frame, not as a special case"
    );
    assert_eq!(
        app.snap.fr_tx[0].sent_text.split_whitespace().count(),
        agg.payload.len(),
        "the row shows the bytes that went out"
    );

    // Pin one signal and the next frame must carry it, read back through the
    // same description the decoder uses.
    app.pin_gen_signal(crate::app::GenRow::Fr(0), &sig_name, want);
    app.settle();
    app.advance_clock(declared_us * 4);
    app.tick(declared_us * 4);
    app.refresh_snapshot();
    let agg = app
        .fr_aggs
        .values()
        .find(|a| a.bus == 0 && a.slot == slot)
        .expect("the slot is still tallied");
    let ix = db.frame_ix_of_slot(slot).expect("described above");
    let got = db
        .decode_signals(ix, &agg.payload)
        .into_iter()
        .find(|d| d.name == sig_name)
        .expect("the pinned signal is in the frame");
    assert!(
        (got.phys - want).abs() <= one_lsb,
        "{sig_name} reads back as {want}, got {} (raw {}), one LSB being {one_lsb}",
        got.phys,
        got.raw
    );

    // And a *driven* signal rides its own waveform: the value in each frame is
    // the source's value at that frame's stamp, laid over the base bytes the pin
    // just wrote. A Step over the slot's period hits both ends of its range, so
    // the set of values read back is exactly {lo, hi} -- never the pinned one.
    let mut src = crate::sim::ValueSrc::new(&sig_name, crate::sim::SrcKind::Step, lo, hi);
    // Four slot periods end to end: sampled once per slot period, the wave
    // would otherwise land on the same phase every frame and read as a
    // constant -- which is the aliasing, not a missing source.
    src.period_us = declared_us * 4;
    app.set_gen_source(crate::app::GenRow::Fr(0), src);
    app.settle();
    let mut seen = Vec::new();
    for n in 5..9u64 {
        app.advance_clock(declared_us * n);
        app.tick(declared_us * n);
        app.refresh_snapshot();
        let agg = app
            .fr_aggs
            .values()
            .find(|a| a.bus == 0 && a.slot == slot)
            .expect("the slot is still tallied");
        let got = db
            .decode_signals(ix, &agg.payload)
            .into_iter()
            .find(|d| d.name == sig_name)
            .expect("the driven signal is in the frame");
        seen.push(got.phys);
    }
    assert!(
        seen.iter().any(|v| (v - lo).abs() <= one_lsb),
        "{sig_name} rode down to its lo: {seen:?}"
    );
    assert!(
        seen.iter().any(|v| (v - hi).abs() <= one_lsb),
        "{sig_name} rode up to its hi: {seen:?}"
    );
    assert!(
        seen.iter().all(|v| (v - want).abs() > one_lsb),
        "the driven value replaced the pinned {want}, not the other way round: {seen:?}"
    );
    app.stop();
}

/// Puts the bundled ARXML on bus 0 the way the Buses window does, and returns
/// the first slot its ECU bindings give a sender to together with that ECU's
/// name -- the only kind of slot a generator entry can be built for.
fn fr_bound_slot(app: &mut App) -> (u16, String) {
    let arxml = crate::test_assets::path("PowerTrain.arxml");
    let db = crate::fr_db::FrDb::parse(&read_fr_asset(arxml)).expect("parses");
    let found = (0..db.frames.len()).find_map(|ix| {
        let ecu = db.frame_sender(ix).map(str::to_string)?;
        db.edit_signals(ix).into_iter().next()?;
        Some((db.frame_index(ix)?.triggering.slot_id as u16, ecu))
    });
    let (slot, ecu) = found.expect("the asset binds a slot with signals to a sending ECU");
    app.fr_buses.insert(0, std::sync::Arc::new(db));
    app.push_fr_db_to_core();
    app.settle();
    (slot, ecu)
}

/// A FlexRay entry whose description went away keeps filling its slot with the
/// bytes it holds and keeps its ECU row: the entry is the user's, and losing the
/// file that named its signals must neither delete the stimulus they built nor
/// hide it behind a tree the document no longer supports. The row says what is
/// missing instead of showing an empty signal list.
#[test]
fn a_flexray_entry_outlives_the_description_it_came_from() {
    let mut app = quiet_app();
    let (slot, ecu) = fr_bound_slot(&mut app);
    app.add_fr_tx(0, slot);
    // The width the description gives this slot, read before anything is typed.
    let wide = app.fr_tx_list[0].len;
    app.set_fr_tx_hex(0, slot, "11 22 33");
    app.set_fr_tx_cycle(0, slot, 10_000);
    app.set_fr_tx_active(0, slot, true);
    app.settle();
    assert_eq!(app.snap.fr_tx.len(), 1, "the entry was built from a slot");
    app.fr_buses.remove(&0);
    app.push_fr_db_to_core();
    app.settle();
    assert!(
        app.snap.fr_tx[0].undescribed,
        "nothing decodes the slot any more, and the row says so"
    );
    assert_eq!(app.snap.fr_tx[0].node, ecu, "and it is still the same row");
    app.start_virtual();
    app.settle();
    for n in 1..=2u64 {
        app.advance_clock(n * 10_000);
        app.tick(n * 10_000);
    }
    app.refresh_snapshot();
    let agg = app
        .fr_aggs
        .values()
        .find(|a| a.bus == 0 && a.slot == slot)
        .expect("the raw bytes still reach the tally");
    // The slot's width came from the description and stays with the entry after
    // it is gone: a FlexRay payload is as wide as the schedule says it is, so
    // typing three bytes sets those three and pads the rest -- it does not
    // shrink the frame the way a CAN DLC would.
    assert_eq!(agg.payload.len(), wide, "the frame keeps the slot's width");
    assert_eq!(
        agg.payload[..3],
        [0x11, 0x22, 0x33],
        "typed bytes lead, zeros pad"
    );
    app.stop();
}

/// A FlexRay payload is as wide as the schedule makes it, so the hex box is
/// bounded by the slot's declared length: three bytes typed into a wider slot
/// fill its head and pad the rest, and more bytes than the slot carries are
/// refused with the number that made the refusal -- silently dropping the tail
/// would send different bytes than the operator typed.
#[test]
fn a_flexray_payload_is_bounded_by_its_slot_width() {
    let mut app = quiet_app();
    let (slot, _) = fr_bound_slot(&mut app);
    app.add_fr_tx(0, slot);
    let wide = app.fr_tx_list[0].len;
    assert!(
        wide > 3,
        "the asset needs a slot wider than 3 bytes: {wide}"
    );

    app.set_fr_tx_hex(0, slot, "11 22 33");
    assert_eq!(
        app.fr_tx_list[0].len, wide,
        "typing fewer bytes does not shrink the slot"
    );
    assert_eq!(app.fr_tx_list[0].data[..3], [0x11, 0x22, 0x33]);
    assert!(
        app.fr_tx_list[0].data[3..].iter().all(|&b| b == 0),
        "the rest is padding"
    );

    app.set_fr_tx_hex(0, slot, &vec!["AA"; wide + 1].join(" "));
    assert_eq!(
        app.fr_tx_list[0].data[0], 0x11,
        "the refused edit changed nothing"
    );
    assert!(
        app.status.contains(&format!("{wide} bytes")),
        "the refusal names the width: {}",
        app.status
    );

    app.set_fr_tx_hex(0, slot, "zz 11");
    assert!(
        app.status.contains("hex"),
        "unreadable text gets its own reason: {}",
        app.status
    );
}

/// The FlexRay cycle box counts **cluster cycles**, not milliseconds: a static
/// slot transmits when the schedule says, and 0 -- CAN's "event-triggered" -- is
/// not a thing it can be. `cycles_of` is the same pair of facts read the other
/// way, and it stays silent about a period that is not a whole number of cycles
/// rather than rounding a row onto the grid it is not on.
#[test]
fn the_flexray_cycle_box_counts_cycles_and_refuses_event_fills() {
    use crate::generator::{cycles_of, fr_cycle_draft};
    let ct = 5_000; // a 5 ms cycle grid
    assert_eq!(fr_cycle_draft("2", ct), Ok((2, 10_000)));
    assert_eq!(fr_cycle_draft(" 12 ", ct), Ok((12, 60_000)));
    assert_eq!(fr_cycle_draft("1", ct), Ok((1, 5_000)));
    // CAN's event mode, refused with the reason and the alternative.
    let zero = fr_cycle_draft("0", ct).expect_err("0 is not a period");
    assert!(zero.contains("event-triggered"), "{zero}");
    assert!(zero.contains("On"), "{zero}");
    assert!(
        fr_cycle_draft("", ct).is_err(),
        "an empty box is not a period"
    );
    assert!(fr_cycle_draft("abc", ct).is_err());
    assert!(
        fr_cycle_draft("1e6", ct).is_err(),
        "and neither is scientific"
    );
    assert!(
        fr_cycle_draft("60001", ct).is_err(),
        "the same ceiling the ms box has, in cycles"
    );
    // No cycle time to count in -- an undescribed 路 -- is refused rather than
    // guessed at, because any number here would be an invented grid.
    assert!(fr_cycle_draft("2", 0).is_err());

    assert_eq!(cycles_of(10_000, ct), Some(2));
    assert_eq!(cycles_of(7_500, ct), None, "off the grid: no cycle count");
    assert_eq!(
        cycles_of(0, ct),
        None,
        "never sending is not every 0 cycles"
    );
    assert_eq!(cycles_of(10_000, 0), None, "and no grid means no counting");
}

/// Which node sends which frame is what the FIBEX/ARXML declares, and the
/// generator does not clean up after a document that fails to: a slot whose
/// frame no ECU is bound to gets no entry at all, and the status line names the
/// missing fact. The same for a bus with no description -- no invented owner,
/// no floating row to be edited somewhere off the tree.
#[test]
fn a_flexray_slot_the_description_leaves_unowned_gets_no_entry() {
    let mut app = App::headless();
    app.add_fr_tx(3, 13);
    app.settle();
    assert!(
        app.snap.fr_tx.is_empty(),
        "nothing owns the slot, so nothing was added"
    );
    assert!(
        app.status.contains("has no cluster description"),
        "{}",
        app.status
    );

    let (bound, _ecu) = fr_bound_slot(&mut app);
    let unowned = {
        let db = app.fr_db(0).expect("installed above");
        (0..db.frames.len())
            .find_map(|ix| {
                if db.frame_sender(ix).is_some() {
                    return None;
                }
                Some(db.frame_index(ix)?.triggering.slot_id as u16)
            })
            .expect("the asset leaves some frames unbound")
    };
    assert_ne!(unowned, bound, "the test needs both kinds of slot");
    app.add_fr_tx(0, unowned);
    app.settle();
    assert!(app.snap.fr_tx.is_empty(), "an unowned slot gets no entry");
    assert!(
        app.status.contains("does not name") && app.status.contains(&format!("slot {unowned}")),
        "{}",
        app.status
    );
}

/// The same standing-down rule the CAN generator follows while a log is being
/// replayed: if the replayed traffic carries this slot, a second sender of the
/// same signals would mix two values into every curve, count and verdict.
#[test]
fn a_generated_slot_stands_down_while_the_replayed_log_carries_it() {
    let mut app = quiet_app();
    let (slot, _ecu) = fr_bound_slot(&mut app);
    app.add_fr_tx(0, slot);
    app.set_fr_tx_cycle(0, slot, 1_000);
    app.set_fr_tx_active(0, slot, true);
    app.settle();
    let src = write_mixed_fr_asc("roxy_can_fr_gen_mute", 4, slot);
    replay_to_the_end(&mut app, &src);
    assert!(app.snap.fr_tx[0].muted, "the row says why it is quiet");
    // Summed over the slot's occupants: with a description loaded, the tally
    // splits the log's four arrivals by which frame the schedule puts in the
    // slot at each cycle -- which is exactly what a generator adding frames of
    // its own would have inflated.
    let total: u64 = app
        .fr_aggs
        .values()
        .filter(|a| a.bus == 0 && a.slot == slot)
        .map(|a| a.count)
        .sum();
    assert_eq!(
        total, 4,
        "only the log's four frames, not four plus a generator each millisecond"
    );
    std::fs::remove_file(&src).ok();
}

/// The spec monitor's rules have FlexRay counterparts, because the description
/// makes the same kind of promises: which slot a frame occupies, in which cycle
/// phase, how often it repeats and how long it is. Four verdicts from one
/// scripted run -- a payload short of its declared length (Dlc), an arrival off
/// the declared period (Cycle), a frame that then fell silent (Missing), and a
/// slot the schedule does not name at all (Unknown).
#[test]
fn the_spec_monitor_judges_flexray_frames_against_their_schedule() {
    use crate::hw::vector::flexray::FrFrame;
    use crate::spec::Kind;
    let arxml = crate::test_assets::path("PowerTrain.arxml");
    let mut app = quiet_app();
    app.tx_list.retain(|t| t.channel != 0);
    let Ok(bytes) = std::fs::read(arxml) else {
        panic!(
            "{arxml} missing -- it is tracked in the repository, so a broken checkout must fail, not skip"
        );
    };
    let db = crate::fr_db::FrDb::parse(&crate::dbc::text_from_bytes(bytes)).expect("parses");
    // The declared period comes out of the schedule itself, so the test states
    // the same promise the monitor checks against rather than a number picked
    // to make an assertion pass.
    let (ix, slot, cycle, declared_us) = {
        let (i, f) = db
            .frames
            .iter()
            .enumerate()
            .find(|(_, f)| f.length >= 8)
            .expect("the asset declares a frame at least 8 bytes long");
        let rep = f.triggering.cycle_repetition.max(1);
        (
            i,
            f.triggering.slot_id as u16,
            f.triggering.base_cycle as u8,
            rep as u64 * (db.params.cycle_time_ms * 1e3) as u64,
        )
    };
    assert!(declared_us > 0, "the cluster declares a cycle time");
    app.fr_buses.insert(0, db.into());
    app.push_fr_db_to_core();
    let q = app.hw.attach_fr_mock(0, 5);
    app.start_virtual();
    let arrive = |app: &mut App, slot: u16, payload: Vec<u8>, at_us: u64| {
        q.lock().expect("mock lock").push_back(FrFrame {
            slot,
            cycle,
            payload,
            header_crc: 0,
            flags: 0,
        });
        app.advance_clock(at_us);
        app.tick(at_us);
        app.refresh_snapshot();
    };
    // A short payload, then the same frame five periods later: both a length and
    // a timing violation in two arrivals.
    arrive(&mut app, slot, vec![0u8; 2], 1);
    let t = declared_us * 5;
    arrive(&mut app, slot, vec![0u8; 2], t);
    // Silence past the grace window (the gap since the last arrival, not the
    // time since the run started), then a slot the schedule names no frame for.
    let t = declared_us * (app.spec_grace + 10);
    app.advance_clock(t);
    app.tick(t);
    app.refresh_snapshot();
    arrive(&mut app, 2000, vec![0u8; 2], t + 1);

    let kinds = |app: &App, slot: u16| -> Vec<Kind> {
        app.snap
            .spec
            .fr_rows
            .iter()
            .filter(|(k, _)| k.0.0 == 0 && k.0.1 == slot)
            .map(|(k, _)| k.1)
            .collect()
    };
    let judged = kinds(&app, slot);
    for want in [Kind::Dlc, Kind::Cycle, Kind::Missing] {
        assert!(
            judged.contains(&want),
            "{slot} was judged {judged:?}, expected {want:?} too"
        );
    }
    assert!(
        kinds(&app, 2000).contains(&Kind::Unknown),
        "an unscheduled slot is the same finding as an id outside the DBC"
    );
    // The report line names the frame from the same description the verdict came
    // out of, and says which cluster and slot it is on.
    let rows = app.spec_rows();
    let fr = rows
        .iter()
        .find(|r| r.bus == "FR0" && r.addr == format!("slot {slot}"))
        .expect("the FlexRay row is in the report");
    assert_eq!(
        fr.name,
        app.fr_db(0).unwrap().frame_index(ix).unwrap().name,
        "the frame's own name, not an index"
    );
    assert_eq!(
        app.spec_rows()
            .iter()
            .filter(|r| r.bus == "FR0" && r.addr == "slot 2000")
            .map(|r| r.name.as_str())
            .next(),
        Some("not in the schedule"),
        "the unresolved slot says so"
    );
    app.stop();
}

/// The Messages window's Clear resets the FlexRay tallies as well: they are rows
/// of that table now, and leaving them to run up from their old counts made the
/// button lie about half of what it shows.
#[test]
fn clearing_the_message_counters_clears_the_flexray_tallies_too() {
    use crate::hw::vector::flexray::FrFrame;
    let mut app = quiet_app();
    app.tx_list.retain(|t| t.channel != 0);
    app.fr_buses.insert(0, one_slot_db(&["One"], 40.0));
    app.push_fr_db_to_core();
    let q = app.hw.attach_fr_mock(0, 5);
    app.start_virtual();
    for i in 0..3u64 {
        q.lock().expect("mock lock").push_back(FrFrame {
            slot: 5,
            cycle: 0,
            payload: vec![0x2A],
            header_crc: 0,
            flags: 0,
        });
        let t = (i + 1) * 20_000;
        app.advance_clock(t);
        app.tick(t);
        app.refresh_snapshot();
    }
    app.sync_msg_text(0);
    assert!(
        app.msg_windows[0]
            .text_rows
            .iter()
            .any(|r| r.bus == "FR0" && r.count == "3"),
        "the tally is on screen before the clear: {:?}",
        app.msg_windows[0]
            .text_rows
            .iter()
            .map(|r| (r.bus.clone(), r.label.clone(), r.count.clone()))
            .collect::<Vec<_>>()
    );

    app.send(crate::bus::BusCommand::ClearAggregates);
    app.advance_clock(100_000);
    app.tick(100_000);
    app.refresh_snapshot();
    assert!(
        app.snap.fr_aggs.is_empty(),
        "the FlexRay counters cleared with the CAN ones"
    );
    app.sync_msg_text(0);
    assert!(
        !app.msg_windows[0].text_rows.iter().any(|r| r.bus == "FR0"),
        "and the window shows nothing until a frame arrives again"
    );
    app.stop();
}

/// What must also throw the expanded cache away: the cluster description a
/// child row was indexed against. An expanded FlexRay row stores `(frame,
/// child)` indices into that description, so when the bus gets a different one
/// the old rows mean something else -- here the new frame declares a signal the
/// old one has no row for, and a cache that kept extending would go on showing
/// one child where the walk now yields two.
#[test]
fn a_new_cluster_description_rebuilds_the_expanded_rows() {
    use crate::hw::vector::flexray::FrFrame;
    let mut app = quiet_app();
    app.tx_list.retain(|t| t.channel != 0);
    let describe = |app: &mut App, signals: &[&str]| {
        app.fr_buses.insert(0, one_slot_db(signals, 40.0));
        app.push_fr_db_to_core();
    };
    describe(&mut app, &["One"]);
    let q = app.hw.attach_fr_mock(0, 5);
    app.start_virtual();
    app.trace_windows[0].fr_expand = true;
    q.lock().expect("mock lock").push_back(FrFrame {
        slot: 5,
        cycle: 0,
        payload: vec![0x2A],
        header_crc: 0,
        flags: 0,
    });
    app.advance_clock(20_000);
    app.tick(20_000);
    app.refresh_snapshot();
    app.text_fresh = true;
    app.sync_trace_rows(0);
    let kids = |app: &App| {
        cache_shape(app)
            .into_iter()
            .filter(|(_, kind, ..)| *kind == 2)
            .map(|(.., text)| text)
            .collect::<Vec<_>>()
    };
    assert_eq!(
        kids(&app),
        vec!["One=42  (2Ah)"],
        "the first description's one child row"
    );

    // The swap, with no new traffic: the filter, the rings and every timestamp
    // still say "extend", so only the description can be what rebuilds.
    describe(&mut app, &["One", "Two"]);
    app.text_fresh = true;
    app.sync_trace_rows(0);
    assert_eq!(
        kids(&app),
        vec!["One=42  (2Ah)", "Two=42  (2Ah)"],
        "rebuilt against the description the bus has now"
    );
    app.stop();
}

/// "仅 DBC" asks one question of every row in a mixed table: does a database
/// account for *this arrival*? A FlexRay frame the cluster description schedules
/// answers yes and stays -- the old reading dropped the entire FlexRay side,
/// which threw away precisely the rows the checkbox exists to keep. What has to
/// go is what no database describes, and there are two distinct kinds of that: a
/// slot this cluster's schedule holds no frame for, and any row on a bus with no
/// description at all.
#[test]
fn dbc_only_keeps_the_flexray_rows_a_description_covers() {
    use crate::aggregate::{FrFrameAgg, FrOccupant};
    let mut app = quiet_app();
    let arxml = crate::test_assets::path("PowerTrain.arxml");
    let text = read_fr_asset(arxml);
    let db = crate::fr_db::FrDb::parse(&text).expect("the asset parses");
    let (slot, cycle) = {
        let f = &db.frames[0];
        (f.triggering.slot_id as u16, f.triggering.base_cycle as u8)
    };
    let ix = db
        .frame_ix_at(slot, cycle, 0)
        .expect("the first frame resolves at its own slot and phase");
    app.fr_buses.insert(0, std::sync::Arc::new(db));
    let row = |bus: u8, at: u16| crate::trace::FrRow {
        bus,
        t_us: 1_000,
        ab: 0,
        slot: at,
        cycle,
        payload: vec![1],
        header_crc: 0,
        flags: 0,
        name: None,
    };
    let flt = |app: &App, dbc_only: bool| {
        let mut w = app.trace_windows[0].clone();
        w.dbc_only = dbc_only;
        w.filter_lens()
    };
    assert!(
        app.trace_fr_match(&flt(&app, true), &row(0, slot)),
        "the description schedules this frame: the row stays"
    );
    assert!(
        !app.trace_fr_match(&flt(&app, true), &row(0, 4095)),
        "no frame is scheduled in that slot"
    );
    assert!(
        !app.trace_fr_match(&flt(&app, true), &row(1, slot)),
        "FR1 has no description at all"
    );
    assert!(
        app.trace_fr_match(&flt(&app, false), &row(1, slot)),
        "unticked, an undescribed row is table material again"
    );

    // Messages asks the same question, because its CSV must match its table.
    // The tally carries its occupant in the row as well as in the map key --
    // that field is what "described" is read from.
    app.fr_aggs.insert(
        (0, slot, FrOccupant::Frame(ix)),
        FrFrameAgg {
            bus: 0,
            slot,
            count: 3,
            ab: 0,
            occupant: FrOccupant::Frame(ix),
            ..Default::default()
        },
    );
    app.fr_aggs.insert(
        (1, slot, FrOccupant::Unknown),
        FrFrameAgg {
            bus: 1,
            slot,
            count: 3,
            ab: 0,
            occupant: FrOccupant::Unknown,
            ..Default::default()
        },
    );
    app.refresh_snapshot();
    app.msg_windows[0].dbc_only = true;
    app.text_fresh = true;
    app.sync_msg_text(0);
    let buses: Vec<String> = app.msg_windows[0]
        .text_rows
        .iter()
        .map(|r| r.bus.clone())
        .collect();
    assert!(
        buses.iter().any(|b| b == "FR0"),
        "the described cluster's row is still listed: {buses:?}"
    );
    assert!(
        !buses.iter().any(|b| b == "FR1"),
        "the row no description covers is not: {buses:?}"
    );
    app.stop();
}

/// Reads a bundled cluster description the way the product does. The real ARXML
/// export is GBK, so `read_to_string` fails on it -- and a test written as
/// "skip when the read fails" then silently never runs. These assets are
/// committed, so a failure here is a broken test, not a stripped checkout.
fn read_fr_asset(path: &str) -> String {
    let bytes = std::fs::read(path).unwrap_or_else(|e| panic!("{path} is committed: {e}"));
    crate::dbc::text_from_bytes(bytes)
}

/// The trace text filter matches FlexRay frame names from the watch's
/// description database (and slot numbers), instead of hiding FR rows
/// wholesale. Value conditions still exclude them.
#[test]
fn the_trace_text_filter_matches_fr_frame_names() {
    let mut app = quiet_app();
    let text = read_fr_asset(crate::test_assets::path("PowerTrain.arxml"));
    let db = crate::fr_db::FrDb::parse(&text).expect("PowerTrain.arxml parses");
    app.fr_buses.insert(0, std::sync::Arc::new(db));
    // The database's first frame defines the identity the filter must
    // match: its name and its slot number.
    let first = &app.fr_db(0).expect("bus 0 has a description").frames[0];
    let name = first.name.clone();
    let slot = first.triggering.slot_id;
    let row = crate::trace::FrRow {
        bus: 0,
        t_us: 1_000,
        ab: 0,
        slot: slot as u16,
        cycle: first.triggering.base_cycle as u8,
        payload: vec![1, 2, 3],
        header_crc: 0,
        flags: 0,
        name: None,
    };

    let mk_flt = |app: &App, filter: &str| {
        let mut w = app.trace_windows[0].clone();
        w.filter = filter.to_string();
        w.filter_lens()
    };
    let flt = mk_flt(&app, &name);
    assert!(app.trace_fr_match(&flt, &row), "name match: {name}");
    let flt = mk_flt(&app, &name.to_lowercase());
    assert!(app.trace_fr_match(&flt, &row), "lowercase matches");
    let flt = mk_flt(&app, &format!("slot {slot}"));
    assert!(app.trace_fr_match(&flt, &row), "slot text matches");
    let flt = mk_flt(&app, "\u{7f}no-such-frame\u{7f}");
    assert!(
        !app.trace_fr_match(&flt, &row),
        "another name filters it out"
    );
    let flt = mk_flt(&app, "EngineSpeed>=200");
    assert!(
        !app.trace_fr_match(&flt, &row),
        "a signal-value condition is CAN-only"
    );
    app.stop();
}

/// A CANoe ASC names the rows it logs even where no cluster description is
/// loaded, and the Trace table shows that name -- so the text filter has to
/// match the name the row *displays*, not just the description's. The visible
/// slot number counts too: it is what the address column shows.
#[test]
fn the_trace_text_filter_matches_a_log_carried_fr_frame_name() {
    let mut app = quiet_app();
    assert!(
        app.fr_buses.is_empty(),
        "this case is about a row with no description behind it"
    );
    let row = crate::trace::FrRow {
        bus: 0,
        t_us: 1_000,
        ab: 2,
        slot: 47,
        cycle: 3,
        payload: vec![1],
        header_crc: 0,
        flags: 0,
        name: Some("Frame_47_3_1".into()),
    };
    let flt = |app: &App, filter: &str| {
        let mut w = app.trace_windows[0].clone();
        w.filter = filter.to_string();
        w.filter_lens()
    };
    assert!(
        app.trace_fr_match(&flt(&app, "frame_47"), &row),
        "the log's own name matches, case-insensitively"
    );
    assert!(
        app.trace_fr_match(&flt(&app, "47"), &row),
        "so does the slot number the address column shows"
    );
    assert!(
        !app.trace_fr_match(&flt(&app, "Frame_13"), &row),
        "another frame's name still filters it out"
    );
    app.stop();
}

/// Slot numbers repeat across clusters, so a table showing two of them cannot
/// be read as one network. `FlexRay: FR{n}` in the scope combo narrows a
/// Trace/Messages/Statistics window to that cluster -- the same number its Bus
/// column prints -- and CAN rows leave with it. Before this the only way to
/// look at one of a two-cluster recording was the text filter, which cannot
/// name a row that has no name.
#[test]
fn an_analysis_window_can_be_scoped_to_one_flexray_cluster() {
    use crate::aggregate::{FrFrameAgg, FrOccupant};
    let mut app = App::headless();
    for (bus, slot) in [(0u8, 13u16), (0, 24), (1, 13)] {
        app.fr_aggs.insert(
            (bus, slot, FrOccupant::Unknown),
            FrFrameAgg {
                bus,
                slot,
                count: 10,
                // ab = 2 ("unknown"): the Messages label would otherwise print
                // the channel letter after the slot, which is not what this
                // test is about.
                ab: 2,
                ..Default::default()
            },
        );
    }
    let cluster = crate::model::ClusterId(app.snap.cluster_ids.can[0]);
    app.aggs.insert(
        (cluster, 0x100, false),
        MessageAgg {
            id: 0x100,
            channel: 0,
            extended: false,
            dir: Direction::Rx,
            count: 5,
            rx: 5,
            tx: 0,
            last_t_us: 0,
            cycle_us: 0.0,
            min_us: 0.0,
            max_us: 0.0,
            jitter_us: 0.0,
            len: 8,
            data: [0; MAX_CAN_FD_LEN],
            flags: FrameFlags::NONE,
        },
    );
    app.refresh_snapshot();
    assert_eq!(app.snap.fr_aggs.len(), 3, "one tally per bus and slot");

    // The combo offers every cluster the windows could show. Nothing is
    // configured here -- the rows alone make both clusters pickable.
    assert_eq!(app.fr_scope_buses(SigScope::All), vec![0, 1]);
    // A choice whose rows have all aged out stays listed, or the combo would
    // drop the entry the window is looking at.
    assert_eq!(app.fr_scope_buses(SigScope::FrBus(3)), vec![0, 1, 3]);

    let fr_row = |bus: u8, slot: u16| crate::trace::FrRow {
        bus,
        t_us: 1_000,
        ab: 0,
        slot,
        cycle: 0,
        payload: vec![0],
        header_crc: 0,
        flags: 0,
        name: None,
    };
    let flt = |app: &App, scope: SigScope| {
        let mut w = app.trace_windows[0].clone();
        w.scope = scope;
        w.filter_lens()
    };
    let (r0, r1) = (fr_row(0, 13), fr_row(1, 13));
    assert!(app.trace_fr_match(&flt(&app, SigScope::All), &r0));
    assert!(
        app.trace_fr_match(&flt(&app, SigScope::FrBus(1)), &r1),
        "the scoped cluster's rows stay"
    );
    assert!(
        !app.trace_fr_match(&flt(&app, SigScope::FrBus(1)), &r0),
        "the other cluster is out, same slot number and all"
    );
    assert!(
        !app.trace_fr_match(&flt(&app, SigScope::Bus(0)), &r0),
        "a CAN channel scope is still CAN-only"
    );
    let mut w = app.trace_windows[0].clone();
    w.scope = SigScope::FrBus(1);
    let can = frame_at(1_000, 0x100, 8, Direction::Rx);
    assert!(
        !app.trace_match(&w, &can),
        "an FR cluster scope keeps CAN out of the table"
    );

    // The Messages table: one cluster's slots, and not the CAN row.
    let shown = |app: &App| -> Vec<(String, String)> {
        app.msg_windows[0]
            .text_rows
            .iter()
            .map(|r| (r.bus.clone(), r.label.clone()))
            .collect()
    };
    app.msg_windows[0].scope = SigScope::FrBus(1);
    app.text_fresh = true;
    app.sync_msg_text(0);
    assert_eq!(
        shown(&app),
        vec![("FR1".to_string(), "slot 13".to_string())]
    );
    app.msg_windows[0].scope = SigScope::FrBus(0);
    app.text_fresh = true;
    app.sync_msg_text(0);
    assert_eq!(
        shown(&app),
        vec![
            ("FR0".to_string(), "slot 13".to_string()),
            ("FR0".to_string(), "slot 24".to_string())
        ],
        "cluster 0 alone, its two slots in slot order"
    );

    // Statistics: the share column is that cluster's own traffic once scoped,
    // not a slice of everything the run carried.
    app.stats_windows[0].scope = SigScope::FrBus(1);
    app.text_fresh = true;
    app.sync_stats_text(0);
    let stats = &app.stats_windows[0].text_rows;
    assert_eq!(stats.len(), 1, "one row for that cluster");
    assert_eq!(
        (stats[0].bus.as_str(), stats[0].share.as_str()),
        ("FR1", "100.0%")
    );
    app.stats_windows[0].scope = SigScope::All;
    app.text_fresh = true;
    app.sync_stats_text(0);
    let shares: Vec<(&str, &str)> = app.stats_windows[0]
        .text_rows
        .iter()
        .map(|r| (r.bus.as_str(), r.share.as_str()))
        .collect();
    assert_eq!(shares.len(), 4, "CAN row plus three slots: {shares:?}");
    assert_eq!(
        shares,
        vec![
            ("CAN1", "14.3%"),
            ("FR0", "28.6%"),
            ("FR0", "28.6%"),
            ("FR1", "28.6%")
        ],
        "unscoped, all 35 frames of the run are shared out"
    );

    // And that number is a *name* once the user gives the 路 one: every table
    // that says which bus a row came from follows it, so a renamed cluster does
    // not reappear as an anonymous FR0 two windows down.
    app.set_flexray_name(0, "动力总成");
    app.msg_windows[0].scope = SigScope::FrBus(0);
    app.text_fresh = true;
    app.sync_msg_text(0);
    assert_eq!(
        shown(&app),
        vec![
            ("动力总成".to_string(), "slot 13".to_string()),
            ("动力总成".to_string(), "slot 24".to_string())
        ],
        "the Messages Bus column prints the name"
    );
    app.stats_windows[0].scope = SigScope::All;
    app.text_fresh = true;
    app.sync_stats_text(0);
    assert!(
        app.stats_windows[0]
            .text_rows
            .iter()
            .filter(|r| r.bus == "动力总成")
            .count()
            == 2,
        "both of cluster 0's slots, and no other row: {:?}",
        app.stats_windows[0]
            .text_rows
            .iter()
            .map(|r| (r.bus.clone(), r.label.clone()))
            .collect::<Vec<_>>()
    );
    app.stop();
}

/// R2 装配层校验：脚本发送了不属于自己节点的报文 → 节点日志出 warning。
/// 脚本绑定 EngineECU，但 send 的是 0x200（ChassisECU 的 VehicleState）。
#[test]
fn a_send_to_another_nodes_message_logs_a_warning() {
    let mut app = App::headless();
    app.send(crate::bus::BusCommand::AddNode {
        name: "gateway".to_string(),
        channel: 0,
        attached: Some((0, "EngineECU".to_string())),
    });
    app.settle();
    let id = app.snap.nodes[0].id;
    app.send(crate::bus::BusCommand::SetNodeSource {
        id,
        source: "on timer 100 { send(0x200, 1); }".to_string(),
    });
    app.send(crate::bus::BusCommand::SetNodeEnabled { id, on: true });
    app.start_virtual();
    app.settle();
    // Give the node a moment to produce the warning.
    std::thread::sleep(std::time::Duration::from_millis(50));
    app.update();
    let node = app.snap.nodes.iter().find(|n| n.id == id).expect("node");
    assert!(
        node.log
            .iter()
            .any(|l| l.contains("0x200") && l.contains("ChassisECU")),
        "log should warn about the foreign message: {:?}",
        node.log
    );
    app.stop();
}

#[test]
fn a_rewound_or_restarted_run_reveals_its_rows_at_once() {
    let mut app = quiet_app();
    if app.trace_windows.is_empty() {
        app.new_trace_window();
    }
    feed_rpm(&mut app, &[(10_000, 100.0), (30_000, 110.0)]);
    app.text_fresh = true;
    app.sync_trace_rows(0);
    assert_eq!(app.trace_windows[0].shown_t_us, 30_000);

    // A backward seek re-stamps frames below the watermark; those must not
    // wait for the gate -- only the fresh tail is batched.
    feed_rpm(&mut app, &[(5_000, 40.0)]);
    app.text_fresh = false;
    app.sync_trace_rows(0);
    assert_eq!(
        app.trace_revealed(&app.trace_windows[0]).count(),
        3,
        "the older-stamped row shows immediately"
    );
    app.stop();
}

#[test]
fn a_log_id_stays_silent_during_replay_and_returns_in_simulation() {
    // The log carries 0x100 at 100 ms. An active generator twin at 10 ms
    // must hold its breath for the run -- replaying a recording of this
    // same simulation used to interleave two senders of one signal -- while
    // an id the log lacks keeps injecting, so "stir a few frames in"
    // survives.
    let path = write_timed_asc("roxy_can_mute_twin.asc", 20, 100_000);
    let mut app = App::headless();
    // The sample config pre-populates a generator entry for the log's 0x100;
    // add one id the log lacks as the stirring control.
    // 角色开关允许发送：EngineECU 模拟（否则开关拦截发送）。
    app.set_node_role(0, "EngineECU", NodeRole::Simulated);
    let twin = app
        .tx_list
        .iter()
        .position(|t| t.channel == 0 && t.id == 0x100)
        .expect("sample config carries a 0x100 entry");
    {
        let tx = &mut app.tx_list[twin];
        tx.cycle_us = 10_000;
        tx.active = true;
    }
    app.add_tx(0, 0x123);
    {
        let tx = app.tx_list.last_mut().unwrap();
        tx.cycle_us = 10_000;
        tx.active = true;
    }
    app.load_log(&path.to_string_lossy());
    app.replay();
    assert!(matches!(app.mode, Mode::Replay), "the replay is running");
    assert!(
        app.replay_ids.contains(&(0u8, 0x100u32)),
        "the scan saw the log's id"
    );

    // ~2.4 s of replay clock covers the whole 1.9 s log.
    let mut now = 0u64;
    for _ in 0..240 {
        now += 10_000;
        app.tick(now);
    }

    assert_eq!(
        slots_of(&app, 0x100).len(),
        20,
        "exactly the log's own 0x100 frames, none injected"
    );
    let stirred = slots_of(&app, 0x123);
    assert!(
        stirred.len() > 100,
        "an id the log lacks still injects freely, got {}",
        stirred.len()
    );

    // Back on the simulated bus the twin transmits again from a clean start.
    app.stop();
    app.start_virtual();
    run_sim(&mut app, 30, 10_000);
    assert!(
        slots_of(&app, 0x100).len() >= 30,
        "the twin speaks again once the replay is over"
    );
    std::fs::remove_file(&path).ok();
}

#[test]
fn re_activating_a_generator_starts_from_now_not_from_history() {
    // The old activation path re-zeroed next_t_us, so the catch-up loop
    // re-emitted the whole off period with wave values at those stale
    // stamps -- Graphics read the refilled gap as a continuous wave.
    let mut app = quiet_app();
    app.add_tx(0, 0x300);
    let i = app.tx_list.len() - 1;
    app.tx_list[i].cycle_us = 10_000;
    app.set_tx_active(i, true);
    run_sim(&mut app, 100, 10_000); // transmits until t = 1 s

    // Off for two seconds of clock -- time moves, no frames go out.
    app.set_tx_active(i, false);
    let mut now = 1_000_000u64;
    while now < 2_990_000 {
        now += 10_000;
        app.sim_t_us = now;
        app.tick(now);
    }
    let before = slots_of(&app, 0x300).len();
    assert_eq!(
        before, 101,
        "one batch on the first tick, one per tick after"
    );

    // On again: anchored at the clock, the silent second stays silent.
    app.set_tx_active(i, true); // schedule starts at t = 2.99 s
    let end = now + 100 * 10_000;
    while now < end {
        now += 10_000;
        app.sim_t_us = now;
        app.tick(now);
    }
    let slots = slots_of(&app, 0x300);
    assert_eq!(slots.len(), 202, "the off period contributes no frames");
    assert!(
        slots.iter().all(|&t| t <= 1_000_000 || t >= 2_990_000),
        "no frame may be dated inside the off period"
    );
    app.stop();
}

#[test]
fn the_trace_ring_wraps_and_keeps_the_newest_limit_frames() {
    let mut app = App::headless();
    app.start_virtual();
    let extra = 100u64;
    let total: u64 = (crate::app::TRACE_LIMIT + extra as usize) as u64;
    let frames: Vec<CanFrame> = (0..total)
        .map(|i| frame_at(i * 1_000, 0x100, 2, Direction::Rx))
        .collect();
    receive(&mut app, total * 1_000, frames);
    assert_eq!(
        app.trace.len(),
        crate::app::TRACE_LIMIT,
        "the ring is capped"
    );
    assert_eq!(
        app.trace.iter().next().unwrap().t_us,
        extra * 1_000,
        "the frames beyond the cap are dropped from the front"
    );
    assert_eq!(app.trace.back().unwrap().t_us, (total - 1) * 1_000);
    // Chunked storage must not break reverse iteration of the view.
    let newest_two: Vec<u64> = app
        .snap
        .trace
        .iter()
        .rev()
        .take(2)
        .map(|f| f.t_us)
        .collect();
    assert_eq!(
        newest_two,
        vec![(total - 1) * 1_000, (total - 2) * 1_000],
        "the view's newest frames sit at the back, sealed chunks notwithstanding"
    );
}

#[test]
fn a_published_trace_view_stays_frozen_while_the_ring_moves_on() {
    let mut app = App::headless();
    app.start_virtual();
    receive(&mut app, 1_000, vec![frame_at(0, 0x100, 2, Direction::Rx)]);
    let view = app.snap.trace.clone();
    assert_eq!(view.len(), 1);

    // The run goes on: the ring grows, the published view does not --
    // the UI keeps reading a stable instant while the bus advances.
    receive(
        &mut app,
        2_000,
        vec![frame_at(1_000, 0x100, 2, Direction::Rx)],
    );
    assert_eq!(view.len(), 1, "the published view is frozen at its instant");
    assert_eq!(app.trace.len(), 2);
    assert_eq!(app.snap.trace.len(), 2, "the next snapshot sees the growth");
}

/// Loading a second cluster's description is not the same act as opening a
/// second port: a recording can hold two clusters and the second one needs its
/// own description before its frames can be named, decoded, subscribed or
/// exported -- with no hardware in the loop. A file that cannot be read is
/// reported and takes no bus index with it.
#[test]
fn a_second_cluster_description_loads_without_any_hardware() {
    let first = crate::test_assets::path("PowerTrain.arxml");
    let second = crate::test_assets::path("PowerTrain_v2.xml");
    if !std::path::Path::new(first).exists() || !std::path::Path::new(second).exists() {
        println!("assets absent -- skipped");
        return;
    }
    let mut app = quiet_app();
    assert_eq!(app.load_cluster_description(first, None), Some(0));
    assert_eq!(app.load_cluster_description(second, None), Some(1));
    assert_eq!(
        app.load_cluster_description("assets/no-such-cluster.xml", None),
        None,
        "a file that cannot be read is refused"
    );
    assert!(app.status.contains("read failed"), "{}", app.status);
    assert_eq!(
        app.fr_buses.keys().copied().collect::<Vec<_>>(),
        [0, 1],
        "each description got its own bus"
    );
    assert_eq!(
        app.fr_dbs.keys().copied().collect::<Vec<_>>(),
        [0, 1],
        "and the core was told about both, so a replay decodes per cluster"
    );
    app.stop();
}

/// Which FlexRay 路 a description file describes is the user's call: a
/// recording numbers its clusters 0/1 and names nothing, and a 路 given the
/// wrong schedule keeps working -- it just names and decodes every frame of
/// one network against the other network's slots. That is the one failure mode
/// worse than having no names at all, so the file lands where it is pointed
/// rather than on "the first 路 without one", in whatever order the files
/// happened to be picked. A 路 with a port open refuses the swap, because its
/// watch was configured from the description already there.
#[test]
fn a_description_lands_on_the_bus_it_is_pointed_at() {
    let first = crate::test_assets::path("PowerTrain.arxml");
    let second = crate::test_assets::path("PowerTrain_v2.xml");
    if !std::path::Path::new(first).exists() || !std::path::Path::new(second).exists() {
        println!("assets absent -- skipped");
        return;
    }
    let mut app = quiet_app();
    let sig = |app: &App, bus: u8| -> (String, usize) {
        let db = app.fr_db(bus).expect("that 路 has a description");
        (db.params.name.clone(), db.frames.len())
    };
    assert_eq!(app.load_cluster_description(first, Some(1)), Some(1));
    assert_eq!(app.load_cluster_description(second, Some(0)), Some(0));
    let (on_first, on_second) = (sig(&app, 1), sig(&app, 0));
    assert_ne!(on_first, on_second, "the two assets are told apart");
    // A misplaced file moves off a 路 by removing the description and binding
    // the wanted one; binding is additive, never a silent replacement.
    app.remove_fr_description(0, second, "");
    assert_eq!(app.load_cluster_description(first, Some(0)), Some(0));
    assert_eq!(
        sig(&app, 0),
        on_first,
        "bus 0 now carries the file it was given"
    );
    assert_eq!(
        app.fr_dbs.keys().copied().collect::<Vec<_>>(),
        [0, 1],
        "the core still has one description per 路"
    );
    assert_eq!(sig(&app, 1), on_first, "bus 1 already had this one");
    // A watched 路 refuses, and keeps what its port runs from.
    let _q = app.hw.attach_fr_mock(1, 5);
    app.refresh_snapshot();
    assert_eq!(app.load_cluster_description(second, Some(1)), None);
    assert!(
        app.status.contains(crate::channel::DETACH_LABEL),
        "the refusal names the button that can do it: {}",
        app.status
    );
    assert_eq!(sig(&app, 1), on_first, "the live 路 kept its file");
    // The rows the Buses table lists: the 路 that exist as state -- the two
    // configured ones. No placeholder for "the next free index"; a bus nobody
    // added is not a bus, and "+ FlexRay" is what adds one.
    assert_eq!(app.fr_bus_rows(), [0, 1]);
    assert_eq!(
        app.next_flexray_bus(),
        Some(2),
        "the add button would place the next description on the first empty index"
    );
    // A 路 that exists only as traffic in the log being replayed is a row: its
    // frames cannot be named until a description lands on it, and that is what
    // its `加载描述…` button is for. Listed in bus order, not appended.
    app.fr_loads.insert(4, crate::load::FrLoad::default());
    // The rollups are only re-published when they changed; a hand-inserted load
    // has to say so, which is what a step does on every arrival.
    app.loads_dirty = true;
    app.refresh_snapshot();
    assert_eq!(app.fr_bus_rows(), [0, 1, 4]);
    app.stop();
}

/// A 路 that has a description but no port is attached with the file it already
/// holds (no second picker); `断开` closes that port and leaves the 路 alone; and
/// removal takes the whole 路, port included -- which a watched 路 allows
/// precisely because closing the port is part of what removal means.
#[test]
fn a_flexray_bus_can_be_detached_and_removed() {
    let arxml = crate::test_assets::path("PowerTrain.arxml");
    if !std::path::Path::new(arxml).exists() {
        panic!(
            "{arxml} missing -- it is tracked in the repository, so a broken checkout must fail, not skip"
        );
    }
    let mut app = quiet_app();
    assert_eq!(app.load_cluster_description(arxml, None), Some(0));

    // Attaching uses the loaded description: what comes back is the missing
    // driver, not a missing file.
    app.attach_fr_watch_on(0, 5);
    assert!(app.status.contains("FR0"), "{}", app.status);
    assert!(
        !app.status.contains("has no cluster description"),
        "the description was there to use: {}",
        app.status
    );
    // A bus with nothing loaded cannot be attached.
    app.attach_fr_watch_on(3, 5);
    assert!(
        app.status.contains("has no cluster description"),
        "{}",
        app.status
    );

    // 断开 closes the port and keeps the 路: it is still the cluster its
    // description describes, so nothing pointed at it has to move.
    app.hw.attach_fr_mock(0, 5);
    app.refresh_snapshot();
    app.detach_flexray_watch(0);
    assert!(app.snap.fr_watches.is_empty(), "the port closed");
    assert!(app.fr_buses.contains_key(&0), "the 路 stayed");
    assert!(
        app.fr_dbs.contains_key(&0),
        "and so did the core's copy of its description"
    );

    // Removal is the whole 路, no prior 断开 needed.
    app.hw.attach_fr_mock(0, 5);
    app.refresh_snapshot();
    app.remove_fr_bus(0);
    assert!(app.fr_buses.is_empty(), "the frontend forgot it");
    assert!(app.fr_dbs.is_empty(), "and told the core");
    assert!(app.snap.fr_watches.is_empty(), "the port went with it");
    assert!(
        app.status.contains("FR0"),
        "the bar names it: {}",
        app.status
    );
    app.stop();
}

/// Deleting a FlexRay 路 takes everything keyed on its cluster index with it --
/// the frames it received, their tallies, its subscriptions, curves, hand-picked
/// rows, send entries and rules -- while the surviving 路 keeps both its number
/// and its data. The number is the point: a cluster index is an identity, not a
/// position in a list. A BLF stamps each row with its own `clusterNo`, a script
/// types `fr_sig(1, ..)` and the record filter types `FR1:5` with it, so
/// shifting the survivors down would silently re-aim every one of those at the
/// wrong network -- and the gap is not a problem, since the next 路 added takes
/// the lowest free index anyway.
#[test]
fn removing_a_flexray_bus_takes_everything_keyed_on_it() {
    use crate::hw::vector::flexray::FrFrame;
    use crate::observe::GfxSignal;
    use crate::workspace::{Pick, SigScope};
    let mut app = quiet_app();
    let (slot, _) = fr_bound_slot(&mut app);
    let db = std::sync::Arc::clone(&app.fr_buses[&0]);
    app.fr_buses.insert(1, db);
    app.push_fr_db_to_core();
    let q0 = app.hw.attach_fr_mock(0, 5);
    let q1 = app.hw.attach_fr_mock(1, 6);
    // One reference of each kind, on each 路.
    for bus in [0u8, 1] {
        let key = app.fr_key(bus, slot, "CarSpeed".to_string());
        app.subscribe(key.clone());
        app.graphics[0].signals.push(GfxSignal {
            key,
            visible: true,
            y_mode: YMode::Auto,
        });
        app.trace_windows[0].manual.insert(Pick::Fr { bus, slot });
        app.add_fr_tx(bus, slot);
    }
    app.msg_windows[0].scope = SigScope::FrBus(0);
    app.stats_windows[0].scope = SigScope::FrBus(1);
    // A rule on each 路: removal has one to take away and one to leave alone.
    // `add_fr_trigger` aims at `fr_default_slot`, which reads the first 路 --
    // the one being deleted.
    app.add_fr_trigger();
    app.send(crate::bus::BusCommand::AddTrigger {
        cond: TriggerCond::FrSignalCross {
            bus: 1,
            slot,
            signal: "CarSpeed".to_string(),
            threshold: 0.0,
            rising: true,
        },
        action: TriggerAction::StartRecording,
        enabled: true,
    });
    let rules_on = |app: &App, bus: u8| -> usize {
        app.snap
            .triggers
            .iter()
            .filter(|t| t.cond.fr_bus() == Some(bus))
            .count()
    };
    assert_eq!(rules_on(&app, 0), 1, "the new FlexRay rule sits on FR0");
    assert_eq!(rules_on(&app, 1), 1, "and the other on FR1");
    app.start_virtual();
    for q in [&q0, &q1] {
        q.lock().expect("mock lock").push_back(FrFrame {
            slot,
            cycle: 0,
            payload: vec![0; 8],
            header_crc: 0,
            flags: 0,
        });
    }
    app.advance_clock(1_000);
    app.tick(1_000);
    app.refresh_snapshot();
    let rows = |app: &App| -> Vec<u8> {
        (0..app.snap.fr_trace.len())
            .filter_map(|i| app.snap.fr_trace.get(i))
            .map(|r| r.bus)
            .collect()
    };
    assert_eq!(rows(&app), [0, 1], "both 路 carried traffic");

    // A spec verdict and its interval memory on each 路, so the report has one
    // row to lose and one to keep. Planted after the last step, on a slot no
    // traffic fills: the monitor rewrites both tables from the arrivals on every
    // step, so anything planted before the run is the monitor's data, not the
    // test's.
    let phantom = slot + 1;
    for bus in [0u8, 1] {
        app.spec.record_fr(
            (bus, phantom, None),
            crate::spec::Kind::Cycle,
            1_000,
            5.0,
            9.0,
        );
        app.spec.note_fr((bus, phantom, 0), 1_000);
    }
    // The port each row's picker last chose, mirroring the mock watches.
    app.fr_pick.insert(0, 5);
    app.fr_pick.insert(1, 6);
    let fr0 = app.fr_key(0, slot, "CarSpeed".to_string());
    let fr1 = app.fr_key(1, slot, "CarSpeed".to_string());

    app.remove_fr_bus(0);
    app.refresh_snapshot();

    // The survivor kept its index, on both sides.
    assert_eq!(app.fr_buses.keys().copied().collect::<Vec<_>>(), [1]);
    assert_eq!(app.fr_dbs.keys().copied().collect::<Vec<_>>(), [1]);
    assert_eq!(
        app.fr_bus_rows(),
        [1],
        "the Buses table has one 路 left, and it is still FR1"
    );
    assert_eq!(
        app.fr_pick.get(&1).copied(),
        Some(6),
        "FR1's picker still names the port it chose"
    );
    assert!(
        !app.fr_pick.contains_key(&0),
        "the deleted 路 leaves no port choice for whoever takes the index later"
    );
    // Its traffic stayed, the deleted 路's went.
    assert_eq!(rows(&app), [1], "FR0's rows are out of the ring");
    assert_eq!(
        app.fr_aggs.values().filter(|a| a.bus == 0).count(),
        0,
        "and its tallies with them"
    );
    assert!(
        app.fr_aggs.values().any(|a| a.bus == 1 && a.count >= 1),
        "FR1 is still tallied: {:?}",
        app.fr_aggs
            .values()
            .map(|a| (a.bus, a.count))
            .collect::<Vec<_>>()
    );
    // No subscription, curve or hand-picked row names the deleted cluster.
    assert!(
        !app.subs.keys().any(|k| k.cluster() == fr0.cluster()),
        "FR0's subscription is gone"
    );
    assert!(app.subs.contains_key(&fr1));
    assert_eq!(
        app.graphics[0]
            .signals
            .iter()
            .map(|s| match &s.key {
                SigKey::Fr { cluster, .. } => cluster.0,
                _ => 255,
            })
            .collect::<Vec<_>>(),
        [fr1.cluster().expect("a cluster").0],
        "only FR1's curve is left"
    );
    assert_eq!(
        app.trace_windows[0]
            .manual
            .iter()
            .copied()
            .collect::<Vec<_>>(),
        [Pick::Fr { bus: 1, slot }],
        "and only FR1's picked row"
    );
    // Its send entries and its rule.
    assert_eq!(
        app.snap.fr_tx.iter().map(|t| t.bus).collect::<Vec<_>>(),
        [1],
        "FR0's generator entry went with it"
    );
    // The spec report loses the deleted 路's verdicts and keeps the survivor's.
    assert!(
        app.spec.fr_rows.keys().all(|(s, _)| s.0 != 0),
        "no verdict about a cluster that is gone"
    );
    assert_eq!(
        app.spec
            .fr_rows
            .keys()
            .filter(|(s, _)| s.0 == 1 && s.1 == phantom)
            .count(),
        1,
        "FR1's row is still reported"
    );
    assert_eq!(
        app.spec.fr_previous((0, phantom, 0)),
        None,
        "and FR0's interval memory with it"
    );
    assert_eq!(app.spec.fr_previous((1, phantom, 0)), Some(1_000));
    assert_eq!(
        rules_on(&app, 0),
        0,
        "a rule about a cluster that is gone can never fire"
    );
    assert_eq!(rules_on(&app, 1), 1, "FR1's rule is untouched");
    // The window pointed at the deleted cluster falls back to the widest view;
    // the one pointed at the survivor is nobody's business.
    assert_eq!(app.msg_windows[0].scope, SigScope::All);
    assert_eq!(app.stats_windows[0].scope, SigScope::FrBus(1));
    // The gap is what the next added 路 uses.
    assert_eq!(app.next_flexray_bus(), Some(0));
    assert!(app.status.contains("FR0"), "{}", app.status);
    app.stop();
}

/// What starting the tool hands you: two windows, not six.
///
/// Statistics / Graphics / Data / State Tracker used to be created on every
/// boot, so each one landed in every project file and each one came back after
/// the user had removed it. They are added from Measurement Setup now.
#[test]
fn a_started_session_boots_with_trace_and_messages_only() {
    let mut app = App::headless();
    for kind in 0..6 {
        match kind {
            0 => app.trace_windows.clear(),
            1 => app.msg_windows.clear(),
            2 => app.stats_windows.clear(),
            3 => app.graphics.clear(),
            4 => app.data_windows.clear(),
            _ => app.state_trackers.clear(),
        }
    }
    app.seed_startup_windows(false);
    assert_eq!(
        (
            app.trace_windows.len(),
            app.msg_windows.len(),
            app.stats_windows.len(),
            app.graphics.len(),
            app.data_windows.len(),
            app.state_trackers.len()
        ),
        (1, 1, 0, 0, 0, 0),
        "the product seeds the two windows an operator looks at first"
    );
    // The manual drive keeps one of each kind, which is what every suite in
    // this file indexes by position.
    app.seed_startup_windows(true);
    assert_eq!(
        (
            app.trace_windows.len(),
            app.stats_windows.len(),
            app.graphics.len(),
            app.data_windows.len(),
            app.state_trackers.len()
        ),
        (2, 1, 1, 1, 1),
        "the extra kinds arrive only on the full workspace"
    );
}

/// The Ethernet side of the data path: a captured frame reaches the shared
/// snapshot the Trace window reads.
#[test]
fn ethernet_frames_reach_the_snapshot() {
    let mut app = App::headless();
    let frame = crate::eth::EthFrame::new(
        1_234,
        0,
        crate::can::frame::Direction::Rx,
        [9, 8, 7, 6, 5, 4],
        [1, 2, 3, 4, 5, 6],
        Some(100),
        0x0800,
        vec![0xAA; 8],
    );
    app.ingest_eth(frame);
    app.refresh_snapshot();
    assert_eq!(app.snap.eth_trace.len(), 1);
    let f = app.snap.eth_trace.last().expect("frame");
    assert_eq!(f.vlan, Some(100));
    assert_eq!(f.ethertype, 0x0800);
    assert_eq!(f.len(), 8);
}

#[test]
fn ethernet_flows_aggregate_like_can_messages() {
    let mut app = App::headless();
    let frame = |t: u64| {
        crate::eth::EthFrame::new(
            t,
            0,
            crate::can::frame::Direction::Rx,
            [9, 8, 7, 6, 5, 4],
            [1, 2, 3, 4, 5, 6],
            Some(100),
            0x0800,
            vec![0xAA; 8],
        )
    };
    app.ingest_eth(frame(0));
    app.ingest_eth(frame(1_000));
    app.refresh_snapshot();
    assert_eq!(app.snap.eth_aggs.len(), 1);
    let agg = &app.snap.eth_aggs[0];
    assert_eq!(agg.count, 2);
    assert_eq!(agg.cycle_us, 1_000.0);
    assert_eq!(agg.key.len, 8);
}

#[test]
fn messages_and_statistics_list_ethernet_flows() {
    let mut app = App::headless();
    app.new_msg_window();
    app.new_stats_window();
    app.ingest_eth(crate::eth::EthFrame::new(
        1_000,
        0,
        crate::can::frame::Direction::Rx,
        [9, 8, 7, 6, 5, 4],
        [1, 2, 3, 4, 5, 6],
        Some(100),
        0x0800,
        vec![0xAA; 8],
    ));
    app.refresh_snapshot();
    app.text_fresh = true;
    app.sync_msg_text(0);
    app.sync_stats_text(0);
    assert!(
        app.msg_windows[0]
            .text_rows
            .iter()
            .any(|r| r.label.contains("ETH0")),
        "Messages lists the Ethernet flow"
    );
    assert!(
        app.stats_windows[0]
            .text_rows
            .iter()
            .any(|r| r.bus == "ETH0"),
        "Statistics lists the Ethernet flow"
    );
}

/// The three streams share one Trace list, interleaved by time, and the
/// scope/manual filters name Ethernet flows by their L2 identity. A frame
/// stamped between a CAN and a FlexRay arrival must land between them, or the
/// merged table is just three tables wearing one header.
#[test]
fn trace_merges_ethernet_rows_and_scopes_them_by_flow() {
    let mut app = App::headless();
    app.new_trace_window();
    app.trace_windows[0].manual.clear();
    app.trace_windows[0].scope = SigScope::All;
    {
        let mut f = crate::can::frame::CanFrame {
            t_us: 1_000,
            channel: 0,
            id: 0x100,
            extended: false,
            len: 2,
            data: [0; MAX_CAN_FD_LEN],
            dir: crate::can::frame::Direction::Rx,
            flags: FrameFlags::NONE,
        };
        f.data[0] = 0xAA;
        app.ingest(f, 1_000);
    }
    app.ingest_fr_row(crate::trace::FrRow {
        t_us: 1_500,
        bus: 0,
        slot: 13,
        cycle: 0,
        ab: 0,
        payload: vec![0x11; 8],
        header_crc: 0,
        flags: 0,
        name: None,
    });
    app.ingest_eth(crate::eth::EthFrame::new(
        2_000,
        0,
        crate::can::frame::Direction::Rx,
        [9, 8, 7, 6, 5, 4],
        [1, 2, 3, 4, 5, 6],
        Some(100),
        0x0800,
        vec![0x22; 8],
    ));
    app.refresh_snapshot();
    app.text_fresh = true;
    app.sync_trace_rows(0);

    let shape: Vec<(u64, u8)> = app.trace_windows[0]
        .rows
        .iter()
        .map(|r| match r {
            TraceRow::Can(f) => (f.t_us, 0),
            TraceRow::Fr(r, _) => (r.t_us, 1),
            TraceRow::Eth(f) => (f.t_us, 2),
        })
        .collect();
    assert_eq!(
        shape,
        [(2_000, 2), (1_500, 1), (1_000, 0)],
        "newest first, with Ethernet between the other two arrivals"
    );
    let eth_frame = match &app.trace_windows[0].rows[0] {
        TraceRow::Eth(f) => f.clone(),
        other => panic!("newest row should be Ethernet, got {other:?}"),
    };

    // One Ethernet bus scope keeps that bus's flow and drops the other
    // streams; the cache rebuilds because the lens changed.
    app.trace_windows[0].scope = SigScope::EthBus(0);
    app.sync_trace_rows(0);
    assert_eq!(app.trace_windows[0].rows.len(), 1);
    assert!(matches!(app.trace_windows[0].rows[0], TraceRow::Eth(_)));

    // A CAN bus scope is a different cable and drops Ethernet entirely.
    app.trace_windows[0].scope = SigScope::Bus(0);
    app.sync_trace_rows(0);
    assert_eq!(app.trace_windows[0].rows.len(), 1);
    assert!(matches!(app.trace_windows[0].rows[0], TraceRow::Can(_)));

    // Manual picks the captured flow, the same key Messages/Statistics use.
    let pick = crate::workspace::Pick::Eth(crate::aggregate::EthFlowKey::of_frame(&eth_frame));
    app.trace_windows[0].manual.insert(pick);
    app.trace_windows[0].scope = SigScope::Manual;
    app.sync_trace_rows(0);
    assert_eq!(app.trace_windows[0].rows.len(), 1);
    assert!(matches!(app.trace_windows[0].rows[0], TraceRow::Eth(_)));

    // Text finds the row through the Ethernet numbers the columns show,
    // which no CAN name or id would match.
    app.trace_windows[0].scope = SigScope::All;
    app.trace_windows[0].filter = "0800".to_string();
    app.sync_trace_rows(0);
    assert_eq!(app.trace_windows[0].rows.len(), 1);
    assert!(matches!(app.trace_windows[0].rows[0], TraceRow::Eth(_)));
}

/// The FRAME-LENGTH convention is a project switch, not a build constant:
/// flipping it must change which ARXML frame a captured flow names and rebuild
/// the Trace cache, or the Name column keeps a stale match.
#[test]
fn the_frame_length_convention_renames_trace_rows() {
    let mut app = App::headless();
    let db = crate::eth::db::EthDb {
        clusters: vec![crate::eth::db::EthCluster {
            name: "Cluster".into(),
            vlans: vec![100],
        }],
        frames: vec![crate::eth::db::EthFrameDef {
            name: "LenFrame".into(),
            cluster: "Cluster".into(),
            vlan: Some(100),
            // 8 payload bytes + the 14-byte L2 header.
            length: 22,
            ethertype_hint: Some(0x0800),
            pdus: vec![crate::eth::db::EthPdu {
                name: "P".into(),
                length: 8,
                start_bit: 0,
                signals: vec![],
            }],
        }],
        ..Default::default()
    };
    app.eth_buses.insert(0, std::sync::Arc::new(db));
    app.ingest_eth(crate::eth::EthFrame::new(
        1_000,
        0,
        crate::can::frame::Direction::Rx,
        [9, 8, 7, 6, 5, 4],
        [1, 2, 3, 4, 5, 6],
        Some(100),
        0x0800,
        vec![0xAA; 8],
    ));
    app.refresh_snapshot();
    app.text_fresh = true;
    app.sync_trace_rows(0);
    let name = |app: &App| match &app.trace_windows[0].rows[0] {
        TraceRow::Eth(f) => app.eth_row_name(f),
        other => panic!("expected an Ethernet row, got {other:?}"),
    };
    assert!(
        name(&app).contains(" > "),
        "the default convention states payload length, so the flow stays unnamed"
    );

    app.set_eth_length_includes_header(true);
    app.settle();
    assert!(app.eth_length_includes_header);
    app.sync_trace_rows(0);
    assert_eq!(
        name(&app),
        "LenFrame",
        "the cache rebuilt under the new convention and matched the frame"
    );
}

/// Trace export speaks PCAP when the chosen name ends in `.pcap`: Ethernet
/// frames keep their MACs, VLAN tag, EtherType and payload, and the same
/// Trace filter decides which records land in the file.
#[test]
fn export_trace_writes_pcap_for_ethernet() {
    let mut app = App::headless();
    app.ingest_eth(crate::eth::EthFrame::new(
        1_000,
        0,
        crate::can::frame::Direction::Rx,
        [9, 8, 7, 6, 5, 4],
        [1, 2, 3, 4, 5, 6],
        Some(100),
        0x0800,
        vec![0xDE, 0xAD, 0xBE, 0xEF],
    ));
    app.ingest_eth(crate::eth::EthFrame::new(
        2_000,
        1,
        crate::can::frame::Direction::Tx,
        [0xAA; 6],
        [0xBB; 6],
        None,
        0x88B5,
        vec![0x01, 0x02],
    ));
    app.refresh_snapshot();

    let path = std::env::temp_dir().join(format!("roxy_can_eth_{}.pcap", std::process::id()));
    app.export_trace(0, &path.to_string_lossy());
    let bytes = std::fs::read(&path).unwrap();
    assert_eq!(
        &bytes[0..4],
        &[0xd4, 0xc3, 0xb2, 0xa1],
        "little-endian magic"
    );
    assert_eq!(u16::from_le_bytes(bytes[4..6].try_into().unwrap()), 2);
    assert_eq!(u16::from_le_bytes(bytes[6..8].try_into().unwrap()), 4);
    assert_eq!(
        u32::from_le_bytes(bytes[20..24].try_into().unwrap()),
        1,
        "LINKTYPE_ETHERNET"
    );

    fn records(bytes: &[u8]) -> Vec<Vec<u8>> {
        let mut out = Vec::new();
        let mut off = 24usize;
        while off + 16 <= bytes.len() {
            let len = u32::from_le_bytes(bytes[off + 8..off + 12].try_into().unwrap()) as usize;
            let orig = u32::from_le_bytes(bytes[off + 12..off + 16].try_into().unwrap()) as usize;
            assert_eq!(len, orig, "nothing was truncated in the record header");
            off += 16;
            out.push(bytes[off..off + len].to_vec());
            off += len;
        }
        assert_eq!(off, bytes.len(), "no trailing bytes");
        out
    }

    let all = records(&bytes);
    assert_eq!(all.len(), 2, "both frames exported, oldest first");
    let tagged = &all[0];
    assert_eq!(&tagged[0..6], &[9, 8, 7, 6, 5, 4], "destination MAC");
    assert_eq!(&tagged[6..12], &[1, 2, 3, 4, 5, 6], "source MAC");
    assert_eq!(&tagged[12..14], &[0x81, 0x00], "802.1Q TPID");
    assert_eq!(&tagged[14..16], &[0x00, 0x64], "VLAN 100");
    assert_eq!(&tagged[16..18], &[0x08, 0x00], "EtherType");
    assert_eq!(&tagged[18..], &[0xDE, 0xAD, 0xBE, 0xEF]);
    let plain = &all[1];
    assert_eq!(plain.len(), 14 + 2, "untagged frame has no VLAN tag");
    assert_eq!(&plain[0..6], &[0xAA; 6]);
    assert_eq!(&plain[6..12], &[0xBB; 6]);
    assert_eq!(&plain[12..14], &[0x88, 0xB5]);

    // The Trace filter narrows the capture: only the 0x88B5 flow remains.
    app.trace_windows[0].filter = "88B5".to_string();
    let filtered =
        std::env::temp_dir().join(format!("roxy_can_eth_filtered_{}.pcap", std::process::id()));
    app.export_trace(0, &filtered.to_string_lossy());
    let filtered_bytes = std::fs::read(&filtered).unwrap();
    let only = records(&filtered_bytes);
    assert_eq!(only.len(), 1);
    assert_eq!(&only[0][12..14], &[0x88, 0xB5]);
    std::fs::remove_file(&path).ok();
    std::fs::remove_file(&filtered).ok();
}

/// Messages and Statistics CSV include Ethernet flows under the same scope
/// rules as their windows, with the EtherType in the id column and the VLAN
/// in the name.
#[test]
fn ethernet_flows_reach_messages_and_statistics_csv() {
    let mut app = App::headless();
    for t in [1_000u64, 2_000] {
        app.ingest_eth(crate::eth::EthFrame::new(
            t,
            0,
            crate::can::frame::Direction::Rx,
            [9, 8, 7, 6, 5, 4],
            [1, 2, 3, 4, 5, 6],
            Some(100),
            0x0800,
            vec![0xAA; 8],
        ));
    }
    app.refresh_snapshot();

    let msgs = std::env::temp_dir().join(format!("roxy_eth_msgs_{}.csv", std::process::id()));
    app.export_messages_csv(0, &msgs.to_string_lossy());
    let text = std::fs::read_to_string(&msgs).unwrap();
    let row = text
        .lines()
        .find(|l| l.starts_with("ETH0,0x0800,"))
        .unwrap_or_else(|| panic!("no Ethernet row in Messages CSV:\n{text}"));
    assert!(row.contains("VLAN 100"), "the VLAN rides the name: {row}");
    assert!(row.contains("01:02:03:04:05:06 > 09:08:07:06:05:04"));
    assert!(row.contains(",2,"), "two arrivals counted: {row}");
    std::fs::remove_file(&msgs).ok();

    let stats = std::env::temp_dir().join(format!("roxy_eth_stats_{}.csv", std::process::id()));
    app.export_stats_csv(0, &stats.to_string_lossy());
    let text = std::fs::read_to_string(&stats).unwrap();
    let row = text
        .lines()
        .find(|l| l.starts_with("ETH0,0x0800,"))
        .unwrap_or_else(|| panic!("no Ethernet row in Statistics CSV:\n{text}"));
    assert!(row.contains("VLAN 100"));
    assert!(row.contains(",2,"), "two arrivals counted: {row}");
    std::fs::remove_file(&stats).ok();

    // A CAN-bus scope is a different cable: Ethernet leaves both exports.
    app.msg_windows[0].scope = crate::app::SigScope::Bus(0);
    app.stats_windows[0].scope = crate::app::SigScope::Bus(0);
    let msgs =
        std::env::temp_dir().join(format!("roxy_eth_msgs_scoped_{}.csv", std::process::id()));
    app.export_messages_csv(0, &msgs.to_string_lossy());
    assert!(!std::fs::read_to_string(&msgs).unwrap().contains("ETH0"));
    std::fs::remove_file(&msgs).ok();
    let stats =
        std::env::temp_dir().join(format!("roxy_eth_stats_scoped_{}.csv", std::process::id()));
    app.export_stats_csv(0, &stats.to_string_lossy());
    assert!(!std::fs::read_to_string(&stats).unwrap().contains("ETH0"));
    std::fs::remove_file(&stats).ok();
}

/// A decoded Ethernet signal feeds the node input table under
/// `(bus, frame, signal)`, so `ETH0.Frame.Signal` reads the same value the
/// observers plot. The starting DB is pushed before measurement, exactly as
/// loading an ARXML does.
#[test]
fn an_ethernet_signal_drives_a_dotted_script_read() {
    use crate::eth::db::{EthCluster, EthDb, EthFrameDef, EthPdu, EthSignal};
    let mut app = quiet_app();
    let db = EthDb {
        clusters: vec![EthCluster {
            name: "Cluster".into(),
            vlans: vec![100],
        }],
        frames: vec![EthFrameDef {
            name: "SpeedFrame".into(),
            cluster: "Cluster".into(),
            vlan: Some(100),
            length: 8,
            ethertype_hint: Some(0x0800),
            pdus: vec![EthPdu {
                name: "SpeedPdu".into(),
                length: 8,
                start_bit: 0,
                signals: vec![EthSignal {
                    name: "Speed".into(),
                    start_bit: 0,
                    size: 8,
                    big_endian: false,
                    signed: false,
                    factor: 2.0,
                    offset: 1.0,
                    unit: "km/h".into(),
                    values: vec![],
                }],
            }],
        }],
        ..Default::default()
    };
    app.eth_buses.insert(0, std::sync::Arc::new(db));
    app.push_eth_db_to_core();
    app.send(crate::bus::BusCommand::AddNode {
        name: "eth-reader".to_string(),
        channel: 0,
        attached: None,
    });
    app.settle();
    let id = app.snap.nodes[0].id;
    app.send(crate::bus::BusCommand::SetNodeSource {
        id,
        source: "on message 0x100 { print(ETH0.SpeedFrame.Speed); }".to_string(),
    });
    app.settle();
    app.ingest_eth(crate::eth::EthFrame::new(
        1_000,
        0,
        Direction::Rx,
        [9, 8, 7, 6, 5, 4],
        [1, 2, 3, 4, 5, 6],
        Some(100),
        0x0800,
        vec![42, 0, 0, 0, 0, 0, 0, 0],
    ));
    receive(
        &mut app,
        2_000,
        vec![rx_frame(2_000, 0x100, 1, FrameFlags::NONE)],
    );
    let node = app.snap.nodes.iter().find(|n| n.id == id).expect("node");
    assert!(!node.errored, "{:?}", node.log);
    assert!(
        node.log.iter().any(|l| l == "85.0"),
        "factor and offset apply: {:?}",
        node.log
    );
    app.stop();
}

#[test]
fn replay_folders_scan_asc_and_blf_files() {
    let dir = std::env::temp_dir().join(format!("roxy-replay-sources-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let white = dir.join("white.asc");
    let blue = dir.join("blue.blf");
    let text = dir.join("notes.txt");
    std::fs::write(&white, b"").unwrap();
    std::fs::write(&blue, b"").unwrap();
    std::fs::write(&text, b"").unwrap();

    let mut app = App::headless();
    let count = app
        .add_replay_folder(dir.to_string_lossy().into_owned())
        .expect("folder source");
    assert_eq!(count, 2, "only ASC/BLF files are listed");
    let names: Vec<&str> = app.replay_scan.iter().map(|f| f.name.as_str()).collect();
    assert!(names.contains(&"white.asc"));
    assert!(names.contains(&"blue.blf"));
    assert_eq!(
        app.replay_folders.len(),
        1,
        "folder source persists in config"
    );

    app.remove_replay_source(&dir.to_string_lossy(), true);
    assert!(app.replay_folders.is_empty());
    assert!(
        app.replay_scan.is_empty(),
        "removing the source clears its files"
    );
    std::fs::remove_dir_all(&dir).ok();
}
