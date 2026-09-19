//! The trace ring: the run's last `TRACE_LIMIT` frames. Storage is
//! chunked so that publishing a snapshot shares sealed chunks instead of
//! copying the whole ring -- the same discipline as the signal-history
//! cache: append into a live tail, seal it by move when it outgrows
//! [`SEAL_FRAMES`], mutate shared chunks only through `Arc::make_mut`,
//! and publish by refcount bumps plus one tail copy.
//!
//! Frames the limit trims are **not** lost: they append to a spill
//! archive (a plain record file in the temp dir) that the trace export
//! replays ahead of the hot ring, so a long capture keeps its head in
//! the file even when the live view has moved past it.

use crate::can::frame::CanFrame;
use std::sync::Arc;

/// One FlexRay frame as the Trace window shows it: the static/dynamic
/// slot address, the cycle it arrived in and the raw payload. FR rows
/// live in their own ring and never enter the CAN aggregates, load
/// rollups or signal subscriptions -- a watch-only diagnostic.
#[derive(Clone, Debug, PartialEq)]
pub struct FrRow {
    pub t_us: u64,
    /// The reception channel: 0 = A, 1 = B. 2 = unknown -- the event
    /// buffer's channel-mask offset is not probe-verified yet.
    pub ab: u8,
    pub slot: u16,
    pub cycle: u8,
    pub payload: Vec<u8>,
    pub header_crc: u16,
    pub flags: u16,
    /// The frame name the log carried, when the format names frames
    /// (CANoe ASC does). The description database, when one is loaded,
    /// takes precedence in the display.
    pub name: Option<String>,
}

/// The FlexRay trace ring: the run's last rows, oldest first. A plain
/// `VecDeque` on purpose -- the CAN ring's chunked Arc sharing exists
/// for 50k-frame pipelines; FR rows are a bounded diagnostic feed whose
/// publish copies one bounded Vec.
#[derive(Debug, Default)]
pub struct FrRing {
    rows: std::collections::VecDeque<FrRow>,
    /// Rows trimmed from the head since the last `clear`.
    dropped: u64,
}

impl FrRing {
    /// Appends a row and trims the head past `limit`.
    pub fn push(&mut self, row: FrRow, limit: usize) {
        self.rows.push_back(row);
        let overflow = self.rows.len().saturating_sub(limit.max(1));
        self.rows.drain(..overflow);
        self.dropped += overflow as u64;
    }

    /// Rows trimmed from the head since the last clear.
    pub fn dropped(&self) -> u64 {
        self.dropped
    }

    pub fn clear(&mut self) {
        self.rows.clear();
        self.dropped = 0;
    }

    /// The snapshot's view: one bounded clone of the whole ring.
    pub fn publish(&self) -> Arc<Vec<FrRow>> {
        Arc::new(self.rows.iter().cloned().collect())
    }

    /// Core-side iteration, for test assertions on the working ring.
    #[cfg(test)]
    pub fn iter(&self) -> impl Iterator<Item = &FrRow> {
        self.rows.iter()
    }
}

/// Frames accumulate in the live tail and are sealed into an immutable
/// shared chunk at this size, bounding a publish's copy to one tail.
const SEAL_FRAMES: usize = 512;

/// One archived frame, fixed stride: t_us(8) channel(1) id(4) ext(1)
/// len(1) dir(1) flags(4) data(64). Simple beats compact -- the archive
/// is written once, read once per export.
const RECORD_LEN: usize = 8 + 1 + 4 + 1 + 1 + 1 + 4 + 64;

/// The bus-side ring: append at the back, trim from the front once the
/// limit is exceeded -- and everything trimmed lands in the spill
/// archive instead of vanishing. Head trims are accounted either way:
/// the dropped count and the first trimmed frame's timestamp surface in
/// the Trace window.
#[derive(Debug, Default)]
pub struct TraceRing {
    chunks: Vec<Arc<Vec<CanFrame>>>,
    tail: Vec<CanFrame>,
    total: usize,
    /// Frames trimmed from the head since the last `clear`.
    dropped: u64,
    /// Timestamp of the first frame the ring ever dropped (its own
    /// timeline), so the loss has a visible position, not just a count.
    first_dropped_t_us: Option<u64>,
    /// The archive the trimmed frames land in. Created lazily on the
    /// first trim; `None` until then and when the archive file could not
    /// be created (head loss is then real loss, and the UI says so).
    spill: Option<SpillFile>,
}

