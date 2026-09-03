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
use crate::app::{SAMPLE_INTERVAL_US, TRACE_LIMIT};
use crate::can::frame::CanFrame;
use crate::channel::Channel;
use crate::dbc::DecodedSignal;
use crate::generator::TxMsg;
use crate::observe::Subscription;

/// The simulation half of the application. Fields move here from `App` in
/// slices; during the migration `App` derefs to this, so existing field
/// access keeps compiling. Crate-wide visibility is migration scaffolding --
/// the command/snapshot boundary (stage 2) replaces it.
pub struct BusCore {
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
}

impl BusCore {
    /// Empty bus state; `App::new` layers the sample channel configuration
    /// and the measurement start on top.
    pub(crate) fn new(bus_loads: Vec<crate::load::BusLoad>) -> Self {
        BusCore {
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
        }
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
}
