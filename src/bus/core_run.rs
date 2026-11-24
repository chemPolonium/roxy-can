//! Run loop: stepping, replay transport, recording and spec checks.

use super::*;

impl BusCore {
    /// Fresh virtual run: new source, blank run state, wall-clock
    /// measuring.
    pub(super) fn start_virtual(&mut self, status: &mut String) {
        self.recorder.close();
        self.source = Box::new(crate::source::virtual_source::VirtualSource::new());
        self.mode = Mode::Virtual;
        self.run_mode = Mode::Virtual;
        self.reset_run();
        self.measuring = true;
        self.nodes_start();
        // Enabled blocks load their queues now: the log is read against
        // the database as it stands at run start, and a broken path or a
        // filtered-to-nothing log names itself instead of going silent.
        let mut block_notes: Vec<String> = Vec::new();
        for block in &mut self.replay_blocks {
            if block.enabled {
                let dbc = self
                    .model_dbs
                    .get(block.channel as usize)
                    .and_then(|db| db.clone());
                block.load_queue(dbc.as_deref(), self.sim_t_us);
                if let Some(e) = &block.last_error {
                    block_notes.push(format!("`{}`: {e}", block.name));
                }
            }
        }
        let note = wire_note(self.hw.attached_count(), self.hw.live);
        *status = if block_notes.is_empty() {
            format!("measuring ({note})")
        } else {
            format!(
                "measuring ({note}); replay block {}",
                block_notes.join("; ")
            )
        };
        if self.recorder.recording {
            match self.recorder.open() {
                Ok(path) => *status = format!("recording to {path}"),
                Err(e) => *status = format!("record failed: {e}"),
            }
        }
    }

    /// Blanks every run-scored counter: rings, aggregates, loads, spec
    /// memory, generator schedules, subscription measurement state.
    /// A real adapter is attached and wired; the event loop can poll RX faster
    /// than it publishes snapshots.
    pub(crate) fn wants_fast_rx(&self) -> bool {
        self.hw.live && self.hw.has_real_rx()
    }

    pub(crate) fn reset_run(&mut self) {
        self.sim_t_us = 0;
        self.sim_prev_us = 0;
        for block in &mut self.replay_blocks {
            block.rewind();
        }
        // A fresh start must not inherit the previous run's pause state or
        // a play-to target.
        self.trace_paused = false;
        self.paused_at_us = None;
        self.replay_stop_at = None;
        self.source.set_poll_limit(None);
        self.trace.clear();
        self.fr_trace.clear();
        self.eth_trace.clear();
        self.someip_events.clear();
        self.someip_tcp = crate::eth::someip::SomeIpTcp::default();
        self.someip_tp = crate::eth::someip::SomeIpTp::default();
        self.someip_ip = crate::eth::someip::SomeIpIp::default();
        self.someip_sessions.clear();
        self.someip_pending.clear();
        self.someip_offers.clear();
        self.someip_subscriptions.clear();
        self.someip_replies.clear();
        self.nm_last.clear();
        self.nm_states.clear();
        self.publish_trace();
        self.frame_counter = 0;
        self.aggs.clear();
        self.fr_aggs.clear();
        self.eth_aggs.clear();
        // 通道计数说的是"这一趟回放里日志带来了哪些路"，和 Trace 同一条命：
        // 按下 Start 就该空着，等下一趟的帧重新数。配好的映射不在这里，
        // 那是工程的决定。
        self.log_census.clear();
        self.drop_news_done.clear();
        for load in &mut self.bus_loads {
            load.clear();
        }
        self.fr_loads.clear();
        self.loads_dirty = true;
        // Along with the aggregates it reads: keeping the previous run's
        // interval memory would turn the first step of a new run into one
        // enormous measured period.
        self.spec = crate::spec::Spec::default();
        self.markers.clear();
        self.trigger_fires.clear();
        self.pre_buffer.clear();
        self.post_roll = None;
        // Send-now intents recorded while the old run was winding down
        // belong to it, not to the fresh one.
        self.injected.clear();
        self.fr_from_scripts.clear();
        self.panel_fr_frames.clear();
        self.panel_frames.clear();
        for tx in &mut self.tx_list {
            tx.next_t_us = 0;
        }
        for tx in &mut self.fr_tx_list {
            tx.next_t_us = 0;
        }
        for sub in self.subs.values_mut() {
            sub.reset_measurement();
        }
        // Derived curves are a view of this run's sources; the fresh run's
        // history starts empty, so the sampler must backfill from zero rather
        // than skip every point older than the last run's tail.
        for runtime in &mut self.derived {
            runtime.last_t_us = None;
        }
        // Every run starts from the declared values, CANoe-style -- and
        // republishes them, so observers never plot the last run's tail.
        self.init_sysvars();
        self.refresh_sub_histories();
    }

    /// One immediate forward step on the replay clock. Runs a normal bus
    /// step at the last wall-clock anchor, so trace, aggregates, spec and
    /// observers all see the frames exactly as a played lap would.
    pub(super) fn replay_step(&mut self, delta_us: u64, status: &mut String) {
        if !matches!(self.mode, Mode::Replay) || !self.measuring {
            *status = "start Replay before stepping".to_string();
            return;
        }
        // A manual step abandons a pending timed play.
        self.replay_stop_at = None;
        self.source.set_poll_limit(None);
        self.source.step_forward(delta_us);
        let now = self.sim_prev_us;
        self.step(
            now,
            self.last_stride_us,
            self.last_tol_pct,
            self.last_grace,
            status,
        );
    }