impl TraceRing {
    pub fn push(&mut self, f: CanFrame) {
        self.tail.push(f);
        self.total += 1;
        if self.tail.len() >= SEAL_FRAMES {
            let sealed = Arc::new(std::mem::take(&mut self.tail));
            self.chunks.push(sealed);
        }
    }

    /// Drops the oldest frames until at most `limit` remain: whole stale
    /// chunks first, then the head chunk trimmed in place (COW, so a
    /// published view keeps reading the chunk it holds).
    pub fn enforce_limit(&mut self, limit: usize) {
        let mut overflow = self.total.saturating_sub(limit);
        while overflow > 0 {
            let Some(head_len) = self.chunks.first().map(|c| c.len()) else {
                let stale = overflow.min(self.tail.len());
                let leaving: Vec<CanFrame> = self.tail[..stale].to_vec();
                self.tail.drain(..stale);
                self.total -= stale;
                self.note_dropped(&leaving);
                return;
            };
            if head_len <= overflow {
                let dropped = self.chunks.remove(0);
                self.note_dropped(&dropped);
                overflow -= dropped.len();
                self.total -= dropped.len();
            } else {
                let head = Arc::make_mut(self.chunks.first_mut().expect("checked above"));
                let leaving: Vec<CanFrame> = head[..overflow].to_vec();
                head.drain(..overflow);
                self.total -= overflow;
                self.note_dropped(&leaving);
                overflow = 0;
            }
        }
    }

    fn note_dropped(&mut self, frames: &[CanFrame]) {
        if frames.is_empty() {
            return;
        }
        if self.spill.is_none() {
            self.spill = SpillFile::create().ok();
        }
        if let Some(spill) = &mut self.spill {
            spill.append(frames);
        }
        self.dropped += frames.len() as u64;
        if self.first_dropped_t_us.is_none() {
            self.first_dropped_t_us = Some(frames[0].t_us);
        }
    }

    /// Rewrites every frame in place and drops the frames the rewrite
    /// rejects -- the bus-removal path (filter out one channel, shift the
    /// others down). One COW per touched chunk.
    pub fn rewrite(&mut self, mut f: impl FnMut(&mut CanFrame) -> bool) {
        let mut dropped = 0usize;
        for c in &mut self.chunks {
            let c = Arc::make_mut(c);
            let before = c.len();
            c.retain_mut(|frame| f(frame));
            dropped += before - c.len();
        }
        let before = self.tail.len();
        self.tail.retain_mut(|frame| f(frame));
        dropped += before - self.tail.len();
        self.total -= dropped;
    }

    /// The snapshot's view: chunk refcounts plus one tail copy.
    pub fn publish(&self) -> Arc<TraceView> {
        Arc::new(TraceView {
            chunks: self.chunks.clone(),
            tail: self.tail.clone(),
            total: self.total,
            dropped: self.dropped,
            first_dropped_t_us: self.first_dropped_t_us,
            archive: self
                .spill
                .as_ref()
                .map(|s| (s.path.clone(), s.frames)),
        })
    }

    pub fn len(&self) -> usize {
        self.total
    }

    /// The newest frame, wherever it lives.
    #[cfg(test)]
    pub fn back(&self) -> Option<&CanFrame> {
        self.tail
            .last()
            .or_else(|| self.chunks.last().and_then(|c| c.last()))
    }

    pub fn clear(&mut self) {
        self.chunks.clear();
        self.tail.clear();
        self.total = 0;
        self.dropped = 0;
        self.first_dropped_t_us = None;
        if let Some(spill) = &mut self.spill {
            spill.reset();
        }
    }

    /// Frames trimmed from the head since the last clear, and where the
    /// loss began (the first dropped frame's own timestamp).
    #[cfg(test)]
    pub fn head_loss(&self) -> (u64, Option<u64>) {
        (self.dropped, self.first_dropped_t_us)
    }

    /// The archive's path and frame count while a spill file exists.
    #[cfg(test)]
    pub fn archive(&self) -> Option<(&std::path::Path, u64)> {
        self.spill.as_ref().map(|s| (s.path.as_path(), s.frames))
    }

    /// Core-side iteration, for test assertions on the working ring.
    #[cfg(test)]
    pub fn iter(&self) -> impl Iterator<Item = &CanFrame> {
        self.chunks
            .iter()
            .flat_map(|c| c.iter())
            .chain(self.tail.iter())
    }

