//! The signal-observation model: per-signal sample caches, the subscription
//! statistics that fold samples into min/avg/max, and the Graphics/Data
//! window signal lists that pick what to observe.

use crate::app::{HISTORY_SPAN_US, SAMPLE_INTERVAL_US};

/// Signal samples, kept ascending by timestamp.
///
/// A `VecDeque` sufficed while samples only ever arrived through the playback
/// stream. Window backfill also decodes the stretch *behind* the playhead, so
/// insertion must work from either end: the hot streaming path still appends in
/// O(1), and out-of-order points fall back to a search + splice. Entries stay
/// 16 bytes with no per-node allocation, which matters because an hour of
/// 50 ms sampling is 72 000 points per signal.
#[derive(Debug, Default)]
pub struct SampleCache {
    pub(crate) points: Vec<(u64, f64)>,
}

impl SampleCache {
    pub fn len(&self) -> usize {
        self.points.len()
    }

    pub fn is_empty(&self) -> bool {
        self.points.is_empty()
    }

    pub fn first(&self) -> Option<(u64, f64)> {
        self.points.first().copied()
    }

    pub fn iter(&self) -> std::slice::Iter<'_, (u64, f64)> {
        self.points.iter()
    }

    pub fn clear(&mut self) {
        self.points.clear();
    }

    /// Merges an ascending batch, keeping the buffer sorted and rejecting any
    /// candidate that lands within `min_gap` of a point already held -- or of
    /// another accepted candidate. Returns the values it actually took, so the
    /// caller only folds those into its statistics.
    ///
    /// The gap rule matters: overlapping window requests re-read ground that is
    /// already cached, and without it each re-scan injects near-duplicate
    /// timestamps whose values differ, so the polyline zig-zags between them and
    /// the curve reads as a thick band instead of a line.
    pub fn merge(&mut self, batch: &[(u64, f64)], min_gap: u64) -> Vec<f64> {
        let gap = min_gap.max(1);
        if batch.is_empty() {
            return Vec::new();
        }
        // Fast path: the batch starts beyond everything held, so nothing can
        // collide except its own first element.
        if self
            .points
            .last()
            .is_none_or(|&(last_t, _)| batch[0].0 >= last_t + gap)
        {
            let taken = batch.iter().map(|&(_, v)| v).collect();
            self.points.extend_from_slice(batch);
            return taken;
        }
        let mut out = Vec::with_capacity(self.points.len() + batch.len());
        let mut taken = Vec::with_capacity(batch.len());
        let mut last_emitted: Option<u64> = None;
        let mut j = 0usize;
        for &(et, ev) in &self.points {
            while j < batch.len() && batch[j].0 < et {
                let (bt, bv) = batch[j];
                j += 1;
                let after_prev = last_emitted.is_none_or(|lt| bt >= lt + gap);
                let before_next = bt.saturating_add(gap) <= et;
                if after_prev && before_next {
                    out.push((bt, bv));
                    taken.push(bv);
                    last_emitted = Some(bt);
                }
            }
            out.push((et, ev));
            last_emitted = Some(et);
        }
        while j < batch.len() {
            let (bt, bv) = batch[j];
            j += 1;
            if last_emitted.is_none_or(|lt| bt >= lt + gap) {
                out.push((bt, bv));
                taken.push(bv);
                last_emitted = Some(bt);
            }
        }
        self.points = out;
        taken
    }

    /// Samples inside `[t_from, t_to]`, ascending. This is the only view the
    /// plotter needs: the visible window is a query, not a mutable buffer.
    pub fn range(&self, t_from: u64, t_to: u64) -> &[(u64, f64)] {
        let lo = self.points.partition_point(|(t, _)| *t < t_from);
        let hi = self.points.partition_point(|(t, _)| *t <= t_to);
        &self.points[lo..hi]
    }

    /// Value of the most recent sample at or before `t_us`.
    pub fn at(&self, t_us: u64) -> Option<f64> {
        let idx = self.points.partition_point(|&(ts, _)| ts <= t_us);
        idx.checked_sub(1).map(|i| self.points[i].1)
    }

    /// Drops the oldest samples until what remains fits inside `span_us` of the
    /// newest point.
    pub fn trim_oldest(&mut self, span_us: u64) {
        let Some(&(newest, _)) = self.points.last() else {
            return;
        };
        let horizon = newest.saturating_sub(span_us);
        let stale = self.points.partition_point(|(t, _)| *t < horizon);
        if stale > 0 {
            self.points.drain(..stale);
        }
    }
}

