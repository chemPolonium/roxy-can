//! The signal-observation model: per-signal sample caches, the subscription
//! statistics that fold samples into min/avg/max, and the Graphics/Data
//! window signal lists that pick what to observe.

use crate::app::{HISTORY_SPAN_US, MIN_STRIDE_US, SAMPLE_INTERVAL_US, STRIDE_POINTS_PER_WINDOW};
use std::sync::Arc;

/// Signal samples, kept ascending by timestamp.
///
/// Storage is chunked so that publishing a snapshot to the frontend never
/// copies the whole buffer: sealed chunks are immutable and shared by
/// `Arc` (a clone bumps refcounts), only the live tail is copied, and a
/// tail that outgrows [`SEAL_POINTS`] is sealed by move -- zero point
/// copies. Mutation of a shared chunk goes through `Arc::make_mut`, so
/// the working cache never disturbs a view the UI is still reading.
///
/// A `VecDeque` sufficed while samples only ever arrived through the playback
/// stream. Window backfill also decodes the stretch *behind* the playhead, so
/// insertion must work from either end: the hot streaming path still appends in
/// O(1), and out-of-order points fall back to a flatten-merge-rechunk (the
/// same O(total) the flat buffer always paid).
#[derive(Debug, Default)]
pub struct SampleCache {
    chunks: Vec<std::sync::Arc<Vec<(u64, f64)>>>,
    tail: Vec<(u64, f64)>,
    total: usize,
}

/// Live points are promoted into an immutable shared chunk once the tail
/// reaches this size, bounding every publish's copy to one tail.
pub(crate) const SEAL_POINTS: usize = 512;

impl Clone for SampleCache {
    fn clone(&self) -> Self {
        // Chunks are shared, not copied; only the tail (bounded by
        // SEAL_POINTS) is a real copy.
        Self {
            chunks: self.chunks.clone(),
            tail: self.tail.clone(),
            total: self.total,
        }
    }
}

impl SampleCache {
    /// Point count; the Data sparkline that read this is gone, but the
    /// accessor belongs with the rest of the slice API and the tests use it.
    #[allow(dead_code)]
    pub fn len(&self) -> usize {
        self.total
    }

    pub fn is_empty(&self) -> bool {
        self.total == 0
    }

    pub fn first(&self) -> Option<(u64, f64)> {
        if let Some(c) = self.chunks.first() {
            return c.first().copied();
        }
        self.tail.first().copied()
    }

    pub fn iter(&self) -> impl Iterator<Item = &(u64, f64)> {
        self.chunks
            .iter()
            .flat_map(|c| c.iter())
            .chain(self.tail.iter())
    }

    pub fn clear(&mut self) {
        self.chunks.clear();
        self.tail.clear();
        self.total = 0;
    }

    /// Stamp of the newest point, wherever it lives.
    fn last_t(&self) -> Option<u64> {
        self.tail
            .last()
            .or_else(|| self.chunks.last().and_then(|c| c.last()))
            .map(|p| p.0)
    }

    fn push_point(&mut self, p: (u64, f64)) {
        self.tail.push(p);
        self.total += 1;
        if self.tail.len() >= SEAL_POINTS {
            let sealed = std::sync::Arc::new(std::mem::take(&mut self.tail));
            self.chunks.push(sealed);
        }
    }