    /// A ring with a test-scoped archive path: parallel tests must not
    /// share the pid-suffixed default file.
    #[cfg(test)]
    fn with_spill_at(path: std::path::PathBuf) -> Self {
        let spill = Some(SpillFile::create_at(path).expect("spill file"));
        TraceRing {
            spill,
            ..Default::default()
        }
    }
}

/// The spill archive: an append-only record file the ring's trimmed
/// frames land in, read back when the trace is exported. Writes happen
/// on the core thread in whole trim batches (sealed chunks, ~44 KB) --
/// the same budget the per-frame ASC recorder already spends on this
/// path. One file per process (pid-suffixed) in the temp dir; a fresh
/// run truncates it.
#[derive(Debug)]
pub struct SpillFile {
    path: std::path::PathBuf,
    /// `None` only inside `Drop` (handle closed before deletion).
    file: Option<std::fs::File>,
    frames: u64,
}

impl SpillFile {
    /// The process's archive, created lazily on the first trim.
    fn create() -> std::io::Result<Self> {
        let path = std::env::temp_dir().join(format!(
            "roxy-can-trace-archive-{}.bin",
            std::process::id()
        ));
        Self::create_at(path)
    }

    /// `create` with an explicit path (tests run in parallel and must
    /// not share the pid-suffixed default).
    fn create_at(path: std::path::PathBuf) -> std::io::Result<Self> {
        let file = std::fs::OpenOptions::new()
            .create(true)
            .truncate(true)
            .write(true)
            .open(&path)?;
        Ok(SpillFile {
            path,
            file: Some(file),
            frames: 0,
        })
    }

    /// Appends frames as fixed-stride records, one buffered write per
    /// call. A failed write keeps the frames counted as dropped but ends
    /// the archive's growth -- partial records never reach the file.
    fn append(&mut self, frames: &[CanFrame]) {
        use std::io::Write;
        let Some(file) = self.file.as_mut() else {
            return;
        };
        let mut buf = Vec::with_capacity(frames.len() * RECORD_LEN);
        for f in frames {
            buf.extend_from_slice(&f.t_us.to_le_bytes());
            buf.push(f.channel);
            buf.extend_from_slice(&f.id.to_le_bytes());
            buf.push(f.extended as u8);
            buf.push(f.len);
            buf.push(f.dir as u8);
            buf.extend_from_slice(&f.flags.bits().to_le_bytes());
            buf.extend_from_slice(&f.data);
        }
        if file.write_all(&buf).is_ok() {
            self.frames += frames.len() as u64;
        }
    }

    /// Truncates the archive for a fresh run; the file itself stays
    /// (the next trim reuses it).
    fn reset(&mut self) {
        if let Some(file) = self.file.as_mut() {
            file.set_len(0).ok();
        }
        self.frames = 0;
    }

    /// Reads every archived frame back, in trim order. Used by the
    /// export path, which opens its own handle -- the writer on the core
    /// thread and a reader elsewhere never hold each other's locks.
    pub fn read_all(path: &std::path::Path) -> std::io::Result<Vec<CanFrame>> {
        let raw = std::fs::read(path)?;
        let mut out = Vec::with_capacity(raw.len() / RECORD_LEN);
        for record in raw.as_chunks::<RECORD_LEN>().0 {
            let u32le = |off: usize| {
                u32::from_le_bytes([record[off], record[off + 1], record[off + 2], record[off + 3]])
            };
            let t_us = u64::from_le_bytes(record[0..8].try_into().expect("fixed stride"));
            let channel = record[8];
            let id = u32le(9);
            let extended = record[13] != 0;
            let len = record[14].min(crate::can::frame::MAX_CAN_FD_LEN as u8);
            let dir = if record[15] == 1 {
                crate::can::frame::Direction::Tx
            } else {
                crate::can::frame::Direction::Rx
            };
            let flags = crate::can::frame::FrameFlags::from_bits(u32le(16));
            let mut data = [0u8; crate::can::frame::MAX_CAN_FD_LEN];
            data.copy_from_slice(&record[20..20 + crate::can::frame::MAX_CAN_FD_LEN]);
            out.push(CanFrame {
                t_us,
                channel,
                id,
                extended,
                len,
                data,
                dir,
                flags,
            });
        }
        Ok(out)
    }
}

impl Drop for SpillFile {
    fn drop(&mut self) {
        // The archive is session scratch: it dies with the process that
        // owns it. Close the handle first -- Windows refuses to remove
        // a file that still has an open handle without FILE_SHARE_DELETE.
        self.file = None;
        let _ = std::fs::remove_file(&self.path);
    }
}

