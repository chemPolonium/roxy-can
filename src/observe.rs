//! The signal-observation model: per-signal sample caches, the subscription
//! statistics that fold samples into min/avg/max, and the Graphics/Data
//! window signal lists that pick what to observe.

use crate::app::{HISTORY_SPAN_US, MIN_STRIDE_US, SAMPLE_INTERVAL_US, STRIDE_POINTS_PER_WINDOW};

/// Signal samples, kept ascending by timestamp.
///
/// A `VecDeque` sufficed while samples only ever arrived through the playback
/// stream. Window backfill also decodes the stretch *behind* the playhead, so
/// insertion must work from either end: the hot streaming path still appends in
/// O(1), and out-of-order points fall back to a search + splice. Entries stay
/// 16 bytes with no per-node allocation, which matters because an hour of
/// coarse-stride sampling is 72 000 points per signal (the stride tightens
/// while a Graphics window is zoomed in, trading cache size for detail).
#[derive(Debug, Default)]
pub struct SampleCache {
    pub(crate) points: Vec<(u64, f64)>,
}

impl SampleCache {
    /// Point count; the Data sparkline that read this is gone, but the
    /// accessor belongs with the rest of the slice API and the tests use it.
    #[allow(dead_code)]
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
    /// The raw integer the latest frame carried for this signal, before
    /// factor and offset -- the Data window's Raw Value column.
    pub last_raw: i64,
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
    pub(crate) fn push_sample(&mut self, t_us: u64, v: f64, min_gap: u64) {
        // Same spacing rule as a backfill merge. Comparing only against the
        // newest cached point would be wrong after a rewind, where the cache
        // legitimately still holds points ahead of the playhead.
        if self.history.merge(&[(t_us, v)], min_gap).is_empty() {
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
        self.last_raw = 0;
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
    /// This signal's value-axis policy. Each curve scales on its own: one
    /// signal riding Auto breathes with its window while a neighbour stays
    /// locked, and in one-plot-per-signal mode every pane follows its own
    /// signal's choice. In overlay the shared axis is the union of what the
    /// signals' policies ask for.
    pub y_mode: YMode,
}

/// How a Graphics window scales its value axis.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum YMode {
    /// Fit whatever the visible time window holds, every frame.
    #[default]
    Auto,
    /// Freeze the axis at the range it shows when the mode is picked. New
    /// data cannot move it until the mode is left and re-entered.
    Lock,
    /// Grow the range to contain every value the run has produced and never
    /// shrink it back, so once a value has arrived it stays on the plot.
    FitAll,
    /// Scale against the database's declared min..max per signal; a signal
    /// without declarations falls back to its observed extremes.
    Dbc,
}

impl YMode {
    pub const ALL: [YMode; 4] = [YMode::Auto, YMode::Lock, YMode::FitAll, YMode::Dbc];

    /// One-letter form for the per-signal badge in the curve list.
    pub fn short(&self) -> &'static str {
        match self {
            YMode::Auto => "A",
            YMode::Lock => "L",
            YMode::FitAll => "F",
            YMode::Dbc => "D",
        }
    }

    /// Full name, in menu and legend positions.
    pub fn label(&self) -> &'static str {
        match self {
            YMode::Auto => "Auto",
            YMode::Lock => "Lock",
            YMode::FitAll => "Fit all",
            YMode::Dbc => "DBC range",
        }
    }

    /// One-line explanation for the hover hints -- the names alone leave
    /// too much to guess.
    pub fn hint(&self) -> &'static str {
        match self {
            YMode::Auto => "Fit the visible time window, rescaling every frame",
            YMode::Lock => "Freeze the axis at the range on screen right now",
            YMode::FitAll => "Grow to fit every value seen, never shrink back",
            YMode::Dbc => "Scale by the min..max declared in the database",
        }
    }

    /// Project-file code; unknown values load as [`YMode::Auto`].
    pub fn to_u8(self) -> u8 {
        self as u8
    }

    pub fn from_u8(v: u8) -> Self {
        Self::ALL.get(v as usize).copied().unwrap_or(YMode::Auto)
    }
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
    /// Frozen ranges while a signal's [`YMode::Lock`] is active, keyed by the
    /// signal key's debug text. Session state only -- leaving the mode clears
    /// the entry.
    pub(crate) y_locks: HashMap<String, (f64, f64)>,
    /// Legend readouts ("EngineSpeed = 100 rpm") as of the last throttled
    /// text refresh, aligned with `legend_keys`; the plot draws these so the
    /// digits hold still while the curve itself animates at full frame rate.
    /// Session state only.
    pub(crate) legend_keys: Vec<(u8, u32, String)>,
    pub(crate) legend: Vec<String>,
}

pub struct DataWindow {
    pub name: String,
    pub signals: Vec<GfxSignal>,
    pub opened: bool,
    /// Value/unit/raw strings as of the last throttled text refresh; the
    /// table draws these so digits hold still long enough to read, while
    /// the bar next to them animates at full frame rate.
    pub(crate) text_keys: Vec<(u8, u32, String)>,
    pub(crate) text_cache: Vec<[String; 3]>,
}

