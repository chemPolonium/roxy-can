#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_assets::TestBusLoad;

    #[test]
    fn panel_action_commands_deserialize_for_the_core() {
        let cmd: WebCommand = serde_json::from_value(serde_json::json!({
            "type": "run_panel_action",
            "name": "Start sequence",
            "value": 42.5,
        }))
        .expect("panel action command");
        match cmd {
            WebCommand::RunPanelAction { name, value } => {
                assert_eq!(name, "Start sequence");
                assert_eq!(value, Some(42.5));
            }
            _ => panic!("wrong command variant"),
        }
        let plain: WebCommand = serde_json::from_value(serde_json::json!({
            "type": "run_panel_action",
            "name": "Reset",
        }))
        .expect("panel action without a value");
        assert!(matches!(
            plain,
            WebCommand::RunPanelAction { value: None, .. }
        ));
    }

    #[test]
    fn bus_binding_commands_deserialize() {
        let add: WebCommand = serde_json::from_value(serde_json::json!({
            "type": "add_bus_description",
            "bus": 1,
            "kind": "can",
            "path": "Matrix.arxml",
            "cluster": "CanA",
        }))
        .expect("add description command");
        assert!(matches!(
            add,
            WebCommand::AddBusDescription {
                bus: 1,
                ref kind,
                ref path,
                ref cluster,
            } if kind == "can" && path == "Matrix.arxml" && cluster == "CanA"
        ));
        let remove: WebCommand = serde_json::from_value(serde_json::json!({
            "type": "remove_bus_description",
            "bus": 0,
            "kind": "flexray",
            "path": "bus.xml",
        }))
        .expect("remove description command");
        assert!(matches!(
            remove,
            WebCommand::RemoveBusDescription {
                bus: 0,
                ref kind,
                ref path,
                ref cluster,
            } if kind == "flexray" && path == "bus.xml" && cluster.is_empty()
        ));
        let scan: WebCommand = serde_json::from_value(serde_json::json!({
            "type": "scan_arxml",
            "path": "Matrix.arxml",
        }))
        .expect("scan command");
        assert!(matches!(scan, WebCommand::ScanArxml { .. }));
        let bus: WebCommand = serde_json::from_value(serde_json::json!({
            "type": "add_bus",
            "kind": "ethernet",
        }))
        .expect("add bus command");
        assert!(matches!(bus, WebCommand::AddBus { ref kind } if kind == "ethernet"));
        let sample: WebCommand = serde_json::from_value(serde_json::json!({
            "type": "create_sample_project",
            "parent": "C:/tmp",
        }))
        .expect("sample project command with a parent");
        assert!(matches!(
            sample,
            WebCommand::CreateSampleProject { ref parent } if parent == "C:/tmp"
        ));
        let set_mode: WebCommand = serde_json::from_value(serde_json::json!({
            "type": "set_run_mode",
            "mode": "replay",
        }))
        .expect("run mode command");
        assert!(matches!(
            set_mode,
            WebCommand::SetRunMode { ref mode } if mode == "replay"
        ));
        let log: WebCommand = serde_json::from_value(serde_json::json!({
            "type": "open_log",
            "path": "drive.asc",
        }))
        .expect("log command");
        assert!(matches!(
            log,
            WebCommand::OpenLog { play: true, ref path } if path == "drive.asc"
        ));
        let parked: WebCommand = serde_json::from_value(serde_json::json!({
            "type": "open_log",
            "path": "drive.asc",
            "play": false,
        }))
        .expect("parked log command");
        assert!(matches!(parked, WebCommand::OpenLog { play: false, .. }));
        let record_settings: WebCommand = serde_json::from_value(serde_json::json!({
            "type": "set_record_settings",
            "name": "Record/drive",
            "format": 1,
        }))
        .expect("record settings command");
        assert!(matches!(
            record_settings,
            WebCommand::SetRecordSettings { ref name, format: 1 }
                if name == "Record/drive"
        ));
        let replay_folder: WebCommand = serde_json::from_value(serde_json::json!({
            "type": "add_replay_folder",
            "path": "C:/logs",
        }))
        .expect("replay folder command");
        assert!(matches!(
            replay_folder,
            WebCommand::AddReplayFolder { ref path } if path == "C:/logs"
        ));
        let replay_file: WebCommand = serde_json::from_value(serde_json::json!({
            "type": "add_replay_file",
            "path": "C:/logs/drive.asc",
        }))
        .expect("replay file command");
        assert!(matches!(
            replay_file,
            WebCommand::AddReplayFile { ref path } if path == "C:/logs/drive.asc"
        ));
        let remove_replay: WebCommand = serde_json::from_value(serde_json::json!({
            "type": "remove_replay_source",
            "path": "C:/logs",
            "folder": true,
        }))
        .expect("remove replay source");
        assert!(matches!(
            remove_replay,
            WebCommand::RemoveReplaySource { ref path, folder: true }
                if path == "C:/logs"
        ));
        let refresh_replay: WebCommand = serde_json::from_value(serde_json::json!({
            "type": "refresh_replay_sources",
        }))
        .expect("refresh replay sources");
        assert!(matches!(refresh_replay, WebCommand::RefreshReplaySources));
    }

    #[test]
    fn panel_setpoint_writes_deserialize_for_the_core() {
        let cmd: WebCommand = serde_json::from_value(serde_json::json!({
            "type": "panel_write_signal",
            "source": "0:256:EngineSpeed",
            "value": 1234.5,
        }))
        .expect("setpoint command");
        match cmd {
            WebCommand::PanelWriteSignal { source, value } => {
                assert_eq!(source, "0:256:EngineSpeed");
                assert_eq!(value, 1234.5);
            }
            _ => panic!("wrong command variant"),
        }
    }

    /// The seeded session must produce real frames through the headless App; a
    /// broken seed would otherwise leave the web UI looking connected but
    /// permanently empty.
    #[test]
    fn stale_workspace_updates_are_dropped_after_a_project_load() {
        let mut app = App::headless();
        set_web_workspace(&mut app, 0, serde_json::json!({ "tab": "statistics" }));
        assert_eq!(
            app.web_workspace,
            Some(serde_json::json!({ "tab": "statistics" }))
        );

        // A new project loaded behind the browser's back bumps the revision;
        // the previous debounce still carries the old one.
        app.workspace_rev = 2;
        set_web_workspace(&mut app, 1, serde_json::json!({ "tab": "messages" }));
        assert_eq!(
            app.web_workspace,
            Some(serde_json::json!({ "tab": "statistics" })),
            "the late update from the old project must not overwrite the new one"
        );

        set_web_workspace(&mut app, 2, serde_json::json!({ "tab": "signals" }));
        assert_eq!(
            app.web_workspace,
            Some(serde_json::json!({ "tab": "signals" }))
        );
    }

    /// Messages expand into decoded signal children only while open: the
    /// collapsed snapshot pays nothing for a DBC with hundreds of signals.
    #[test]
    fn expanded_messages_carry_decoded_signal_children() {
        let mut app = App::headless();
        seed_test_session(&mut app);
        for _ in 0..40 {
            std::thread::sleep(Duration::from_millis(5));
            app.update();
        }
        let series = VecDeque::new();
        let find = |app: &App| {
            build_state(app, &series, app.frame_rate)
                .messages
                .into_iter()
                .find(|message| message.kind == "can" && message.name == "EngineStatus")
                .expect("EngineStatus aggregate")
        };
        let collapsed = find(&app);
        assert!(collapsed.key.starts_with("can:"));
        assert!(collapsed.signal_count >= 2, "EngineStatus declares signals");
        assert!(
            collapsed.signals.is_empty(),
            "a collapsed row must not decode its signals"
        );

        app.set_message_expanded(&collapsed.key, true);
        let expanded = find(&app);
        assert_eq!(expanded.signals.len(), expanded.signal_count);
        assert!(
            expanded
                .signals
                .iter()
                .any(|signal| signal.name == "EngineSpeed" && !signal.value.is_empty()),
            "the child carries the decoded physical value"
        );

        app.set_message_expanded(&collapsed.key, false);
        assert!(find(&app).signals.is_empty(), "closing drops the children");
    }

    #[test]
    fn a_seeded_session_publishes_real_traffic() {
        let mut app = App::headless();
        seed_test_session(&mut app);
        for _ in 0..40 {
            std::thread::sleep(Duration::from_millis(5));
            app.update();
        }
        let series = VecDeque::new();
        let state = build_state(&app, &series, app.frame_rate);
        assert_eq!(state.channels.len(), 2);
        assert!(state.frames > 0, "the generator ran");
        assert!(!state.messages.is_empty(), "the aggregates saw traffic");
        assert!(!state.stats.is_empty(), "statistics share the aggregates");

        let mut trace = TraceStore::default();
        trace.sync(&app);
        let seed = trace.seed();
        assert!(!seed.is_empty(), "the trace log has rows");
        assert!(
            seed.iter().any(|frame| !frame.name.is_empty()),
            "at least one row decoded through its DBC"
        );
        let before = trace.next_seq;
        for _ in 0..20 {
            std::thread::sleep(Duration::from_millis(5));
            app.update();
        }
        trace.sync(&app);
        assert!(trace.next_seq > before, "later laps append rows");
        assert!(
            !trace.after(before).is_empty(),
            "the client only needs the delta"
        );
    }

    /// The sample project is a normal file-based project: the DBCs sit in
    /// its databases folder, reopen through relative paths, and start on
    /// demand.
    #[test]
    fn the_sample_project_is_file_based_and_reopenable() {
        let dir =
            std::env::temp_dir().join(format!("roxy_can_sample_project_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("temp dir");
        let project = write_sample_project(&dir).expect("sample project written");
        assert!(project.ends_with("roxy-can-sample/roxy-can-sample.rxproj"));
        let databases = project.parent().expect("project folder").join("databases");
        assert!(databases.join("sample.dbc").exists());
        assert!(databases.join("motbus.dbc").exists());
        let json = std::fs::read_to_string(&project).expect("project json");
        let parsed: crate::model::ProjectV2 = serde_json::from_str(&json).expect("project parses");
        let paths: Vec<&str> = parsed
            .clusters
            .iter()
            .flat_map(|cluster| cluster.sources.iter())
            .filter_map(|source| match source {
                crate::model::ClusterSource::Dbc { path } => Some(path.as_str()),
                _ => None,
            })
            .collect();
        assert!(
            paths
                .iter()
                .any(|path| std::path::Path::new(path)
                    == std::path::Path::new("databases/sample.dbc")),
            "relative DBC path: {paths:?}"
        );
        assert!(
            paths
                .iter()
                .any(|path| std::path::Path::new(path)
                    == std::path::Path::new("databases/motbus.dbc")),
            "relative DBC path: {paths:?}"
        );
        assert!(
            paths
                .iter()
                .all(|path| !path.starts_with("builtin:")
                    && !std::path::Path::new(path).is_absolute()),
            "no internal or absolute source in the project"
        );

        let mut app = App::blank();
        app.open_project_path(&project);
        assert_eq!(app.snap.channel_count, 2);
        assert!(
            app.can_paths(0)
                .iter()
                .all(|path| !path.starts_with("builtin:")),
            "the loaded buses point at files"
        );
        assert_eq!(app.graphics[0].signals.len(), 8, "sample lanes carry over");
        assert!(
            app.snap.tx.iter().any(|entry| entry.active),
            "generators stay armed for the Start button"
        );
        assert!(
            !app.snap.measuring,
            "opening the sample does not auto-start"
        );

        app.start_virtual();
        for now in [50_000, 100_000, 150_000, 200_000] {
            app.advance_clock(now);
            app.tick(now);
        }
        assert!(!app.snap.trace.is_empty(), "the sample produces frames");
        assert!(
            TraceStore::seed_rows(&app)
                .iter()
                .any(|frame| !frame.name.is_empty()),
            "the reopened sample decodes signal/message names"
        );

        let reopened = write_sample_project(&dir).expect("existing sample reopens");
        assert_eq!(reopened, project, "the same sample is reused");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Loading a project through the web leaves the workspace clean: the
    /// plot's fine stride is frontend state, not a config edit that a later
    /// save would write back.
    #[test]
    fn a_web_project_load_stays_clean() {
        let path = std::env::temp_dir().join("roxy_web_layout_project.rxproj");
        let mut source = App::headless();
        assert!(source.save_project(Some(path.clone())), "fixture saves");
        let mut app = App::headless();
        prepare_session(&mut app, Some(path.to_str().unwrap()), None);
        assert!(!app.is_dirty(), "web startup must not edit project config");
        app.plot_stride_us = Some(5_000);
        assert!(!app.is_dirty(), "the plot stride is frontend state");
        std::fs::remove_file(&path).ok();
    }

    /// The web form's checks match the core validation, so a bad shape is
    /// rejected at the host boundary instead of becoming a broken variable.
    #[test]
    fn web_sysvar_definitions_follow_core_rules() {
        let ok = checked_sysvar(
            " Demo ".into(),
            "Setpoint".into(),
            100.0,
            Some(0.0),
            Some(500.0),
            " rpm ".into(),
            "target".into(),
            Vec::new(),
        )
        .expect("valid definition");
        assert_eq!(ok.namespace, "Demo");
        assert_eq!(ok.unit, "rpm");
        let labeled = checked_sysvar(
            "Demo".into(),
            "State".into(),
            0.0,
            None,
            None,
            String::new(),
            String::new(),
            vec![crate::bus::SysVarLabelSet {
                id: "status".into(),
                name: "Status".into(),
                is_default: true,
                bands: vec![
                    crate::band::Band {
                        upper: Some(1.0),
                        inclusive: false,
                        text: "Normal".into(),
                        color: Some(1),
                    },
                    crate::band::Band {
                        upper: Some(1.0),
                        inclusive: true,
                        text: "Fault".into(),
                        color: Some(2),
                    },
                    crate::band::Band {
                        upper: None,
                        inclusive: false,
                        text: String::new(),
                        color: None,
                    },
                ],
            }],
        )
        .expect("labels");
        assert_eq!(labeled.label_sets[0].bands[0].text, "Normal");
        assert_eq!(labeled.label_sets[0].bands[1].text, "Fault");
        assert_eq!(labeled.label_sets[0].bands[1].color, Some(2));
        assert!(
            checked_sysvar(
                "Demo".into(),
                "Bad".into(),
                0.0,
                None,
                None,
                String::new(),
                String::new(),
                vec![crate::bus::SysVarLabelSet {
                    id: "bad".into(),
                    name: "Bad".into(),
                    is_default: true,
                    bands: vec![
                        crate::band::Band {
                            upper: Some(1.0),
                            inclusive: false,
                            text: "A".into(),
                            color: None,
                        },
                        crate::band::Band {
                            upper: Some(1.0),
                            inclusive: false,
                            text: "B".into(),
                            color: None,
                        },
                        crate::band::Band {
                            upper: None,
                            inclusive: false,
                            text: String::new(),
                            color: None,
                        },
                    ],
                }],
            )
            .is_err()
        );
        assert!(
            checked_sysvar(
                String::new(),
                "x".into(),
                0.0,
                None,
                None,
                String::new(),
                String::new(),
                Vec::new()
            )
            .is_err()
        );
        assert!(
            checked_sysvar(
                "A::B".into(),
                "x".into(),
                0.0,
                None,
                None,
                String::new(),
                String::new(),
                Vec::new()
            )
            .is_err()
        );
        assert!(
            checked_sysvar(
                "A".into(),
                "x".into(),
                0.0,
                Some(5.0),
                Some(1.0),
                String::new(),
                String::new(),
                Vec::new()
            )
            .is_err()
        );
    }

    /// Definitions and live values reach the web state through the core
    /// manager commands.
    #[test]
    fn the_state_lists_system_variables_and_live_values() {
        let mut app = App::headless();
        app.send(BusCommand::DefineSysVar(
            checked_sysvar(
                "Demo".into(),
                "Setpoint".into(),
                100.0,
                Some(0.0),
                Some(500.0),
                "rpm".into(),
                "target speed".into(),
                Vec::new(),
            )
            .expect("definition"),
        ));
        app.send(BusCommand::SetSysVar {
            namespace: "Demo".into(),
            name: "Setpoint".into(),
            value: 900.0,
        });
        app.settle();
        let state = build_state(&app, &VecDeque::new(), 0.0);
        assert_eq!(state.sysvars.len(), 1);
        let v = &state.sysvars[0];
        assert_eq!(
            (v.namespace.as_str(), v.name.as_str()),
            ("Demo", "Setpoint")
        );
        assert_eq!(v.value, 500.0, "writes clamp into the declared bounds");
        assert_eq!((v.min, v.max), (Some(0.0), Some(500.0)));
        assert_eq!(v.unit, "rpm");
    }

    /// Duplicating a rule copies its whole shape -- condition, action and
    /// limits -- while the copy starts disarmed with a clean latch and zero
    /// fire count, exactly like a freshly added rule.
    #[test]
    fn a_duplicated_trigger_rule_copies_the_shape_but_starts_clean() {
        let mut app = App::headless();
        app.apply_trigger_cmds(vec![
            TriggerCmd::Add(TriggerKind::Id),
            TriggerCmd::Edit {
                index: 0,
                cond: TriggerCond::ErrorFrame { ch: 1 },
                action: TriggerAction::InsertMarker,
                cooldown_us: 5_000_000,
                max_fires: 3,
                pre_frames: Some(12),
                post_frames: Some(4),
            },
            TriggerCmd::SetEnabled { index: 0, on: true },
            TriggerCmd::Duplicate(0),
        ]);
        app.settle();
        let state = build_state(&app, &VecDeque::new(), 0.0);
        assert_eq!(state.triggers.len(), 2);
        let first = state.triggers[0].clone();
        let copy = state.triggers[1].clone();
        assert!(first.enabled);
        assert!(!copy.enabled, "the copy starts disarmed");
        assert_eq!(copy.kind, first.kind);
        assert_eq!(copy.ch, first.ch);
        assert_eq!(copy.action_kind, first.action_kind);
        assert_eq!(copy.cooldown_ms, first.cooldown_ms);
        assert_eq!(copy.max_fires, first.max_fires);
        assert_eq!(copy.pre_frames, first.pre_frames);
        assert_eq!(copy.post_frames, first.post_frames);
        assert_eq!(copy.fired, 0, "the copy has no fire history");
    }

    /// The generator's replay blocks duplicate as disabled copies that keep
    /// the log, node filter and id filter of the original.
    #[test]
    fn a_duplicated_replay_block_is_a_disabled_copy() {
        let mut app = App::headless();
        app.add_replay_block(
            0,
            "restbus".to_string(),
            "log.asc".to_string(),
            Some("Engine".to_string()),
            None,
        );
        app.settle();
        let id = app.snap.blocks.first().expect("block exists").id;
        app.duplicate_replay_block(id);
        app.settle();
        assert_eq!(app.snap.blocks.len(), 2);
        let copy = app.snap.blocks[1].clone();
        assert_ne!(copy.id, id, "the copy mints its own id");
        assert!(copy.name.contains("copy"));
        assert!(!copy.enabled, "the copy starts disabled like a new block");
        assert_eq!(copy.path, "log.asc");
        assert_eq!(
            copy.node_filter.as_deref(),
            Some("Engine"),
            "the node filter carries over for id-filter variants"
        );
    }

    /// Monitor rows carry their canonical key so the web can address them
    /// for row reordering.
    #[test]
    fn the_state_lists_monitor_rows_with_their_keys() {
        let mut app = App::headless();
        let key = SigKey::can_in(crate::model::ClusterId(1), 0x100, false, "EngineSpeed");
        app.set_win_signal(crate::workspace::PopupTarget::Monitor, key.clone(), true);
        app.settle();
        let state = build_state(&app, &VecDeque::new(), 0.0);
        assert_eq!(state.monitor.len(), 1);
        assert_eq!(
            state.monitor[0].key,
            series_key(&key),
            "the row exposes the key text the move command resolves"
        );
    }

    /// Monitor rows carry their identity, live value and coloring rule to
    /// the web; edits stay in row state and removal unsubscribes the signal.
    #[test]
    fn the_state_lists_monitor_rows_and_edits_them() {
        let key = SigKey::can_in(crate::model::ClusterId(1), 0x100, false, "EngineSpeed");
        let mut app = App::headless();
        app.set_win_signal(crate::workspace::PopupTarget::Monitor, key.clone(), true);
        app.settle();
        let state = build_state(&app, &VecDeque::new(), 0.0);
        assert_eq!(state.monitor.len(), 1);
        assert_eq!(state.monitor[0].name, "EngineSpeed");
        assert_eq!(state.monitor[0].label, "EngineSpeed");
        assert_eq!(
            state.monitor[0].path, "EngineStatus / EngineSpeed",
            "monitor rows carry the frame path"
        );
        assert_eq!(state.monitor[0].digits, 1, "core default");
        app.set_monitor_columns(2);
        let state = build_state(&app, &VecDeque::new(), 0.0);
        assert_eq!(state.monitor_columns, 2);

        edit_monitor_row(
            &mut app,
            &series_key(&key),
            Some(2),
            Some(vec![
                crate::band::Band {
                    upper: Some(87.5),
                    inclusive: false,
                    text: String::new(),
                    color: Some(3),
                },
                crate::band::Band::final_band(Some(5)),
            ]),
        )
        .unwrap();
        let state = build_state(&app, &VecDeque::new(), 0.0);
        let row = &state.monitor[0];
        assert_eq!(row.digits, 2);
        assert_eq!(row.bands[0].upper, Some(87.5));
        assert_eq!(row.bands[0].color, Some(3));
        assert_eq!(row.bands[1].color, Some(5));

        assert!(
            edit_monitor_row(
                &mut app,
                &series_key(&key),
                None,
                Some(vec![
                    crate::band::Band {
                        upper: Some(10.0),
                        inclusive: false,
                        text: String::new(),
                        color: Some(1),
                    },
                    crate::band::Band {
                        upper: Some(5.0),
                        inclusive: false,
                        text: String::new(),
                        color: Some(2),
                    },
                    crate::band::Band::final_band(Some(0)),
                ]),
            )
            .is_err(),
            "descending bands are rejected"
        );
        let state = build_state(&app, &VecDeque::new(), 0.0);
        assert_eq!(
            state.monitor[0].bands[0].upper,
            Some(87.5),
            "a failed edit is atomic"
        );

        app.set_win_signal(crate::workspace::PopupTarget::Monitor, key, false);
        app.settle();
        let state = build_state(&app, &VecDeque::new(), 0.0);
        assert!(state.monitor.is_empty());
    }

    /// The value table mirrors the first Data window: selected rows, their
    /// live strings and the visibility toggle all reach the web state.
    #[test]
    fn the_state_lists_data_rows_and_visibility() {
        let key = SigKey::can_in(crate::model::ClusterId(1), 0x100, false, "EngineSpeed");
        let mut app = App::headless();
        app.new_data_window();
        app.set_win_signal(crate::workspace::PopupTarget::Data(0), key.clone(), true);
        app.settle();
        let state = build_state(&app, &VecDeque::new(), 0.0);
        assert_eq!(state.data.len(), 1);
        let row = &state.data[0];
        assert_eq!(row.name, "EngineSpeed");
        assert_eq!(row.path, "EngineStatus / EngineSpeed");
        assert!(row.visible);
        assert!((0.0..=1.0).contains(&row.frac));

        app.data_windows[0].signals[0].visible = false;
        let state = build_state(&app, &VecDeque::new(), 0.0);
        assert!(!state.data[0].visible);

        app.set_win_signal(crate::workspace::PopupTarget::Data(0), key, false);
        app.settle();
        let state = build_state(&app, &VecDeque::new(), 0.0);
        assert!(state.data.is_empty());
    }

    /// The seeded session's generator sources reach the web with their shape and
    /// timing, not just a count, so the editor can round-trip them.
    #[test]
    fn the_state_lists_generator_sources() {
        let mut app = App::headless();
        seed_test_session(&mut app);
        for _ in 0..20 {
            app.update();
        }
        let state = build_state(&app, &VecDeque::new(), 0.0);
        let src = state
            .tx
            .iter()
            .flat_map(|entry| &entry.srcs)
            .find(|src| src.name == "EngineSpeed")
            .expect("the seeded session drives EngineSpeed");
        assert_eq!(src.kind, "Sine");
        assert!(src.period_us > 0);
        assert!(src.hi > src.lo);
    }

    /// Visible samples collapse into one band per change of the quantized
    /// value; empty columns carry the last state forward.
    #[test]
    fn state_bands_split_on_value_changes() {
        let (current, bands) = state_bands_from(
            Some(0.0),
            &[(10_000, 0.0), (20_000, 1.0), (30_000, 1.0)],
            0,
            40_000,
        );
        assert_eq!(current, "1");
        assert_eq!(bands.len(), 2);
        assert_eq!(bands[0].start, 0.0);
        assert_eq!(bands[0].end, 0.5);
        assert_eq!(bands[0].label, "0");
        assert_eq!(bands[1].start, 0.5);
        assert_eq!(bands[1].end, 1.0);
        assert_eq!(bands[1].label, "1");
    }

    /// The first State Tracker window feeds the web: identity, bus and the
    /// window length reach the state, so the tab can draw and resize it.
    #[test]
    fn the_state_lists_state_tracker_rows_and_window() {
        let key = SigKey::can_in(crate::model::ClusterId(1), 0x100, false, "EngineSpeed");
        let mut app = App::headless();
        app.new_state_window();
        app.set_win_signal(crate::workspace::PopupTarget::State(0), key.clone(), true);
        app.settle();
        let state = build_state(&app, &VecDeque::new(), 0.0);
        assert_eq!(state.state_rows.len(), 1);
        assert_eq!(state.state_rows[0].name, "EngineSpeed");
        assert_eq!(
            state.state_rows[0].key, "sig:can:1:256:false:EngineSpeed",
            "pickers match the catalog's canonical key"
        );
        assert_eq!(state.state_rows[0].bus, "CAN1");
        assert_eq!(state.state_window_s, 20.0);

        app.set_win_signal(crate::workspace::PopupTarget::State(0), key, false);
        app.settle();
        let state = build_state(&app, &VecDeque::new(), 0.0);
        assert!(state.state_rows.is_empty());
    }

    /// Bus nodes carry their role and generator-entry count, so the Buses
    /// dialog can switch simulation per ECU instead of all-or-nothing.
    #[test]
    fn the_state_lists_bus_nodes_and_roles() {
        let mut app = App::headless();
        let node = app.snap.channels[0]
            .dbc
            .as_ref()
            .and_then(|db| db.nodes.first().cloned())
            .expect("sample DBC nodes");
        app.set_node_role(0, &node, crate::app::NodeRole::Simulated);
        app.settle();
        let state = build_state(&app, &VecDeque::new(), 0.0);
        let bus = state
            .buses
            .iter()
            .find(|bus| bus.index == 0)
            .expect("first bus");
        let row = bus
            .can
            .as_ref()
            .expect("CAN detail")
            .nodes
            .iter()
            .find(|row| row.name == node)
            .expect("the node is listed");
        assert!(row.simulated);
        assert!(row.entries > 0, "a simulated node owns generator entries");

        app.set_node_role(0, &node, crate::app::NodeRole::Absent);
        app.settle();
        let state = build_state(&app, &VecDeque::new(), 0.0);
        let bus = state.buses.iter().find(|bus| bus.index == 0).unwrap();
        let row = bus
            .can
            .as_ref()
            .expect("CAN detail")
            .nodes
            .iter()
            .find(|row| row.name == node)
            .unwrap();
        assert!(!row.simulated);
    }

    /// The help endpoint serves the language table verbatim.
    #[test]
    fn the_script_help_lists_every_callable() {
        let items = script_help_items();
        assert_eq!(items.len(), crate::script::SCRIPT_HELP.len());
        assert!(items.iter().any(|item| item.name == "derived"));
        assert!(items.iter().any(|item| item.name == "set_sig"));
        assert!(items.iter().all(|item| !item.usage.trim().is_empty()));
    }

    /// Node scripts reach the web with their source, binding and summary;
    /// the editor cache only refreshes when the fingerprint changes.
    #[test]
    fn scripts_reach_the_web_with_source_and_runtime_state() {
        let mut app = App::headless();
        app.send(BusCommand::AddNode {
            name: "Node A".to_string(),
            channel: 0,
            attached: Some((0, "EngineECU".to_string())),
        });
        app.settle();
        let id = app.snap.nodes[0].id;
        let before = scripts_signature(&app);
        app.send(BusCommand::SetNodeSource {
            id,
            source: "on start { print(\"hi\"); } on panel_action(\"Go\") { }".to_string(),
        });
        app.settle();
        assert_ne!(
            scripts_signature(&app),
            before,
            "source edits refresh /nodes"
        );
        let snapshots = scripts_snapshot(&app);
        assert_eq!(snapshots.len(), 1);
        assert_eq!(snapshots[0].name, "Node A");
        assert!(snapshots[0].source.contains("print"));
        assert_eq!(snapshots[0].attached_node.as_deref(), Some("EngineECU"));
        assert_eq!(
            snapshots[0].panel_actions,
            ["Go"],
            "declared panel actions reach the editor"
        );
        let state = build_state(&app, &VecDeque::new(), 0.0);
        assert_eq!(state.scripts.len(), 1);
        assert_eq!(state.scripts[0].id, id);
        assert!(state.scripts[0].enabled);
        assert_eq!(state.scripts[0].panel_actions, ["Go"]);
    }

    /// The web export paths write the established formats: ASC for trace and
    /// CSV headers for Messages/Statistics/Data/State.
    #[test]
    fn web_exports_write_standard_formats() {
        let mut app = App::headless();
        seed_test_session(&mut app);
        for _ in 0..20 {
            app.update();
        }
        chart_select(&mut app, 0, 0x100, false, "EngineSpeed");
        let dir = std::env::temp_dir();
        let files = [
            (
                "trace",
                dir.join("roxy_web_export.asc"),
                "Begin TriggerBlock",
            ),
            (
                "messages",
                dir.join("roxy_web_export_messages.csv"),
                "bus,id,name,dir,count",
            ),
            (
                "stats",
                dir.join("roxy_web_export_stats.csv"),
                "bus,id,name,count",
            ),
            (
                "data",
                dir.join("roxy_web_export_data.csv"),
                "bus,signal,value,unit",
            ),
            (
                "graphics",
                dir.join("roxy_web_export_graphics.csv"),
                "time_us,bus,signal,value",
            ),
            ("spec", dir.join("roxy_web_export_spec.csv"), "# database"),
        ];
        for (kind, path, header) in &files {
            export_file(&mut app, kind, path.to_str().unwrap(), None);
            let text = std::fs::read_to_string(path).unwrap_or_default();
            assert!(
                text.contains(header),
                "{kind} export should contain {header:?}, got {:?}",
                text.chars().take(60).collect::<String>()
            );
            std::fs::remove_file(path).ok();
        }
    }

    /// A command that cannot run reports on the bar AND lands in the Write
    /// ring: the bar holds one line and the next action erases it, so the
    /// ring is the only place a minute-old failure can still be read.
    #[test]
    fn a_failed_command_lands_in_the_write_ring() {
        let mut app = App::headless();
        export_file(&mut app, "trace", "   ", None);
        app.settle();
        assert!(app.status.contains("export needs a path"));
        assert!(
            app.snap
                .write
                .iter()
                .any(|line| line.kind == crate::bus::WriteKind::Error
                    && line.text.contains("export needs a path")),
            "the failure is still readable in Write"
        );
    }

    /// Chart selection is also Graphics-window state, so it survives a
    /// project save and seeds the lanes when the project is opened again.
    #[test]
    fn chart_selection_round_trips_through_the_graphics_window() {
        let mut app = App::headless();
        chart_select(&mut app, 0, 0x100, false, "EngineSpeed");
        let key = SigKey::can_in(crate::model::ClusterId(1), 0x100, false, "EngineSpeed");
        assert!(
            app.graphics
                .first()
                .is_some_and(|window| window.signals.iter().any(|row| row.key == key))
        );
        assert!(app.config_snapshot().contains("EngineSpeed"));

        let mut seeded = SignalStore::default();
        seed_chart_selection(&mut app, &mut seeded);
        assert!(seeded.is_selected(&key), "a load repopulates the lanes");

        chart_unselect(&mut app, &key);
        assert!(
            app.graphics
                .first()
                .is_none_or(|window| window.signals.iter().all(|row| row.key != key))
        );
    }

    /// A saved project can carry FlexRay and Ethernet lanes too; restoring
    /// only CAN keys would silently drop them from the chart on every load.
    #[test]
    fn a_load_restores_flexray_and_ethernet_chart_lanes() {
        let mut app = App::headless();
        // A lane names its cluster, so the test has to mint the two clusters a
        // loaded project would have carried.
        let fr_cluster = app.cluster_ids.allocate();
        app.fr_cluster_ids.insert(0, fr_cluster);
        let eth_cluster = app.cluster_ids.allocate();
        app.eth_cluster_ids.insert(0, eth_cluster);
        let fr = app.fr_key(0, 13, "CarSpeed".to_string());
        let eth = app.eth_key(0, "SpeedFrame".to_string(), "Speed".to_string());
        chart_select_key(&mut app, fr.clone());
        chart_select_key(&mut app, eth.clone());

        let mut store = SignalStore::default();
        seed_chart_selection(&mut app, &mut store);
        assert!(store.is_selected(&fr), "FlexRay lane restored");
        assert!(store.is_selected(&eth), "Ethernet lane restored");
    }

    /// A chart drag must reorder both the session lanes and the Graphics
    /// window, because only the window order reaches a saved project.
    #[test]
    fn moving_a_chart_lane_updates_store_and_graphics_order() {
        let mut app = App::headless();
        let keys = [
            SigKey::can_in(crate::model::ClusterId(1), 0x100, false, "EngineSpeed"),
            SigKey::can_in(crate::model::ClusterId(1), 0x200, false, "VehicleSpeed"),
            SigKey::can_in(crate::model::ClusterId(1), 0x320, false, "BrakePressure"),
        ];
        let mut store = SignalStore::default();
        for key in &keys {
            chart_select_key(&mut app, key.clone());
            store.select_key(&app, key.clone());
        }

        chart_move(&mut app, &mut store, &keys[0], 2);

        let expected = vec![keys[1].clone(), keys[2].clone(), keys[0].clone()];
        assert_eq!(store.keys(), expected);
        let window = app.graphics.first().expect("a graphics window");
        let saved: Vec<SigKey> = window
            .signals
            .iter()
            .map(|signal| signal.key.clone())
            .collect();
        assert_eq!(saved, expected);
        assert!(!store.primed, "the next sync reseeds in the new order");
    }

    /// The lane context menu can add a signal to each observer and remove it
    /// from the chart without disturbing observers that already watch it.
    #[test]
    fn chart_signal_actions_reach_observers_and_remove_only_the_lane() {
        let mut app = App::headless();
        let key = SigKey::can_in(crate::model::ClusterId(1), 0x100, false, "EngineSpeed");
        let mut store = SignalStore::default();
        chart_select_key(&mut app, key.clone());
        store.select_key(&app, key.clone());

        for action in ["monitor", "data", "state"] {
            assert!(chart_signal_action(&mut app, &mut store, &key, action));
        }
        assert!(app.monitor_view.rows.iter().any(|row| row.key == key));
        assert!(
            app.data_windows
                .first()
                .is_some_and(|window| window.signals.iter().any(|row| row.key == key))
        );
        assert!(
            app.state_trackers
                .first()
                .is_some_and(|window| window.signals.iter().any(|row| row.key == key))
        );

        assert!(chart_signal_action(&mut app, &mut store, &key, "remove"));
        assert!(!store.is_selected(&key));
        assert!(
            app.graphics
                .first()
                .is_none_or(|window| window.signals.iter().all(|row| row.key != key))
        );
        assert!(
            app.monitor_view.rows.iter().any(|row| row.key == key),
            "removing a chart lane keeps observer rows"
        );
        assert!(!chart_signal_action(&mut app, &mut store, &key, "nope"));
    }

    #[test]
    fn monitor_band_set_binding_can_be_set_and_cleared() {
        let mut app = App::headless();
        let key = SigKey::can_in(crate::model::ClusterId(1), 0x100, false, "EngineSpeed");
        app.monitor_view.rows.push(crate::app::MonitorRow {
            key: key.clone(),
            label: key.name().to_string(),
            ..Default::default()
        });
        let id = key.text();

        super::bind_monitor_band_set(&mut app, &id, Some("speed".to_string()));
        assert_eq!(app.monitor_view.rows[0].band_set_id.as_deref(), Some("speed"));

        super::bind_monitor_band_set(&mut app, &id, None);
        assert_eq!(app.monitor_view.rows[0].band_set_id, None);

        super::bind_monitor_band_set(&mut app, &id, Some("speed".to_string()));
        super::bind_monitor_band_set(&mut app, &id, Some(String::new()));
        assert_eq!(app.monitor_view.rows[0].band_set_id, None);
    }

    /// The Bus Statistics tab reads the same BusLoad numbers as the core
    /// model: load, send distance, bursts and per-class rates.
    #[test]
    fn the_state_lists_bus_load_statistics() {
        let mut app = App::headless();
        seed_test_session(&mut app);
        for _ in 0..30 {
            app.update();
        }
        let state = build_state(&app, &VecDeque::new(), 0.0);
        assert!(!state.bus_stats.is_empty());
        let bus = &state.bus_stats[0];
        assert_eq!(bus.name, "CAN1");
        assert!(!bus.load_history.is_empty(), "the sparkline has buckets");
        assert!(
            bus.rows.iter().any(|row| row.label == "Busload [%]"),
            "the load row is present"
        );
        assert!(
            bus.rows
                .iter()
                .any(|row| row.label == "Errorframes [total]"),
            "error counters are present"
        );
    }

    /// The Specification tab reads the project's tolerance/grace, the
    /// periodic-message count and the latched verdict rows.
    #[test]
    fn the_state_lists_specification_controls() {
        let mut app = App::headless();
        seed_test_session(&mut app);
        for _ in 0..20 {
            app.update();
        }
        let state = build_state(&app, &VecDeque::new(), 0.0);
        assert!(state.spec_periodic > 0, "the DBC declares periodic frames");
        assert_eq!(state.spec_tolerance, app.spec_tol_pct);
        assert_eq!(state.spec_grace, app.spec_grace);
        assert!(
            state.spec_rows.is_empty(),
            "a clean session has no verdicts"
        );
    }

    /// Custom cuts replace the quantized value with a band index and use the
    /// band's own name, like the core Value Definition model.
    #[test]
    fn state_custom_bands_use_cut_names() {
        let rule = crate::observe::StateRule {
            band_set_id: None,
            bands: vec![
                crate::band::Band {
                    upper: Some(1.0),
                    inclusive: false,
                    text: "low".to_string(),
                    color: None,
                },
                crate::band::Band {
                    upper: None,
                    inclusive: false,
                    text: "high".to_string(),
                    color: None,
                },
            ],
        };
        let (current, bands) =
            state_bands_custom(Some(0.0), &[(10_000, 0.0), (20_000, 2.0)], 0, 40_000, &rule);
        assert_eq!(current, "high");
        assert_eq!(bands.len(), 2);
        assert_eq!(bands[0].label, "low");
        assert_eq!(bands[1].label, "high");
    }

    /// A saved custom rule reaches the state and can be removed again.
    #[test]
    fn state_rules_reach_the_web_and_clear() {
        let key = SigKey::can_in(crate::model::ClusterId(1), 0x100, false, "EngineSpeed");
        let mut app = App::headless();
        app.new_state_window();
        app.set_win_signal(crate::workspace::PopupTarget::State(0), key.clone(), true);
        app.settle();
        set_state_rule(
            &mut app,
            &key,
            vec![
                crate::band::Band {
                    upper: Some(1000.0),
                    inclusive: false,
                    text: "idle".into(),
                    color: None,
                },
                crate::band::Band {
                    upper: Some(5000.0),
                    inclusive: false,
                    text: "cruise".into(),
                    color: None,
                },
                crate::band::Band {
                    upper: None,
                    inclusive: false,
                    text: "fast".into(),
                    color: None,
                },
            ],
            None,
        );
        let state = build_state(&app, &VecDeque::new(), 0.0);
        let rule = state.state_rows[0].rule.as_ref().expect("rule saved");
        assert_eq!(rule.bands[1].upper, Some(5000.0));
        assert_eq!(rule.bands[2].text, "fast");

        clear_state_rule(&mut app, &key);
        let state = build_state(&app, &VecDeque::new(), 0.0);
        assert!(state.state_rows[0].rule.is_none());
    }

    /// FR/Eth descriptions loaded into the core appear as bus sections with
    /// their cluster identity and frame counts.
    #[test]
    fn the_state_lists_flexray_and_ethernet_buses() {
        let mut app = App::headless();
        let fr = app
            .load_cluster_description(crate::test_assets::path("PowerTrain_v2.xml"), None)
            .expect("FIBEX fixture loads");
        let eth = app
            .load_eth_description(crate::test_assets::path("PowerTrain.arxml"), None)
            .expect("ARXML fixture loads");
        let state = build_state(&app, &VecDeque::new(), 0.0);
        let fr_bus = state
            .buses
            .iter()
            .find(|bus| bus.kind == "flexray" && bus.index == fr)
            .expect("FlexRay bus listed");
        assert!(!fr_bus.name.is_empty());
        assert!(fr_bus.frames > 0);
        let eth_bus = state
            .buses
            .iter()
            .find(|bus| bus.kind == "ethernet" && bus.index == eth)
            .expect("Ethernet bus listed");
        assert!(!eth_bus.name.is_empty());
        assert!(
            eth_bus
                .descriptions
                .first()
                .is_some_and(|description| description.path.contains("PowerTrain"))
        );
        assert!(fr_bus.id > 0);
        assert!(eth_bus.id > 0);
        assert!(state.buses.iter().filter(|bus| bus.kind == "can").count() > 0);
    }

    /// Replay rows from the FlexRay ring join the web trace alongside CAN,
    /// with a slot address and the cluster's bus label.
    #[test]
    fn the_trace_store_merges_flexray_rows() {
        let mut app = App::headless();
        app.load_cluster_description(crate::test_assets::path("PowerTrain.arxml"), Some(0));
        app.load_cluster_description(crate::test_assets::path("PowerTrain_v2.xml"), Some(1));
        prepare_session(&mut app, None, Some(crate::test_assets::logging_blf_two()));
        for _ in 0..40 {
            std::thread::sleep(Duration::from_millis(5));
            app.update();
        }
        let mut store = TraceStore::default();
        store.sync(&app);
        let rows = store.seed();
        let fr = rows
            .iter()
            .find(|row| row.kind == "fr")
            .expect("FlexRay rows reach the web trace");
        assert!(fr.addr.starts_with("slot "));
        assert!(fr.bus_name.starts_with("FR"));
        assert!(!fr.data.is_empty());
        assert!(
            rows.iter()
                .any(|row| row.kind == "fr" && !row.name.is_empty()),
            "at least one row resolves through its cluster description"
        );
    }

    /// FlexRay generator rows reach the web with their slot, sender and
    /// payload, so the Generator dialog can edit them like CAN entries.
    #[test]
    fn the_state_lists_flexray_generator_entries() {
        let mut app = App::headless();
        let bus = app
            .load_cluster_description(crate::test_assets::path("PowerTrain.arxml"), None)
            .expect("cluster loads");
        app.add_fr_tx(bus, 13);
        app.settle();
        let state = build_state(&app, &VecDeque::new(), 0.0);
        assert!(!state.fr_tx.is_empty(), "the FR entry is listed");
        let row = state.fr_tx.iter().find(|row| row.slot == 13).unwrap();
        assert_eq!(row.bus, bus);
        assert!(row.bus_name.starts_with("FR"));
        assert!(!row.node.is_empty(), "the description names the sender");
    }

    /// The signal catalog exposes the loaded FlexRay cluster's slots and
    /// signals, and a selected FR key keeps the `fr:` series identity.
    #[test]
    fn the_catalog_offers_flexray_signals() {
        let mut app = App::headless();
        let bus = app
            .load_cluster_description(crate::test_assets::path("PowerTrain.arxml"), None)
            .expect("cluster loads");
        let catalog = build_catalog(&app);
        let fr = catalog
            .channels
            .iter()
            .find(|channel| channel.kind == "fr" && channel.index == bus)
            .expect("FR catalog channel");
        assert!(fr.name.starts_with("FR"));
        let message = fr
            .messages
            .iter()
            .find(|message| message.id == 13)
            .expect("slot 13 message");
        let signal = message.signals.first().expect("a signal");
        assert!(signal.key.starts_with("sig:fr:"));

        let mut store = SignalStore::default();
        let key = app.fr_key(bus, 13, signal.name.clone());
        app.subscribe(key.clone());
        store.select_key(&app, key);
        store.sync(&app);
        let seeded = store.seed();
        assert!(seeded[0].key.starts_with("sig:fr:"));
        assert_eq!(seeded[0].bus, fr.name);
    }

    /// Ethernet signals enter the same picker and observer tables. The
    /// catalog names the frame because a matrix has no numeric id, and the
    /// series key carries that name end to end.
    #[test]
    fn the_catalog_and_observers_offer_ethernet_signals() {
        use crate::workspace::PopupTarget;
        let mut app = App::headless();
        let bus = app
            .load_eth_description(crate::test_assets::eth_arxml(), Some(0))
            .expect("Ethernet matrix loads");
        let catalog = build_catalog(&app);
        let eth = catalog
            .channels
            .iter()
            .find(|channel| channel.kind == "eth" && channel.index == bus)
            .expect("Ethernet catalog channel");
        assert_eq!(eth.name, "ETH0");
        let message = eth.messages.first().expect("frame");
        assert_eq!(message.name, "EthFrame");
        assert_eq!(message.frame.as_deref(), Some("EthFrame"));
        let signal = message.signals.first().expect("signal");
        assert_eq!(
            signal.key,
            format!("sig:eth:{}:EthFrame:Sig", app.snap.cluster_ids.eth[&bus])
        );

        let key = app.eth_key(bus, "EthFrame".to_string(), "Sig".to_string());
        app.set_win_signal(PopupTarget::Monitor, key.clone(), true);
        app.new_data_window();
        app.set_win_signal(PopupTarget::Data(0), key.clone(), true);
        app.new_state_window();
        app.set_win_signal(PopupTarget::State(0), key.clone(), true);
        let state = build_state(&app, &std::collections::VecDeque::new(), 60.0);
        let monitor = state.monitor.first().expect("monitor row");
        assert_eq!(monitor.kind, "eth");
        assert_eq!(monitor.frame.as_deref(), Some("EthFrame"));
        assert_eq!(monitor.bus, "ETH0");
        assert!(!monitor.has_value, "no sample has arrived yet");
        assert_eq!(state.data[0].kind, "eth");
        assert_eq!(state.data[0].value, "-", "no samples yet, so dashes");
        assert_eq!(state.data[0].min, "-");
        assert_eq!(state.state_rows[0].kind, "eth");

        let mut store = SignalStore::default();
        store.select_key(&app, key.clone());
        store.sync(&app);
        assert_eq!(
            store.seed()[0].key,
            format!("sig:eth:{}:EthFrame:Sig", app.snap.cluster_ids.eth[&bus])
        );
        assert_eq!(store.seed()[0].bus, "ETH0");
    }

    /// The observer tables share one address shape: a FlexRay key keeps its
    /// cluster in `ch` and its slot in `id`, so Monitor/Data/State can list
    /// both bus kinds without a second set of DTOs.
    #[test]
    fn observers_list_flexray_rows_next_to_can() {
        use crate::workspace::PopupTarget;
        let mut app = App::headless();
        let bus = app
            .load_cluster_description(crate::test_assets::path("PowerTrain.arxml"), Some(0))
            .expect("cluster loads");
        let key = app.fr_key(bus, 13, "CarSpeed".to_string());
        app.set_win_signal(PopupTarget::Monitor, key.clone(), true);
        app.new_data_window();
        app.set_win_signal(PopupTarget::Data(0), key.clone(), true);
        app.new_state_window();
        app.set_win_signal(PopupTarget::State(0), key.clone(), true);

        let state = build_state(&app, &std::collections::VecDeque::new(), 60.0);
        let monitor = state.monitor.first().expect("monitor row");
        assert_eq!(monitor.kind, "fr");
        assert_eq!((monitor.ch, monitor.id, monitor.ext), (bus, 13, false));
        assert_eq!(monitor.name, "CarSpeed");
        assert_eq!(monitor.bus, "FR0");
        let data = state.data.first().expect("data row");
        assert_eq!(data.kind, "fr");
        assert_eq!((data.ch, data.id), (bus, 13));
        let state_row = state.state_rows.first().expect("state row");
        assert_eq!(state_row.kind, "fr");
        assert_eq!((state_row.ch, state_row.id), (bus, 13));
    }

    #[test]
    fn uploads_keep_only_a_safe_file_name() {
        assert_eq!(upload_name("C:\\logs\\drive.asc"), "drive.asc");
        assert_eq!(upload_name("/tmp/project.rxproj"), "project.rxproj");
        assert_eq!(upload_name("a b:c?.dbc"), "a_b_c_.dbc");
        assert_eq!(upload_name(".."), "upload");
        assert_eq!(upload_name(""), "upload");
        assert_eq!(upload_name("\u{4e2d}\u{6587}.dbc"), "__.dbc");
    }

    #[test]
    fn bare_file_names_land_in_the_download_folder() {
        let folder = download_dir();
        let path = std::path::PathBuf::from(download_file_path("export.csv"));
        assert_eq!(path.parent(), Some(folder.as_path()));
        assert_eq!(
            path.file_name().and_then(|n| n.to_str()),
            Some("export.csv")
        );
        assert_eq!(download_file_path("logs/export.csv"), "logs/export.csv");
        assert_eq!(download_file_path(""), "");
        let nested = to_download_dir("../../secret.csv");
        assert_eq!(nested.parent(), Some(folder.as_path()));
        assert_eq!(
            nested.file_name().and_then(|n| n.to_str()),
            Some("secret.csv")
        );
    }

    #[test]
    fn flexray_rows_report_the_attached_port() {
        let mut app = App::headless();
        app.load_cluster_description(crate::test_assets::path("PowerTrain.arxml"), Some(0));
        app.hw.attach_fr_mock(0, 5);
        app.refresh_snapshot();
        let state = build_state(&app, &std::collections::VecDeque::new(), 60.0);
        let bus = state
            .buses
            .iter()
            .find(|bus| bus.kind == "flexray")
            .expect("FR bus row");
        let detail = bus.flexray.as_ref().expect("FlexRay detail");
        assert!(detail.attached);
        assert_eq!(detail.channel, 5);
    }

    #[test]
    fn replay_rows_merge_census_and_configured_channels() {
        use crate::replay_map::{LogBusKind, ReplayMap};
        let mut map = ReplayMap::default();
        map.set(LogBusKind::Can, 2, None);
        map.set(LogBusKind::Can, 4, Some(crate::model::ClusterId(1)));
        map.set(LogBusKind::Can, 5, Some(crate::model::ClusterId(2)));
        let rows = replay_rows(&[(2, 5), (0, 9)], &map, LogBusKind::Can, |raw| {
            if raw == 2 { None } else { Some(1) }
        });
        assert_eq!(
            rows.iter()
                .map(|row| (row.raw, row.count, row.target, row.configured))
                .collect::<Vec<_>>(),
            [
                (0, 9, Some(1), false),
                (2, 5, None, true),
                (4, 0, Some(1), true),
                (5, 0, Some(1), true),
            ],
            "census and remembered rows share one sorted table"
        );
        assert_eq!(
            crate::replay_map::LogBusKind::parse("FlexRay"),
            Some(crate::replay_map::LogBusKind::Fr)
        );
        assert!(crate::replay_map::LogBusKind::parse("lin").is_none());
    }

    #[test]
    fn replay_channel_commands_accept_same_drop_and_a_bus() {
        let parse = |target: serde_json::Value| -> ReplayChannelChoice {
            let cmd: WebCommand = serde_json::from_value(serde_json::json!({
                "type": "set_replay_channel",
                "kind": "can",
                "raw": 3,
                "target": target,
            }))
            .expect("replay channel command");
            match cmd {
                WebCommand::SetReplayChannel { target, .. } => target,
                _ => panic!("wrong command variant"),
            }
        };
        assert!(matches!(
            parse(serde_json::json!("same")),
            ReplayChannelChoice::Word(word) if word == "same"
        ));
        assert!(matches!(
            parse(serde_json::json!("drop")),
            ReplayChannelChoice::Word(word) if word == "drop"
        ));
        assert!(matches!(
            parse(serde_json::json!(2)),
            ReplayChannelChoice::Bus(2)
        ));
    }

    #[test]
    fn replay_channel_choices_forget_and_drop() {
        use crate::replay_map::{LogBusKind, ReplayTarget};
        let mut app = App::headless();
        apply_replay_channel(
            &mut app,
            LogBusKind::Can,
            3,
            ReplayChannelChoice::Word("drop".into()),
        )
        .expect("drop accepted");
        assert_eq!(
            app.snap.replay_map.target(LogBusKind::Can, 3),
            ReplayTarget::Drop
        );
        apply_replay_channel(
            &mut app,
            LogBusKind::Can,
            3,
            ReplayChannelChoice::Word("same".into()),
        )
        .expect("same accepted");
        assert_eq!(
            app.snap.replay_map.target(LogBusKind::Can, 3),
            ReplayTarget::Same,
            "as-is removes the entry instead of pinning today's bus"
        );
        assert!(
            apply_replay_channel(
                &mut app,
                LogBusKind::Can,
                3,
                ReplayChannelChoice::Word("bogus".into()),
            )
            .is_err()
        );
    }

    #[test]
    fn observer_commands_resolve_can_and_flexray_addresses() {
        let app = App::headless();
        let core = &*app;
        assert_eq!(
            command_key(core, "can", 1, 0x100, true, None, "EngineSpeed".to_string()),
            Some(SigKey::can_in(
                crate::model::ClusterId(2),
                0x100,
                true,
                "EngineSpeed"
            ))
        );
        assert_eq!(
            command_key(core, "fr", 2, 13, false, None, "CarSpeed".to_string()),
            Some(SigKey::fr_in(crate::model::ClusterId(0), 13, "CarSpeed")),
            "no FlexRay cluster is loaded, so the key has no cluster yet"
        );
        assert_eq!(
            command_key(
                core,
                "eth",
                0,
                0,
                false,
                Some("EthFrame".to_string()),
                "Sig".to_string()
            ),
            Some(SigKey::eth_in(
                crate::model::ClusterId(0),
                "EthFrame",
                "Sig"
            ))
        );
        assert!(
            command_key(core, "eth", 0, 0, false, None, "x".to_string()).is_none(),
            "an Ethernet address without a frame name is incomplete"
        );
        assert_eq!(
            command_key(
                core,
                "derived",
                0,
                0,
                false,
                Some("d1".to_string()),
                "Double".to_string()
            ),
            Some(SigKey::derived("d1", "Double")),
            "observer rows can address a computed channel by definition id"
        );
        assert!(command_key(core, "fr", 0, 70_000, false, None, "x".to_string()).is_none());
    }

    /// A replay path must open the log and start playback in Replay mode,
    /// without falling back to the interactive file picker.
    #[test]
    fn a_replay_path_starts_the_web_session() {
        let path = std::env::temp_dir().join("roxy_web_session_replay.asc");
        let text = "date Sat Jan 01 00:00:00.000 2022\n\
                    base hex  timestamps absolute\n\
                    Begin Triggerblock Sat Jan 01 00:00:00.000 2022\n\
                    1.0 1 416 Rx d 8 17 34 51 68 85 102 119 136\n\
                    2.0 1 416 Rx d 8 17 34 51 68 85 102 119 136\n\
                    End TriggerBlock\n";
        std::fs::write(&path, text).expect("write replay fixture");
        let mut app = App::headless();
        prepare_session(&mut app, None, Some(path.to_str().unwrap()));
        assert_eq!(app.snap.run_mode, Mode::Replay);
        assert!(!app.log_path.is_empty(), "the log was accepted");
        assert!(app.snap.measuring, "playback started");
        assert!(app.snap.replay.is_some(), "the playhead is visible");
        std::fs::remove_file(&path).ok();
    }

    /// Selecting a signal creates its lane metadata even before the first
    /// sample, and unselecting removes it again.
    #[test]
    fn the_store_tracks_the_selected_signals() {
        let app = App::headless();
        let key = SigKey::can_in(crate::model::ClusterId(1), 0x100, false, "EngineSpeed");
        let mut store = SignalStore::default();
        store.select(&app, 0, 0x100, false, "EngineSpeed");
        assert!(store.is_selected(&key));
        store.sync(&app);
        let seeded = store.seed();
        assert_eq!(seeded.len(), 1);
        assert_eq!(seeded[0].name, "EngineSpeed");
        assert_eq!(seeded[0].unit, "rpm");
        store.unselect(&key);
        store.sync(&app);
        assert!(store.seed().is_empty());
    }

    /// A read-only signal stays subscribed and sampled without creating a
    /// lane; plotting it later only flips the mode.
    #[test]
    fn read_signals_subscribe_without_a_lane() {
        let mut app = App::headless();
        let key = SigKey::can_in(crate::model::ClusterId(1), 0x100, false, "EngineSpeed");
        let mut store = SignalStore::default();
        set_signal_mode(&mut app, &mut store, key.clone(), true);
        app.settle();
        assert!(store.is_selected(&key) && store.is_read(&key));
        assert!(store.chart_keys().is_empty(), "read is not a lane");
        assert!(app.subs.contains_key(&key), "read signals still sample");
        assert!(
            app.graphics
                .first()
                .is_none_or(|window| window.signals.iter().all(|row| row.key != key)),
            "read does not enter the Graphics window"
        );
        store.sync(&app);
        let seeded = store.seed();
        assert_eq!(seeded.len(), 1);
        assert!(seeded[0].read, "the series tells the frontend not to draw");

        set_signal_mode(&mut app, &mut store, key.clone(), false);
        assert!(!store.is_read(&key));
        assert_eq!(store.chart_keys(), vec![key.clone()]);
        assert!(
            app.graphics
                .first()
                .is_some_and(|window| window.signals.iter().any(|row| row.key == key)),
            "switching to plot restores the Graphics row"
        );
        assert!(app.subs.contains_key(&key));
        store.sync(&app);
        assert!(!store.seed()[0].read);

        set_signal_mode(&mut app, &mut store, key.clone(), true);
        assert!(
            app.graphics
                .first()
                .is_none_or(|window| window.signals.iter().all(|row| row.key != key)),
            "switching back removes the lane"
        );
        assert!(app.subs.contains_key(&key), "but keeps sampling");

        store.unselect(&key);
        app.set_session_chart_keep(store.keys());
        chart_unselect(&mut app, &key);
        assert!(
            !app.subs.contains_key(&key),
            "unselect drops the read subscription"
        );
    }

    /// Context menus and the editor use the same snake_case command shape as
    /// the rest of the API; a renamed field would fail silently at runtime.
    #[test]
    fn trigger_seed_and_edit_commands_deserialize() {
        let seed: WebCommand = serde_json::from_value(serde_json::json!({
            "type": "add_trigger",
            "kind": "id",
            "ch": 1,
            "id": 256,
        }))
        .expect("seed command");
        match seed {
            WebCommand::AddTrigger { ch, id, .. } => {
                assert_eq!(ch, Some(1));
                assert_eq!(id, Some(256));
            }
            _ => panic!("wrong command variant"),
        }

        let edit: WebCommand = serde_json::from_value(serde_json::json!({
            "type": "edit_trigger",
            "index": 0,
            "kind": "sysvar",
            "ch": 0,
            "id": 0,
            "ext": false,
            "signal": "",
            "threshold": 0.0,
            "rising": true,
            "bus": 0,
            "slot": 0,
            "key": "Bench::Ready",
            "derived_id": "",
            "action": "set_sysvar",
            "send_ch": 0,
            "send_id": 0,
            "sys_var_key": "Bench::Ready",
            "sys_var_value": 3.5,
            "script_name": "",
            "script_value": null,
            "writes": [],
            "cooldown_ms": 250,
            "max_fires": 4,
        }))
        .expect("edit command");
        match edit {
            WebCommand::EditTrigger {
                action,
                sys_var_key,
                sys_var_value,
                cooldown_ms,
                max_fires,
                ..
            } => {
                assert_eq!(action, "set_sysvar");
                assert_eq!(sys_var_key, "Bench::Ready");
                assert_eq!(sys_var_value, Some(3.5));
                assert_eq!(cooldown_ms, 250);
                assert_eq!(max_fires, 4);
            }
            _ => panic!("wrong command variant"),
        }
    }

    /// The Triggers panel reads the live rule list and its fire state.
    #[test]
    fn the_state_lists_triggers() {
        let mut app = App::headless();
        app.apply_trigger_cmds(vec![
            TriggerCmd::Add(TriggerKind::Id),
            TriggerCmd::Edit {
                index: 0,
                cond: TriggerCond::ErrorFrame { ch: 1 },
                action: TriggerAction::InsertMarker,
                cooldown_us: 0,
                max_fires: 0,
                pre_frames: None,
                post_frames: None,
            },
            TriggerCmd::SetPreFrames(128),
            TriggerCmd::SetPostFrames(16),
            TriggerCmd::SetMarkerCap(64),
        ]);
        let state = build_state(&app, &VecDeque::new(), 0.0);
        assert_eq!(state.triggers.len(), 1);
        assert!(!state.triggers[0].enabled, "new rules start disarmed");
        assert_eq!(state.triggers[0].kind, "error");
        assert_eq!(state.triggers[0].ch, 1);
        assert_eq!(state.triggers[0].action_kind, "insert_marker");
        assert!(!state.triggers[0].summary.is_empty());
        assert_eq!(state.trigger_limits.pre_frames, 128);
        assert_eq!(state.trigger_limits.post_frames, 16);
        assert_eq!(state.trigger_limits.marker_cap, 64);
    }

    /// The generator panel reads live TxView rows, including their cycle,
    /// payload and node gate.
    #[test]
    fn the_state_lists_generator_entries() {
        let app = App::headless();
        let state = build_state(&app, &VecDeque::new(), 0.0);
        assert!(!state.tx.is_empty(), "the bootstrap generator has entries");
        let entry = state
            .tx
            .iter()
            .find(|entry| entry.channel == 0 && entry.id == 0x100)
            .expect("EngineStatus is in the sample generator");
        assert_eq!(entry.name, "EngineStatus");
        assert!(entry.cycle_us > 0);
        assert_eq!(entry.bus, "CAN1");
    }

    /// Reloading a database must retire selections it no longer declares,
    /// otherwise the lane can never be removed from the picker.
    #[test]
    fn a_database_reload_drops_missing_signals() {
        let app = App::headless();
        let mut store = SignalStore::default();
        store.select(&app, 0, 0x100, false, "NoSuchSignal");
        let dropped = store.prune_missing(&app);
        assert_eq!(dropped.len(), 1);
        assert!(store.selected.is_empty());
    }

    /// Removing a CAN bus drops the selections on its cluster; the ones on
    /// the surviving buses keep their keys untouched.
    #[test]
    fn removing_a_bus_remaps_the_selection() {
        let app = App::headless();
        let mut store = SignalStore::default();
        store.select(&app, 0, 0x100, false, "EngineSpeed");
        store.select(&app, 1, 0x64, false, "EngSpeed");
        assert_eq!(store.selected.len(), 2);
        store.remap_after_remove(Some(crate::model::ClusterId(1)));
        assert_eq!(store.selected.len(), 1);
        assert_eq!(store.selected[0].name, "EngSpeed");
        assert_eq!(
            store.selected[0].key.cluster(),
            Some(crate::model::ClusterId(2)),
            "the survivor keeps its cluster identity"
        );
        assert_eq!(key_id(&store.selected[0].key), 0x64);
    }

    /// The Buses panel gets its rows from the snapshot, including the DBC
    /// names and the absence of an attachment.
    #[test]
    fn the_state_lists_buses_and_hardware_fields() {
        let app = App::headless();
        let state = build_state(&app, &VecDeque::new(), 0.0);
        assert_eq!(state.buses.len(), 2);
        assert_eq!(state.buses[0].name, "CAN1");
        assert!(state.buses[0].descriptions.iter().any(|description| {
            description.kind == "dbc" && description.path.contains("sample.dbc")
        }));
        assert!(
            state
                .buses
                .iter()
                .all(|bus| bus.can.as_ref().is_none_or(|can| can.attached.is_none()))
        );
        assert!(parse_driver("Vector").is_some());
        assert!(parse_driver("Kvaser").is_some());
        assert!(parse_driver("other").is_none());
    }

    /// The picker catalog comes from the loaded DBCs, not a hard-coded list.
    #[test]
    fn the_catalog_lists_the_dbc_signals() {
        let app = App::headless();
        let catalog = build_catalog(&app);
        let message = catalog
            .channels
            .iter()
            .flat_map(|channel| &channel.messages)
            .find(|message| message.name == "EngineStatus")
            .expect("EngineStatus is in sample.dbc");
        assert!(
            message
                .signals
                .iter()
                .any(|signal| signal.name == "EngineSpeed" && signal.unit == "rpm")
        );
    }

    /// The seeded session must actually drive and sample the plotted signals; an empty
    /// SignalStore would draw eight empty lanes forever.
    #[test]
    fn the_seeded_session_samples_the_plotted_signals() {
        let mut app = App::headless();
        seed_test_session(&mut app);
        for _ in 0..40 {
            std::thread::sleep(Duration::from_millis(5));
            app.update();
        }
        let mut signals = SignalStore::default();
        for spec in &SAMPLE_SIGNALS {
            signals.select(&app, spec.ch, spec.id, false, spec.name);
        }
        signals.sync(&app);
        let seeded = signals.seed();
        assert_eq!(seeded.len(), SAMPLE_SIGNALS.len());
        assert!(
            seeded.iter().any(|series| !series.points.is_empty()),
            "the subscriptions received samples"
        );
        assert!(
            seeded
                .iter()
                .flat_map(|series| series.points.iter())
                .any(|[_, value]| *value != 0.0),
            "the generator sources produced non-zero signal values"
        );
    }

    /// A derived definition becomes an ordinary selectable series: the core
    /// samples it, selecting it puts it on the chart, and the observer rows
    /// name it like any other signal, with the State bands from its definition.
    #[test]
    fn a_derived_channel_reaches_the_chart_and_the_state_observer() {
        let mut app = App::headless();
        seed_test_session(&mut app);
        for _ in 0..30 {
            std::thread::sleep(Duration::from_millis(5));
            app.update();
        }
        app.set_derived_defs(vec![crate::derived::DerivedDef {
            id: "d1".into(),
            name: "Double".into(),
            unit: "rpm".into(),
            expression: "[EngineSpeed] * 2".into(),
            band_set_id: None,
            bands: Vec::new(),
            cuts: vec![1000.0],
            names: vec!["low".into(), "high".into()],
            sources: Vec::new(),
        }]);
        for _ in 0..10 {
            std::thread::sleep(Duration::from_millis(5));
            app.update();
        }
        let key = SigKey::derived("d1", "Double");
        app.set_win_signal(crate::workspace::PopupTarget::State(0), key.clone(), true);
        app.set_win_signal(crate::workspace::PopupTarget::Monitor, key.clone(), true);

        let mut signals = SignalStore::default();
        for spec in &SAMPLE_SIGNALS {
            signals.select(&app, spec.ch, spec.id, false, spec.name);
        }
        signals.sync(&app);
        assert!(
            !signals
                .seed()
                .iter()
                .any(|series| series.key == "derived:d1"),
            "a definition alone does not put a lane on the chart"
        );
        signals.select_key(&app, key.clone());
        signals.sync(&app);
        let seeded = signals.seed();
        let derived = seeded
            .iter()
            .find(|series| series.key == "derived:d1")
            .expect("the derived lane rides after the selected signals");
        assert_eq!(derived.bus, "Derived");
        assert_eq!(derived.unit, "rpm");
        assert!(!derived.points.is_empty(), "the core sampled the lane");

        let state = build_state(&app, &VecDeque::new(), 0.0);
        let row = state
            .state_rows
            .iter()
            .find(|row| row.kind == "derived")
            .expect("the State observer lists the lane");
        assert_eq!(row.name, "Double");
        assert!(row.rule.is_some(), "the definition's bands reached the row");
        assert!(!row.bands.is_empty());
        let monitor = state
            .monitor
            .iter()
            .find(|row| row.kind == "derived")
            .expect("the Monitor observer lists the lane");
        assert_eq!(monitor.name, "Double");
        app.stop();
    }

    /// At the cap the store must trim its oldest row without discarding the
    /// sequence range the client already holds.
    #[test]
    fn the_trace_store_keeps_the_newest_rows_at_the_cap() {
        let mut store = TraceStore::default();
        for i in 0..TRACE_LOG_CAP + 10 {
            store.push(WebFrame {
                t_us: i as u64,
                ..Default::default()
            });
        }
        assert_eq!(store.log.len(), TRACE_LOG_CAP);
        assert_eq!(store.first_seq(), 10);
        let tail = store.after(store.next_seq - 6);
        assert_eq!(tail.len(), 5);
        assert_eq!(tail[0].t_us, (TRACE_LOG_CAP + 9) as u64);
    }

    #[test]
    fn listing_a_folder_puts_folders_first_with_metadata() {
        let dir = std::env::temp_dir().join(format!("roxy-can-fs-list-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("sub")).unwrap();
        std::fs::write(dir.join("alpha.asc"), b"abc").unwrap();
        std::fs::write(dir.join("notes.txt"), b"x").unwrap();
        let list = list_directory(Some(&dir.to_string_lossy())).unwrap();
        let names: Vec<&str> = list.entries.iter().map(|e| e.name.as_str()).collect();
        assert_eq!(names, ["sub", "alpha.asc", "notes.txt"]);
        assert!(list.entries[0].dir);
        assert_eq!(list.entries[1].size, 3);
        assert!(list.parent.is_some());
        assert!(!list.roots.is_empty());
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn creating_a_folder_rejects_duplicates_and_path_escapes() {
        let dir = std::env::temp_dir().join(format!("roxy-can-mkdir-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let parent = dir.to_string_lossy().into_owned();
        let created = create_folder(&parent, "take 1").unwrap();
        assert_eq!(created.name, "take 1");
        assert!(std::path::Path::new(&created.path).is_dir());
        assert!(create_folder(&parent, "take 1").is_err());
        assert!(create_folder(&parent, "../escape").is_err());
        assert!(create_folder(&parent, "nested/name").is_err());
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn renaming_and_deleting_folders_checks_names_and_emptiness() {
        let dir = std::env::temp_dir().join(format!("roxy-can-rename-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let parent = dir.to_string_lossy().into_owned();
        let old = create_folder(&parent, "old name").unwrap();
        let renamed = rename_folder(&old.path, "new name").unwrap();
        assert!(!std::path::Path::new(&old.path).exists());
        assert!(std::path::Path::new(&renamed.path).is_dir());
        assert!(rename_folder(&renamed.path, "../escape").is_err());
        let other = create_folder(&parent, "other").unwrap();
        assert!(rename_folder(&renamed.path, "other").is_err());
        std::fs::write(std::path::Path::new(&renamed.path).join("keep.txt"), b"x").unwrap();
        assert!(delete_folder(&renamed.path).is_err(), "non-empty folder");
        std::fs::remove_file(std::path::Path::new(&renamed.path).join("keep.txt")).ok();
        delete_folder(&renamed.path).unwrap();
        delete_folder(&other.path).unwrap();
        assert!(delete_folder(&renamed.path).is_err(), "already gone");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn someip_field_defs_keep_text_enum_and_nested_paths() {
        use crate::eth::arxml_service::{ServiceField, ServiceType, SomeIpProps};
        let props = SomeIpProps::default();
        let field = ServiceField {
            name: "details".to_string(),
            ty: ServiceType::Record(vec![
                ServiceField {
                    name: "speed".to_string(),
                    ty: ServiceType::U16,
                    props: props.clone(),
                },
                ServiceField {
                    name: "label".to_string(),
                    ty: ServiceType::String,
                    props: props.clone(),
                },
                ServiceField {
                    name: "state".to_string(),
                    ty: ServiceType::Enum {
                        base: Box::new(ServiceType::U8),
                        labels: vec![(0, "Off".to_string()), (1, "On".to_string())],
                    },
                    props: props.clone(),
                },
            ]),
            props,
        };
        let defs = someip_field_defs("details", &field);
        assert_eq!(
            defs.iter().map(|def| def.name.as_str()).collect::<Vec<_>>(),
            ["details.speed", "details.label", "details.state"]
        );
        assert_eq!(defs[0].kind, "number");
        assert_eq!(defs[1].kind, "text");
        assert_eq!(defs[2].labels.len(), 2);
    }

    #[test]
    fn elevated_or_captured_launches_skip_the_auto_open() {
        assert!(should_open_browser(true, false, true));
        assert!(!should_open_browser(true, true, true));
        assert!(!should_open_browser(true, false, false));
        assert!(!should_open_browser(false, false, true));
    }
}
