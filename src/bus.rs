//! The bus core: everything that must keep running at the bus's own rate,
//! independent of the frontend's frame loop.
//!
//! Phase 1 of the split (TODO.md, 主线): the struct exists and `App` derefs
//! to it, so bus fields and methods migrate slice by slice while every
//! existing access site keeps compiling. Everything is still driven from the
//! UI thread; the command/snapshot boundary and the dedicated thread are
//! stages 2 and 3.

use std::collections::{HashMap, VecDeque};

use crate::aggregate::MessageAgg;
use crate::app::{Mode, SAMPLE_INTERVAL_US, TRACE_LIMIT};
use crate::can::frame::{CanFrame, Direction};
use crate::channel::Channel;
use crate::dbc::DecodedSignal;
use crate::generator::TxMsg;
use crate::observe::Subscription;
use crate::spec::{Kind, cycle_offender, dlc_offender, missing_offender};
use crate::trigger::{TriggerAction, TriggerCond};

/// Slots one generator entry may backfill per step after the clock jumped
/// (a frozen UI, a seek). Bounds the burst; anything longer streams over the
/// following steps. At 100 Hz one step covers ~10 s of missed timeline.
pub(crate) const MAX_TX_CATCHUP: u32 = 1024;

/// What the frontend may ask the bus to do. One variant per transport
/// action, deliberately carrying no UI state -- no picked file paths, no
/// window selections; the frontend resolves those before asking. That is
/// what lets the same enum cross a thread unchanged in stage 3.
#[derive(Clone, Debug, PartialEq)]
pub enum BusCommand {
    /// Fresh virtual run: swap in a new virtual source, blank every
    /// run-scored counter, measure against the wall clock.
    StartVirtual,
    /// Stop measuring and close the recorder.
    Stop,
    /// Freeze or unfreeze the trace views; frozen arrivals stamp at the
    /// freeze instant.
    SetTracePaused(bool),
    /// Toggle ASC recording. While stopped, the file is created by the
    /// next start -- checking Record must not leave an empty file behind.
    ToggleRecord,
}

/// The simulation half of the application. Fields move here from `App` in
/// slices; during the migration `App` derefs to this, so existing field
/// access keeps compiling. Crate-wide visibility is migration scaffolding --
/// the command/snapshot boundary (stage 2) replaces it.
pub struct BusCore {
    /// Which timeline the bus runs on: Virtual generates against the sim
    /// clock, Replay follows a loaded log's own stamps.
    pub(crate) mode: Mode,
    /// The mode the current run started in; switching buses restores it.
    pub(crate) run_mode: Mode,
    /// The bus's frame input: the virtual idle bus, or the log being
    /// replayed / scanned for backfill.
    pub(crate) source: Box<dyn crate::source::FrameSource>,
    /// Every (channel, id) the loaded log carries. While replaying,
    /// generator entries carrying one of these ids stand down -- replaying
    /// a recording of this very simulation used to interleave two senders
    /// of one signal. Filled by [`App::replay`], consulted only in Replay
    /// mode, never persisted.
    pub(crate) replay_ids: std::collections::HashSet<(u8, u32)>,
    pub(crate) channels: Vec<Channel>,
    /// The interactive generator's entries: what this tool transmits as.
    pub(crate) tx_list: Vec<TxMsg>,
    /// Simulation clock: accumulates only while measuring and unpaused.
    /// Generator frames are stamped on it and their signal values are
    /// evaluated from it, so a pause freezes the bus in place instead of
    /// letting it jump phase. Replay polling still uses the wall clock
    /// (`last_tick_us`, still on `App` during the migration).
    pub(crate) sim_t_us: u64,
    /// `now_us()` at the previous accepted tick, the reference for `sim_t_us`.
    pub(crate) sim_prev_us: u64,
    /// Next palette index handed to a new subscription.
    pub(crate) color_counter: usize,
    /// Frames received this run, capped at [`TRACE_LIMIT`] as a ring.
    pub(crate) trace: VecDeque<CanFrame>,
    /// Trace-window freeze: arrivals keep coming but are stamped at the
    /// pause instant instead of their own time, so the view stays still and
    /// resuming does not dump a burst of backdated rows.
    pub(crate) trace_paused: bool,
    /// Wall-clock instant the freeze started; None while not frozen.
    pub(crate) paused_at_us: Option<u64>,
    /// Frames received this run, for the run-total counter in the status bar.
    pub(crate) frame_counter: u64,
    /// Per-(bus, id) aggregates behind the Messages / Statistics views.
    pub(crate) aggs: HashMap<(u8, u32), MessageAgg>,
    /// Per-bus load / frame-rate / error rolling state, one entry per channel.
    pub(crate) bus_loads: Vec<crate::load::BusLoad>,
    /// Subscribed signals: latest value, min/max/avg, sampled history.
    pub(crate) subs: HashMap<(u8, u32, String), Subscription>,
    /// Contiguous log-time span whose frames have already been decoded into
    /// the signal caches. A Graphics window asking for a range outside it
    /// triggers a backfill scan.
    pub(crate) sample_cover: Option<(u64, u64)>,
    /// The sampling stride currently applied to the signal caches. When the
    /// smallest Graphics window shrinks, the stride gets finer -- and spans
    /// already scanned at the coarse stride must rescan, so `sample_cover`
    /// is invalidated here too.
    pub(crate) applied_stride_us: u64,
    /// True while the measurement runs: the clock advances, generators
    /// emit, replay polls. A pause stops all of it in place.
    pub(crate) measuring: bool,
    /// Frames polled this step, plus frames pushed by Send reactions;
    /// the tick loop walks it by index so late arrivals are processed.
    pub(crate) buf: Vec<CanFrame>,
    /// ASC recording state: the checkbox intent plus the open file.
    pub(crate) recorder: crate::recorder::Recorder,
    /// User rules judged per frame (edges) and per step (aggregate sweeps).
    pub(crate) triggers: Vec<crate::trigger::Trigger>,
    /// Observed-versus-declared violations, recomputed on every step from
    /// `aggs` and the loaded databases.
    pub(crate) spec: crate::spec::Spec,
}