    /// Replaces the whole contents with an ascending buffer, re-chunked.
    fn install(&mut self, points: Vec<(u64, f64)>) {
        self.total = points.len();
        self.chunks = points
            .chunks(SEAL_POINTS)
            .map(|c| std::sync::Arc::new(c.to_vec()))
            .collect();
        self.tail = Vec::new();
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
        // collide except its own first element. This is the live sampling
        // path -- append-only, no existing point is touched.
        if self
            .last_t()
            .is_none_or(|last_t| batch[0].0 >= last_t + gap)
        {
            let taken = batch.iter().map(|&(_, v)| v).collect();
            for &p in batch {
                self.push_point(p);
            }
            return taken;
        }
        // Overlap or rewind: flatten, merge, re-chunk. The same O(total)
        // the flat buffer always paid on this path.
        let mut old: Vec<(u64, f64)> = Vec::with_capacity(self.total);
        for c in &self.chunks {
            old.extend_from_slice(c);
        }
        old.extend_from_slice(&self.tail);

        let mut merged: Vec<(u64, f64)> = Vec::with_capacity(old.len() + batch.len());
        let mut taken = Vec::with_capacity(batch.len());
        let mut last_emitted: Option<u64> = None;
        let mut j = 0usize;
        for &(et, ev) in &old {
            while j < batch.len() && batch[j].0 < et {
                let (bt, bv) = batch[j];
                j += 1;
                let after_prev = last_emitted.is_none_or(|lt| bt >= lt + gap);
                let before_next = bt.saturating_add(gap) <= et;
                if after_prev && before_next {
                    merged.push((bt, bv));
                    taken.push(bv);
                    last_emitted = Some(bt);
                }
            }
            merged.push((et, ev));
            last_emitted = Some(et);
        }
        while j < batch.len() {
            let (bt, bv) = batch[j];
            j += 1;
            if last_emitted.is_none_or(|lt| bt >= lt + gap) {
                merged.push((bt, bv));
                taken.push(bv);
                last_emitted = Some(bt);
            }
        }
        self.install(merged);
        taken
    }

    /// Samples inside `[t_from, t_to]`, ascending. This is the only view the
    /// plotter needs: the visible window is a query, not a mutable buffer.
    /// Chunk-level bounds skip whole chunks; only the window's edges pay a
    /// binary search.
    pub fn range(&self, t_from: u64, t_to: u64) -> Samples<'_> {
        let mut segs: Vec<&[(u64, f64)]> = Vec::new();
        let mut remaining = 0usize;
        for c in &self.chunks {
            // Stamps ascend across chunks, so scanning can stop for good.
            if c.last().is_none_or(|p| p.0 < t_from) {
                continue;
            }
            if c.first().is_some_and(|p| p.0 > t_to) {
                break;
            }
            let lo = c.partition_point(|(t, _)| *t < t_from);
            let hi = c.partition_point(|(t, _)| *t <= t_to);
            if hi > lo {
                segs.push(&c[lo..hi]);
                remaining += hi - lo;
            }
        }
        let lo = self.tail.partition_point(|(t, _)| *t < t_from);
        let hi = self.tail.partition_point(|(t, _)| *t <= t_to);
        if hi > lo {
            segs.push(&self.tail[lo..hi]);
            remaining += hi - lo;
        }
        Samples {
            segs: segs.into_iter(),
            cur: &[],
            remaining,
        }
    }

    /// Value of the most recent sample at or before `t_us`.
    pub fn at(&self, t_us: u64) -> Option<f64> {
        let full = self
            .chunks
            .partition_point(|c| c.last().is_some_and(|p| p.0 <= t_us));
        if full < self.chunks.len() {
            let c = &self.chunks[full];
            let pos = c.partition_point(|&(ts, _)| ts <= t_us);
            if pos > 0 {
                return Some(c[pos - 1].1);
            }
            if full > 0 {
                return self.chunks[full - 1].last().map(|p| p.1);
            }
            return None;
        }
        let pos = self.tail.partition_point(|&(ts, _)| ts <= t_us);
        if pos > 0 {
            return Some(self.tail[pos - 1].1);
        }
        self.chunks.last().and_then(|c| c.last()).map(|p| p.1)
    }

    /// Drops the oldest samples until what remains fits inside `span_us` of the
    /// newest point.
    pub fn trim_oldest(&mut self, span_us: u64) {
        let Some(horizon) = self.last_t().map(|newest| newest.saturating_sub(span_us)) else {
            return;
        };
        while self
            .chunks
            .first()
            .is_some_and(|c| c.last().is_some_and(|p| p.0 < horizon))
        {
            let dropped = self.chunks.remove(0);
            self.total -= dropped.len();
        }
        if let Some(c) = self.chunks.first_mut() {
            let stale = c.partition_point(|(t, _)| *t < horizon);
            if stale > 0 {
                // COW: a published view may still hold this chunk.
                let c = std::sync::Arc::make_mut(c);
                c.drain(..stale);
                self.total -= stale;
            }
            return;
        }
        let stale = self.tail.partition_point(|(t, _)| *t < horizon);
        if stale > 0 {
            self.tail.drain(..stale);
            self.total -= stale;
        }
    }
}