    /// Arms a forward-only stop target and resumes the replay clock. The
    /// target is absolute log time; a value behind the playhead is refused
    /// rather than silently ignored.
    pub(super) fn replay_play_to(&mut self, target_us: u64, status: &mut String) {
        if !matches!(self.mode, Mode::Replay) || !self.measuring {
            *status = "start Replay before setting a play target".to_string();
            return;
        }
        let pos = self.source.position().unwrap_or(0);
        if target_us <= pos {
            *status = format!(
                "play target {:.3}s is behind the playhead {:.3}s (forward only)",
                target_us as f64 / 1e6,
                pos as f64 / 1e6
            );
            return;
        }
        self.replay_stop_at = Some(target_us);
        self.source.set_poll_limit(Some(target_us));
        self.trace_paused = false;
        *status = format!("replaying to {:.3}s", target_us as f64 / 1e6);
    }

    pub(super) fn replay_play_for(&mut self, duration_us: u64, status: &mut String) {
        let pos = self.source.position().unwrap_or(0);
        self.replay_play_to(pos.saturating_add(duration_us), status);
    }

    pub(super) fn stop_bus(&mut self, status: &mut String) {
        self.measuring = false;
        self.replay_stop_at = None;
        self.source.set_poll_limit(None);
        self.recorder.close();
        self.finish_nodes();
        self.nodes_dirty = true;
        *status = "stopped".to_string();
    }

    /// Gives every live node its `on stop` run with the final inputs, keeps
    /// the test results and mirrors the new log lines into Write. Safe to
    /// call when nothing is running (all nodes have no runtime).
    pub(super) fn finish_nodes(&mut self) {
        let inputs = self.build_node_inputs(self.sim_t_us);
        let mut node_lines: Vec<(String, String)> = Vec::new();
        for node in &mut self.nodes {
            let input = inputs.get(&node.channel).cloned().unwrap_or_default();
            node.stop_with(&input);
            let name = node.name.clone();
            for line in node.take_new_lines() {
                node_lines.push((name.clone(), line));
            }
            if node.take_log_if_dirty().is_some() {
                self.nodes_dirty = true;
            }
        }
        for (name, line) in node_lines {
            self.write_node_line(&name, &line);
        }
    }

    pub(super) fn toggle_record(&mut self, status: &mut String) {
        // A manual stop is immediate: any pending trigger post-roll dies.
        self.post_roll = None;
        if self.recorder.recording {
            self.recorder.close();
            self.recorder.recording = false;
        } else {
            self.recorder.recording = true;
            // While stopped, the file is created by the next start;
            // checking Record must not leave an empty record file behind.
            if self.measuring {
                let opened = self.recorder.open();
                self.recorder.recording = opened.is_ok();
                match opened {
                    Ok(path) => {
                        *status = if self.recorder.last_eth_record.is_empty()
                            || self.recorder.last_eth_record == path
                        {
                            format!("recording to {path}")
                        } else {
                            format!(
                                "recording to {path} (Ethernet: {})",
                                self.recorder.last_eth_record
                            )
                        };
                    }
                    Err(e) => *status = format!("record failed: {e}"),
                }
            }
        }
    }

    /// Manual re-arm: clears the level latch of every appearance-type
    /// condition (ID present / error frame) so the next occurrence fires
    /// again. Signal-cross and cycle-timeout levels are real levels and
    /// are never touched.
    pub(super) fn rearm_latched_triggers(&mut self, status: &mut String) {
        let mut n = 0;
        for t in &mut self.triggers {
            if t.cond.latches_once() && t.level {
                t.rearm();
                n += 1;
            }
        }
        *status = format!("re-armed {n} latched trigger(s)");
    }

    /// Offline analysis (R1): ingests the whole loaded log in one pass --
    /// Blanks the trace the way the Clear button means it: the frames on
    /// display (and their archive) go away; the measurement itself keeps
    /// running -- recording stays open, counts keep counting, arrivals
    /// after the clear show up as usual.
    pub(super) fn clear_trace_view(&mut self, status: &mut String) {
        self.trace.clear();
        self.fr_trace.clear();
        self.eth_trace.clear();
        self.someip_events.clear();
        self.someip_tcp = crate::eth::someip::SomeIpTcp::default();
        self.someip_tp = crate::eth::someip::SomeIpTp::default();
        self.someip_ip = crate::eth::someip::SomeIpIp::default();
        self.someip_sessions.clear();
        self.someip_pending.clear();
        self.someip_offers.clear();
        self.someip_subscriptions.clear();
        self.someip_replies.clear();
        self.nm_last.clear();
        self.nm_states.clear();
        self.publish_trace();
        *status = "trace cleared".to_string();
    }

    /// Resets the per-message counters behind the Messages and Statistics
    /// rows. Deliberately narrower than `reset_run`: load, spec memory and
    /// the recording are mid-run state the user did not ask to lose.
    pub(super) fn clear_aggregates(&mut self, status: &mut String) {
        let n = self.aggs.len() + self.fr_aggs.len() + self.eth_aggs.len();
        self.aggs.clear();
        // The FlexRay tallies too: the Messages window lists them beside the
        // CAN rows now, and clearing it left the FR counts running up from
        // wherever they were instead of from zero.
        self.fr_aggs.clear();
        self.eth_aggs.clear();
        self.e2e_tracks.clear();
        *status = format!("cleared {n} message counter(s)");
    }