impl BusCore {
    /// Empty bus state; `App::new` layers the sample channel configuration
    /// and the measurement start on top.
    pub(crate) fn new(bus_loads: Vec<crate::load::BusLoad>) -> Self {
        BusCore {
            mode: Mode::Virtual,
            run_mode: Mode::Virtual,
            source: Box::new(crate::source::virtual_source::VirtualSource::new()),
            replay_ids: std::collections::HashSet::new(),
            channels: Vec::new(),
            tx_list: Vec::new(),
            sim_t_us: 0,
            sim_prev_us: 0,
            color_counter: 0,
            trace: VecDeque::with_capacity(TRACE_LIMIT.min(8_192)),
            trace_paused: false,
            paused_at_us: None,
            frame_counter: 0,
            aggs: HashMap::new(),
            bus_loads,
            subs: HashMap::new(),
            sample_cover: None,
            applied_stride_us: SAMPLE_INTERVAL_US,
            measuring: false,
            buf: Vec::new(),
            recorder: crate::recorder::Recorder::new(),
            triggers: Vec::new(),
            spec: crate::spec::Spec::default(),
        }
    }

    /// Executes a command from the frontend. Single-threaded this is a
    /// plain call (`App::send`); stage 3 sends the same enum over a
    /// channel instead. Status text an action produces goes into `status`
    /// -- the bus has no display of its own.
    pub(crate) fn handle(&mut self, cmd: BusCommand, status: &mut String) {
        match cmd {
            BusCommand::StartVirtual => self.start_virtual(status),
            BusCommand::Stop => self.stop_bus(status),
            BusCommand::SetTracePaused(on) => self.trace_paused = on,
            BusCommand::ToggleRecord => self.toggle_record(status),
        }
    }

    /// Fresh virtual run: new source, blank run state, wall-clock
    /// measuring.
    fn start_virtual(&mut self, status: &mut String) {
        self.recorder.close();
        self.source = Box::new(crate::source::virtual_source::VirtualSource::new());
        self.mode = Mode::Virtual;
        self.run_mode = Mode::Virtual;
        self.reset_run();
        self.measuring = true;
        *status = "measuring (virtual)".to_string();
        if self.recorder.recording {
            match self.recorder.open() {
                Ok(path) => *status = format!("recording to {path}"),
                Err(e) => *status = format!("record failed: {e}"),
            }
        }
    }

