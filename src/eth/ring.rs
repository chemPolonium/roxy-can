use super::EthFrame;
use std::sync::Arc;

const SEAL_FRAMES: usize = 512;
/// Default byte budget per ring (rows are also capped).
pub const DEFAULT_ETH_RING_BYTES: usize = 64 * 1024 * 1024;
pub const DEFAULT_ETH_RING_ROWS: usize = 200_000;

/// Ethernet frames of one run, newest last. Storage follows the FlexRay
/// ring's chunk/Arc discipline; unlike that ring it is capped by **bytes**
/// as well as rows, because one jumbo frame is 9000 bytes.
#[derive(Debug)]
pub struct EthRing {
    chunks: Vec<Arc<Vec<EthFrame>>>,
    tail: Vec<EthFrame>,
    total: usize,
    bytes: usize,
    dropped: u64,
    next_seq: u64,
    cap_bytes: usize,
    cap_rows: usize,
}

impl Default for EthRing {
    fn default() -> Self {
        Self {
            chunks: Vec::new(),
            tail: Vec::new(),
            total: 0,
            bytes: 0,
            dropped: 0,
            next_seq: 0,
            cap_bytes: DEFAULT_ETH_RING_BYTES,
            cap_rows: DEFAULT_ETH_RING_ROWS,
        }
    }
}

impl EthRing {
    pub fn push(&mut self, mut frame: EthFrame) {
        frame.seq = self.next_seq;
        self.next_seq += 1;
        self.bytes += frame.len();
        self.total += 1;
        self.tail.push(frame);
        if self.tail.len() >= SEAL_FRAMES {
            let sealed = Arc::new(std::mem::take(&mut self.tail));
            self.chunks.push(sealed);
        }
        self.enforce_limit();
    }

    fn enforce_limit(&mut self) {
        while self.total > 0 && (self.bytes > self.cap_bytes || self.total > self.cap_rows) {
            let Some(head_len) = self.chunks.first().map(|c| c.len()) else {
                let frame = self.tail.remove(0);
                self.total -= 1;
                self.bytes -= frame.len();
                self.dropped += 1;
                continue;
            };
            let over_rows = self.total > self.cap_rows;
            let over_bytes = self.bytes > self.cap_bytes;
            if over_rows && self.total - head_len >= self.cap_rows {
                let gone = self.chunks.remove(0);
                self.total -= gone.len();
                self.bytes -= gone.iter().map(EthFrame::len).sum::<usize>();
                self.dropped += gone.len() as u64;
            } else if over_bytes && head_len > 0 {
                let head = Arc::make_mut(self.chunks.first_mut().expect("checked above"));
                // Drop just enough frames from the head to fit the byte cap.
                let mut remove = 0;
                let mut freed = 0;
                while remove < head.len() && self.bytes - freed > self.cap_bytes {
                    freed += head[remove].len();
                    remove += 1;
                }
                if remove == 0 {
                    break;
                }
                head.drain(..remove);
                self.total -= remove;
                self.bytes -= freed;
                self.dropped += remove as u64;
            } else {
                break;
            }
        }
    }

    pub fn clear(&mut self) {
        self.chunks.clear();
        self.tail.clear();
        self.total = 0;
        self.bytes = 0;
        self.dropped = 0;
        self.next_seq = 0;
    }

    pub fn len(&self) -> usize {
        self.total
    }

    pub fn bytes(&self) -> usize {
        self.bytes
    }

    pub fn dropped(&self) -> u64 {
        self.dropped
    }

    pub fn publish(&self) -> Arc<EthTraceView> {
        Arc::new(EthTraceView {
            chunks: self.chunks.clone(),
            tail: self.tail.clone(),
            total: self.total,
            bytes: self.bytes,
        })
    }

    pub fn iter(&self) -> impl Iterator<Item = &EthFrame> {
        self.chunks
            .iter()
            .flat_map(|c| c.iter())
            .chain(self.tail.iter())
    }
}

/// The frontend's frozen view of the ring at a publish instant.
#[derive(Debug, Default)]
pub struct EthTraceView {
    chunks: Vec<Arc<Vec<EthFrame>>>,
    tail: Vec<EthFrame>,
    total: usize,
    bytes: usize,
}

impl EthTraceView {
    pub fn len(&self) -> usize {
        self.total
    }

    pub fn is_empty(&self) -> bool {
        self.total == 0
    }

    pub fn bytes(&self) -> usize {
        self.bytes
    }

    pub fn get(&self, i: usize) -> Option<&EthFrame> {
        let mut off = i;
        for c in &self.chunks {
            if off < c.len() {
                return c.get(off);
            }
            off -= c.len();
        }
        self.tail.get(off)
    }

    pub fn last(&self) -> Option<&EthFrame> {
        self.get(self.total.saturating_sub(1))
    }

    pub fn iter(&self) -> impl Iterator<Item = &EthFrame> {
        self.chunks
            .iter()
            .flat_map(|c| c.iter())
            .chain(self.tail.iter())
    }

    /// Newest first, for the Trace section and future merged rows.
    pub fn iter_rev(&self) -> impl Iterator<Item = &EthFrame> {
        self.tail
            .iter()
            .rev()
            .chain(self.chunks.iter().rev().flat_map(|c| c.iter().rev()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::can::frame::Direction;

    fn frame(t: u64, len: usize) -> EthFrame {
        EthFrame::new(
            t,
            0,
            Direction::Rx,
            [0; 6],
            [1; 6],
            Some(1),
            0x0800,
            vec![0xAA; len],
        )
    }

    #[test]
    fn a_publish_shares_sealed_chunks() {
        let mut ring = EthRing::default();
        for i in 0..(SEAL_FRAMES as u64 * 3) {
            ring.push(frame(i, 64));
        }
        let first = ring.publish();
        let sealed = Arc::as_ptr(first.chunks.first().expect("chunk"));
        ring.push(frame(9999, 64));
        let second = ring.publish();
        assert_eq!(
            Arc::as_ptr(second.chunks.first().expect("chunk")),
            sealed,
            "an already published chunk is shared"
        );
        assert_eq!(second.len(), first.len() + 1);
    }

    #[test]
    fn the_byte_cap_trims_the_head() {
        let mut ring = EthRing {
            cap_bytes: 3000,
            cap_rows: usize::MAX,
            ..Default::default()
        };
        for i in 0..10 {
            ring.push(frame(i, 1000));
        }
        assert!(ring.bytes() <= 3000);
        assert!(ring.len() <= 3);
        assert!(ring.dropped() >= 7);
    }

    #[test]
    fn the_row_cap_trims_the_head() {
        let mut ring = EthRing {
            cap_bytes: usize::MAX,
            cap_rows: 4,
            ..Default::default()
        };
        for i in 0..10 {
            ring.push(frame(i, 64));
        }
        assert_eq!(ring.len(), 4);
        assert_eq!(ring.iter().next().map(|f| f.t_us), Some(6));
    }

    /// Two frames captured in the same microsecond still get distinct pick
    /// identities: the Trace row key includes the ring's sequence number.
    #[test]
    fn every_pushed_frame_gets_its_own_sequence() {
        let mut ring = EthRing::default();
        ring.push(frame(1_000, 8));
        ring.push(frame(1_000, 8));
        let seqs: Vec<u64> = ring.iter().map(|f| f.seq).collect();
        assert_eq!(seqs, [0, 1]);
        let published = ring.publish();
        assert_eq!(
            published.iter().map(|f| f.seq).collect::<Vec<_>>(),
            [0, 1],
            "the identity survives the snapshot"
        );
    }
}