/// The frontend's frozen view of the ring at a publish instant.
#[derive(Debug, Default)]
pub struct TraceView {
    chunks: Vec<Arc<Vec<CanFrame>>>,
    tail: Vec<CanFrame>,
    total: usize,
    dropped: u64,
    first_dropped_t_us: Option<u64>,
    /// The spill archive while one exists: file path plus archived frame
    /// count. The export replays the archive ahead of the hot ring.
    archive: Option<(std::path::PathBuf, u64)>,
}

impl TraceView {
    pub fn len(&self) -> usize {
        self.total
    }

    pub fn is_empty(&self) -> bool {
        self.total == 0
    }

    /// Frames the ring trimmed from its head (the loss the limit causes),
    /// and the first lost frame's timestamp when any were lost.
    pub fn head_loss(&self) -> (u64, Option<u64>) {
        (self.dropped, self.first_dropped_t_us)
    }

    /// Where the trimmed frames are archived, and how many there are.
    /// `None` while nothing has been trimmed or the archive could not be
    /// created.
    pub fn archive(&self) -> Option<(&std::path::Path, u64)> {
        self.archive
            .as_ref()
            .map(|(p, n)| (p.as_path(), *n))
    }

    pub fn last(&self) -> Option<&CanFrame> {
        self.tail
            .last()
            .or_else(|| self.chunks.last().and_then(|c| c.last()))
    }

    pub fn iter(&self) -> TraceIter<'_> {
        let mut front_segs: Vec<&[CanFrame]> = Vec::with_capacity(self.chunks.len() + 1);
        let mut back_segs: Vec<&[CanFrame]> = Vec::with_capacity(self.chunks.len() + 1);
        for c in &self.chunks {
            front_segs.push(c);
            back_segs.push(c);
        }
        front_segs.push(&self.tail);
        back_segs.push(&self.tail);
        // The back cursor walks segments last-to-first and each segment
        // from its own end.
        back_segs.reverse();
        TraceIter {
            front_segs: front_segs.into_iter(),
            front_cur: &[],
            back_segs: back_segs.into_iter(),
            back_cur: &[],
            remaining: self.total,
        }
    }
}

/// Double-ended iteration over a [`TraceView`]. Front and back cursors
/// walk their own copies of the segment list; the shared `remaining`
/// count is what keeps a mixed-direction walk yielding every frame
/// exactly once.
pub struct TraceIter<'a> {
    front_segs: std::vec::IntoIter<&'a [CanFrame]>,
    front_cur: &'a [CanFrame],
    back_segs: std::vec::IntoIter<&'a [CanFrame]>,
    back_cur: &'a [CanFrame],
    remaining: usize,
}

impl<'a> Iterator for TraceIter<'a> {
    type Item = &'a CanFrame;

    fn next(&mut self) -> Option<Self::Item> {
        if self.remaining == 0 {
            return None;
        }
        loop {
            if let Some((f, rest)) = self.front_cur.split_first() {
                self.front_cur = rest;
                self.remaining -= 1;
                return Some(f);
            }
            self.front_cur = self.front_segs.next()?;
        }
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        (self.remaining, Some(self.remaining))
    }
}

impl ExactSizeIterator for TraceIter<'_> {}