    /// The wall-clock instant the bus next has work due: the next replay
    /// frame, or the next generator slot against the sim clock (which runs
    /// 1:1 with the wall in Virtual). `None` means nothing is scheduled --
    /// the event loop may sleep until a command arrives. Paused or stopped
    /// buses are never due.
    pub(crate) fn next_deadline(&self, now_us: u64) -> Option<u64> {
        if !self.measuring || self.trace_paused {
            return None;
        }
        let bus_due = match self.mode {
            Mode::Replay => self.source.next_deadline(now_us),
            Mode::Virtual => {
                let min_next = self
                    .tx_list
                    .iter()
                    // 角色开关与发送循环同规则：开关关闭的条目不产生发送时刻。
                    .filter(|t| {
                        t.active
                            && t.cycle_us != 0
                            && (t.node.is_empty()
                                || self.can_role_of(t.channel as usize, &t.node)
                                    == NodeRole::Simulated)
                    })
                    .map(|t| t.next_t_us)
                    .min()?;
                Some(now_us + min_next.saturating_sub(self.sim_t_us))
            }
        };
        // Node timers are absolute wall-clock dues and never delay the
        // bus past its own work, but the loop must wake for them.
        let node_due = self
            .nodes
            .iter()
            .filter(|n| n.running())
            .flat_map(|n| n.timer_dues())
            .min();
        match (bus_due, node_due) {
            (Some(a), Some(b)) => Some(a.min(b)),
            (Some(a), None) => Some(a),
            (None, Some(b)) => Some(b),
            (None, None) => None,
        }
    }

    /// Advance the bus's own clocks to wall-clock `now_us`, before a step.
    /// A span spent frozen is skipped rather than accumulated: replay
    /// resumes in place instead of fast-forwarding through it, and the
    /// generator's schedule and waveform phase do not jump by the paused
    /// span. In Replay the sim clock is owned by [`Self::step`] instead,
    /// which reads it off the log frames; adding wall time here would let
    /// the generator march on while the log is quiet and snap back on the
    /// next frame.
    pub(crate) fn advance_clock(&mut self, now_us: u64) {
        if let Some(t) = self.paused_at_us.take() {
            self.source.shift_time(now_us.saturating_sub(t));
            self.sim_prev_us = now_us;
        }
        if !matches!(self.mode, Mode::Replay) {
            self.sim_t_us += now_us.saturating_sub(self.sim_prev_us);
        }
        self.sim_prev_us = now_us;
    }

    /// The event loop's unit of work: advance the clocks to `now_us`, then
    /// run one full step. `step` alone assumes the caller maintains the
    /// clocks (as the UI loop does); `step_to` is what a deadline-driven
    /// loop calls when it wakes.
    pub(crate) fn step_to(
        &mut self,
        now_us: u64,
        stride: u64,
        tol_pct: u64,
        grace: u64,
        status: &mut String,
    ) -> bool {
        self.advance_clock(now_us);
        self.step(now_us, stride, tol_pct, grace, status)
    }