pub struct Subscription {
    pub latest: f64,
    pub unit: String,
    /// The database's enum label for the latest value, if one names it.
    pub label: Option<String>,
    /// How the database types this signal (`u8`/`i16`/`f32`/`f64`); the value
    /// prints in its own format and the tag goes after it. Identity like
    /// `unit`: set from the database at subscribe time, refreshed per frame.
    pub type_tag: String,
    pub min: f64,
    pub max: f64,
    /// Running average over the sampled values.
    pub avg: f64,
    pub(crate) sum: f64,
    pub(crate) n: u64,
    pub last_update_us: u64,
    pub last_sample_us: u64,
    pub history: SampleCache,
    pub color: usize,
}

impl Subscription {
    /// Records one sample and drops whatever has fallen out of the retention
    /// span. `min`/`max`/`avg` stay cumulative over everything sampled since the
    /// run began -- recomputing them every time the oldest point is trimmed would
    /// cost an O(n) rescan per sample.
    pub(crate) fn push_sample(&mut self, t_us: u64, v: f64) {
        // Same spacing rule as a backfill merge. Comparing only against the
        // newest cached point would be wrong after a rewind, where the cache
        // legitimately still holds points ahead of the playhead.
        if self
            .history
            .merge(&[(t_us, v)], SAMPLE_INTERVAL_US)
            .is_empty()
        {
            return;
        }
        self.last_sample_us = t_us;
        self.observe(v);
        self.history.trim_oldest(HISTORY_SPAN_US);
    }

    /// Folds one sampled value into the running statistics.
    pub(crate) fn observe(&mut self, v: f64) {
        if v < self.min {
            self.min = v;
        }
        if v > self.max {
            self.max = v;
        }
        self.sum += v;
        self.n += 1;
        self.avg = self.sum / self.n as f64;
    }

    /// Forgets everything the sampler accumulates, keeping the signal's identity
    /// (`unit`, `type_tag`, `color`). Emptying the cache alone is not a reset: the sampler
    /// gates on `t_us >= last_sample_us + SAMPLE_INTERVAL_US`, so a baseline
    /// inherited from the previous run silently rejects every frame until the
    /// playhead climbs past where the old run ended -- which reads as the start
    /// of the trace going missing.
    pub(crate) fn reset_measurement(&mut self) {
        self.history.clear();
        self.clear_accumulators();
    }

    pub(crate) fn clear_accumulators(&mut self) {
        self.latest = 0.0;
        self.label = None;
        self.min = f64::INFINITY;
        self.max = f64::NEG_INFINITY;
        self.avg = 0.0;
        self.sum = 0.0;
        self.n = 0;
        self.last_update_us = 0;
        self.last_sample_us = 0;
    }

    /// Lets the sampler resume at a scrubbed playhead. The cache is deliberately
    /// left alone: points ahead of the playhead are simply outside the visible
    /// window, and deleting them is what used to blank the curve on a rewind.
    pub(crate) fn resume_sampling_at(&mut self, t_us: u64) {
        if self.last_sample_us > t_us {
            self.last_sample_us = t_us;
        }
    }
}

pub struct GfxSignal {
    pub key: (u8, u32, String),
    pub visible: bool,
}

pub struct GraphicsWindow {
    pub name: String,
    pub signals: Vec<GfxSignal>,
    pub time_window_s: f64,
    pub stacked: bool,
    pub opened: bool,
    pub t_offset_s: f64,
    pub show_cursor: bool,
    pub zoom_enabled: bool,
    /// Draw a dot on each sample when the visible points are sparse enough to
    /// read individually.
    pub show_markers: bool,
}

pub struct DataWindow {
    pub name: String,
    pub signals: Vec<GfxSignal>,
    pub opened: bool,
    /// Visualization column style: true = value bar, false = sparkline;
    /// clicking the column toggles it.
    pub viz_bar: bool,
}



use std::collections::HashMap;

use crate::app::{App, MAX_SCAN_FRAMES};
use crate::can::frame::CanFrame;
use crate::dbc::DecodedSignal;

impl App {

    /// Decodes whatever frames cover `[t_from_us, t_to_us]` into the signal
    /// caches, unless an earlier scan already read that span.
    ///
    /// This is what makes the plot independent of the playback cursor. Deriving
    /// samples only as playback walks past them meant a Graphics window showing
    /// ground the cursor had not reached contained no points at all -- which is
    /// exactly what a forward scrub looked like.
    pub fn ensure_samples_in(&mut self, t_from_us: u64, t_to_us: u64) {
        if t_to_us <= t_from_us {
            return;
        }
        // Read only the edges the cache does not already cover. During playback
        // the visible window slides forward by tens of milliseconds every frame,
        // and re-reading the whole window each time would both cost that much
        // again and overwrite the cache's spacing.
        let (cov_lo, cov_hi) = self.sample_cover.unwrap_or((t_from_us, t_from_us));
        let mut edges = Vec::new();
        if t_from_us < cov_lo {
            edges.push((t_from_us, t_to_us.min(cov_lo)));
        }
        if t_to_us > cov_hi {
            edges.push((cov_hi.max(t_from_us), t_to_us));
        }
        for (from, to) in edges {
            if to > from {
                self.backfill(from, to);
            }
        }
    }