use std::collections::HashMap;

use crate::app::{App, MAX_SCAN_FRAMES};

impl GraphicsWindow {
    /// The throttled legend strings for `keys`, in the same order; an empty
    /// string stands in until the first text refresh fills the snapshot.
    pub(crate) fn legend_for(&self, keys: &[(u8, u32, String)]) -> Vec<String> {
        keys.iter()
            .map(|key| {
                self.legend_keys
                    .iter()
                    .position(|k| k == key)
                    .and_then(|n| self.legend.get(n))
                    .cloned()
                    .unwrap_or_default()
            })
            .collect()
    }
}

impl App {
    /// The sampling stride the signal caches should hold right now: derived
    /// from the smallest open Graphics window so a tight zoom gets every
    /// signal update, while wide windows keep the coarse stride that bounds
    /// the cache to a few ten-thousand points per signal.
    pub(crate) fn wanted_stride_us(&self) -> u64 {
        let min_window_us = self
            .graphics
            .iter()
            .filter(|g| g.opened)
            .map(|g| (g.time_window_s * 1e6) as u64)
            .min()
            .unwrap_or(u64::MAX);
        (min_window_us / STRIDE_POINTS_PER_WINDOW).clamp(MIN_STRIDE_US, SAMPLE_INTERVAL_US)
    }

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
        let stride = self.wanted_stride_us();
        let mut frames = Vec::new();
        let capped = !self
            .source
            .scan_range(t_from_us, t_to_us, MAX_SCAN_FRAMES, &mut frames);
        // A scan-local stride: the shared per-signal baseline sits at the
        // playhead and would reject every point that lies behind it.
        let mut stride_map: HashMap<(u8, u32, String), u64> = HashMap::new();
        let mut batches: HashMap<(u8, u32, String), Vec<(u64, f64)>> = HashMap::new();
        for f in &frames {
            for (key, d) in self.subscribed_values(f) {
                if stride_map.get(&key).is_some_and(|&lt| f.t_us < lt + stride) {
                    continue;
                }
                stride_map.insert(key.clone(), f.t_us);
                batches.entry(key).or_default().push((f.t_us, d.phys));
            }
        }
        for (key, pts) in batches {
            if let Some(sub) = self.subs.get_mut(&key) {
                let taken = sub.history.merge(&pts, stride);
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

    /// The database's declared min..max for a signal -- the scale the Data
    /// window's bar draws against. None when no database names the signal
    /// or declares a usable range on it.
    pub(crate) fn declared_range(&self, key: &(u8, u32, String)) -> Option<(f64, f64)> {
        self.channels
            .get(key.0 as usize)
            .and_then(|c| c.dbc.as_ref())
            .and_then(|db| db.messages.get(&key.1))
            .and_then(|m| m.signals.iter().find(|s| s.name == key.2))
            .and_then(|s| (s.max > s.min).then_some((s.min, s.max)))
    }

    /// Refreshes Data window `i`'s throttled text snapshot. Called every
    /// frame: it does nothing unless the text gate says so or the visible
    /// signal set changed (adding a signal must not wait a full period to
    /// show its first value).
    pub(crate) fn sync_data_text(&mut self, i: usize) {
        let keys: Vec<(u8, u32, String)> = self.data_windows[i]
            .signals
            .iter()
            .filter(|s| s.visible)
            .map(|s| s.key.clone())
            .collect();
        if self.data_windows[i].text_keys == keys && !self.text_fresh {
            return;
        }
        let mut rows = Vec::with_capacity(keys.len());
        for key in &keys {
            let Some(sub) = self.subs.get(key) else {
                continue;
            };
            rows.push([
                sub.label
                    .clone()
                    .unwrap_or_else(|| crate::dbc::fmt_decoded(&sub.type_tag, sub.latest)),
                sub.unit.clone(),
                sub.last_raw.to_string(),
            ]);
        }
        let win = &mut self.data_windows[i];
        win.text_keys = keys;
        win.text_cache = rows;
    }

    /// Refreshes Graphics window `i`'s throttled legend snapshot -- the
    /// "{name} = {value}" readouts the plot draws over the curves. Same gate
    /// as [`App::sync_data_text`]: no-op unless the text gate says so or the
    /// visible signal set changed.
    pub(crate) fn sync_gfx_legend(&mut self, i: usize) {
        let keys: Vec<(u8, u32, String)> = self.graphics[i]
            .signals
            .iter()
            .filter(|s| s.visible)
            .map(|s| s.key.clone())
            .collect();
        if self.graphics[i].legend_keys == keys && !self.text_fresh {
            return;
        }
        let legend = keys
            .iter()
            .map(|key| match self.subs.get(key) {
                Some(sub) => {
                    format!(
                        "{} = {}",
                        key.2,
                        crate::dbc::fmt_signal_value(
                            sub.latest,
                            &sub.unit,
                            &sub.type_tag,
                            sub.label.as_deref(),
                        )
                    )
                }
                None => format!("{} = -", key.2),
            })
            .collect();
        let win = &mut self.graphics[i];
        win.legend_keys = keys;
        win.legend = legend;
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
                    last_raw: 0,
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