impl<'a> DoubleEndedIterator for TraceIter<'a> {
    fn next_back(&mut self) -> Option<Self::Item> {
        if self.remaining == 0 {
            return None;
        }
        loop {
            if let Some((f, rest)) = self.back_cur.split_last() {
                self.back_cur = rest;
                self.remaining -= 1;
                return Some(f);
            }
            self.back_cur = self.back_segs.next()?;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::can::frame::{Direction, FrameFlags, MAX_CAN_FD_LEN};

    fn frame(t_us: u64) -> CanFrame {
        CanFrame {
            t_us,
            channel: 0,
            id: 0x100,
            extended: false,
            len: 0,
            data: [0; MAX_CAN_FD_LEN],
            dir: Direction::Rx,
            flags: FrameFlags::NONE,
        }
    }

    /// The ring's limit trims the head, and the trim is *accounted*: how
    /// many frames were lost and where the loss began.
    #[test]
    fn head_drops_are_accounted_with_their_first_timestamp() {
        let mut ring = TraceRing::default();
        for t in 0..10u64 {
            ring.push(frame(t * 100));
        }
        ring.enforce_limit(5);
        assert_eq!(ring.len(), 5);
        let (dropped, first) = ring.head_loss();
        assert_eq!((dropped, first), (5, Some(0)), "the five oldest, from t=0");

        // Losing more moves the count, not the remembered origin.
        for t in 10..15u64 {
            ring.push(frame(t * 100));
        }
        ring.enforce_limit(5);
        let (dropped, first) = ring.head_loss();
        assert_eq!((dropped, first), (10, Some(0)));

        ring.clear();
        assert_eq!(ring.head_loss(), (0, None), "a fresh run forgets the loss");
    }

    /// Trimmed frames are archived, not lost: the spill file reads back
    /// exactly what was trimmed, in order, with payloads and flags
    /// intact. A fresh run truncates the archive.
    #[test]
    fn trimmed_frames_are_archived_and_read_back_exactly() {
        let path = std::env::temp_dir().join(format!(
            "roxy_can_spill_{}_{}.bin",
            std::process::id(),
            0x51
        ));
        let mut ring = TraceRing::with_spill_at(path.clone());
        for t in 0..10u64 {
            let mut f = frame(t * 100);
            f.data[0] = t as u8;
            if t == 3 {
                f.flags = crate::can::frame::FrameFlags::FD;
                f.len = 12;
            }
            ring.push(f);
        }
        ring.enforce_limit(5);

        let (p, n) = ring.archive().expect("the archive was created on first trim");
        assert_eq!(p, path, "the test-scoped path is used");
        assert_eq!(n, 5);
        let archived = SpillFile::read_all(p).expect("read back");
        assert_eq!(archived.len(), 5);
        for (k, f) in archived.iter().enumerate() {
            assert_eq!(f.t_us, k as u64 * 100);
            assert_eq!(f.data[0], k as u8, "payloads round-trip");
        }
        assert_eq!(archived[3].flags, crate::can::frame::FrameFlags::FD);
        assert_eq!(archived[3].len, 12, "FD payload length survives");

        // More trims append; a clear truncates for the fresh run.
        for t in 10..15u64 {
            ring.push(frame(t * 100));
        }
        ring.enforce_limit(5);
        let (_, n) = ring.archive().expect("archive");
        assert_eq!(n, 10, "trims append to the same archive");
        ring.clear();
        let (p, n) = ring.archive().expect("file kept for reuse");
        assert_eq!(n, 0);
        assert!(SpillFile::read_all(p).unwrap().is_empty());
        std::fs::remove_file(p).ok();
    }

    /// The archive is session scratch: dropping the ring (process exit,
    /// workspace reset) removes the file instead of littering the temp
    /// directory.
    #[test]
    fn the_spill_file_dies_with_its_ring() {
        let path = std::env::temp_dir().join(format!(
            "roxy_can_spill_{}_{}.bin",
            std::process::id(),
            0x52
        ));
        {
            let mut ring = TraceRing::with_spill_at(path.clone());
            ring.push(frame(0));
            ring.enforce_limit(0); // force the first trim -> file created
            assert!(path.exists(), "the archive exists while the ring does");
        }
        assert!(!path.exists(), "dropping the ring removes the archive");
    }

    fn fr_row(slot: u16, cycle: u8) -> FrRow {
        FrRow {
            t_us: 0,
            ab: 2,
            slot,
            cycle,
            payload: vec![slot as u8; 8],
            header_crc: 0xABCD,
            flags: 0,
            name: None,
        }
    }

    /// The FR ring trims its head past the limit and accounts the loss;
    /// a clear forgets everything. Published views are frozen copies.
    #[test]
    fn the_fr_ring_trims_and_accounts() {
        let mut ring = FrRing::default();
        for slot in 1..=5u16 {
            ring.push(fr_row(slot, slot as u8), 3);
        }
        assert_eq!(ring.iter().count(), 3, "only the last three remain");
        assert_eq!(ring.dropped(), 2);
        let kept: Vec<u16> = ring.iter().map(|r| r.slot).collect();
        assert_eq!(kept, vec![3, 4, 5]);

        let view = ring.publish();
        assert_eq!(view.len(), 3);
        assert_eq!(view[0].slot, 3);
        assert_eq!(view[2].header_crc, 0xABCD);

        ring.clear();
        assert_eq!(ring.iter().count(), 0);
        assert_eq!(ring.dropped(), 0, "a fresh run forgets the loss");
    }
}