    /// One full step of the bus against wall-clock `now_us`: poll the
    /// source, let generators emit against the sim clock, walk the queue
    /// through triggers / recorder / ingest, sample the load rollups, check
    /// the databases' promises, sweep timeout triggers, and settle a
    /// finished replay. Returns true when a replay finished this step.
    /// `stride` is the frontend's requested sampling period for signal
    /// history; `tol_pct` and `grace` its spec tolerances -- frontend
    /// policy the bus applies but does not own. Status messages from
    /// trigger actions or the finish go into `status` (last write wins;
    /// the status bar has one line).
    pub(crate) fn step(
        &mut self,
        now_us: u64,
        stride: u64,
        tol_pct: u64,
        grace: u64,
        status: &mut String,
    ) -> bool {
        // Remember the frontend's policy for step commands that run a step
        // outside the normal tick.
        self.last_stride_us = stride;
        self.last_tol_pct = tol_pct;
        self.last_grace = grace;
        // Every step moves the clock, so the load windows drain and resample
        // whether or not a frame arrived; publish the rollups after it.
        self.loads_dirty = true;
        self.buf.clear();
        self.source.poll(now_us, &mut self.buf);
        // FlexRay rows a BLF replay carries come through the same source;
        // they skip the CAN pipeline and land in the FR ring directly.
        let mut fr_replay: Vec<crate::trace::FrRow> = Vec::new();
        self.source.poll_fr(now_us, &mut fr_replay);
        let mut eth_replay: Vec<crate::eth::EthFrame> = Vec::new();
        self.source.poll_eth(now_us, &mut eth_replay);
        // 日志的每条帧只带一个通道**编号**，那是录它的那个人工程里的编号，
        // 文件里没有总线名可对照。所以在进环之前换成本工程的第几路，让解码、
        // 负载、Trace、聚合看见的都是自己这几条路；对不上任何一路的不回放
        // （规则在 `ReplayMap::can_channel`，映射表用的是同一个函数）。
        // 计数按文件里的编号记，被挡掉的那些也记：Buses 的映射表就是靠它说出
        // "这个文件里有这几路、各多少条"，不必把文件再读一遍。
        if matches!(self.mode, Mode::Replay) {
            use crate::replay_map::LogBusKind;
            let map = self.replay_map.clone();
            let can_ids = self.can_cluster_ids.clone();
            let fr_ids = self.fr_cluster_ids.clone();
            let eth_ids = self.eth_cluster_ids.clone();
            let buses = self.can_count();
            let census = &mut self.log_census;
            let mut unaimed: Vec<u8> = Vec::new();
            self.buf.retain_mut(|f| {
                census.bump(LogBusKind::Can, f.channel);
                match replay_bus_in(
                    &map,
                    LogBusKind::Can,
                    f.channel,
                    &can_ids,
                    &fr_ids,
                    &eth_ids,
                )
                .filter(|t| (*t as usize) < buses)
                {
                    Some(t) => {
                        f.channel = t;
                        true
                    }
                    None => {
                        if !unaimed.contains(&f.channel) {
                            unaimed.push(f.channel);
                        }
                        false
                    }
                }
            });
            // FlexRay 一侧不给这样的兜底：一个没有定义的 cluster 号也能进环，
            // Buses 窗口会为它列一行"日志里有这路流量"，那是有用的信息，
            // 不是错位的编号。
            fr_replay.retain_mut(|r| {
                census.bump(LogBusKind::Fr, r.bus);
                match replay_bus_in(&map, LogBusKind::Fr, r.bus, &can_ids, &fr_ids, &eth_ids) {
                    Some(t) => {
                        r.bus = t;
                        true
                    }
                    None => false,
                }
            });
            // An Ethernet capture's channel is the recording tool's own
            // numbering, unrelated to CAN channels or clusters. As on the
            // FlexRay side there is no bus-count fallback: an index with no
            // matrix is still a real flow, and an explicit "don't replay"
            // is the only thing that drops it.
            eth_replay.retain_mut(|f| {
                census.bump(LogBusKind::Eth, f.bus);
                match replay_bus_in(&map, LogBusKind::Eth, f.bus, &can_ids, &fr_ids, &eth_ids) {
                    Some(t) => {
                        f.bus = t;
                        true
                    }
                    None => false,
                }
            });
            // 有人被挡在门外就说一句。状态行只有一行，所以一个通道一趟一次。
            for raw in unaimed {
                if self.drop_news_done.insert(raw) {
                    *status = format!(
                        "log channel {} has no matching bus in this project and is not replayed; Setup > Replay > Channel map can point it at one",
                        raw + 1
                    );
                }
            }
        }
        let fr_replayed = !fr_replay.is_empty();
        let eth_landed = !eth_replay.is_empty();
        // Ethernet frames enter their ring after the replay channel mapping,
        // so the whole pipeline -- matrix match, subscriptions, Trace,
        // aggregates -- sees the project's bus index, not the log's.
        for frame in eth_replay {
            self.ingest_eth(frame);
        }
        // Send-now requests are intents recorded by commands; building the
        // frames here stamps them with this step's clock, so a request that
        // waited out a pause never sends a pre-pause timestamp.
        let injected: Vec<(u8, u32)> = std::mem::take(&mut self.injected);
        for (ch, id) in injected {
            self.send_one_shot(ch, id, now_us, &HashMap::new());
        }
        let source_empty = self.buf.is_empty();
        if stride != self.applied_stride_us {
            self.applied_stride_us = stride;
        }

        // The log clock is primary while replaying: `sim_t_us` follows the
        // newest log frame's own stamp and holds between frames, so injected
        // frames land on the same timeline the log carries and every
        // consumer -- aggregates, plots, the spec check -- sees one clock.
        // Live simulation keeps the wall-derived clock `update` maintains.
        if matches!(self.mode, Mode::Replay)
            && let Some(t) = self.buf.last().map(|f| f.t_us)
        {
            self.sim_t_us = t;
        }
        let sim = self.sim_t_us;

        // One-shot Panel frames join the same walk as generated traffic, so
        // they are recorded, counted, decoded and sent to attached hardware
        // exactly like a generator frame.
        for (channel, id, extended, data) in std::mem::take(&mut self.panel_frames) {
            let len = data.len() as u8;
            let mut payload = [0u8; crate::can::frame::MAX_CAN_FD_LEN];
            payload[..data.len()].copy_from_slice(&data);
            self.buf.push(CanFrame {
                t_us: sim,
                channel,
                id,
                extended,
                len,
                data: payload,
                dir: crate::can::frame::Direction::Tx,
                flags: if len > 8 {
                    crate::can::frame::FrameFlags::FD
                } else {
                    crate::can::frame::FrameFlags::NONE
                },
            });
        }

        // Generators transmit in every running mode: an active entry during
        // replay injects onto the log timeline, which is what makes
        // "replay a real log and stir a few frames in" possible. One
        // exception: an id the log itself carries stays silent, or replaying
        // a recording of this same simulation interleaves two senders of one
        // signal and every consumer sees their mixed values. Only consulted
        // in Replay mode, so nothing to restore when the run ends.
        let models: Vec<Option<ModelHandle>> = (0..self.can_count())
            .map(|index| {
                let cluster = *self.can_cluster_ids.get(index)?;
                let comm = self.model_comms.get(index)?.as_ref()?;
                let plans = self.model_plans.get(index)?.as_ref()?;
                Some((cluster, Arc::clone(comm), Arc::clone(plans)))
            })
            .collect();
        let mut emitted: Vec<CanFrame> = Vec::new();
        for tx in &mut self.tx_list {
            if matches!(self.mode, Mode::Replay) && tx.next_t_us == 0 {
                // Log time has no slot zero: an entry picked up mid-log is
                // anchored at the playhead instead of emitting one frame
                // dated the epoch.
                tx.next_t_us = sim;
            }
            let muted =
                matches!(self.mode, Mode::Replay) && self.replay_ids.contains(&(tx.channel, tx.id));
            // 角色开关：绑定到 DBC 节点的条目只有在节点角色为「模拟」时才
            // 发送；未分配条目（无 node 戳）不受角色开关约束。开关关闭时发送循环不运行、
            // next_t_us 冻结，切回模拟由 set_node_role 锚定排程。
            let gate_open = tx.node.is_empty()
                || Self::role_in(
                    &self.can_cluster_ids,
                    &self.can_node_roles,
                    tx.channel as usize,
                    &tx.node,
                ) == NodeRole::Simulated;
            // Every slot the clock has passed goes out at its own stamp.
            // Skipping the backlog after a UI stall (the old policy) kept the
            // tick cheap but punched a hole into the bus's own timeline --
            // and at Graphics strides fine enough to show single updates,
            // that hole reads as the curve being eaten while the plot
            // slides on. Frames carry their slot's timestamp, so spacing
            // stays exactly `cycle_us` even in the catch-up burst.
            let mut budget = MAX_TX_CATCHUP;
            while budget > 0
                && tx.active
                && gate_open
                && !muted
                && tx.cycle_us != 0
                && tx.next_t_us <= sim
            {
                budget -= 1;
                // Values are read at the slot, not at `sim`: a frame stamped
                // `slot` must carry the waveform's value at `slot`, or every
                // payload would lead its own timestamp by up to a full cycle.
                let slot = tx.next_t_us;
                tx.next_t_us += tx.cycle_us;
                let (data, len, flags) = models
                    .get(tx.channel as usize)
                    .and_then(|model| model.as_ref())
                    .and_then(|(cluster, comm, plans)| {
                        crate::generator::tx_payload_with_model(*cluster, comm, plans, tx, slot)
                    })
                    // A frame the model does not name carries no signal values:
                    // the entry's own bytes are the whole payload.
                    .unwrap_or((tx.data, tx.len, tx.flags));
                // 发送时计算并存储：快照直接读存储值，UI 显示随消息周期
                // 跳变，不再逐帧重算。
                tx.last_sent = data;
                tx.last_len = len;
                let frame = CanFrame {
                    t_us: slot,
                    channel: tx.channel,
                    id: tx.id,
                    extended: tx.extended,
                    len,
                    data,
                    dir: Direction::Tx,
                    flags,
                };
                // The wire egress in Real bus mode: simulated frames go
                // onto the attached channel *and* stay on the internal
                // bus so every view sees them.
                self.hw.write_if_live(tx.channel, &frame);
                emitted.push(frame);
            }
        }
        self.buf.extend(emitted);

        // Steps that only received FlexRay frames publish too, or the FR
        // section would freeze until the next CAN arrival.
        let mut fr_landed = fr_replayed;
        // A FlexRay log delivers a frame every macrotick, and a row clone is
        // what keeping the arrival around for its handlers costs -- so the copy
        // happens only when some node actually declared `on fr slot`.
        let fr_listeners = self.nodes.iter().any(|n| n.waits_on_flexray());
        let mut fr_queued: Vec<CanFrame> = Vec::new();
        for row in fr_replay {
            self.accept_fr_row(row, now_us, fr_listeners, &mut fr_queued, status);
        }

        // Replay blocks stream their recorded traffic onto the same sim
        // clock the generator uses. Replay mode is excluded: the whole log
        // already IS the source there, and doubling its frames up would
        // poison every aggregate, load figure, and spec verdict.
        if !matches!(self.mode, Mode::Replay) {
            let mut block_out: Vec<CanFrame> = Vec::new();
            for block in &mut self.replay_blocks {
                block.poll(sim, &mut block_out, MAX_TX_CATCHUP as usize);
            }
            self.buf.extend(block_out);

            // Hardware RX: whatever an attached adapter received is real
            // bus traffic and ingests like everything else. Replay mode
            // stays wire-free for the same double-delivery reason.
            let mut hw_rx: Vec<CanFrame> = Vec::new();
            self.hw.poll_rx(sim, &mut hw_rx);
            self.buf.extend(hw_rx);

            // The FlexRay watch drains on the same cadence as the CAN
            // adapters. Its rows skip the CAN pipeline: the ingest goes
            // straight to the FR ring.
            let mut fr_rx: Vec<crate::trace::FrRow> = Vec::new();
            self.hw.poll_fr(sim, &mut fr_rx);
            if !fr_rx.is_empty() {
                fr_landed = true;
                for row in fr_rx {
                    self.accept_fr_row(row, now_us, fr_listeners, &mut fr_queued, status);
                }
            }
            for frame in self.hw.poll_eth(sim) {
                self.ingest_eth(frame);
            }
        }
        // FlexRay generator entries fill their slot on the same clock and with
        // the same catch-up discipline as the CAN ones, and stand down for a slot
        // the replayed log carries itself. Their rows enter through the receive
        // path like any other arrival; a live TX-capable watch also receives the
        // same payload, while a parked/missing port leaves the row session-only.
        let mut fr_gen: Vec<crate::trace::FrRow> = Vec::new();
        let dbs = &self.fr_dbs;
        let fr_models: std::collections::BTreeMap<u8, ModelHandle> = self
            .fr_models
            .iter()
            .filter_map(|(bus, runtime)| {
                let cluster = *self.fr_cluster_ids.get(bus)?;
                let current = self.fr_dbs.get(bus)?;
                Arc::ptr_eq(&runtime.db, current).then(|| {
                    (
                        *bus,
                        (
                            cluster,
                            Arc::clone(&runtime.comm),
                            Arc::clone(&runtime.plans),
                        ),
                    )
                })
            })
            .collect();
        let replaying = matches!(self.mode, Mode::Replay);
        for tx in &mut self.fr_tx_list {
            if replaying && tx.next_t_us == 0 {
                // Log time has no slot zero: an entry picked up mid-log starts at
                // the playhead rather than emitting one frame dated the epoch.
                tx.next_t_us = sim;
            }
            let muted = replaying && self.replay_fr_ids.contains(&(tx.bus, tx.slot));
            let mut budget = MAX_TX_CATCHUP;
            while budget > 0 && tx.active && !muted && tx.cycle_us != 0 && tx.next_t_us <= sim {
                budget -= 1;
                // Values are read at the slot's own stamp, exactly as on the CAN
                // side: a frame dated `at` must carry the waveform at `at`.
                let at = tx.next_t_us;
                tx.next_t_us += tx.cycle_us;
                let payload = fr_models
                    .get(&tx.bus)
                    .and_then(|(cluster, comm, plans)| {
                        crate::generator::fr_tx_payload_with_model(*cluster, comm, plans, tx, at)
                    })
                    // Without a model the description drives no signal; the
                    // entry's stored bytes are sent unchanged.
                    .unwrap_or_else(|| tx.data.clone());
                tx.last_sent = payload.clone();
                let cycle = dbs
                    .get(&tx.bus)
                    .map_or(0, |db| crate::generator::fr_cycle_at(db, at));
                fr_gen.push(crate::trace::FrRow {
                    bus: tx.bus,
                    t_us: at,
                    // Not a reception on a channel: 2 is the "unknown" the log
                    // readers already use for a row that answers that question
                    // nowhere.
                    ab: 2,
                    slot: tx.slot,
                    cycle,
                    payload,
                    header_crc: 0,
                    flags: 0,
                    name: None,
                });
            }
        }
        // A script's `fr_send` joins the session through the same funnel a
        // generator slot uses, so everything that sees an arrival sees it: Trace,
        // Messages, load and spec, triggers, `on fr slot` handlers, the recording.
        // The width is the description's, not the script's: a short payload is
        // zero-padded to the slot, and one longer than the slot is refused with a
        // line in that node's log rather than truncated into a frame nobody asked
        // for. A handler that wakes on this step's own rows queues its frames for
        // the next generator block, which is what keeps a script that answers
        // itself to one round per step.
        let from_scripts = std::mem::take(&mut self.fr_from_scripts);
        for (node_id, bus, slot, data) in from_scripts {
            // A script cannot invent a bus. `fr_send` to a cluster that carries
            // neither a description nor a watch would otherwise conjure a row in
            // the Buses table that reads "日志里有这路流量" -- traffic nobody
            // received, named after a typo. What the 路 *is* is the frontend's
            // `fr_buses` plus the watches, and this is the core-side reading of
            // the same two facts (see `App::fr_bus_rows`).
            let known = self.fr_dbs.contains_key(&bus) || self.hw.fr_watches.contains_key(&bus);
            if !known {
                if let Some(n) = self.nodes.iter_mut().find(|n| n.id == node_id) {
                    n.note(format!(
                        "[fr_send] dropped: FR{bus} is not a configured bus (load a cluster description or attach a watch first)"
                    ));
                }
                continue;
            }
            let declared = self.fr_dbs.get(&bus).and_then(|db| {
                db.frame_ix_of_slot(slot)
                    .and_then(|ix| db.frame_index(ix))
                    .map(|f| f.length as usize)
            });
            let payload = match declared {
                Some(w) if data.len() > w => {
                    if let Some(n) = self.nodes.iter_mut().find(|n| n.id == node_id) {
                        n.note(format!(
                            "[fr_send] dropped: FR{bus} slot {slot} declares {w} bytes, script supplied {}",
                            data.len()
                        ));
                    }
                    continue;
                }
                Some(w) => {
                    let mut p = data;
                    p.resize(w, 0);
                    p
                }
                None => data,
            };
            let cycle = self
                .fr_dbs
                .get(&bus)
                .map_or(0, |db| crate::generator::fr_cycle_at(db, sim));
            fr_gen.push(crate::trace::FrRow {
                bus,
                t_us: sim,
                ab: 2,
                slot,
                cycle,
                payload,
                header_crc: 0,
                flags: 0,
                name: None,
            });
        }
        for (bus, slot, payload, cycle) in std::mem::take(&mut self.panel_fr_frames) {
            fr_gen.push(crate::trace::FrRow {
                bus,
                t_us: sim,
                ab: 2,
                slot,
                cycle,
                payload,
                header_crc: 0,
                flags: 0,
                name: None,
            });
        }
        if !fr_gen.is_empty() {
            if !replaying {
                for row in &fr_gen {
                    match self.hw.write_fr(row.bus, row.slot, &row.payload) {
                        Ok(()) => {
                            self.fr_tx_errors.remove(&row.bus);
                        }
                        Err(error) => {
                            if self.fr_tx_errors.insert(row.bus) {
                                self.write_push(
                                    WriteKind::Warning,
                                    format!("FR{} transmit refused: {error}", row.bus),
                                );
                            }
                        }
                    }
                }
            }
            fr_landed = true;
            for row in fr_gen {
                self.accept_fr_row(row, now_us, fr_listeners, &mut fr_queued, status);
            }
        }
        // Frames an `on fr slot` handler sent join this same step's walk, so a
        // FlexRay arrival reacts on the tick it arrived -- not one later.
        if !fr_queued.is_empty() {
            self.buf.extend(fr_queued);
        }

        // Node timers fire before the ingest walk so the frames they
        // queue join this same step. Their clock is the step's wall
        // clock: periodic node behaviour keeps its cadence in replay too.
        let node_inputs = self.build_node_inputs(now_us);
        self.run_node_timers(now_us, &node_inputs);
        self.run_panel_actions(&node_inputs, status);

        let replay_done =
            matches!(self.mode, Mode::Replay) && source_empty && self.source.is_done();

        // Index walk rather than `for f in &self.buf`: a frame is copied
        // out one at a time, and a `while`, not a `for` over `0..len()`:
        // the range freezes its end before the loop, and a Send reaction
        // pushing onto `buf` mid-loop must be processed by this same step,
        // not wiped by the next one.
        let mut i = 0;
        let mut node_reactions = 0usize;
        let mut reaction_limit_noted = false;
        while i < self.buf.len() {
            let f = self.buf[i];
            i += 1;
            // Triggers judge the frame before anything else consumes it,
            // so a trigger that starts a recording captures the very
            // frame that fired it.
            self.eval_triggers(&f, status);
            // The record filter gates only what lands in the file: direct
            // writes, the pre-trigger context, and post-roll counting.
            // Trace, aggregates, and the spec always see the whole bus.
            let record_this = self.recorder.admits(&f);
            if record_this {
                self.recorder.write_event(&BusEvent::Can(&f));
                // Pre-trigger memory: a trigger that starts a recording drains
                // this ring into the file first, so the event keeps its past.
                self.pre_buffer.push_back(f);
                if self.pre_buffer.len() > self.pre_frames {
                    self.pre_buffer.pop_front();
                }
                // Post-roll: a trigger-initiated stop keeps the file open for
                // this many frames before the recorder actually closes.
                if let Some(left) = self.post_roll.as_mut() {
                    if *left <= 1 {
                        self.post_roll = None;
                        self.recorder.close();
                        self.recorder.recording = false;
                        *status = "trigger post-roll finished".to_string();
                    } else {
                        *left -= 1;
                    }
                }
            }
            // Ingest first, then dispatch to nodes: `sig()` in node
            // handlers reads the freshly-updated aggregates.
            self.ingest(f, stride);
            // Build fresh inputs after ingest so `sig()` reads current
            // values, not stale ones from before the frame arrived.
            let node_inputs = self.build_node_inputs(now_us);
            let input = node_inputs.get(&f.channel).cloned().unwrap_or_default();
            let queued = self.dispatch_node_frame(&f, &input);
            for (id, ext, data, flags) in queued {
                if node_reactions >= MAX_NODE_REACTIONS_PER_STEP {
                    if !reaction_limit_noted {
                        reaction_limit_noted = true;
                        self.write_push(
                            WriteKind::Warning,
                            format!(
                                "node reaction limit ({MAX_NODE_REACTIONS_PER_STEP}/step) reached; \
                                 extra frames were dropped — a handler is probably sending the frame it receives"
                            ),
                        );
                    }
                    continue;
                }
                node_reactions += 1;
                self.buf
                    .push(Self::node_frame(f.channel, id, ext, &data, flags, f.t_us));
            }
        }
        if i > 0 || fr_landed || eth_landed {
            self.publish_trace();
        }
        self.evaluate_derived();
        self.refresh_sub_histories();

        // One sample of the windowed numbers per step feeds the Min/Max/Avg
        // columns of the Bus Statistics window.
        for load in &mut self.bus_loads {
            load.sample();
        }
        for load in self.fr_loads.values_mut() {
            load.sample();
        }

        self.check_spec(tol_pct, grace);
        self.eval_timeout_triggers(now_us, grace, status);

        // A wire that refuses writes says so once per bus, here at the end of
        // the step so every write of this lap is behind it. The count keeps
        // growing on the Buses row; one line per refused frame would bury every
        // other news in the log, and a dead wire refuses thousands a second.
        for (bus, reason) in self.hw.take_tx_fail_news() {
            let name = if (bus as usize) < self.can_count() {
                self.can_name(bus as usize)
            } else {
                format!("bus {}", bus + 1)
            };
            self.write_push(
                WriteKind::Error,
                format!("{name}: transmit refused by the wire: {reason} (reported once; count is in Buses)"),
            );
        }

        // The record file is written as the measurement runs, not at Stop: this
        // pushes whatever the step added down to disk. A BLF container is a size
        // decision (128 KiB of objects), so on a slow bench the file would
        // otherwise sit at its 144-byte header for minutes while frames scroll
        // past in Trace.
        self.recorder.flush();

        if replay_done {
            self.measuring = false;
            self.replay_stop_at = None;
            self.source.set_poll_limit(None);
            self.recorder.close();
            self.finish_nodes();
            self.nodes_dirty = true;
            let dur = self.source.duration().unwrap_or(0) as f64 / 1e6;
            *status = format!("replay finished at {dur:.2}s");
        } else if self.measuring
            && matches!(self.mode, Mode::Replay)
            && !self.trace_paused
            && let Some(target) = self.replay_stop_at
            && self.source.position().is_some_and(|pos| pos >= target)
        {
            self.replay_stop_at = None;
            self.source.set_poll_limit(None);
            self.trace_paused = true;
            *status = format!("replay paused at {:.3}s", target as f64 / 1e6);
        }
        replay_done
    }