/// One ascending window of samples, borrowed from a [`SampleCache`]'s
/// chunks. Knows its exact length so the plotter can size its fold
/// without a first pass.
pub struct Samples<'a> {
    segs: std::vec::IntoIter<&'a [(u64, f64)]>,
    cur: &'a [(u64, f64)],
    remaining: usize,
}

impl<'a> Iterator for Samples<'a> {
    type Item = &'a (u64, f64);

    fn next(&mut self) -> Option<Self::Item> {
        loop {
            if let Some((p, rest)) = self.cur.split_first() {
                self.cur = rest;
                self.remaining -= 1;
                return Some(p);
            }
            self.cur = self.segs.next()?;
        }
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        (self.remaining, Some(self.remaining))
    }
}

impl ExactSizeIterator for Samples<'_> {}

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
    /// The published view of `history`, shared with the frontend by Arc
    /// and rebuilt only when sampling or a backfill actually changed the
    /// cache -- the same discipline as the trace ring's `publish_trace`.
    /// Core-side only; the snapshot carries `published`.
    pub(crate) published: Arc<SampleCache>,
    pub(crate) history_dirty: bool,
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
        self.history_dirty = true;
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
        self.history_dirty = true;
    }

    /// Rebuilds the published Arc when the working cache changed; a
    /// pointer clone otherwise. Called once per step (and after backfills
    /// and resets), never per frame.
    pub(crate) fn refresh_published_history(&mut self) {
        if self.history_dirty {
            self.published = Arc::new(self.history.clone());
            self.history_dirty = false;
        }
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

/// A signal's custom state definitions -- CANoe's "Value Definition"
/// rows. Ascending cut points split the value axis into bands; band i
/// holds `cuts[i-1] <= v < cuts[i]` (the first from -inf, the last to
/// +inf, and a cut value itself belongs to the band above it). Every
/// band carries its own name and fill color; `None` means automatic --
/// the band takes a stable palette slot like auto-colored values do.
#[derive(Clone)]
pub struct StateRule {
    pub cuts: Vec<f64>,
    pub names: Vec<String>,
    pub colors: Vec<Option<[f32; 3]>>,
}

impl StateRule {
    /// The band index holding `v`.
    pub fn band(&self, v: f64) -> usize {
        self.cuts.partition_point(|&c| c <= v)
    }

    /// Inserts a cut, growing names/colors with generated entries so the
    /// band bookkeeping stays consistent.
    pub fn add_cut(&mut self, c: f64) {
        let i = self.cuts.partition_point(|&x| x < c);
        self.cuts.insert(i, c);
        self.names.insert(i, format!("State {}", self.names.len()));
        self.colors.insert(i, None);
    }

    /// Removes cut `i`, merging the band below the boundary with the one
    /// above it (the lower band's name and color survive).
    pub fn remove_cut(&mut self, i: usize) {
        if i >= self.cuts.len() {
            return;
        }
        self.cuts.remove(i);
        self.names.remove(i + 1);
        self.colors.remove(i + 1);
    }
}

/// Which swatch of a State Tracker editor the color picker is editing.
#[derive(Clone, Copy, PartialEq)]
pub enum PickTarget {
    /// A custom band, by index into the rule's color list.
    Band(usize),
    /// A default-mode state, by the normalized bits of its value.
    Value(u64),
}

/// The State Tracker window: signals rendered as constant-value state
/// bands over a live trailing window, CANoe-style. An observer like
/// Graphics/Data: rows share their signal model so the selection popup,
/// the shared list, and persistence treat all three alike (the value-axis
/// policy is dead weight here but harmless).
pub struct StateWin {
    pub name: String,
    pub opened: bool,
    pub signals: Vec<GfxSignal>,
    /// Width of the live trailing window in seconds. The tracker always
    /// rides the live edge; panning and scrubbing belong to the curve
    /// windows.
    pub time_window_s: f64,
    /// Session memory of one palette slot per state value, keyed by signal
    /// then by the value's bits: a value keeps the slot it first drew
    /// with, so the same value keeps the same color as the run goes on
    /// and new states never reshuffle the old ones. Not persisted.
    pub(crate) color_slots: HashMap<(u8, u32, String), HashMap<u64, usize>>,
    /// Custom state bands per signal key (CANoe's Value Definition). When
    /// present, the signal is in custom mode and the rule drives both the
    /// band labels and the fill colors. Absent means default mode: states
    /// come from the VAL_ table or observed values.
    pub rules: HashMap<(u8, u32, String), StateRule>,
    /// Default-mode color overrides per signal key, keyed by the state
    /// value's normalized bits: an entry pins one state's color while the
    /// rest stay automatic.
    pub overrides: HashMap<(u8, u32, String), HashMap<u64, [f32; 3]>>,
}

impl Default for StateWin {
    fn default() -> Self {
        Self {
            name: String::new(),
            opened: false,
            signals: Vec::new(),
            time_window_s: 20.0,
            color_slots: HashMap::new(),
            rules: HashMap::new(),
            overrides: HashMap::new(),
        }
    }
}

use std::collections::HashMap;

use crate::app::App;

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

    /// Requests that whatever frames cover `[t_from_us, t_to_us]` be
    /// decoded into the signal caches, unless an earlier scan already read
    /// that span.
    ///
    /// This is what makes the plot independent of the playback cursor. Deriving
    /// samples only as playback walks past them meant a Graphics window showing
    /// ground the cursor had not reached contained no points at all -- which is
    /// exactly what a forward scrub looked like. The scan itself is core work
    /// (it owns the log source), so it goes through as a command; the filled
    /// history arrives in a later snapshot. Cover tracking rides the snapshot
    /// too, so an in-flight request is not re-sent.
    pub fn ensure_samples_in(&mut self, t_from_us: u64, t_to_us: u64) {
        if t_to_us <= t_from_us {
            return;
        }
        // Read only the edges the cache does not already cover. During playback
        // the visible window slides forward by tens of milliseconds every frame,
        // and re-reading the whole window each time would both cost that much
        // again and overwrite the cache's spacing.
        let (cov_lo, cov_hi) = self.snap.sample_cover.unwrap_or((t_from_us, t_from_us));
        let mut edges = Vec::new();
        if t_from_us < cov_lo {
            edges.push((t_from_us, t_to_us.min(cov_lo)));
        }
        if t_to_us > cov_hi {
            edges.push((cov_hi.max(t_from_us), t_to_us));
        }
        let stride = self.wanted_stride_us();
        for (from, to) in edges {
            if to > from {
                self.send(crate::bus::BusCommand::Backfill {
                    from_us: from,
                    to_us: to,
                    stride_us: stride,
                });
            }
        }
    }

    /// The database's declared min..max for a signal -- the scale the Data
    /// window's bar draws against. None when no database names the signal
    /// or declares a usable range on it.
    pub(crate) fn declared_range(&self, key: &(u8, u32, String)) -> Option<(f64, f64)> {
        self.snap
            .channels
            .get(key.0 as usize)
            .and_then(|c| c.dbc.as_deref())
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
            let Some(sub) = self.sub_view(key) else {
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
            .map(|key| match self.sub_view(key) {
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

    /// Starts caching one signal on the bus (command `Subscribe`; the
    /// creation semantics live there now).
    pub fn subscribe(&mut self, key: (u8, u32, String)) {
        self.send(crate::bus::BusCommand::Subscribe { key });
    }

    /// This frame's snapshot view of subscribed signal `key`. All frontend
    /// reads of subscription state go through here, never the live map.
    pub(crate) fn sub_view(&self, key: &(u8, u32, String)) -> Option<&crate::bus::SubView> {
        self.snap.subs.iter().find(|s| &s.key == key)
    }

    /// Drops the subscription if no Data/Graphics window or State Tracker
    /// row references the signal anymore. The "still referenced" judgement
    /// is frontend policy; the removal itself goes through the command so
    /// the bus owns its own map.
    pub fn prune_signal(&mut self, key: &(u8, u32, String)) {
        let in_use = self
            .graphics
            .iter()
            .any(|g| g.signals.iter().any(|s| &s.key == key))
            || self
                .data_windows
                .iter()
                .any(|d| d.signals.iter().any(|s| &s.key == key))
            || self
                .state_trackers
                .iter()
                .any(|w| w.signals.iter().any(|s| &s.key == key));
        if !in_use {
            self.send(crate::bus::BusCommand::Unsubscribe { key: key.clone() });
        }
    }
}
