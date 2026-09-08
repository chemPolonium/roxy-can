//! The trace ring: the run's last `TRACE_LIMIT` frames. Storage is
//! chunked so that publishing a snapshot shares sealed chunks instead of
//! copying the whole ring -- the same discipline as the signal-history
//! cache: append into a live tail, seal it by move when it outgrows
//! [`SEAL_FRAMES`], mutate shared chunks only through `Arc::make_mut`,
//! and publish by refcount bumps plus one tail copy.

use crate::can::frame::CanFrame;
use std::sync::Arc;

/// Frames accumulate in the live tail and are sealed into an immutable
/// shared chunk at this size, bounding a publish's copy to one tail.
const SEAL_FRAMES: usize = 512;

/// The bus-side ring: append at the back, drop from the front once the
/// limit is exceeded. Head drops are *accounted*, never silent: the
/// dropped count and the first dropped frame's timestamp surface in the
/// Trace window.
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
    }

    /// Frames trimmed from the head since the last clear, and where the
    /// loss began (the first dropped frame's own timestamp).
    #[cfg(test)]
    pub fn head_loss(&self) -> (u64, Option<u64>) {
        (self.dropped, self.first_dropped_t_us)
    }

    /// Core-side iteration, for test assertions on the working ring.
    #[cfg(test)]
    pub fn iter(&self) -> impl Iterator<Item = &CanFrame> {
        self.chunks
            .iter()
            .flat_map(|c| c.iter())
            .chain(self.tail.iter())
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
}