    /// Compare what arrived against what the databases promise.
    ///
    /// Once per step, not per frame: every verdict here is a claim about a
    /// *message's* timing or identity, and the loop above has already folded the
    /// frames into one aggregate per `(bus, id)`. Sweeping the aggregates turns a
    /// step of two hundred frames into one verdict each instead of two hundred
    /// latch writes, and it cannot read an aggregate mid-update the way a check
    /// inside that loop would. `tol_pct` and `grace` are the frontend's
    /// configured tolerances.
    pub(crate) fn check_spec(&mut self, tol_pct: u64, grace: u64) {
        let now = self.sim_t_us;
        // "Dropped" is the only verdict that needs a present tense. Replay runs
        // on the log's own timestamps, where "still going" has no meaning, so
        // only live simulation may call a message gone; pausing is covered
        // separately by the step not running at all. The other three are facts
        // about frames already seen and stay on in every mode.
        let live = matches!(self.mode, Mode::Virtual) && !self.trace_paused;
        /// A verdict bound to one message's class, with the declared and
        /// measured quantities for the report row.
        type Hit = ((u8, u32, bool, Kind), f64, f64);
        let mut hits: Vec<Hit> = Vec::new();
        let mut seen: Vec<((u8, u32, bool), u64)> = Vec::with_capacity(self.aggs.len());
        for agg in self.aggs.values() {
            let (ch, id, ext) = (agg.channel, agg.id, agg.extended);
            seen.push(((ch, id, ext), agg.last_t_us));
            // The model names the frame and states its period/length; a
            // channel without a database contributes no verdict at all.
            let model_decl = self.can_codec(ch).and_then(|(comm, _, cluster)| {
                comm.frame_by_can_id(cluster, id)
                    .into_iter()
                    .find(|frame| {
                        matches!(
                            frame.address,
                            crate::model::FrameAddress::Can { extended, .. } if extended == ext
                        )
                    })
                    .map(|frame| (frame.timing.cycle_us, u64::from(frame.length)))
            });
            let (declared, dlc) = match model_decl {
                Some(decl) => decl,
                None if self.model_dbs.get(ch as usize).is_some_and(Option::is_some) => {
                    hits.push(((ch, id, ext, Kind::Unknown), 0.0, 0.0));
                    continue;
                }
                None => continue,
            };
            // A declaration of 0 is event-triggered, which the two timing
            // predicates below reject on their own; `None` is the database
            // saying nothing, and neither is a period to check against.
            // Our own frames are exempt from the length and period verdicts.
            // Driving a signal that reaches past the base length widens the
            // frame on purpose (see `tx_payload`), so judging a Tx frame by the
            // declared DLC would convict a configuration chosen deliberately;
            // and the generator row already offers to restore a hand-tuned
            // period. Transmitting an id the database lacks is still reported.
            if matches!(agg.dir, Direction::Rx) {
                if dlc_offender(agg.len, dlc) {
                    hits.push(((ch, id, ext, Kind::Dlc), dlc as f64, f64::from(agg.len)));
                }
                // The interval since the previous step, never the running
                // average in `agg.cycle_us`: an EMA reads a five-fold stall as
                // 1.4x and takes twenty samples to converge, and `min_us` /
                // `max_us` latch forever, so either would hide or keep a
                // violation that a single real interval states plainly. A
                // message with no previous step, or with a step that brought it
                // no new frame, has no interval to judge yet.
                let elapsed = self
                    .spec
                    .previous((ch, id, ext))
                    .and_then(|from| agg.last_t_us.checked_sub(from))
                    .filter(|i| *i > 0);
                if let (Some(d), Some(interval)) = (declared, elapsed)
                    && cycle_offender(interval, d, tol_pct)
                {
                    hits.push(((ch, id, ext, Kind::Cycle), d as f64, interval as f64));
                }
            }
            if let (true, Some(d)) = (live, declared)
                && missing_offender(now, agg.last_t_us, d, grace)
            {
                hits.push((
                    (ch, id, ext, Kind::Missing),
                    d as f64,
                    now.saturating_sub(agg.last_t_us) as f64,
                ));
            }
        }
        // The FlexRay sweep. The description declares each frame's slot, cycle
        // phase, cycle repetition and payload length, so the same questions can
        // be asked of it: does an arrival name a frame the schedule knows at all
        // (Unknown), is it at least as long as declared (Dlc), does it come as
        // often as its repetition says (Cycle), and did one that was coming stop
        // (Missing). A FlexRay period is stated in cycles rather than
        // microseconds, so the declared interval is `cycle_repetition × the
        // cluster's cycle time`.
        let mut fr_hits: Vec<(crate::spec::FrSubject, Kind, f64, f64)> = Vec::new();
        let mut fr_seen: Vec<((u8, u16, usize), u64)> = Vec::with_capacity(self.fr_aggs.len());
        for ((bus, slot, occupant), agg) in &self.fr_aggs {
            // No description on this cluster: no opinion, not a clean bill.
            let Some(db) = self.fr_dbs.get(bus) else {
                continue;
            };
            let Some(frame_ix) = occupant.frame_ix() else {
                // The arrival's slot and phase name no frame in this cluster's
                // schedule -- the same fact an id outside the DBC states.
                fr_hits.push(((*bus, *slot, None), Kind::Unknown, 0.0, 0.0));
                continue;
            };
            let Some(f) = db.frame_index(frame_ix) else {
                continue;
            };
            let key = (*bus, *slot, frame_ix);
            let subject = (*bus, *slot, Some(frame_ix));
            fr_seen.push((key, agg.last_t_us));
            // Shorter than declared cannot be a complete frame. Longer is no
            // verdict: a FlexRay slot carries a fixed payload, and a logger that
            // records the whole slot leaves padding past the frame's own length
            // -- unlike CAN, where the frame *is* the thing on the wire.
            if agg.payload.len() < f.length as usize {
                fr_hits.push((
                    subject,
                    Kind::Dlc,
                    f64::from(f.length),
                    agg.payload.len() as f64,
                ));
            }
            let declared = if db.params.cycle_time_ms > 0.0 {
                (f.triggering.cycle_repetition.max(1) as f64 * db.params.cycle_time_ms * 1e3) as u64
            } else {
                0
            };
            // The interval since the previous step, exactly as on the CAN side:
            // the aggregate's smoothed period would read a five-fold stall as
            // 1.4x and hide the violation it is meant to report.
            let elapsed = self
                .spec
                .fr_previous(key)
                .and_then(|from| agg.last_t_us.checked_sub(from))
                .filter(|i| *i > 0);
            if declared > 0 {
                if let Some(interval) = elapsed
                    && cycle_offender(interval, declared, tol_pct)
                {
                    fr_hits.push((subject, Kind::Cycle, declared as f64, interval as f64));
                }
                if live && missing_offender(now, agg.last_t_us, declared, grace) {
                    fr_hits.push((
                        subject,
                        Kind::Missing,
                        declared as f64,
                        now.saturating_sub(agg.last_t_us) as f64,
                    ));
                }
            }
        }
        for (key, declared, measured) in hits {
            self.spec.record(key, now, declared, measured);
        }
        for (key, last_t_us) in seen {
            self.spec.note(key, last_t_us);
        }
        for (subject, kind, declared, measured) in fr_hits {
            self.spec.record_fr(subject, kind, now, declared, measured);
        }
        for (key, last_t_us) in fr_seen {
            self.spec.note_fr(key, last_t_us);
        }
    }
}
