#[test]
fn csv_exports_match_window_state() {
    let mut app = App::headless();
    let db = app.channel_dbc(0).expect("sample DBC loaded");
    let id = db.order[0].0;
    let sig = db.messages[&(id, false)].signals[0].name.clone();
    let key = SigKey::can_in(crate::model::ClusterId(1), id, false, sig);
    app.subscribe(key.clone());
    app.graphics[0].signals.push(GfxSignal {
        key: key.clone(),
        visible: true,
        y_mode: YMode::Auto,
    });
    app.data_windows[0].signals.push(GfxSignal {
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
    for _ in 0..8 {
        std::thread::sleep(std::time::Duration::from_millis(11));
        app.update();
    }
    let dir = std::env::temp_dir();
    let stats = dir.join("roxy_stats_test.csv");
    app.export_stats_csv(0, &stats.to_string_lossy());
    let s = std::fs::read_to_string(&stats).unwrap();
    assert!(s.lines().count() > 1, "stats should have data rows");

    let msgs = dir.join("roxy_msgs_test.csv");
    app.export_messages_csv(0, &msgs.to_string_lossy());
    let m = std::fs::read_to_string(&msgs).unwrap();
    assert!(m.lines().count() > 1, "messages should have data rows");

    let gfx = dir.join("roxy_gfx_test.csv");
    app.export_graphics_csv(0, &gfx.to_string_lossy());
    let g = std::fs::read_to_string(&gfx).unwrap();
    assert!(g.contains(key.name()), "graphics history names the signal");

    let data = dir.join("roxy_data_test.csv");
    app.export_data_csv(0, &data.to_string_lossy());
    let d = std::fs::read_to_string(&data).unwrap();
    assert!(d.contains(key.name()), "data snapshot names the signal");

    for p in [&stats, &msgs, &gfx, &data] {
        std::fs::remove_file(p).ok();
    }
    app.stop();
}

#[test]
fn trace_filter_matches_by_name_id_and_direction() {
    let app = App::headless();
    let rx = CanFrame {
        t_us: 0,
        channel: 0,
        id: 0x100,
        extended: false,
        len: 8,
        data: [0; MAX_CAN_FD_LEN],
        dir: Direction::Rx,
        flags: FrameFlags::NONE,
    };
    let tx = CanFrame {
        id: 0x320,
        dir: Direction::Tx,
        ..rx
    };
    let rx_ch1 = CanFrame { channel: 1, ..rx };
    let unknown = CanFrame { id: 0x777, ..rx };
    let mut w = app.trace_windows[0].clone();
    assert!(app.trace_match(&w, &rx));
    assert!(app.trace_match(&w, &unknown));

    w.filter = "eng".to_string();
    assert!(app.trace_match(&w, &rx), "name match is case-insensitive");
    assert!(!app.trace_match(&w, &unknown));

    w.filter = "77".to_string();
    assert!(app.trace_match(&w, &unknown), "hex id substring");
    assert!(!app.trace_match(&w, &rx));

    w.filter.clear();
    w.dir = 1;
    assert!(app.trace_match(&w, &rx));
    assert!(!app.trace_match(&w, &tx), "Rx-only filter drops Tx frames");

    w.dir = 0;
    w.dbc_only = true;
    assert!(app.trace_match(&w, &rx));
    assert!(!app.trace_match(&w, &unknown), "DBC-only drops unknown IDs");

    w.dbc_only = false;
    w.scope = SigScope::Bus(0);
    assert!(app.trace_match(&w, &rx), "Bus scope passes its own bus");
    assert!(!app.trace_match(&w, &rx_ch1), "Bus scope drops other buses");

    w.scope = SigScope::Manual;
    w.manual
        .insert(crate::workspace::Pick::Can { ch: 0, id: 0x320 });
    assert!(
        !app.trace_match(&w, &rx),
        "Manual selection drops unselected IDs"
    );
    assert!(
        app.trace_match(&w, &tx),
        "Manual selection passes the chosen ID"
    );
    w.manual.clear();
    assert!(
        !app.trace_match(&w, &tx),
        "empty Manual selection passes nothing"
    );
    w.scope = SigScope::All;
    assert!(app.trace_match(&w, &rx), "All scope passes everything");
}

/// `slot:13` is the address the plain text cannot express: a `13` search also
/// matches slot 113, slot 130 and any frame name holding those digits. The two
/// numbering spaces stay apart -- an id names no slot and a slot names no CAN
/// frame -- because translating between them is exactly the confusion the
/// filters exist to avoid.
#[test]
fn the_trace_filter_takes_an_exact_slot_or_id() {
    let app = quiet_app();
    let row = |slot: u16| crate::trace::FrRow {
        bus: 0,
        t_us: 1_000,
        ab: 0,
        slot,
        cycle: 0,
        payload: vec![1],
        header_crc: 0,
        flags: 0,
        name: None,
    };
    let mk = |app: &App, filter: &str| {
        let mut w = app.trace_windows[0].clone();
        w.filter = filter.to_string();
        w.filter_lens()
    };
    let flt = mk(&app, "13");
    assert!(app.trace_fr_match(&flt, &row(13)));
    assert!(
        app.trace_fr_match(&flt, &row(113)),
        "the substring search is still a substring search"
    );
    let flt = mk(&app, "slot:13");
    assert!(app.trace_fr_match(&flt, &row(13)), "slot 13 stays");
    assert!(!app.trace_fr_match(&flt, &row(113)), "slot 113 leaves");
    assert!(!app.trace_fr_match(&flt, &row(130)), "and so does slot 130");
    let can = frame_at(1_000, 0x13, 8, Direction::Rx);
    assert!(
        !app.trace_match_lens(&flt, &can),
        "a slot is not a CAN id, so the CAN rows leave"
    );

    let flt = mk(&app, "id:13");
    assert!(app.trace_match_lens(&flt, &can));
    assert!(
        !app.trace_fr_match(&flt, &row(13)),
        "and an id is not a slot, so the FlexRay rows leave"
    );
    // The frame class is part of a CAN address here, as it is in the ID column.
    let mut ext = frame_at(2_000, 0x13, 8, Direction::Rx);
    ext.extended = true;
    assert!(
        !app.trace_match_lens(&flt, &ext),
        "id:13 means the standard frame"
    );
    let flt = mk(&app, "id:13x");
    assert!(
        app.trace_match_lens(&flt, &ext),
        "id:13x means the extended one"
    );
    assert!(!app.trace_match_lens(&flt, &can));
    // A signal really can be named `Slot`, and `Slot=3` must stay the value
    // condition it has always been -- which is why the exact form uses a colon.
    let flt = mk(&app, "Slot=3");
    assert_eq!(
        flt.exact, None,
        "an `=` text is a condition, not an address"
    );
    assert!(!flt.value_conds.is_empty());
    // A half-typed address falls back to the substring search it would have
    // been: the table goes empty and the box still shows exactly what was
    // typed. The alternative -- ignoring a filter that looks armed -- would
    // hide the reason instead of stating it.
    for typo in ["id:", "id:XYZ", "slot:", "slot:70000"] {
        let flt = mk(&app, typo);
        assert_eq!(flt.exact, None, "{typo} is not an address");
        assert_eq!(
            flt.query.as_deref(),
            Some(typo.to_ascii_uppercase().as_str()),
            "{typo} stays the text search it was"
        );
    }
}

/// The other half of the same defect: a closed 筛选 row keeps filtering with
/// nothing on screen to say so. The toggle now carries the count and names the
/// conditions on hover, so the report is built from what actually filters --
/// text that fails to parse hides nothing and must not be counted.
#[test]
fn a_closed_filter_row_names_the_conditions_still_working() {
    let mut app = App::headless();
    let w = &mut app.trace_windows[0];
    assert!(
        w.hidden_conds().is_empty(),
        "an untouched window hides nothing"
    );
    // Filled in but not filtering: unparseable payload text, an unparsable
    // second, a frame kind left on Any.
    w.payload = "zz".to_string();
    w.time_from = "abc".to_string();
    assert!(
        w.hidden_conds().is_empty(),
        "text that does not filter is not reported as filtering: {:?}",
        w.hidden_conds()
    );
    w.payload = "11 22".to_string();
    w.time_from = "1.0".to_string();
    w.flags_kind = 1;
    w.dbc_only = true;
    assert_eq!(
        w.hidden_conds(),
        ["payload", "frame kind", "DBC only", "time range"],
        "every condition that is working while the row is closed"
    );
    // The main row's own controls are always visible, so they never join the
    // list -- and neither does the FR expansion, which adds rows.
    w.filter = "Motor".to_string();
    w.dir = 1;
    w.fr_expand = true;
    assert_eq!(
        w.hidden_conds(),
        ["payload", "frame kind", "DBC only", "time range"],
        "visible controls and row-expansion are not conditions in disguise"
    );
}

/// `CarSpeed>60` is not a CAN-only question. A FlexRay row is filtered by the
/// value its own frame carries -- the same physical number the curve plots and
/// the expanded child row prints -- and a row nothing can decode leaves the
/// table rather than sitting in it unexplained.
#[test]
fn a_flexray_row_is_filtered_by_the_value_its_frame_carries() {
    let mut app = App::headless();
    let arxml = crate::test_assets::path("PowerTrain.arxml");
    assert!(
        app.load_cluster_description(arxml, Some(0)).is_some(),
        "the bundled description loads"
    );
    // The first signal the description really decodes, with the value an
    // all-zero payload reads as -- the number the row itself would print.
    let (slot, cycle, ab, name, phys, payload) = {
        let db = app.fr_db(0).expect("loaded above");
        let mut found = None;
        for ix in 0..db.frames.len() {
            let Some(f) = db.frame_index(ix) else {
                continue;
            };
            let bytes = vec![0u8; (f.length as usize).max(1)];
            if let Some(sig) = db.decode_signals(ix, &bytes).into_iter().next() {
                // The filter compares the value the *model* decodes, so a
                // signal whose model transform disagrees with the legacy
                // table would make this assertion read the wrong number.
                // Both decoders must agree before the row becomes the probe.
                let candidate = crate::trace::FrRow {
                    bus: 0,
                    t_us: 1_000,
                    ab: 2,
                    slot: f.triggering.slot_id as u16,
                    cycle: f.triggering.base_cycle as u8,
                    payload: bytes.clone(),
                    header_crc: 0,
                    flags: 0,
                    name: None,
                };
                let agrees = app
                    .model_fr_all_values(&candidate, ix)
                    .and_then(|values| values.into_iter().find(|value| value.name == sig.name))
                    .is_some_and(|value| (value.phys - sig.phys).abs() < 1e-9);
                if agrees {
                    found = Some((
                        f.triggering.slot_id as u16,
                        f.triggering.base_cycle as u8,
                        2u8,
                        sig.name,
                        sig.phys,
                        bytes,
                    ));
                    break;
                }
            }
        }
        found.expect("the bundled description decodes at least one signal")
    };
    let row = crate::trace::FrRow {
        bus: 0,
        t_us: 1_000,
        ab,
        slot,
        cycle,
        // The payload the value was read out of: a narrower one would drop the
        // signal for running out of bytes, which is a different rule.
        payload,
        header_crc: 0,
        flags: 0,
        name: None,
    };
    let mk = |app: &App, filter: &str| {
        let mut w = app.trace_windows[0].clone();
        w.filter = filter.to_string();
        w.filter_lens()
    };
    let probe = |app: &App, filter: &str| app.trace_fr_match(&mk(app, filter), &row);

    assert!(
        probe(&app, &format!("{name}>{}", phys - 1.0)),
        "the row's own value clears a lower bound"
    );
    assert!(
        !probe(&app, &format!("{name}>{}", phys)),
        "and does not clear itself: `>` is strict"
    );
    assert!(probe(&app, &format!("{name}>={}", phys)));
    assert!(
        probe(&app, &format!("{name}=={phys}")),
        "the compared number is the decoded one, printed or not"
    );
    assert!(
        !probe(&app, "NoSuchSignal>0"),
        "a signal this frame does not carry drops the row rather than passing it"
    );

    // A row on a bus with no description cannot be read at all, so a value
    // condition has no opinion to keep it for.
    let bare = crate::trace::FrRow {
        bus: 7,
        ..row.clone()
    };
    assert!(
        !app.trace_fr_match(&mk(&app, &format!("{name}>-1e18")), &bare),
        "undescribed: no decoding, no match"
    );
    // ...while the same row is still visible under a filter that asks nothing of
    // its values -- the condition, not the missing file, is what hides it.
    assert!(
        app.trace_fr_match(&mk(&app, ""), &bare),
        "no condition: it shows"
    );
}

#[test]
fn channels_can_be_added_removed_and_renamed() {
    let mut app = App::headless();
    assert_eq!(app.can_count(), 2);
    app.set_can_name_for_test(0, "Powertrain");
    app.refresh_snapshot();
    assert_eq!(app.channel_name(0), "Powertrain");

    app.add_channel();
    assert_eq!(app.can_count(), 3);
    assert_eq!(app.channel_name(2), "CAN3");

    let cluster = crate::model::ClusterId(app.snap.cluster_ids.can[1]);
    app.aggs.insert(
        (cluster, 0x100, false),
        MessageAgg {
            id: 0x100,
            channel: 1,
            extended: false,
            dir: Direction::Rx,
            count: 1,
            rx: 1,
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
    app.trace_windows[0]
        .manual
        .insert(crate::workspace::Pick::Can { ch: 2, id: 0x200 });
    // A hand-picked FlexRay slot in the same window: a cluster index is not a
    // channel index, so removing a CAN channel must leave it alone.
    app.trace_windows[0]
        .manual
        .insert(crate::workspace::Pick::Fr { bus: 2, slot: 71 });
    app.trace_windows[0].scope = SigScope::Bus(2);
    // A FlexRay cluster scope on another window: cluster indices are their own
    // numbering space, so removing a CAN channel must not renumber it -- the
    // shift below would otherwise point the window at a different cable.
    app.msg_windows[0].scope = SigScope::FrBus(1);
    let w = app.trace_windows[0].clone();

    app.remove_channel(0);
    assert_eq!(app.can_count(), 2);
    assert_eq!(app.channel_name(0), "CAN2", "remaining buses shift down");
    assert!(
        app.aggs.contains_key(&(
            crate::model::ClusterId(app.snap.cluster_ids.can[0]),
            0x100,
            false
        )),
        "agg remapped 1 -> 0"
    );
    assert!(
        app.trace_windows[0]
            .manual
            .contains(&crate::workspace::Pick::Can { ch: 1, id: 0x200 }),
        "filter remapped 2 -> 1"
    );
    assert!(
        app.trace_windows[0]
            .manual
            .contains(&crate::workspace::Pick::Fr { bus: 2, slot: 71 }),
        "the FlexRay pick is not a channel index and did not shift"
    );
    assert_eq!(w.scope, SigScope::Bus(2), "cloned window is untouched");
    assert_eq!(
        app.trace_windows[0].scope,
        SigScope::Bus(1),
        "Bus scope indices shift with the channels"
    );
    assert_eq!(
        app.msg_windows[0].scope,
        SigScope::FrBus(1),
        "a FlexRay cluster scope is not a CAN channel index and does not move"
    );

    while app.can_count() > 1 {
        app.remove_channel(0);
    }
    assert_eq!(app.can_count(), 1, "last bus cannot be removed");
}

#[test]
fn replay_position_tracks_playback() {
    let mut app = App::headless();
    let path = std::env::temp_dir().join("roxy_can_seek_test.asc");
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
    let file = app.recorder.last_record.clone();
    app.load_log(&file);
    app.replay();
    let (pos0, dur) = app.replay_position().expect("replay has a timeline");
    assert!(dur > 0.0, "timeline covers the whole log");
    assert!(pos0 < 0.01, "playback starts at the beginning");
    // The first poll only anchors the replay clock, so a second
    // cycle is needed before the position actually advances.
    std::thread::sleep(std::time::Duration::from_millis(15));
    app.update();
    std::thread::sleep(std::time::Duration::from_millis(15));
    app.update();
    let (pos1, _) = app.replay_position().unwrap();
    assert!(pos1 > pos0, "position advances while replaying");
    app.stop();
    std::fs::remove_file(&file).ok();
}

/// A log of `n` frames spaced `step_us` apart, so the timeline is exactly
/// `(n-1) * step_us` long and every frame sits on a round second.
fn write_timed_asc(name: &str, n: u32, step_us: u64) -> std::path::PathBuf {
    let path = std::env::temp_dir().join(name);
    {
        let mut w = AscWriter::new(&path.to_string_lossy()).unwrap();
        for i in 0..n {
            let mut f = CanFrame {
                t_us: u64::from(i) * step_us,
                channel: 0,
                id: 0x100,
                extended: false,
                len: 2,
                data: [0; MAX_CAN_FD_LEN],
                dir: Direction::Rx,
                flags: FrameFlags::NONE,
            };
            f.data[0] = 0xAA;
            f.data[1] = 0xBB;
            w.write(&f).unwrap();
        }
        w.finish().unwrap();
    }
    path
}

#[test]
fn play_after_a_finished_run_restarts_from_the_top() {
    // Regression: pressing Play once a replay had run out resumed an
    // exhausted source -- a silent no-op that left the readout stuck at the
    // tail. Play must instead re-open the log and run from zero.
    let mut app = App::headless();
    let path = write_timed_asc("roxy_can_replay_restart.asc", 100, 10_000);
    app.load_log(&path.to_string_lossy());
    app.send(crate::bus::BusCommand::AddNode {
        name: "replay check".to_string(),
        channel: 0,
        attached: None,
    });
    app.settle();
    let node_id = app.snap.nodes[0].id;
    app.send(crate::bus::BusCommand::SetNodeSource {
        id: node_id,
        source: "on stop { test(\"replay done\", true); }".to_string(),
    });
    app.settle();
    // Run the log out the far end without touching Stop: the infinite speed
    // drains it in two laps.
    app.set_replay_speed(f64::INFINITY);
    app.replay();
    app.update();
    app.update();
    assert!(!app.measuring, "the replay finished on its own");
    assert_eq!(
        app.snap.nodes[0]
            .tests
            .iter()
            .find(|test| test.name == "replay done")
            .map(|test| (test.runs, test.failures)),
        Some((1, 0)),
        "the end of a replay runs on stop"
    );
    let (pos_end, dur) = app.replay_position().unwrap();
    assert!(
        pos_end >= dur - 1e-6,
        "playhead parked at the end, got {pos_end} of {dur}"
    );

    app.toggle_play(); // Play, with no Stop in between
    assert!(
        app.measuring,
        "Play restarts a finished replay, not a no-op"
    );
    let (pos_top, _) = app.replay_position().unwrap();
    assert!(
        pos_top < pos_end,
        "the playhead returned to the top, got {pos_top}"
    );
    app.stop();
    std::fs::remove_file(&path).ok();
}

#[test]
fn stop_makes_the_next_play_restart_from_zero() {
    let mut app = App::headless();
    let path = write_timed_asc("roxy_can_replay_stop.asc", 100, 10_000);
    app.load_log(&path.to_string_lossy());
    // Park the playhead deep in the log, then Stop explicitly -- the gesture
    // that used to leave a resume-in-place option open.
    app.set_replay_speed(f64::INFINITY);
    app.replay();
    app.update();
    app.update();
    app.stop();
    app.toggle_play();
    let (pos, _) = app.replay_position().unwrap();
    assert!(
        pos < 0.01,
        "Stop is an explicit request to re-open from the beginning, got {pos}"
    );
    app.stop();
    std::fs::remove_file(&path).ok();
}

#[test]
fn the_plot_clock_follows_the_replay_playhead() {
    let mut app = App::headless();
    let path = write_timed_asc("roxy_can_plot_clock.asc", 100, 10_000);
    app.load_log(&path.to_string_lossy());
    app.replay();
    app.update();
    let advanced = app.plot_now_s();
    assert!(
        (advanced - app.replay_position().unwrap().0).abs() < 1e-9,
        "the Graphics axis must ride the playhead, got {advanced}"
    );
    // A faster clock moves the axis with it rather than lagging on wall time.
    app.set_replay_speed(f64::INFINITY);
    app.update();
    let (pos, dur) = app.replay_position().unwrap();
    assert!(
        (app.plot_now_s() - dur).abs() < 1e-9,
        "the axis follows the drained playhead, got {} of {dur}",
        app.plot_now_s()
    );
    assert!(pos >= dur - 1e-6, "setup: the log ran to its end");
    app.stop();
    std::fs::remove_file(&path).ok();
}

#[test]
fn loading_another_log_mid_replay_is_refused() {
    let mut app = App::headless();
    let a = write_timed_asc("roxy_can_guard_a.asc", 50, 10_000);
    let b = write_timed_asc("roxy_can_guard_b.asc", 50, 10_000);
    app.load_log(&a.to_string_lossy());
    app.replay();
    assert!(app.measuring);

    app.load_log(&b.to_string_lossy());
    assert_eq!(
        app.log_path,
        a.to_string_lossy(),
        "the running log must stay selected"
    );
    assert!(
        app.status.contains("stop the replay"),
        "the refusal should say why, got {:?}",
        app.status
    );
    assert!(
        !app.recent_log
            .iter()
            .any(|p| p == &b.to_string_lossy().to_string()),
        "a refused load must not enter the recent list"
    );

    // Stopped, the same selection goes through.
    app.stop();
    app.load_log(&b.to_string_lossy());
    assert_eq!(app.log_path, b.to_string_lossy());
    app.stop();
    std::fs::remove_file(&a).ok();
    std::fs::remove_file(&b).ok();
}

#[test]
fn play_after_choosing_a_new_log_opens_that_log() {
    let mut app = App::headless();
    let a = write_timed_asc("roxy_can_switch_a.asc", 100, 10_000);
    let b = write_timed_asc("roxy_can_switch_b.asc", 20, 10_000);
    app.load_log(&a.to_string_lossy());
    app.replay();
    app.set_replay_speed(8.0);
    // Let the first log run to its natural end, which leaves the run settled
    // but still replay-able -- exactly where the old code could resume the
    // wrong file.
    let (_, dur_a) = app.replay_position().unwrap();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while app.measuring && std::time::Instant::now() < deadline {
        std::thread::sleep(std::time::Duration::from_millis(11));
        app.update();
    }
    assert!(
        !app.measuring,
        "setup: the first log finished on its own, {} s of it unread",
        dur_a
    );

    app.load_log(&b.to_string_lossy());
    app.toggle_play();
    let (_, dur_b) = app.replay_position().expect("the new log has a timeline");
    assert!(
        dur_b < dur_a,
        "Play must open the newly selected log ({dur_b}s) rather than resume \
             the finished one ({dur_a}s)"
    );
    app.stop();
    std::fs::remove_file(&a).ok();
    std::fs::remove_file(&b).ok();
}

fn blank_sub() -> Subscription {
    Subscription {
        latest: 0.0,
        last_raw: 0,
        unit: String::new(),
        label: None,
        type_tag: String::new(),
        min: f64::INFINITY,
        max: f64::NEG_INFINITY,
        avg: 0.0,
        sum: 0.0,
        n: 0,
        last_update_us: 0,
        last_sample_us: 0,
        history: SampleCache::default(),
        published: std::sync::Arc::new(SampleCache::default()),
        history_dirty: false,
    }
}

#[test]
fn the_head_of_the_curve_survives_past_the_old_point_cap() {
    // 250 s of samples at the real 50 ms interval: beyond the 4000-point
    // cap this replaces, which began popping the head at exactly 200 s and
    // made the left end of a running trace vanish on its own.
    let mut sub = blank_sub();
    for i in 0..5_000u64 {
        sub.push_sample(i * SAMPLE_INTERVAL_US, (i % 97) as f64, SAMPLE_INTERVAL_US);
    }
    assert_eq!(sub.history.len(), 5_000, "250 s fits inside the span");
    assert_eq!(
        sub.history.first().unwrap().0,
        0,
        "the head must still be there mid-run"
    );
}

#[test]
fn eviction_begins_only_past_the_retention_span() {
    let mut sub = blank_sub();
    let n = HISTORY_SPAN_US / SAMPLE_INTERVAL_US + 10;
    for i in 0..n {
        sub.push_sample(i * SAMPLE_INTERVAL_US, 1.0, SAMPLE_INTERVAL_US);
    }
    assert!(sub.history.len() < n as usize, "stale head is dropped");
    let kept = sub.history.len() as u64 * SAMPLE_INTERVAL_US;
    assert!(
        kept <= HISTORY_SPAN_US + SAMPLE_INTERVAL_US,
        "retained span {kept} us exceeds the cap"
    );
    assert!(
        sub.n > sub.history.len() as u64,
        "min/max/avg stay cumulative over the whole run"
    );
}

/// Records `iters` frames of generator traffic to an ASC, then returns an
/// App with the first sample.dbc signal subscribed and that log loaded but
/// not yet playing. The traffic has to be DBC-decodable for the Graphics
/// history to fill, so a hand-written fixture will not do.
fn app_with_replayable_recording(
    name: &str,
    iters: usize,
) -> (App, crate::observe::SigKey, String) {
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
    let out = std::env::temp_dir().join(format!("roxy_can_{name}.asc"));
    app.record_path_buf = out.to_string_lossy().to_string();
    app.toggle_record();
    for tx in &mut app.tx_list {
        tx.active = true;
        tx.cycle_us = 10_000;
    }
    open_all_gates(&mut app);
    app.start_virtual();
    for _ in 0..iters {
        std::thread::sleep(std::time::Duration::from_millis(11));
        app.update();
    }
    app.stop();
    std::fs::remove_file(&out).ok();
    let file = app.recorder.last_record.clone();
    app.load_log(&file);
    (app, key, file)
}

#[test]
fn the_published_cache_view_stays_still_while_the_run_goes_on() {
    let mut c = SampleCache::default();
    c.merge(
        &(0..4u64)
            .map(|i| (i * 10_000, i as f64))
            .collect::<Vec<_>>(),
        10_000,
    );
    let published = c.clone(); // what a snapshot hands the UI

    // More samples arrive and the retention span trims the oldest away;
    // neither may disturb the view the UI is still reading.
    c.merge(
        &(10..12u64)
            .map(|i| (i * 10_000, i as f64))
            .collect::<Vec<_>>(),
        10_000,
    );
    c.trim_oldest(50_000);

    assert_eq!(
        published.len(),
        4,
        "the published view is frozen at its publish instant"
    );
    assert_eq!(published.first().unwrap().0, 0);
    assert_eq!(
        c.first().unwrap().0,
        100_000,
        "the working cache trims independently (50 000 span keeps two points)"
    );
    assert_eq!(c.len(), 2);
}

#[test]
fn merging_across_sealed_chunks_keeps_order_and_gap_rules() {
    let mut c = SampleCache::default();
    let n = crate::observe::SEAL_POINTS * 3 + 5;
    c.merge(
        &(0..n as u64)
            .map(|i| (i * 1_000, i as f64))
            .collect::<Vec<_>>(),
        1_000,
    );
    assert_eq!(c.len(), n);

    // Both candidates sit inside the gap of their neighbours: rejected,
    // and the ascending order across three sealed chunks survives.
    let back: Vec<(u64, f64)> = [3_500u64, 7_500].iter().map(|&t| (t, t as f64)).collect();
    let taken = c.merge(&back, 1_000);
    assert!(taken.is_empty(), "in-gap candidates are rejected");
    assert!(
        c.iter().zip(c.iter().skip(1)).all(|(a, b)| a.0 < b.0),
        "a merge spanning sealed chunks keeps the order"
    );
    assert_eq!(c.at(3_599), Some(3.0), "3_500 was rejected; 3_000 answers");

    // Trimming to a 500 ms span drops the first two sealed chunks whole
    // and trims the third at the horizon: 1_540_000 - 500_000.
    c.trim_oldest(500_000);
    assert_eq!(
        c.first().unwrap().0,
        1_040_000,
        "stale head chunks are dropped wholesale, the head chunk is trimmed in place"
    );
    assert_eq!(c.at(1_100_000), Some(1_100.0));
}

#[test]
fn sample_cache_range_and_lookup_are_inclusive_and_ordered() {
    let mut c = SampleCache::default();
    c.merge(
        &(0..10u64)
            .map(|i| (i * 1_000, i as f64))
            .collect::<Vec<_>>(),
        1_000,
    );
    let win = c.range(2_000, 5_000);
    assert_eq!(
        win.map(|(t, _)| *t).collect::<Vec<_>>(),
        vec![2_000, 3_000, 4_000, 5_000],
        "both ends of the window are included"
    );
    assert_eq!(c.at(4_500), Some(4.0), "last value at or before");
    assert_eq!(
        c.at(0),
        Some(0.0),
        "the first sample resolves on its own edge"
    );
    assert_eq!(c.at(999), Some(0.0), "step-signal semantics hold");
    assert_eq!(
        SampleCache::default().at(999),
        None,
        "an empty cache has no value to report"
    );
}

#[test]
fn sample_cache_trims_by_span_not_by_count() {
    let mut c = SampleCache::default();
    c.merge(
        &(0..30u64).map(|i| (i * 10_000, 1.0)).collect::<Vec<_>>(),
        10_000,
    );
    c.trim_oldest(100_000);
    assert_eq!(
        c.first().unwrap().0,
        190_000,
        "newest is 290 s, so everything from 190 s on survives"
    );
    assert_eq!(c.len(), 11);
}

#[test]
fn a_second_replay_run_samples_from_the_top() {
    let (mut app, key, file) = app_with_replayable_recording("resample", 60);

    // First run: play the log out so the subscription ends up with a
    // sampling baseline near the end of it.
    app.replay();
    app.set_replay_speed(4.0);
    let deadline = std::time::Instant::now() + std::time::Duration::from_millis(250);
    while std::time::Instant::now() < deadline {
        std::thread::sleep(std::time::Duration::from_millis(11));
        app.update();
    }
    let stale_baseline = app.subs.get(&key).unwrap().last_sample_us;
    assert!(
        stale_baseline > 200_000,
        "setup: the first run should have sampled deep into the log, got {stale_baseline} us"
    );

    // Second run. Replaying used to inherit that baseline, and the sampler
    // gate then rejected every frame until the playhead climbed past it --
    // visibly, the start of the curve was simply missing.
    app.replay();
    {
        let sub = app.subs.get(&key).unwrap();
        assert!(sub.history.is_empty(), "a fresh run drops the old trace");
        assert_eq!(
            sub.last_sample_us, 0,
            "a fresh run must not inherit the sampling baseline"
        );
        assert_eq!(sub.n, 0, "a fresh run resets the sample count");
        assert!(
            !sub.min.is_finite() && sub.max == f64::NEG_INFINITY,
            "a fresh run resets min/max instead of keeping the old extremes"
        );
    }
    // The first poll only anchors the replay clock; the second is what
    // moves the playhead past the log's opening frames.
    std::thread::sleep(std::time::Duration::from_millis(20));
    app.update();
    std::thread::sleep(std::time::Duration::from_millis(20));
    app.update();
    let sub = app.subs.get(&key).unwrap();
    assert!(
        !sub.history.is_empty(),
        "sampling must start at the top of the log, not at {stale_baseline} us"
    );
    assert!(
        sub.history.first().unwrap().0 < stale_baseline,
        "the first sample of the new run should precede the previous run's last"
    );
    app.stop();
    std::fs::remove_file(&file).ok();
}

#[test]
fn replay_injection_lands_on_the_log_timeline() {
    let mut app = App::headless();
    let file = write_timed_asc("roxy_can_inject.asc", 3, 100_000);
    app.load_log(&file.to_string_lossy());
    // An id the log does NOT carry: a log-carried id stands down during
    // replay (see the muting test below), and what this test pins is the
    // injection's timing.
    app.tx_list[0].id = 0x123;
    let tx_id = app.tx_list[0].id;
    let tx_ch = app.tx_list[0].channel;
    app.tx_list[0].active = true;
    app.tx_list[0].cycle_us = 40_000;
    app.tx_list[0].data = [0xDE; MAX_CAN_FD_LEN];
    // 角色开关允许发送：条目 node 戳仍是 EngineECU。
    app.set_node_role(tx_ch, "EngineECU", NodeRole::Simulated);
    app.replay();
    // Drive the replay by hand: 50 ms wall steps release the log frames at
    // 0 / 100 / 200 ms. Injection at 40 ms into a quiet stretch must wait
    // for the log's next frame rather than ride the wall clock.
    for step in 1..=7u64 {
        app.tick(step * 50_000);
    }
    let injected: Vec<u64> = app
        .trace
        .iter()
        .filter(|f| f.id == tx_id && matches!(f.dir, Direction::Tx))
        .map(|f| f.t_us)
        .collect();
    assert!(!injected.is_empty(), "the generator injects during replay");
    assert!(
        injected.iter().all(|&t| t <= 200_000),
        "injections ride the log clock, not the wall clock: {injected:?}"
    );
    assert!(
        injected.iter().all(|t| t % 40_000 == 0),
        "injections sit on their declared cycle within the log timeline: {injected:?}"
    );
    let agg = app
        .aggs
        .get(&(
            crate::model::ClusterId(app.snap.cluster_ids.can[tx_ch as usize]),
            tx_id,
            false,
        ))
        .expect("injected frames aggregate");
    assert_eq!(agg.count, injected.len() as u64);
    assert!(
        app.trace
            .iter()
            .filter(|f| f.id == 0x100 && matches!(f.dir, Direction::Rx))
            .count()
            == 3,
        "the log frames themselves survive alongside the injections"
    );
    app.stop();
    std::fs::remove_file(&file).ok();
}

#[test]
fn replay_speed_steps_along_the_ladder() {
    let mut app = App::headless();
    assert_eq!(app.replay_speed, 1.0);
    app.step_replay_speed(1);
    assert_eq!(app.replay_speed, 2.0, "one notch faster");
    app.step_replay_speed(-1);
    app.step_replay_speed(-1);
    assert_eq!(app.replay_speed, 0.5, "two notches slower");
    app.step_replay_speed(-1);
    assert_eq!(app.replay_speed, 0.5, "clamped at the slow end");
    app.step_replay_speed(99);
    assert!(
        app.replay_speed.is_infinite(),
        "the fast end is as-fast-as-possible"
    );
    app.step_replay_speed(-1);
    assert_eq!(app.replay_speed, 4.0, "one notch back from infinity");
}

#[test]
fn starting_clears_the_previous_pause() {
    let mut app = App::headless();
    app.start_virtual();
    app.trace_paused = true;
    app.stop();
    app.start_virtual();
    assert!(!app.trace_paused, "a new start must not stay paused");
    app.update();
    app.stop();
}

#[test]
fn switching_run_mode_stops_a_running_measurement() {
    let mut app = App::headless();
    app.start_virtual();
    assert!(app.measuring);
    app.switch_run_mode(Mode::Replay);
    assert!(!app.measuring, "switching mode stops the run");
    assert!(matches!(app.run_mode, Mode::Replay));
    app.switch_run_mode(Mode::Replay);
    assert!(matches!(app.run_mode, Mode::Replay), "no-op keeps the mode");
}

#[test]
fn recent_lists_dedup_and_cap() {
    let mut app = App::headless();
    for i in 0..10 {
        app.push_recent_dbc(format!("f{i}.dbc"));
    }
    assert_eq!(app.recent_dbc.len(), 8, "recent list is capped");
    assert_eq!(app.recent_dbc[0], "f9.dbc", "newest first");
    app.push_recent_dbc("f3.dbc".to_string());
    assert_eq!(app.recent_dbc[0], "f3.dbc", "reopen moves to the front");
    assert_eq!(
        app.recent_dbc.iter().filter(|p| *p == "f3.dbc").count(),
        1,
        "no duplicates"
    );
}

#[test]
fn dropping_a_dbc_loads_it_into_the_first_bus() {
    let mut app = App::headless();
    let motbus = crate::test_assets::path("motbus.dbc");
    // The default bus already carries the built-in sample database; a drop
    // **attaches** the new database as an extra instead of replacing it.
    app.open_dropped(std::path::Path::new(motbus));
    assert_eq!(
        app.can_paths(0),
        ["builtin:sample.dbc".to_string(), motbus.to_string()]
    );
    assert!(
        app.channel_dbc(0).is_some(),
        "dropped DBC is parsed into the first bus"
    );
    assert_eq!(app.recent_dbc[0], motbus);

    // Dropping the same file again never duplicates the attach.
    app.open_dropped(std::path::Path::new(motbus));
    assert_eq!(
        app.can_paths(0).len(),
        2,
        "an already-attached path is not re-added"
    );
}

#[test]
fn jump_to_live_resets_plot_offsets() {
    let mut app = App::headless();
    app.graphics[0].t_offset_s = -42.0;
    app.jump_to_live();
    assert_eq!(app.graphics[0].t_offset_s, 0.0);
}

#[test]
fn the_blank_shell_starts_without_buses() {
    let app = App::blank();
    assert_eq!(app.snap.channel_count, 0);
    assert!(app.snap.channels.is_empty());
    assert!(!app.snap.measuring, "the empty shell does not auto-start");
    assert!(app.project_path.is_none());
}

#[test]
fn reset_restores_the_default_workspace() {
    let mut app = App::headless();
    app.new_trace_window();
    app.push_recent_dbc("keep.dbc".to_string());
    app.start_virtual();
    app.reset_to_defaults();
    assert!(!app.measuring, "reset stops a running measurement");
    assert_eq!(app.trace_windows.len(), 1, "default has one trace window");
    assert!(app.project_path.is_none());
    assert_eq!(app.recent_dbc[0], "keep.dbc", "recents survive the reset");
}

#[test]
fn new_project_starts_completely_empty() {
    let mut app = App::headless();
    app.new_project();
    assert!(
        (0..app.can_count())
            .all(|ch| { app.channel_dbc(ch as u8).is_none() && app.can_paths(ch).is_empty() }),
        "no DBCs on any bus"
    );
    assert!(app.trace_windows.is_empty());
    assert!(app.msg_windows.is_empty());
    assert!(app.stats_windows.is_empty());
    assert!(app.graphics.is_empty());
    assert!(app.data_windows.is_empty());
    assert!(app.tx_list.is_empty());
    assert!(app.project_path.is_none());
    assert!(!app.is_dirty(), "a fresh project has nothing to save");
}

#[test]
fn an_empty_project_mints_can1() {
    let mut app = App::headless();
    ProjectV2::default().apply(&mut app);
    assert_eq!(app.can_count(), 0, "an empty project has no buses");
    app.add_empty_channel();
    app.settle();
    assert_eq!(app.channel_name(0), "CAN1");
}

#[test]
fn a_saved_project_keeps_attached_hardware() {
    let mut app = App::headless();
    app.hw.attach_mock(0);
    app.refresh_snapshot();
    let project = ProjectV2::from_app(&app, None);
    let hw = project
        .hardware
        .iter()
        .find(|hw| hw.cluster_id == app.snap.cluster_ids.can[0])
        .expect("hardware mapping");
    assert_eq!(hw.driver, "Kvaser");
    assert_eq!(hw.channel, -1);
}

#[test]
fn a_contended_mailbox_keeps_last_frames_snapshot() {
    let mut app = App::headless();
    assert_eq!(app.channel_name(0), "CAN1");
    app.set_can_name_for_test(0, "Renamed");
    app.refresh_snapshot();
    assert_eq!(app.channel_name(0), "Renamed");

    // A newer snapshot is published while the reader is mid-read: the
    // try-read bows out (None) instead of waiting on the bus.
    app.set_can_name_for_test(0, "Renamed again");
    let newer = std::sync::Arc::new(app.snapshot());
    let mut guard = app.mail.lock().unwrap();
    *guard = newer;
    assert!(
        App::peek_mailbox(&app.mail, &app.snap).is_none(),
        "a held mailbox reads as no-new-snapshot, never as a wait"
    );
    drop(guard);

    // Released, the same read picks the fresh frame up and the regular
    // path lands it in `app.snap`.
    let fresh = App::peek_mailbox(&app.mail, &app.snap)
        .expect("the release makes the newest frame visible");
    assert_eq!(fresh.channels[0].name, "Renamed again");
    app.read_snapshot();
    assert_eq!(app.channel_name(0), "Renamed again");
}

#[test]
fn a_command_status_rides_the_next_snapshot() {
    let mut app = App::headless();
    app.send(crate::bus::BusCommand::StartReplay {
        path: "no/such/log.asc".to_string(),
        speed: 1.0,
    });
    assert!(
        app.status.contains("replay failed"),
        "the drained command's text reached the status line: {:?}",
        app.status
    );
    assert!(
        app.snap.status.is_some(),
        "and it travelled inside the snapshot that carried it"
    );

    // Status is news, not state: a publish without fresh text carries
    // None and the status line keeps the old message.
    app.send(crate::bus::BusCommand::SetEntryActive {
        ch: 9,
        id: 0x999,
        on: true,
    });
    assert!(app.snap.status.is_none(), "routine publishes carry no text");
    assert!(
        app.status.contains("replay failed"),
        "an empty-handed command leaves the old status standing"
    );
}

#[test]
fn project_round_trips_through_an_rxproj_file() {
    let mut app = App::headless();
    app.trace_windows[0].filter = "Motor".to_string();
    let path = std::env::temp_dir().join("roxy_can_test.rxproj");
    assert!(app.save_project(Some(path.clone())), "save writes the file");
    assert_eq!(app.project_path.as_deref(), Some(path.as_path()));

    let mut restored = App::headless();
    restored.open_project_path(&path);
    assert_eq!(restored.project_path.as_deref(), Some(path.as_path()));
    assert_eq!(restored.trace_windows[0].filter, "Motor");
    assert_eq!(restored.can_count(), app.can_count());
    assert_eq!(restored.tx_list.len(), app.tx_list.len());
    std::fs::remove_file(&path).ok();
}

#[test]
fn the_web_workspace_round_trips_without_dirtying_the_project() {
    let mut app = App::headless();
    app.mark_clean();
    let workspace = serde_json::json!({
        "tab": "statistics",
        "chartTab": "signals",
        "signalView": { "windowS": 45.0, "endS": null },
        "cursors": [{ "id": 3, "t": 12.5 }],
        "traceColumns": { "widths": { "name": 150 }, "hidden": ["bus"] },
        "layout": { "main": 0.5, "right": 0.6 }
    });
    app.web_workspace = Some(workspace.clone());
    assert!(
        !app.is_dirty(),
        "moving panels or cursors is not a semantic workspace change"
    );

    let path =
        std::env::temp_dir().join(format!("roxy_web_workspace_{}.rxproj", std::process::id()));
    assert!(app.save_project(Some(path.clone())), "save writes the file");
    let rev = app.workspace_rev;

    let mut restored = App::headless();
    restored.open_project_path(&path);
    assert_eq!(restored.web_workspace, Some(workspace));
    assert!(restored.workspace_rev > rev, "loading bumps the revision");
    assert!(!restored.is_dirty(), "the loaded file is clean");

    restored.new_project();
    assert_eq!(restored.web_workspace, None);
    assert!(
        restored.workspace_rev > rev + 1,
        "a new project also resets the revision"
    );
    std::fs::remove_file(&path).ok();
}

#[test]
fn signal_stats_track_min_avg_max() {
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
    for _ in 0..8 {
        std::thread::sleep(std::time::Duration::from_millis(11));
        app.update();
    }
    app.stop();
    let sub = app.subs.get(&key).expect("signal subscribed");
    assert!(
        sub.min.is_finite() && sub.max.is_finite(),
        "samples update min/max"
    );
    assert!(sub.min <= sub.avg && sub.avg <= sub.max, "avg within range");
    assert!(!sub.history.is_empty(), "history sampled");
}

#[test]
fn restored_signals_are_resubscribed() {
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
    app.graphics[0].signals.push(GfxSignal {
        key: key.clone(),
        visible: true,
        y_mode: YMode::Auto,
    });
    let path = std::env::temp_dir().join("roxy_can_resub.rxproj");
    assert!(app.save_project(Some(path.clone())));
    let mut restored = App::headless();
    restored.open_project_path(&path);
    assert!(
        restored.subs.contains_key(&key),
        "restored signal is resubscribed so it is not grey"
    );
    std::fs::remove_file(&path).ok();
}

#[test]
fn recording_captures_generator_data_faithfully() {
    let mut app = App::headless();
    app.tx_list[0].active = true;
    app.tx_list[0].cycle_us = 10_000;
    // 角色开关允许发送：0x100 属 EngineECU。
    app.set_node_role(0, "EngineECU", NodeRole::Simulated);
    let mut payload = [0u8; MAX_CAN_FD_LEN];
    payload[..8].copy_from_slice(&[0x11, 0x22, 0x33, 0x44, 0x55, 0x66, 0x77, 0x88]);
    app.tx_list[0].data = payload;
    app.record_path_buf = "target/test_record".to_string();
    app.toggle_record();
    app.start_virtual();
    for _ in 0..8 {
        std::thread::sleep(std::time::Duration::from_millis(11));
        app.update();
    }
    app.stop();
    let path = app.recorder.last_record.clone();
    assert!(!path.is_empty(), "recording produced a file");
    let content = std::fs::read_to_string(&path).expect("record file readable");
    let parsed = crate::log::asc::parse_asc(&content);
    let (id, ch) = (app.tx_list[0].id, app.tx_list[0].channel);
    let hit = parsed
        .iter()
        .find(|f| f.id == id && f.channel == ch)
        .expect("recorded frames parsed back");
    assert_eq!(
        hit.payload(),
        &[0x11u8, 0x22, 0x33, 0x44, 0x55, 0x66, 0x77, 0x88][..],
        "recorded data matches what the generator sent"
    );
    std::fs::remove_file(&path).ok();
}

use crate::spec::Kind;

/// A database covering the three declarations the monitor distinguishes:
/// 100 on a declared 100 ms period, 300 declared event-triggered, and 200
/// with no cycle at all. There is deliberately no `BA_DEF_DEF_` line, so an
/// unannotated message gets no declaration rather than a default one.
const SPEC_DBC: &str = r#"VERSION "roxy-can spec test"

NS_ :

BU_: ECU

BO_ 100 Periodic: 8 ECU
 SG_ S : 0|8@1+ (1,0) [0|0] "" ECU

BO_ 200 Undeclared: 8 ECU
 SG_ S : 0|8@1+ (1,0) [0|0] "" ECU

BO_ 300 EventMsg: 8 ECU
 SG_ S : 0|8@1+ (1,0) [0|0] "" ECU

BA_DEF_ BO_  "GenMsgCycleTime" INT 0 10000;
BA_ "GenMsgCycleTime" BO_ 100 100;
BA_ "GenMsgCycleTime" BO_ 300 0;
"#;

/// A virtual bus with a silent generator, so everything the monitor sees
/// arrived through `receive`.
fn spec_app() -> App {
    let mut app = App::headless();
    app.set_can_table_for_test(
        0,
        Some(std::sync::Arc::new(
            crate::dbc::load_dbc_str(SPEC_DBC).unwrap(),
        )),
    );
    app.tx_list.retain(|t| t.channel != 0);
    app.start_virtual();
    app
}

fn frame_at(t_us: u64, id: u32, len: u8, dir: Direction) -> CanFrame {
    CanFrame {
        t_us,
        channel: 0,
        id,
        extended: false,
        len,
        data: [0; MAX_CAN_FD_LEN],
        dir,
        flags: FrameFlags::NONE,
    }
}

/// Runs exactly one measurement step in which `frames` arrive and the
/// simulation clock reads `t_us`. A scripted source is the only way to get
/// received traffic through the real aggregation path without writing a log
/// file, and it keeps the step exact: no wall clock, no sleeping.
fn receive(app: &mut App, t_us: u64, frames: Vec<CanFrame>) {
    struct Scripted(Option<Vec<CanFrame>>);
    impl FrameSource for Scripted {
        fn poll(&mut self, _now_us: u64, out: &mut Vec<CanFrame>) {
            if let Some(v) = self.0.take() {
                out.extend(v);
            }
        }
    }
    app.sim_t_us = t_us;
    app.source = Box::new(Scripted(Some(frames)));
    app.tick(t_us);
}

fn flagged(app: &App, ch: u8, id: u32, kind: Kind) -> bool {
    app.spec.rows.contains_key(&(ch, id, false, kind))
}

fn verdict(app: &App, ch: u8, id: u32, kind: Kind) -> crate::spec::Latch {
    app.spec.rows[&(ch, id, false, kind)]
}

#[test]
fn an_identifier_the_database_lacks_is_reported_unknown() {
    let mut app = spec_app();
    receive(&mut app, 0, vec![frame_at(0, 0x777, 8, Direction::Rx)]);
    assert!(flagged(&app, 0, 0x777, Kind::Unknown));
    receive(
        &mut app,
        10_000,
        vec![frame_at(10_000, 100, 8, Direction::Rx)],
    );
    assert!(
        !flagged(&app, 0, 100, Kind::Unknown),
        "a declared id is not a violation"
    );
    app.stop();
}

#[test]
fn a_bus_with_no_database_reports_nothing_at_all() {
    let mut app = spec_app();
    app.set_can_table_for_test(0, None);
    app.refresh_snapshot();
    assert!(app.channel_dbc(0).is_none(), "test setup: no database");
    receive(&mut app, 0, vec![frame_at(0, 0x777, 3, Direction::Rx)]);
    receive(&mut app, 900_000, vec![]);
    assert!(app.spec.rows.is_empty(), "no database, no opinion");
    app.stop();
}

/// The frame facts are judged only for traffic we did not produce: driving a
/// signal past the base length widens a Tx frame on purpose, and the
/// generator row already offers to restore a hand-tuned period.
#[test]
fn our_own_transmission_is_never_a_cycle_or_dlc_violation() {
    let mut tx = spec_app();
    receive(&mut tx, 0, vec![frame_at(0, 100, 6, Direction::Tx)]);
    receive(
        &mut tx,
        115_000,
        vec![frame_at(115_000, 100, 6, Direction::Tx)],
    );
    assert!(!flagged(&tx, 0, 100, Kind::Dlc), "we chose that length");
    assert!(!flagged(&tx, 0, 100, Kind::Cycle));

    let mut rx = spec_app();
    receive(&mut rx, 0, vec![frame_at(0, 100, 6, Direction::Rx)]);
    receive(
        &mut rx,
        115_000,
        vec![frame_at(115_000, 100, 6, Direction::Rx)],
    );
    assert!(
        flagged(&rx, 0, 100, Kind::Dlc),
        "the same frame received is"
    );
    assert!(flagged(&rx, 0, 100, Kind::Cycle));
    rx.stop();
    tx.stop();
}

#[test]
fn a_frame_shorter_than_the_declared_size_is_a_dlc_mismatch() {
    let mut app = spec_app();
    receive(&mut app, 0, vec![frame_at(0, 100, 6, Direction::Rx)]);
    assert_eq!(
        verdict(&app, 0, 100, Kind::Dlc),
        crate::spec::Latch {
            count: 1,
            first_t_us: 0,
            last_t_us: 0,
            declared: 8.0,
            measured: 6.0,
        }
    );
    app.stop();
}

#[test]
fn timing_is_silent_where_the_database_declares_no_cycle() {
    let mut app = spec_app();
    assert!(
        app.dbc_cycle_us(0, 200).is_none(),
        "test setup: 200 declares no period"
    );
    receive(&mut app, 0, vec![frame_at(0, 200, 8, Direction::Rx)]);
    receive(
        &mut app,
        5_000_000,
        vec![frame_at(5_000_000, 200, 8, Direction::Rx)],
    );
    assert!(
        !flagged(&app, 0, 200, Kind::Cycle),
        "five seconds between two frames promises nothing was broken"
    );
    assert!(!flagged(&app, 0, 200, Kind::Missing));
    app.stop();
}

#[test]
fn an_event_triggered_message_is_never_reported_missing() {
    let mut app = spec_app();
    assert_eq!(
        app.dbc_cycle_us(0, 300),
        Some(0),
        "test setup: a declared 0 means event-triggered"
    );
    receive(&mut app, 0, vec![frame_at(0, 300, 8, Direction::Rx)]);
    receive(&mut app, 5_000_000, vec![]);
    assert!(!flagged(&app, 0, 300, Kind::Missing));
    assert!(!flagged(&app, 0, 300, Kind::Cycle));
    app.stop();
}

/// The report must not become a list of everyone we chose not to simulate:
/// a virtual bus only ever carries the nodes the user switched on.
#[test]
fn a_message_that_never_appeared_is_not_reported_missing() {
    let mut app = spec_app();
    assert_eq!(
        app.dbc_cycle_us(0, 100),
        Some(100_000),
        "test setup: 100 is declared periodic"
    );
    run_sim(&mut app, 20, 50_000);
    assert!(
        app.spec.rows.is_empty(),
        "never seen is not the same as dropped: {:?}",
        app.spec.rows.keys().collect::<Vec<_>>()
    );
    app.stop();
}

#[test]
fn a_message_that_went_silent_beyond_the_grace_is_reported_missing() {
    let mut app = spec_app();
    receive(&mut app, 0, vec![frame_at(0, 100, 8, Direction::Rx)]);
    receive(
        &mut app,
        100_000,
        vec![frame_at(100_000, 100, 8, Direction::Rx)],
    );
    receive(&mut app, 300_000, vec![]);
    assert!(
        !flagged(&app, 0, 100, Kind::Missing),
        "two silent periods is still inside a grace of three"
    );
    receive(&mut app, 420_000, vec![]);
    assert!(flagged(&app, 0, 100, Kind::Missing));
    assert_eq!(verdict(&app, 0, 100, Kind::Missing).measured, 320_000.0);
    app.stop();
}

#[test]
fn the_cycle_check_uses_the_last_interval_not_the_running_average() {
    let mut app = spec_app();
    for i in 0..10u64 {
        let t = i * 100_000;
        receive(&mut app, t, vec![frame_at(t, 100, 8, Direction::Rx)]);
    }
    assert!(!flagged(&app, 0, 100, Kind::Cycle), "ten on-time frames");
    // 115 ms is 15% late, which the aggregate's running average smooths down
    // to 1.5% and would never report.
    receive(
        &mut app,
        1_015_000,
        vec![frame_at(1_015_000, 100, 8, Direction::Rx)],
    );
    assert!(
        flagged(&app, 0, 100, Kind::Cycle),
        "agg.cycle_us reads 1.5% off here; only the raw interval is 15%"
    );
    assert_eq!(verdict(&app, 0, 100, Kind::Cycle).measured, 115_000.0);
    app.stop();
}

#[test]
fn the_tolerance_setting_decides_how_late_is_late() {
    let mut app = spec_app();
    for i in 0..10u64 {
        let t = i * 100_000;
        receive(&mut app, t, vec![frame_at(t, 100, 8, Direction::Rx)]);
    }
    app.spec_tol_pct = 20;
    receive(
        &mut app,
        1_015_000,
        vec![frame_at(1_015_000, 100, 8, Direction::Rx)],
    );
    assert!(
        !flagged(&app, 0, 100, Kind::Cycle),
        "15% late is clean at a 20% tolerance"
    );
    app.spec_tol_pct = 5;
    receive(
        &mut app,
        1_126_000,
        vec![frame_at(1_126_000, 100, 8, Direction::Rx)],
    );
    assert!(
        flagged(&app, 0, 100, Kind::Cycle),
        "the next interval is only 11% late, but the tolerance now says 5"
    );
    app.stop();
}

#[test]
fn the_grace_setting_decides_when_silence_counts_as_dropout() {
    let mut app = spec_app();
    receive(&mut app, 0, vec![frame_at(0, 100, 8, Direction::Rx)]);
    app.spec_grace = 10;
    receive(&mut app, 950_000, vec![]);
    assert!(
        !flagged(&app, 0, 100, Kind::Missing),
        "9.5 periods of silence is inside a grace of ten"
    );
    app.spec_grace = 2;
    receive(&mut app, 960_000, vec![]);
    assert!(
        flagged(&app, 0, 100, Kind::Missing),
        "tightening the grace convicts the same continuing silence"
    );
    app.stop();
}

#[test]
fn one_bad_interval_is_counted_once_and_never_forgotten() {
    let mut app = spec_app();
    receive(&mut app, 0, vec![frame_at(0, 100, 8, Direction::Rx)]);
    receive(
        &mut app,
        200_000,
        vec![frame_at(200_000, 100, 8, Direction::Rx)],
    );
    assert_eq!(verdict(&app, 0, 100, Kind::Cycle).count, 1);
    for i in 1..=5u64 {
        // Continue the declared spacing from 200 ms, so each of these is a
        // clean interval rather than another gap.
        let t = 200_000 + i * 100_000;
        receive(&mut app, t, vec![frame_at(t, 100, 8, Direction::Rx)]);
    }
    assert_eq!(
        verdict(&app, 0, 100, Kind::Cycle).count,
        1,
        "a verdict from min/max would keep convicting"
    );
    app.stop();
}

#[test]
fn the_spec_clear_forgets_the_latches_without_a_new_run() {
    let mut app = spec_app();
    receive(&mut app, 0, vec![frame_at(0, 100, 8, Direction::Rx)]);
    receive(
        &mut app,
        200_000,
        vec![frame_at(200_000, 100, 8, Direction::Rx)],
    );
    assert_eq!(verdict(&app, 0, 100, Kind::Cycle).count, 1);

    app.send(crate::bus::BusCommand::ClearSpec);
    assert!(
        app.snap.spec.rows.is_empty(),
        "clear empties the report in place"
    );
    // The interval memory went with the rows: the next arrival is a first
    // sample again, so a clean frame does not re-convict.
    receive(
        &mut app,
        300_000,
        vec![frame_at(300_000, 100, 8, Direction::Rx)],
    );
    assert!(
        app.snap.spec.rows.is_empty(),
        "a clean interval after clear stays clean"
    );
    app.stop();
}

#[test]
fn an_entry_config_can_pin_the_exact_frame_flags() {
    let mut app = App::headless();
    app.add_tx(0, 0x100);
    // A trace row added to the generator keeps the flags it was seen with,
    // even when they disagree with the fd checkbox's derived default.
    app.send(crate::bus::BusCommand::SetEntryConfig {
        ch: 0,
        id: 0x100,
        active: true,
        cycle_us: 100_000,
        fd: false,
        flags: Some(crate::can::frame::FrameFlags::FD),
        data_text: None,
        srcs: Vec::new(),
    });
    let view = app
        .snap
        .tx
        .iter()
        .find(|t| t.id == 0x100)
        .expect("the entry exists");
    assert!(view.fd, "explicit flags win over the fd-derived ones");
    app.stop();
}

#[test]
fn the_first_sample_of_a_message_is_never_a_cycle_violation() {
    let mut app = spec_app();
    // Arrives five periods late, so the only thing standing between this and
    // a verdict is that nothing preceded it.
    receive(
        &mut app,
        500_000,
        vec![frame_at(500_000, 100, 8, Direction::Rx)],
    );
    assert_eq!(
        app.aggs[&(
            crate::model::ClusterId(app.snap.cluster_ids.can[0]),
            100,
            false
        )]
            .count,
        1,
        "test setup: one frame, no interval yet"
    );
    assert!(!flagged(&app, 0, 100, Kind::Cycle));
    app.stop();
}

#[test]
fn a_step_that_brought_no_new_frame_is_not_a_new_interval() {
    let mut app = spec_app();
    receive(&mut app, 0, vec![frame_at(0, 100, 8, Direction::Rx)]);
    receive(
        &mut app,
        100_000,
        vec![frame_at(100_000, 100, 8, Direction::Rx)],
    );
    assert!(!flagged(&app, 0, 100, Kind::Cycle), "test setup: on time");
    // Nothing arrives for two steps. The aggregate has not moved, so there
    // is no period to measure -- only the silence clock runs here.
    receive(&mut app, 200_000, vec![]);
    receive(&mut app, 300_000, vec![]);
    assert!(!flagged(&app, 0, 100, Kind::Cycle));
    app.stop();
}

#[test]
fn replay_traffic_never_raises_a_missing_violation() {
    let mut app = spec_app();
    app.mode = Mode::Replay;
    receive(&mut app, 0, vec![frame_at(0, 100, 8, Direction::Rx)]);
    receive(&mut app, 5_000_000, vec![]);
    assert!(
        !flagged(&app, 0, 100, Kind::Missing),
        "a log's clock cannot say what is still talking"
    );
    // The frame facts stay judged in replay, because they need no clock.
    receive(
        &mut app,
        5_100_000,
        vec![frame_at(5_100_000, 100, 5, Direction::Rx)],
    );
    assert!(flagged(&app, 0, 100, Kind::Dlc));
    app.stop();
}

#[test]
fn a_paused_clock_does_not_make_a_message_missing() {
    let mut app = spec_app();
    receive(&mut app, 0, vec![frame_at(0, 100, 8, Direction::Rx)]);
    app.trace_paused = true;
    assert!(app.trace_paused, "test setup: paused");
    // The real loop never calls `tick` while paused; driving it anyway
    // shows the verdict is gated on the pause itself, not on a missing step.
    for i in 1..=10u64 {
        receive(&mut app, i * 200_000, vec![]);
    }
    assert!(!flagged(&app, 0, 100, Kind::Missing));
    app.stop();
}

#[test]
fn a_verdict_follows_its_bus_when_an_earlier_bus_is_deleted() {
    let mut app = spec_app();
    // Move the fixture database onto the second bus and leave the first
    // without one, then delete the first.
    app.move_can_table_for_test(0, 1);
    receive(
        &mut app,
        0,
        vec![CanFrame {
            channel: 1,
            ..frame_at(0, 100, 6, Direction::Rx)
        }],
    );
    assert!(flagged(&app, 1, 100, Kind::Dlc), "test setup: on bus 1");
    app.remove_channel(0);
    assert!(
        flagged(&app, 0, 100, Kind::Dlc),
        "the row must move with the bus, not stay behind at the old index"
    );
    app.stop();
}

#[test]
fn the_monitor_forgets_everything_when_a_new_run_starts() {
    let mut app = spec_app();
    receive(&mut app, 0, vec![frame_at(0, 100, 8, Direction::Rx)]);
    receive(
        &mut app,
        200_000,
        vec![frame_at(200_000, 100, 8, Direction::Rx)],
    );
    assert!(
        flagged(&app, 0, 100, Kind::Cycle),
        "test setup: a verdict worth forgetting"
    );
    app.start_virtual();
    assert!(app.spec.rows.is_empty(), "the report belongs to a run");
    assert_eq!(
        app.spec.previous((0, 100, false)),
        None,
        "and so does the interval memory"
    );
    // Five seconds later on a clock that started over. Against the previous
    // run's interval this would read as one enormous measured period.
    receive(
        &mut app,
        5_000_000,
        vec![frame_at(5_000_000, 100, 8, Direction::Rx)],
    );
    assert!(
        !flagged(&app, 0, 100, Kind::Cycle),
        "a stale interval from the old run would convict this frame"
    );
    app.stop();
}

/// Same mux layout as `MUX_DBC` in the dbc tests, one bus, generator
/// silenced so only the frames built here reach the sampler.
const MUX_SAMPLE_DBC: &str = r#"VERSION "roxy-can mux sampling test"

NS_ :

BU_: ECU

BO_ 400 Muxed: 8 ECU
 SG_ Switch M : 0|8@1+ (1,0) [0|0] "" ECU
 SG_ G1_A m1 : 16|16@1+ (0.1,0) [0|0] "" ECU
 SG_ G2_C m2 : 16|16@1+ (0.5,0) [0|0] "" ECU
"#;

fn mux_app() -> App {
    let mut app = App::headless();
    app.set_can_table_for_test(
        0,
        Some(std::sync::Arc::new(
            crate::dbc::load_dbc_str(MUX_SAMPLE_DBC).unwrap(),
        )),
    );
    app.tx_list.retain(|t| t.channel != 0);
    app.start_virtual();
    app
}

fn mux_frame(t_us: u64, switch: u8) -> CanFrame {
    let mut f = frame_at(t_us, 400, 8, Direction::Rx);
    f.data[0] = switch;
    f.data[2] = 100;
    f
}

/// A standard and an extended message sharing one numeric id are two
/// different signals: each subscription samples only its own class's
/// frames, even though the id and the channel are identical.
#[test]
fn twin_class_subscriptions_never_cross_feed() {
    const TWIN_DBC: &str = r#"VERSION "roxy-can twin sub test"

NS_ :

BS_:

BU_: ECU

BO_ 256 Twin: 2 ECU
 SG_ StdSig : 0|16@1+ (0.1,0) [0|0] ""  ECU

BO_ 2147483904 Twin: 2 ECU
 SG_ ExtSig : 0|16@1+ (0.1,0) [0|0] ""  ECU
"#;
    let mut app = App::headless();
    app.set_can_table_for_test(
        0,
        Some(std::sync::Arc::new(
            crate::dbc::load_dbc_str(TWIN_DBC).unwrap(),
        )),
    );
    app.tx_list.retain(|t| t.channel != 0);
    app.start_virtual();
    let std_key = SigKey::can_in(crate::model::ClusterId(1), 0x100, false, "StdSig");
    let ext_key = SigKey::can_in(crate::model::ClusterId(1), 0x100, true, "ExtSig");
    app.subscribe(std_key.clone());
    app.subscribe(ext_key.clone());
    assert!(
        app.model_can_all_values(&frame_at(0, 0x100, 2, Direction::Rx))
            .is_some()
    );
    let mut ext_probe = frame_at(0, 0x100, 2, Direction::Rx);
    ext_probe.extended = true;
    assert!(
        app.model_can_all_values(&ext_probe).is_some(),
        "the extended class finds its own frame in the model"
    );

    // A standard frame: only the standard subscription sees it...
    receive(
        &mut app,
        10_000,
        vec![frame_at(10_000, 0x100, 2, Direction::Rx)],
    );
    assert!(
        !app.subs[&std_key].history.is_empty(),
        "standard class sampled"
    );
    assert!(
        app.subs[&ext_key].history.is_empty(),
        "extended class got nothing from a standard frame"
    );

    // ...an extended frame: only the extended subscription sees it.
    let mut ext = frame_at(20_000, 0x100, 2, Direction::Rx);
    ext.extended = true;
    ext.data[0] = 200;
    receive(&mut app, 20_000, vec![ext]);
    assert_eq!(
        app.subs[&std_key].latest, 0.0,
        "standard class never saw the extended frame"
    );
    assert!(
        app.subs[&ext_key].latest > 0.0,
        "extended class sampled the extended frame"
    );
    app.stop();
}

#[test]
fn a_signal_of_the_inactive_group_is_not_sampled() {
    let mut app = mux_app();
    let g1 = SigKey::can_in(crate::model::ClusterId(1), 400, false, "G1_A");
    let g2 = SigKey::can_in(crate::model::ClusterId(1), 400, false, "G2_C");
    app.subscribe(g1.clone());
    app.subscribe(g2.clone());
    assert!(app.subs.contains_key(&g1), "both signals are subscribed");
    assert!(app.subs.contains_key(&g2));

    receive(&mut app, 100_000, vec![mux_frame(100_000, 1)]);
    assert!(
        app.subs[&g2].history.is_empty(),
        "group 2 was not in the frame, so it has no samples"
    );
    assert!(
        !app.subs[&g1].history.is_empty(),
        "group 1 was active and got sampled"
    );
    app.stop();
}

#[test]
fn a_group_signal_gains_samples_once_its_group_is_switched_in() {
    let mut app = mux_app();
    let g2 = SigKey::can_in(crate::model::ClusterId(1), 400, false, "G2_C");
    app.subscribe(g2.clone());

    receive(&mut app, 100_000, vec![mux_frame(100_000, 1)]);
    let before = app.subs[&g2].history.len();
    receive(&mut app, 200_000, vec![mux_frame(200_000, 2)]);
    assert_eq!(before, 0, "inactive until the switch changes");
    assert!(
        app.subs[&g2].history.len() > before,
        "once its group is switched in, the signal is sampled again"
    );
    app.stop();
}

#[test]
fn an_insert_marker_trigger_stamps_the_bus_clock() {
    let mut app = quiet_app();
    app.triggers.push(Trigger::new(
        TriggerCond::IdPresent { ch: 0, id: 0x777 },
        TriggerAction::InsertMarker,
    ));
    receive(
        &mut app,
        10_000,
        vec![rx_frame(10_000, 0x100, 8, FrameFlags::NONE)],
    );
    assert!(app.snap.markers.is_empty(), "an unwatched id marks nothing");
    receive(
        &mut app,
        20_000,
        vec![rx_frame(20_000, 0x777, 8, FrameFlags::NONE)],
    );
    assert_eq!(app.snap.markers, [20_000], "the edge stamps the bus clock");
    // A new measurement forgets the markers with the rest of the run.
    app.reset_run();
    app.refresh_snapshot();
    assert!(app.snap.markers.is_empty(), "markers are run-scored state");
}

/// Every edge that runs an action lands in the Write log and in the
/// replay bar's navigation list, not only marker actions.
#[test]
fn a_trigger_edge_lands_in_the_write_log_and_the_fire_list() {
    let mut app = quiet_app();
    app.triggers.push(Trigger::new(
        TriggerCond::IdPresent { ch: 0, id: 0x777 },
        TriggerAction::InsertMarker,
    ));
    receive(
        &mut app,
        10_000,
        vec![rx_frame(10_000, 0x777, 8, FrameFlags::NONE)],
    );
    app.refresh_snapshot();
    assert_eq!(app.snap.trigger_fires.len(), 1, "one edge, one entry");
    let fire = &app.snap.trigger_fires[0];
    assert_eq!(fire.at_us, 10_000);
    assert!(fire.ok);
    assert!(fire.text.contains("0x777"), "{}", fire.text);
    assert!(
        app.snap
            .write
            .iter()
            .any(|line| line.text.contains("trigger 1")),
        "the edge is visible in Write"
    );
}

/// Cooldown holds an oscillating rule back, the fire cap stops it without
/// disabling the row, and re-arming one rule resets only that rule.
#[test]
fn trigger_cooldown_max_fires_and_per_rule_rearm_gate_edges() {
    let mut app = quiet_app();
    let mut trigger = Trigger::new(
        TriggerCond::IdPresent { ch: 0, id: 0x777 },
        TriggerAction::InsertMarker,
    );
    trigger.cooldown_us = 50_000;
    trigger.max_fires = 2;
    app.triggers.push(trigger);

    let hit = |app: &mut App, t: u64| {
        // Appearance conditions latch; drop the latch so the next frame is
        // a fresh edge, exactly what Re-arm does for a live rule.
        if let Some(trigger) = app.triggers.first_mut() {
            trigger.level = false;
        }
        receive(app, t, vec![rx_frame(t, 0x777, 8, FrameFlags::NONE)]);
    };

    hit(&mut app, 10_000);
    assert_eq!(app.triggers[0].fired, 1);
    hit(&mut app, 40_000);
    assert_eq!(app.triggers[0].fired, 1, "inside the 50 ms cooldown");
    hit(&mut app, 100_000);
    assert_eq!(app.triggers[0].fired, 2);
    hit(&mut app, 200_000);
    assert_eq!(app.triggers[0].fired, 2, "fire cap reached");
    assert!(app.triggers[0].enabled, "the row stays enabled at the cap");

    app.send(crate::bus::BusCommand::RearmTrigger { index: 0 });
    assert_eq!(app.triggers[0].fired, 0, "per-rule re-arm resets the count");
    hit(&mut app, 300_000);
    assert_eq!(app.triggers[0].fired, 1, "and the rule runs again");
}

/// New actions reuse the Panel/core write paths: a sysvar write is clamped,
/// a script action is queued, and both report success in the fire list.
#[test]
fn trigger_actions_set_a_sysvar_and_queue_a_script() {
    let mut app = quiet_app();
    app.send(crate::bus::BusCommand::DefineSysVar(
        crate::bus::SysVarDef {
            namespace: "Bench".to_string(),
            name: "Ready".to_string(),
            init: 0.0,
            min: Some(0.0),
            max: Some(10.0),
            unit: String::new(),
            comment: String::new(),
            label_sets: Vec::new(),
            band_set_id: None,
            labels: Vec::new(),
        },
    ));
    app.triggers.push(Trigger::new(
        TriggerCond::IdPresent { ch: 0, id: 0x777 },
        TriggerAction::SetSysVar {
            key: "Bench::Ready".to_string(),
            value: 7.0,
        },
    ));
    app.triggers.push(Trigger::new(
        TriggerCond::IdPresent { ch: 0, id: 0x777 },
        TriggerAction::RunScript {
            name: "Go".to_string(),
            value: Some(1.5),
        },
    ));
    receive(
        &mut app,
        10_000,
        vec![rx_frame(10_000, 0x777, 8, FrameFlags::NONE)],
    );
    app.refresh_snapshot();
    assert_eq!(app.snap.sysvars[0].value, 7.0, "the sysvar write landed");
    assert_eq!(app.snap.trigger_fires.len(), 2);
    assert!(app.snap.trigger_fires.iter().all(|fire| fire.ok));
    assert!(
        app.snap
            .trigger_fires
            .iter()
            .any(|fire| fire.result.contains("queued \"Go\"")),
        "the script action reports its queue hand-off"
    );
}

/// A payload rule reads one masked byte, so an undescribed frame can be
/// watched without a database entry. A changed byte clears the level and the
/// next match is a fresh edge.
#[test]
fn a_payload_trigger_fires_on_the_masked_byte() {
    let mut app = quiet_app();
    app.triggers.push(Trigger::new(
        TriggerCond::PayloadMatch {
            ch: 0,
            id: 0x700,
            ext: false,
            offset: 1,
            mask: 0xF0,
            value: 0xA0,
        },
        TriggerAction::InsertMarker,
    ));
    let mut frame = rx_frame(10_000, 0x700, 8, FrameFlags::NONE);
    frame.data[1] = 0xAB;
    receive(&mut app, 10_000, vec![frame]);
    assert_eq!(app.snap.markers, [10_000], "masked byte matched");

    let mut miss = rx_frame(20_000, 0x700, 8, FrameFlags::NONE);
    miss.data[1] = 0x0B;
    receive(&mut app, 20_000, vec![miss]);
    let mut hit = rx_frame(30_000, 0x700, 8, FrameFlags::NONE);
    hit.data[1] = 0xAF;
    receive(&mut app, 30_000, vec![hit]);
    assert_eq!(
        app.snap.markers,
        [10_000, 30_000],
        "a non-matching frame cleared the level"
    );
}

/// An Ethernet flow rule latches once per run like the other presence
/// conditions; the tag and EtherType both have to match.
#[test]
fn an_ethernet_flow_trigger_latches_on_its_flow() {
    let mut app = quiet_app();
    app.triggers.push(Trigger::new(
        TriggerCond::EthFlow {
            bus: 0,
            ethertype: 0x88B5,
            vlan: Some(7),
        },
        TriggerAction::InsertMarker,
    ));
    app.ingest_eth(crate::eth::EthFrame::session(
        10_000,
        0,
        Some(8),
        0x88B5,
        vec![1, 2],
    ));
    assert!(app.snap.markers.is_empty(), "wrong VLAN does not match");
    app.ingest_eth(crate::eth::EthFrame::session(
        20_000,
        0,
        Some(7),
        0x88B5,
        vec![1, 2],
    ));
    app.refresh_snapshot();
    assert_eq!(app.snap.markers, [20_000]);
    app.ingest_eth(crate::eth::EthFrame::session(
        30_000,
        0,
        Some(7),
        0x88B5,
        vec![3],
    ));
    app.refresh_snapshot();
    assert_eq!(app.snap.markers, [20_000], "presence latches for the run");
}

/// The new action payloads and the per-rule gates survive a project save.
#[test]
fn trigger_actions_and_limits_round_trip_through_a_project() {
    let mut app = quiet_app();
    app.send(crate::bus::BusCommand::AddChannel);
    app.settle();
    let mut setpoint = Trigger::new(
        TriggerCond::IdPresent { ch: 0, id: 0x777 },
        TriggerAction::SetSysVar {
            key: "Bench::Ready".to_string(),
            value: 3.5,
        },
    );
    setpoint.cooldown_us = 250_000;
    setpoint.max_fires = 4;
    app.triggers.push(setpoint);
    app.triggers.push(Trigger::new(
        TriggerCond::IdPresent { ch: 0, id: 0x778 },
        TriggerAction::RunScript {
            name: "Go".to_string(),
            value: None,
        },
    ));
    app.triggers.push(Trigger::new(
        TriggerCond::PayloadMatch {
            ch: 0,
            id: 0x700,
            ext: false,
            offset: 3,
            mask: 0xF0,
            value: 0xA0,
        },
        TriggerAction::InsertMarker,
    ));

    app.triggers[2].pre_frames = Some(2);
    app.triggers[2].post_frames = Some(3);
    app.panel_pages =
        vec![serde_json::json!({"format": "roxy-can-panel", "page": {"name": "Bench"}})];
    app.triggers.push(Trigger::new(
        TriggerCond::IdPresent { ch: 0, id: 0x779 },
        TriggerAction::SendEth {
            bus: 0,
            frame: "Payload".into(),
            data: vec![0xAA, 0xBB],
        },
    ));
    app.refresh_snapshot();
    let project = ProjectV2::from_app(&app, None);
    assert_eq!(project.triggers.len(), 4, "four rules were saved");
    let mut restored = App::headless();
    project.apply(&mut restored);
    restored.refresh_snapshot();

    assert_eq!(restored.triggers.len(), 4);
    assert_eq!(restored.triggers[0].action, app.triggers[0].action);
    assert_eq!(restored.triggers[0].cooldown_us, 250_000);
    assert_eq!(restored.triggers[0].max_fires, 4);
    assert_eq!(restored.triggers[1].action, app.triggers[1].action);
    assert_eq!(restored.triggers[2].cond, app.triggers[2].cond);
    assert_eq!(restored.triggers[2].pre_frames, Some(2));
    assert_eq!(restored.triggers[2].post_frames, Some(3));
    assert_eq!(restored.triggers[3].action, app.triggers[3].action);
    assert_eq!(restored.panel_pages, app.panel_pages);
}

/// A trigger-started recording opens with the frames that came before the
/// event, oldest first, and the triggering frame itself lands right after
/// them.
#[test]
fn a_trigger_started_recording_includes_the_pre_buffer() {
    let mut app = quiet_app();
    let base = std::env::temp_dir().join("roxy_can_pre_buffer.asc");
    app.send(crate::bus::BusCommand::SetRecordPath(
        base.to_string_lossy().into_owned(),
    ));
    app.triggers.push(Trigger::new(
        TriggerCond::IdPresent { ch: 0, id: 0x777 },
        TriggerAction::StartRecording,
    ));
    app.recorder.recording = false;

    receive(
        &mut app,
        10_000,
        vec![
            rx_frame(10_000, 0x100, 8, FrameFlags::NONE),
            rx_frame(20_000, 0x200, 8, FrameFlags::NONE),
        ],
    );
    assert!(
        !app.recorder.recording,
        "no watched id yet: the recorder stayed closed"
    );
    receive(
        &mut app,
        30_000,
        vec![rx_frame(30_000, 0x777, 8, FrameFlags::NONE)],
    );
    assert!(app.recorder.recording, "the watched id opened the file");
    app.recorder.close();

    let content = std::fs::read_to_string(&app.recorder.last_record).unwrap();
    let frames = crate::log::asc::parse_asc(&content);
    let ids: Vec<u32> = frames.iter().map(|f| f.id).collect();
    assert_eq!(
        ids,
        [0x100, 0x200, 0x777],
        "pre-context oldest first, then the trigger frame"
    );
    std::fs::remove_file(&app.recorder.last_record).ok();
}

/// A start-recording trigger arms the same flag the toolbar toggles: while
/// stopped it leaves the recorder enabled for the next Start instead of
/// silently doing nothing.
#[test]
fn a_start_trigger_arms_recording_while_stopped() {
    let mut app = quiet_app();
    app.triggers.push(Trigger::new(
        TriggerCond::IdPresent { ch: 0, id: 0x777 },
        TriggerAction::StartRecording,
    ));
    app.stop();
    receive(
        &mut app,
        10_000,
        vec![rx_frame(10_000, 0x777, 8, FrameFlags::NONE)],
    );
    assert!(!app.measuring);
    assert!(
        app.recorder.recording,
        "trigger enabled recording for the next start"
    );
}

/// A stop-recording trigger disarms the flag while stopped; the post-roll is
/// a measurement-time grace, not something to carry into the next Start.
#[test]
fn a_stop_trigger_disarms_recording_while_stopped() {
    let mut app = quiet_app();
    app.triggers.push(Trigger::new(
        TriggerCond::IdPresent { ch: 0, id: 0x777 },
        TriggerAction::StopRecording,
    ));
    app.recorder.recording = true;
    app.stop();
    receive(
        &mut app,
        10_000,
        vec![rx_frame(10_000, 0x777, 8, FrameFlags::NONE)],
    );
    assert!(!app.measuring);
    assert!(!app.recorder.recording, "trigger disarmed the next Start");
}

/// The post-trigger roll: a trigger stop keeps recording for a bounded
/// number of frames, so the aftermath of the event stays in the file.
#[test]
fn a_stop_trigger_rolls_on_for_the_post_frames() {
    let mut app = quiet_app();
    let base = std::env::temp_dir().join("roxy_can_post_roll.asc");
    app.send(crate::bus::BusCommand::SetRecordPath(
        base.to_string_lossy().into_owned(),
    ));
    app.triggers.push(Trigger::new(
        TriggerCond::IdPresent { ch: 0, id: 0x777 },
        TriggerAction::StopRecording,
    ));
    app.recorder.recording = true;
    app.recorder.open().unwrap();

    receive(
        &mut app,
        10_000,
        vec![rx_frame(10_000, 0x777, 8, FrameFlags::NONE)],
    );
    assert!(
        app.recorder.recording,
        "the stop rolls on instead of closing"
    );

    // Far more frames than the post-roll can hold.
    let fillers: Vec<CanFrame> = (0..60)
        .map(|i| rx_frame(20_000 + i as u64 * 1_000, 0x100, 8, FrameFlags::NONE))
        .collect();
    receive(&mut app, 100_000, fillers);
    assert!(!app.recorder.recording, "the post-roll expired");
    app.recorder.close();

    let text = std::fs::read_to_string(&app.recorder.last_record).unwrap();
    let frames = crate::log::asc::parse_asc(&text);
    assert_eq!(
        frames.len(),
        32, // POST_ROLL_FRAMES: the edge frame plus the roll
        "the file holds exactly the post-roll window"
    );
    assert_eq!(frames[0].id, 0x777, "the event frame is first");
    assert_eq!(frames[0].t_us, 10_000);
}

/// The re-arm half of the appearance-watch story: the ClearTrace action
/// resets the firing condition's own latch, and the manual command
/// resets latches without touching real-level conditions.
#[test]
fn clear_trace_and_rearm_reset_latched_conditions() {
    let mut app = quiet_app();
    app.triggers.push(Trigger::new(
        TriggerCond::IdPresent { ch: 0, id: 0x777 },
        TriggerAction::ClearTrace,
    ));
    receive(
        &mut app,
        10_000,
        vec![rx_frame(10_000, 0x777, 8, FrameFlags::NONE)],
    );
    assert_eq!(app.triggers[0].fired, 1);
    // The action re-armed its own condition: the next occurrence fires
    // again instead of being swallowed by the latch.
    receive(
        &mut app,
        20_000,
        vec![rx_frame(20_000, 0x777, 8, FrameFlags::NONE)],
    );
    assert_eq!(app.triggers[0].fired, 2, "re-armed by its own clear");
}

#[test]
fn the_rearm_command_resets_latches_but_not_real_levels() {
    let mut app = quiet_app();
    // Both actions start recordings: give them temp paths so nothing
    // lands in the repo root.
    let base = std::env::temp_dir().join("roxy_can_rearm_trigger.asc");
    app.send(crate::bus::BusCommand::SetRecordPath(
        base.to_string_lossy().into_owned(),
    ));
    app.triggers.push(Trigger::new(
        TriggerCond::IdPresent { ch: 0, id: 0x777 },
        TriggerAction::StartRecording,
    ));
    app.triggers.push(Trigger::new(
        TriggerCond::SignalCross {
            ch: 0,
            id: 0x100,
            ext: false,
            signal: "EngineSpeed".to_string(),
            threshold: 3000.0,
            rising: true,
        },
        TriggerAction::StartRecording,
    ));
    receive(
        &mut app,
        10_000,
        vec![
            rx_frame(10_000, 0x777, 8, FrameFlags::NONE),
            rpm_frame(10_000, 4000.0),
        ],
    );
    assert_eq!(app.triggers[0].fired, 1);
    assert_eq!(app.triggers[1].fired, 1);
    assert!(app.triggers[1].level, "the crossing level is a real level");

    app.send(crate::bus::BusCommand::RearmTriggers);

    // Still above threshold: the crossing must NOT re-fire, while the
    // re-armed presence watch does.
    receive(
        &mut app,
        20_000,
        vec![
            rpm_frame(20_000, 4000.0),
            rx_frame(20_000, 0x777, 8, FrameFlags::NONE),
        ],
    );
    assert_eq!(
        app.triggers[0].fired, 2,
        "presence fires again after re-arm"
    );
    assert_eq!(
        app.triggers[1].fired, 1,
        "a real level is not disturbed by the re-arm"
    );
    assert!(app.triggers[1].level);
}

/// "Switch All Blocks to Simulation" at the bus level: simulate-all 补建
/// 全部条目并把角色开关全部打开（条目保持默认关——发送由用户逐条或经
/// All On 打开）；stop-all 关闭全部开关。
#[test]
fn simulate_all_activates_every_dbc_node() {
    let mut app = App::headless();
    app.simulate_all_nodes(0);
    let db = app.channel_dbc(0).expect("sample DBC loaded");
    let expected: Vec<u32> = db
        .order
        .iter()
        .filter(|&&(_, ext)| !ext)
        .map(|&(id, _)| id)
        .collect();
    for id in &expected {
        let entry = app.tx_list.iter().find(|t| t.channel == 0 && t.id == *id);
        assert!(
            entry.is_some(),
            "node message 0x{id:X} should have an entry"
        );
    }
    // 全部开关开放：逐条目打开后按条目开关发送。
    let bus_entries: Vec<(u8, u32)> = app
        .tx_list
        .iter()
        .filter(|t| t.channel == 0)
        .map(|t| (t.channel, t.id))
        .collect();
    for (ch, id) in &bus_entries {
        app.send(crate::bus::BusCommand::SetEntryActive {
            ch: *ch,
            id: *id,
            on: true,
        });
    }
    let flowing = app
        .tx_list
        .iter()
        .filter(|t| t.channel == 0 && t.active)
        .count();
    assert!(flowing > 0, "entries exist for the whole bus");

    // The reverse sweep closes every gate on the bus (switches preserved).
    app.stop_all_nodes(0);
    app.settle();
    let any_gate = app.snap.tx.iter().any(|t| t.channel == 0 && t.gate_open);
    assert!(!any_gate, "stop-all closes every gate on the bus");
}

/// The State Tracker CSV export mirrors what the bands draw: one row per
/// state segment per visible signal over the window's live span.
#[test]
fn the_state_csv_writes_one_row_per_state_segment() {
    let mut app = App::headless();
    app.tx_list.retain(|t| t.channel != 0);
    app.new_state_window();
    let key = SigKey::can_in(crate::model::ClusterId(1), 0x100, false, "EngineSpeed");
    app.subscribe(key.clone());
    app.state_trackers[0]
        .signals
        .push(crate::observe::GfxSignal {
            key: key.clone(),
            visible: true,
            y_mode: YMode::Auto,
        });
    app.start_virtual();
    // 1000 rpm then 8000 rpm: two clearly distinct states.
    receive(&mut app, 10_000, vec![rpm_frame(10_000, 1000.0)]);
    // 70 ms later: beyond the subscription stride, so the second state is
    // actually sampled into the history.
    receive(&mut app, 80_000, vec![rpm_frame(80_000, 8000.0)]);
    // Push the clock past the last frame so the 8000 band has width at
    // export time.
    receive(&mut app, 100_000, vec![]);
    let path = std::env::temp_dir().join("roxy_can_state_export.csv");
    app.export_state_csv(0, &path.to_string_lossy());
    let content = std::fs::read_to_string(&path).unwrap();
    let rows: Vec<&str> = content.lines().skip(1).collect();
    assert!(
        rows.len() >= 2,
        "two rpm states must appear as at least two rows: {content}"
    );
    assert!(content.contains("1000"), "{content}");
    assert!(content.contains("8000"), "{content}");
    std::fs::remove_file(&path).ok();
    app.stop();
}

fn rx_frame(t_us: u64, id: u32, len: u8, flags: FrameFlags) -> CanFrame {
    CanFrame {
        t_us,
        channel: 0,
        id,
        extended: false,
        len,
        data: [0u8; MAX_CAN_FD_LEN],
        dir: Direction::Rx,
        flags,
    }
}

fn ext_rx_frame(t_us: u64, id: u32, len: u8) -> CanFrame {
    CanFrame {
        t_us,
        channel: 0,
        id,
        extended: true,
        len,
        data: [0u8; MAX_CAN_FD_LEN],
        dir: Direction::Rx,
        flags: FrameFlags::NONE,
    }
}

/// One bus carrying **two** DBC files: both decode live traffic, and on
/// a duplicate message id the earlier database wins. The attach list
/// round-trips through the project file.
#[test]
fn one_bus_attaches_several_databases() {
    const DB_A: &str = r#"VERSION "multi A"

NS_ :

BS_:

BU_: ECU

BO_ 256 PrimaryMsg: 2 ECU
 SG_ PSig : 0|16@1+ (0.1,0) [0|0] ""  ECU
"#;
    const DB_B: &str = r#"VERSION "multi B"

NS_ :

BS_:

BU_: ECU

BO_ 256 SecondaryMsg: 3 ECU
 SG_ SSig : 0|24@1+ (1,0) [0|0] ""  ECU

BO_ 257 OnlyInB: 1 ECU
 SG_ BOnly : 0|8@1+ (1,0) [0|0] ""  ECU
"#;
    let a = std::env::temp_dir().join("roxy_can_multi_a.dbc");
    let b = std::env::temp_dir().join("roxy_can_multi_b.dbc");
    std::fs::write(&a, DB_A).unwrap();
    std::fs::write(&b, DB_B).unwrap();

    let mut app = App::headless();
    app.send(crate::bus::BusCommand::LoadDbc {
        ch: 0,
        paths: vec![
            a.to_string_lossy().into_owned(),
            b.to_string_lossy().into_owned(),
        ],
    });
    app.settle();
    let db = app.channel_dbc(0).expect("both databases merged");
    // Three declarations collapse to two keys: the duplicated 256 is one
    // message (first database wins), 257 is the second file's own.
    assert_eq!(db.messages.len(), 2);
    // Duplicate numeric id: the earlier database wins.
    assert_eq!(db.message_name_of((0x100, false)), Some("PrimaryMsg"));
    assert_eq!(db.message_name_of((0x101, false)), Some("OnlyInB"));

    // Both databases decode live traffic on the same bus.
    let psig = SigKey::can_in(crate::model::ClusterId(1), 0x100, false, "PSig");
    let bonly = SigKey::can_in(crate::model::ClusterId(1), 0x101, false, "BOnly");
    app.subscribe(psig.clone());
    app.subscribe(bonly.clone());
    app.start_virtual();
    receive(
        &mut app,
        10_000,
        vec![
            frame_at(10_000, 0x100, 2, Direction::Rx),
            frame_at(10_000, 0x101, 1, Direction::Rx),
        ],
    );
    assert!(
        !app.subs[&psig].history.is_empty(),
        "the primary database decodes 0x100"
    );
    assert!(
        !app.subs[&bonly].history.is_empty(),
        "the second database decodes 0x101 on the same bus"
    );

    // The attach list survives a project round trip.
    let project = std::env::temp_dir().join("roxy_can_multi.rxproj");
    assert!(app.save_project(Some(project.clone())), "save writes");
    let mut restored = App::headless();
    restored.open_project_path(&project);
    let restored_db = restored.channel_dbc(0).unwrap();
    assert_eq!(restored_db.messages.len(), 2, "both databases re-attach");
    assert_eq!(restored_db.message_name_of((0x101, false)), Some("OnlyInB"));
    std::fs::remove_file(&project).ok();
    app.stop();
}

/// External DBC edits are detected by checksum and reload automatically:
/// the merged table picks up the new message without a manual reload.
#[test]
fn dbc_file_changes_are_detected_and_reloaded() {
    let a = std::env::temp_dir().join("roxy_can_autoreload_a.dbc");
    std::fs::write(
        &a,
        r#"VERSION "auto a"

NS_ :

BS_:

BU_: ECU

BO_ 300 FirstMsg: 1 ECU
 SG_ A1 : 0|8@1+ (1,0) [0|0] ""  ECU
"#,
    )
    .unwrap();

    let mut app = App::headless();
    app.send(crate::bus::BusCommand::LoadDbc {
        ch: 0,
        paths: vec![a.to_string_lossy().into_owned()],
    });
    app.settle();
    let db = app.channel_dbc(0).expect("loaded");
    assert!(
        db.message_name_of((300, false)) == Some("FirstMsg"),
        "BO_ 300 is decimal id 300"
    );

    // External edit: rename the message in the file.
    std::fs::write(
        &a,
        r#"VERSION "auto a"

NS_ :

BS_:

BU_: ECU

BO_ 300 RenamedMsg: 1 ECU
 SG_ A1 : 0|8@1+ (1,0) [0|0] ""  ECU
"#,
    )
    .unwrap();
    assert!(app.maybe_reload_changed_dbcs().is_some());
    app.refresh_snapshot();
    let db = app.channel_dbc(0).expect("still loaded");
    assert_eq!(
        db.message_name_of((300, false)),
        Some("RenamedMsg"),
        "the reload picked up the external edit"
    );
    std::fs::remove_file(&a).ok();
}

/// A DBC reload that moves a message id refreshes the bound nodes'
/// `$` name→id maps in place (with a `[reload]` line in the node log):
/// a wildcard node keeps reading the renamed message off its new id.
#[test]
fn a_named_read_follows_a_reload_that_moves_the_message_id() {
    let a = std::env::temp_dir().join("roxy_can_reload_named.dbc");
    let dbc = |id: u32| {
        format!(
            "VERSION \"named\"\n\nNS_ :\n\nBS_:\n\nBU_: EngineECU\n\nBO_ {id} EngineStatus: 8 EngineECU\n SG_ RPM : 0|16@1+ (1,0) [0|8000] \"rpm\" EngineECU\n"
        )
    };
    std::fs::write(&a, dbc(0x100)).unwrap();
    let mut app = App::headless();
    app.send(crate::bus::BusCommand::LoadDbc {
        ch: 0,
        paths: vec![a.to_string_lossy().into_owned()],
    });
    app.settle();
    app.send(crate::bus::BusCommand::AddNode {
        name: "watcher".to_string(),
        channel: 0,
        attached: Some((0, "EngineECU".to_string())),
    });
    app.settle();
    let id = app.snap.nodes[0].id;
    app.send(crate::bus::BusCommand::SetNodeSource {
        id,
        source: "on message * { print(EngineStatus.RPM); }".to_string(),
    });
    app.send(crate::bus::BusCommand::SetNodeEnabled { id, on: true });
    app.start_virtual();
    app.settle();

    let frame = |t: u64, fid: u32, rpm: u16| CanFrame {
        t_us: t,
        channel: 0,
        id: fid,
        extended: false,
        len: 2,
        data: {
            let mut d = [0; MAX_CAN_FD_LEN];
            d[..2].copy_from_slice(&rpm.to_le_bytes());
            d
        },
        dir: Direction::Rx,
        flags: FrameFlags::NONE,
    };
    receive(&mut app, 1_000, vec![frame(1_000, 0x100, 1234)]);
    assert!(
        app.snap.nodes[0].log.iter().any(|l| l.starts_with("1234")),
        "the pre-reload read works: {:?}",
        app.snap.nodes[0].log
    );

    std::fs::write(&a, dbc(0x103)).unwrap();
    assert!(app.maybe_reload_changed_dbcs().is_some());
    app.settle();
    receive(&mut app, 2_000, vec![frame(2_000, 0x103, 4321)]);
    let log = &app.snap.nodes[0].log;
    assert!(
        log.iter().any(|l| l.starts_with("4321")),
        "the refreshed map reads the new id: {log:?}"
    );
    assert!(
        log.iter()
            .any(|l| l.contains("[reload]") && l.contains("0x100") && l.contains("0x103")),
        "the reload is explained in the node log: {log:?}"
    );
    std::fs::remove_file(&a).ok();
    app.stop();
}

/// A named handler follows a DBC reload too: the name re-points to the new
/// id without recompiling the script, and the old id stops waking it.
#[test]
fn a_named_handler_follows_a_reload_that_moves_the_message_id() {
    let a = std::env::temp_dir().join("roxy_can_reload_handler.dbc");
    let dbc = |id: u32| {
        format!(
            "VERSION \"named\"\n\nNS_ :\n\nBS_:\n\nBU_: EngineECU\n\nBO_ {id} EngineStatus: 8 EngineECU\n SG_ RPM : 0|16@1+ (1,0) [0|8000] \"rpm\" EngineECU\n"
        )
    };
    std::fs::write(&a, dbc(0x100)).unwrap();
    let mut app = App::headless();
    app.send(crate::bus::BusCommand::LoadDbc {
        ch: 0,
        paths: vec![a.to_string_lossy().into_owned()],
    });
    app.settle();
    app.send(crate::bus::BusCommand::AddNode {
        name: "listener".to_string(),
        channel: 0,
        attached: Some((0, "EngineECU".to_string())),
    });
    app.settle();
    let id = app.snap.nodes[0].id;
    app.send(crate::bus::BusCommand::SetNodeSource {
        id,
        source: "on message EngineStatus { print(\"hit\", frame_byte(0)); }".to_string(),
    });
    app.send(crate::bus::BusCommand::SetNodeEnabled { id, on: true });
    app.start_virtual();
    app.settle();

    let frame = |t: u64, fid: u32| CanFrame {
        t_us: t,
        channel: 0,
        id: fid,
        extended: false,
        len: 1,
        data: {
            let mut d = [0; MAX_CAN_FD_LEN];
            d[0] = 7;
            d
        },
        dir: Direction::Rx,
        flags: FrameFlags::NONE,
    };
    receive(&mut app, 1_000, vec![frame(1_000, 0x100)]);
    assert!(
        app.snap.nodes[0].log.iter().any(|l| l.starts_with("hit")),
        "the named handler fired before the reload: {:?}",
        app.snap.nodes[0].log
    );

    std::fs::write(&a, dbc(0x103)).unwrap();
    assert!(app.maybe_reload_changed_dbcs().is_some());
    app.settle();
    let before = app.snap.nodes[0].log.len();
    receive(&mut app, 2_000, vec![frame(2_000, 0x100)]);
    assert!(
        !app.snap.nodes[0]
            .log
            .iter()
            .skip(before)
            .any(|line| line.starts_with("hit")),
        "the old id no longer wakes the handler: {:?}",
        app.snap.nodes[0].log
    );
    receive(&mut app, 3_000, vec![frame(3_000, 0x103)]);
    assert!(
        app.snap.nodes[0].log.iter().any(|l| l.starts_with("hit")),
        "the renamed id wakes it after the reload: {:?}",
        app.snap.nodes[0].log
    );
    assert!(
        app.snap.nodes[0]
            .log
            .iter()
            .any(|l| l.contains("[reload]") && l.contains("handler")),
        "the handler re-point is explained: {:?}",
        app.snap.nodes[0].log
    );
    std::fs::remove_file(&a).ok();
    app.stop();
}

/// A simulated node whose database gains a message gets the new entry at
/// reload time -- but **inactive**: an external DBC edit must not begin
/// transmitting on its own.
#[test]
fn a_reload_seeds_new_entries_for_simulated_nodes_inactive() {
    let a = std::env::temp_dir().join("roxy_can_role_seed.dbc");
    std::fs::write(
        &a,
        r#"VERSION "role seed"

NS_ :

BS_:

BU_: ECU

BO_ 300 FirstMsg: 1 ECU
 SG_ A1 : 0|8@1+ (1,0) [0|0] ""  ECU
"#,
    )
    .unwrap();

    let mut app = App::headless();
    app.send(crate::bus::BusCommand::LoadDbc {
        ch: 0,
        paths: vec![a.to_string_lossy().into_owned()],
    });
    app.settle();
    app.set_node_role(0, "ECU", NodeRole::Simulated);
    // 新语义：模拟只开放开关，条目默认关——用户显式启用后发送。
    app.send(crate::bus::BusCommand::SetEntryActive {
        ch: 0,
        id: 300,
        on: true,
    });
    app.settle();
    assert_eq!(
        active_ids(&app, 0),
        [300],
        "the enabled entry transmits under the open gate"
    );

    // External edit: the node gains a second message.
    std::fs::write(
        &a,
        r#"VERSION "role seed"

NS_ :

BS_:

BU_: ECU

BO_ 300 FirstMsg: 1 ECU
 SG_ A1 : 0|8@1+ (1,0) [0|0] ""  ECU

BO_ 301 SecondMsg: 1 ECU
 SG_ A2 : 0|8@1+ (1,0) [0|0] ""  ECU
"#,
    )
    .unwrap();
    assert!(app.maybe_reload_changed_dbcs().is_some());
    app.refresh_snapshot();

    let fresh = app
        .tx_list
        .iter()
        .find(|t| t.channel == 0 && t.id == 301)
        .expect("the new message became a generator entry");
    assert!(
        !fresh.active,
        "an external edit must not start traffic by itself"
    );
    assert_eq!(
        active_ids(&app, 0),
        [300],
        "the previously enabled message keeps transmitting"
    );
    std::fs::remove_file(&a).ok();
}

/// A standard and an extended frame sharing one numeric id are two
/// messages as far as every consumer is concerned: two aggregates, two
/// spec watch keys, no verdicts bleeding across the class boundary.
#[test]
fn a_shared_numeric_id_aggregates_as_two_messages() {
    let mut app = App::headless();
    let db = app.channel_dbc(0).expect("sample DBC loaded");
    // Standard 0x100 exists in the sample database; the extended twin
    // deliberately does not, so only the extended class reads Unknown.
    let std_id = db.order.iter().find(|&&(_, ext)| !ext).map(|&(id, _)| id);
    let Some(std_id) = std_id else {
        panic!("sample database has a standard message");
    };
    receive(
        &mut app,
        1_000,
        vec![
            rx_frame(1_000, std_id, 8, FrameFlags::NONE),
            ext_rx_frame(1_000, std_id, 8),
        ],
    );
    assert!(
        app.aggs.contains_key(&(
            crate::model::ClusterId(app.snap.cluster_ids.can[0]),
            std_id,
            false
        )),
        "the standard class aggregates"
    );
    assert!(
        app.aggs.contains_key(&(
            crate::model::ClusterId(app.snap.cluster_ids.can[0]),
            std_id,
            true
        )),
        "the extended class aggregates separately"
    );
    // The standard id is declared, so no verdict; the extended one is
    // unknown to the database and gets exactly one Unknown row.
    assert!(
        !app.spec
            .rows
            .contains_key(&(0, std_id, false, crate::spec::Kind::Unknown)),
        "the declared standard message stays clean"
    );
    assert!(
        app.spec
            .rows
            .contains_key(&(0, std_id, true, crate::spec::Kind::Unknown)),
        "the undeclared extended twin is judged on its own"
    );
}