    /// Blanks every run-scored counter: rings, aggregates, loads, spec
    /// memory, generator schedules, subscription measurement state.
    pub(crate) fn reset_run(&mut self) {
        self.sim_t_us = 0;
        self.sim_prev_us = 0;
        // A fresh start must not inherit the previous run's pause state.
        self.trace_paused = false;
        self.paused_at_us = None;
        self.trace.clear();
        self.frame_counter = 0;
        self.aggs.clear();
        for load in &mut self.bus_loads {
            load.clear();
        }
        // Along with the aggregates it reads: keeping the previous run's
        // interval memory would turn the first step of a new run into one
        // enormous measured period.
        self.spec = crate::spec::Spec::default();
        self.sample_cover = None;
        for tx in &mut self.tx_list {
            tx.next_t_us = 0;
        }
        for sub in self.subs.values_mut() {
            sub.reset_measurement();
        }
    }

    fn stop_bus(&mut self, status: &mut String) {
        self.measuring = false;
        self.recorder.close();
        *status = "stopped".to_string();
    }

    fn toggle_record(&mut self, status: &mut String) {
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
                    Ok(path) => *status = format!("recording to {path}"),
                    Err(e) => *status = format!("record failed: {e}"),
                }
            }
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
        self.buf.clear();
        self.source.poll(now_us, &mut self.buf);
        let source_empty = self.buf.is_empty();
        if stride != self.applied_stride_us {
            self.sample_cover = None;
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

        // Generators transmit in every running mode: an active entry during
        // replay injects onto the log timeline, which is what makes
        // "replay a real log and stir a few frames in" possible. One
        // exception: an id the log itself carries stays silent, or replaying
        // a recording of this same simulation interleaves two senders of one
        // signal and every consumer sees their mixed values. Only consulted
        // in Replay mode, so nothing to restore when the run ends.
        let channels = &self.channels;
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
            // Every slot the clock has passed goes out at its own stamp.
            // Skipping the backlog after a UI stall (the old policy) kept the
            // tick cheap but punched a hole into the bus's own timeline --
            // and at Graphics strides fine enough to show single updates,
            // that hole reads as the curve being eaten while the plot
            // slides on. Frames carry their slot's timestamp, so spacing
            // stays exactly `cycle_us` even in the catch-up burst.
            let mut budget = MAX_TX_CATCHUP;
            while budget > 0 && tx.active && !muted && tx.cycle_us != 0 && tx.next_t_us <= sim {
                budget -= 1;
                // Values are read at the slot, not at `sim`: a frame stamped
                // `slot` must carry the waveform's value at `slot`, or every
                // payload would lead its own timestamp by up to a full cycle.
                let slot = tx.next_t_us;
                tx.next_t_us += tx.cycle_us;
                let (data, len, flags) = crate::generator::tx_payload(channels, tx, slot);
                emitted.push(CanFrame {
                    t_us: slot,
                    channel: tx.channel,
                    id: tx.id,
                    extended: tx.extended,
                    len,
                    data,
                    dir: Direction::Tx,
                    flags,
                });
            }
        }
        self.buf.extend(emitted);

        let replay_done =
            matches!(self.mode, Mode::Replay) && source_empty && self.source.is_done();

        // Index walk rather than `for f in &self.buf`: a frame is copied
        // out one at a time, and a `while`, not a `for` over `0..len()`:
        // the range freezes its end before the loop, and a Send reaction
        // pushing onto `buf` mid-loop must be processed by this same step,
        // not wiped by the next one.
        let mut i = 0;
        while i < self.buf.len() {
            let f = self.buf[i];
            i += 1;
            // Triggers judge the frame before anything else consumes it,
            // so a trigger that starts a recording captures the very
            // frame that fired it.
            self.eval_triggers(&f, status);
            self.recorder.write(&f);
            self.ingest(f, stride);
        }

        // One sample of the windowed numbers per step feeds the Min/Max/Avg
        // columns of the Bus Statistics window.
        for load in &mut self.bus_loads {
            load.sample();
        }

        self.check_spec(tol_pct, grace);
        self.eval_timeout_triggers(now_us, grace, status);

        if replay_done {
            self.measuring = false;
            self.recorder.close();
            let dur = self.source.duration().unwrap_or(0) as f64 / 1e6;
            *status = format!("replay finished at {dur:.2}s");
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
        let mut hits: Vec<((u8, u32, Kind), f64, f64)> = Vec::new();
        let mut seen: Vec<((u8, u32), u64)> = Vec::with_capacity(self.aggs.len());
        for (&key, agg) in &self.aggs {
            let (ch, id) = key;
            seen.push((key, agg.last_t_us));
            // No database on this bus means no opinion, not a clean bill.
            let Some(db) = self.channel_dbc(ch) else {
                continue;
            };
            let Some(m) = db.messages.get(&id) else {
                hits.push(((ch, id, Kind::Unknown), 0.0, 0.0));
                continue;
            };
            // A declaration of 0 is event-triggered, which the two timing
            // predicates below reject on their own; `None` is the database
            // saying nothing, and neither is a period to check against.
            let declared = m.cycle_us;
            // Our own frames are exempt from the length and period verdicts.
            // Driving a signal that reaches past the base length widens the
            // frame on purpose (see `tx_payload`), so judging a Tx frame by the
            // declared DLC would convict a configuration chosen deliberately;
            // and the generator row already offers to restore a hand-tuned
            // period. Transmitting an id the database lacks is still reported.
            if matches!(agg.dir, Direction::Rx) {
                if dlc_offender(agg.len, m.dlc) {
                    hits.push(((ch, id, Kind::Dlc), m.dlc as f64, f64::from(agg.len)));
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
                    .previous(key)
                    .and_then(|from| agg.last_t_us.checked_sub(from))
                    .filter(|i| *i > 0);
                if let (Some(d), Some(interval)) = (declared, elapsed)
                    && cycle_offender(interval, d, tol_pct)
                {
                    hits.push(((ch, id, Kind::Cycle), d as f64, interval as f64));
                }
            }
            if let (true, Some(d)) = (live, declared)
                && missing_offender(now, agg.last_t_us, d, grace)
            {
                hits.push((
                    (ch, id, Kind::Missing),
                    d as f64,
                    now.saturating_sub(agg.last_t_us) as f64,
                ));
            }
        }
        for (key, declared, measured) in hits {
            self.spec.record(key, now, declared, measured);
        }
        for (key, last_t_us) in seen {
            self.spec.note(key, last_t_us);
        }
    }

    /// The database loaded on bus `ch`, if any.
    pub(crate) fn channel_dbc(&self, ch: u8) -> Option<&crate::dbc::SymbolTable> {
        self.channels.get(ch as usize).and_then(|c| c.dbc.as_ref())
    }

    /// What the database declares for this message: `Some(0)` for an
    /// event-triggered one, `None` when it says nothing at all.
    pub(crate) fn dbc_cycle_us(&self, ch: u8, id: u32) -> Option<u64> {
        self.channel_dbc(ch)
            .and_then(|db| db.messages.get(&id))
            .and_then(|m| m.cycle_us)
    }

    /// Folds one accepted frame into every bus-side consumer: the trace
    /// ring, the run counter, the per-bus load rollups, the per-message
    /// aggregates and the subscribed-signal caches. `stride` is the sampling
    /// period the frontend currently wants for signal history; everything
    /// else this touches is the bus's own state.
    pub(crate) fn ingest(&mut self, f: CanFrame, stride: u64) {
        if self.trace.len() >= TRACE_LIMIT {
            self.trace.pop_front();
        }
        self.frame_counter += 1;
        // Error frames included: they occupy the bus and the load view
        // counts them; per-message aggregation below skips them.
        if let Some(load) = self.bus_loads.get_mut(f.channel as usize) {
            let (arb, data) = {
                let ch = &self.channels[f.channel as usize];
                (ch.bitrate_kbps, ch.fd_data_kbps)
            };
            let wire = crate::load::wire_time_us(&f, arb, data);
            load.note(&f, wire);
        }
        if f.is_error() {
            // Error frames carry no identifier and no payload; they are
            // intentionally kept out of per-message aggregation.
            self.trace.push_back(f);
            return;
        }
        let agg = self.aggs.entry((f.channel, f.id)).or_insert(MessageAgg {
            id: f.id,
            extended: f.extended,
            channel: f.channel,
            dir: f.dir,
            count: 0,
            last_t_us: 0,
            cycle_us: 0.0,
            min_us: f64::MAX,
            max_us: 0.0,
            len: f.len,
            data: f.data,
            flags: f.flags,
        });
        // Only a strictly later timestamp marks a real cycle. A backwards
        // or repeated one is a discontinuity -- a seek, or an out-of-order
        // log row -- and folding it in used to pin `min_us` at zero for the
        // rest of the run. The running average keeps its pre-seek value and
        // resumes blending at the next real interval.
        if agg.count > 0 && f.t_us > agg.last_t_us {
            let dt = (f.t_us - agg.last_t_us) as f64;
            agg.cycle_us = if agg.count == 1 {
                dt
            } else {
                agg.cycle_us * 0.9 + dt * 0.1
            };
            if dt < agg.min_us {
                agg.min_us = dt;
            }
            if dt > agg.max_us {
                agg.max_us = dt;
            }
        }
        agg.count += 1;
        agg.last_t_us = f.t_us;
        agg.channel = f.channel;
        agg.dir = f.dir;
        agg.len = f.len;
        agg.data = f.data;
        agg.flags = f.flags;
        for (key, d) in self.subscribed_values(&f) {
            let Some(entry) = self.subs.get_mut(&key) else {
                continue;
            };
            entry.latest = d.phys;
            entry.last_raw = d.raw;
            entry.unit = d.unit;
            entry.type_tag = d.type_tag;
            entry.label = d.label;
            entry.last_update_us = f.t_us;
            if f.t_us >= entry.last_sample_us + stride || entry.history.is_empty() {
                entry.push_sample(f.t_us, d.phys, stride);
            }
        }
        self.trace.push_back(f);
    }

    /// The frames a frame carries for signals this run subscribes to, looked
    /// up in the sending bus's database.
    pub(crate) fn subscribed_values(
        &self,
        f: &CanFrame,
    ) -> Vec<((u8, u32, String), DecodedSignal)> {
        let Some(db) = self
            .channels
            .get(f.channel as usize)
            .and_then(|c| c.dbc.as_ref())
        else {
            return Vec::new();
        };
        db.decode_signals(f)
            .into_iter()
            .filter_map(|d| {
                let key = (f.channel, f.id, d.name.clone());
                self.subs.contains_key(&key).then_some((key, d))
            })
            .collect()
    }

    /// Folds one frame into every enabled trigger and acts on edges.
    /// Runs as the first thing that happens to a received frame, so a
    /// trigger that starts a recording still captures the frame that
    /// fired it. Status messages an action wants to show go into
    /// `status` (last write wins, as the status bar only has one line).
    pub(crate) fn eval_triggers(&mut self, f: &CanFrame, status: &mut String) {
        if self.triggers.is_empty() {
            return;
        }
        let mut fired: Vec<(TriggerAction, u64)> = Vec::new();
        for i in 0..self.triggers.len() {
            // Observe under shared borrows first: acting on an edge
            // needs `self` exclusively, and the two cannot overlap.
            let now = {
                let t = &self.triggers[i];
                if !t.enabled {
                    continue;
                }
                match &t.cond {
                    TriggerCond::SignalCross {
                        ch,
                        id,
                        signal,
                        threshold,
                        rising,
                    } => {
                        if f.channel != *ch || f.id != *id || f.is_error() {
                            // Not this message's frame: the level holds.
                            continue;
                        }
                        let Some(db) = self.channel_dbc(*ch) else {
                            continue; // no database on the bus, no opinion
                        };
                        match db.decode_signals(f).into_iter().find(|d| d.name == *signal) {
                            Some(d) => {
                                if *rising {
                                    d.phys >= *threshold
                                } else {
                                    d.phys <= *threshold
                                }
                            }
                            // The condition names a signal the database
                            // lacks -- same courtesy as a missing db.
                            None => continue,
                        }
                    }
                    TriggerCond::IdPresent { ch, id } => {
                        if f.channel != *ch || f.id != *id || f.is_error() {
                            continue;
                        }
                        true // latch: once seen, it stays seen
                    }
                    TriggerCond::ErrorFrame { ch } => {
                        if f.channel != *ch || !f.is_error() {
                            continue;
                        }
                        true
                    }
                    // Not a frame condition: swept once per step against
                    // the aggregates in `eval_timeout_triggers`.
                    TriggerCond::CycleTimeout { .. } => continue,
                }
            };
            let t = &mut self.triggers[i];
            let was = t.level;
            t.level = now;
            if now && !was {
                t.fired += 1;
                t.last_fire_t_us = f.t_us;
                fired.push((t.action, f.t_us));
            }
        }
        self.run_actions(fired, status);
    }

    /// Sweep conditions: evaluated once per measurement step against the
    /// aggregates, not per frame. A message only convicts after it has
    /// been seen once (the Missing verdict takes the same stance), and
    /// the level clears when traffic resumes, so every new dropout is a
    /// fresh edge. `grace` is the spec's missed-cycles tolerance, a
    /// frontend setting the bus does not own.
    pub(crate) fn eval_timeout_triggers(&mut self, now_us: u64, grace: u64, status: &mut String) {
        if self.triggers.is_empty() {
            return;
        }
        let mut fired: Vec<(TriggerAction, u64)> = Vec::new();
        for i in 0..self.triggers.len() {
            let (ch, id) = match &self.triggers[i].cond {
                TriggerCond::CycleTimeout { ch, id } => (*ch, *id),
                _ => continue,
            };
            if !self.triggers[i].enabled {
                continue;
            }
            let silent = self.timeout_silent(ch, id, now_us, grace);
            let t = &mut self.triggers[i];
            let was = t.level;
            t.level = silent;
            if silent && !was {
                t.fired += 1;
                t.last_fire_t_us = now_us;
                fired.push((t.action, now_us));
            }
        }
        self.run_actions(fired, status);
    }

    /// The spec's own grace comparison decides silence, so a trigger and
    /// the Dropped verdict can never disagree about the same message.
    fn timeout_silent(&self, ch: u8, id: u32, now_us: u64, grace: u64) -> bool {
        let Some(agg) = self.aggs.get(&(ch, id)) else {
            return false; // never seen: no opinion, not a dropout
        };
        let Some(declared) = self.dbc_cycle_us(ch, id) else {
            return false; // no database, message, or declared period
        };
        crate::spec::missing_offender(now_us, agg.last_t_us, declared, grace)
    }

    fn run_actions(&mut self, fired: Vec<(TriggerAction, u64)>, status: &mut String) {
        for (action, at_us) in fired {
            match action {
                TriggerAction::StartRecording => {
                    if self.measuring && !self.recorder.recording {
                        self.recorder.recording = true;
                        let opened = self.recorder.open();
                        self.recorder.recording = opened.is_ok();
                        *status = match opened {
                            Ok(path) => format!("trigger started recording to {path}"),
                            Err(e) => format!("trigger record failed: {e}"),
                        };
                    }
                }
                TriggerAction::StopRecording => {
                    if self.recorder.recording {
                        self.recorder.close();
                        self.recorder.recording = false;
                        *status = "trigger stopped recording".to_string();
                    }
                }
                TriggerAction::Send { ch, id } => {
                    self.send_one_shot(ch, id, at_us);
                }
            }
        }
    }

    /// Transmits one frame from the generator entry `(ch, id)`, stamped
    /// `at_us`. The frame goes onto `buf`, so the running tick processes
    /// it exactly like received traffic -- trace, aggregates, load,
    /// recording -- and the trigger evaluator sees it too. That is safe:
    /// every frame-driven condition latches on the frame it matched, so
    /// a rule reacting to its own output cannot loop.
    fn send_one_shot(&mut self, ch: u8, id: u32, at_us: u64) {
        let Some(i) = self
            .tx_list
            .iter()
            .position(|t| t.channel == ch && t.id == id)
        else {
            return; // the generator row is gone: the rule idles
        };
        let (data, len, flags) =
            crate::generator::tx_payload(&self.channels, &self.tx_list[i], at_us);
        let (tch, tid, ext) = (
            self.tx_list[i].channel,
            self.tx_list[i].id,
            self.tx_list[i].extended,
        );
        self.buf.push(CanFrame {
            t_us: at_us,
            channel: tch,
            id: tid,
            extended: ext,
            len,
            data,
            dir: crate::can::frame::Direction::Tx,
            flags,
        });
    }
}