    /// One scan + merge over a span known to be uncovered.
    fn backfill(&mut self, t_from_us: u64, t_to_us: u64) {
        let mut frames = Vec::new();
        let capped = !self
            .source
            .scan_range(t_from_us, t_to_us, MAX_SCAN_FRAMES, &mut frames);
        // A scan-local stride: the shared per-signal baseline sits at the
        // playhead and would reject every point that lies behind it.
        let mut stride: HashMap<(u8, u32, String), u64> = HashMap::new();
        let mut batches: HashMap<(u8, u32, String), Vec<(u64, f64)>> = HashMap::new();
        for f in &frames {
            for (key, d) in self.subscribed_values(f) {
                if stride
                    .get(&key)
                    .is_some_and(|&lt| f.t_us < lt + SAMPLE_INTERVAL_US)
                {
                    continue;
                }
                stride.insert(key.clone(), f.t_us);
                batches.entry(key).or_default().push((f.t_us, d.phys));
            }
        }
        for (key, pts) in batches {
            if let Some(sub) = self.subs.get_mut(&key) {
                let taken = sub.history.merge(&pts, SAMPLE_INTERVAL_US);
                for v in taken {
                    sub.observe(v);
                }
            }
        }
        // Claim the span that was *asked for*, not merely what was read. Repeating
        // an unsatisfied request every frame would rescan the same stretch
        // forever; a scan stopped by the frame cap can therefore leave the tail of
        // a very dense window thin until the view moves enough to ask again.
        self.sample_cover = match self.sample_cover {
            Some((lo, hi)) if t_from_us <= hi && t_to_us >= lo => {
                Some((lo.min(t_from_us), hi.max(t_to_us)))
            }
            _ => Some((t_from_us, t_to_us)),
        };
        if capped {
            self.status = format!(
                "plot: window too dense to decode fully ({} frames)",
                MAX_SCAN_FRAMES
            );
        }
    }

    /// Lets every signal's sampler resume at a scrubbed playhead. Retained
    /// samples are left in place; see [`Subscription::resume_sampling_at`].
    pub(crate) fn rewind_samples_to(&mut self, t_us: u64) {
        for sub in self.subs.values_mut() {
            sub.resume_sampling_at(t_us);
        }
    }

    /// Decodes one frame into `(signal key, decoded signal)` pairs for the
    /// signals that are currently subscribed. Shared by the playback loop and
    /// the Graphics window backfill so the two cannot drift on what a signal's
    /// value is.
    pub(crate) fn subscribed_values(&self, f: &CanFrame) -> Vec<((u8, u32, String), DecodedSignal)> {
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

    /// The display type the database declares for a subscribed signal, used
    /// until the first frame refreshes it -- and forever when no database
    /// names it.
    fn signal_meta(&self, key: &(u8, u32, String)) -> String {
        self.channels
            .get(key.0 as usize)
            .and_then(|c| c.dbc.as_ref())
            .and_then(|db| db.messages.get(&key.1))
            .and_then(|m| m.signals.iter().find(|s| s.name == key.2))
            .map(|s| s.type_tag.clone())
            .unwrap_or_default()
    }

    pub fn subscribe(&mut self, key: (u8, u32, String)) {
        if !self.subs.contains_key(&key) {
            let color = self.color_counter;
            self.color_counter += 1;
            let type_tag = self.signal_meta(&key);
            self.subs.insert(
                key,
                Subscription {
                    latest: 0.0,
                    unit: String::new(),
                    label: None,
                    type_tag,
                    min: f64::INFINITY,
                    max: f64::NEG_INFINITY,
                    avg: 0.0,
                    sum: 0.0,
                    n: 0,
                    last_update_us: 0,
                    last_sample_us: 0,
                    history: SampleCache::default(),
                    color,
                },
            );
        }
    }

    /// Drops the subscription if no Data/Graphics window references the signal anymore.
    pub fn prune_signal(&mut self, key: &(u8, u32, String)) {
        let in_use = self
            .graphics
            .iter()
            .any(|g| g.signals.iter().any(|s| &s.key == key))
            || self
                .data_windows
                .iter()
                .any(|d| d.signals.iter().any(|s| &s.key == key));
        if !in_use {
            self.subs.remove(key);
        }
    }
}

