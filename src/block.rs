//! Replay blocks: a recorded log, filtered, replayed back onto the
//! simulated bus as an independently toggleable traffic driver. This is
//! the "restbus from a recording" story -- the nodes the simulation does
//! not play are covered by the frames they actually sent, exactly as
//! recorded, without dragging the whole log's timeline in with them.
//!
//! A block is *declarative* state (name, bus, log path, filters, the
//! enable bit); the preloaded, time-normalized frame queue is runtime
//! state the core rebuilds at every measurement start, like the generator
//! schedules. Blocks emit in Simulation mode only: in Replay mode the
//! whole log is already the source, and doubling its frames up would
//! poison every aggregate.

use crate::can::frame::{CanFrame, Direction};
use crate::dbc::SymbolTable;

/// One replay block: the declaration plus its runtime queue.
#[derive(Clone, Debug)]
pub struct ReplayBlock {
    pub id: u64,
    pub name: String,
    /// The bus the recorded traffic is injected onto. Recorded channel
    /// numbers are a property of the source log, not of our virtual bus,
    /// so every emitted frame is retargeted here.
    pub channel: u8,
    pub path: String,
    /// Replay only the frames this DBC node sends; `None` = no node filter.
    pub node_filter: Option<String>,
    /// Replay only these `(id, extended)` frames; empty = no id filter.
    pub ids: Vec<(u32, bool)>,
    pub enabled: bool,
    /// Set when the last load failed: the frontend shows it instead of
    /// pretending the block contributes nothing on purpose.
    pub last_error: Option<String>,
    /// Preloaded frames, timestamps normalized so the first one is due at
    /// the block's anchor point on the sim timeline.
    queue: Vec<(u64, CanFrame)>,
    /// Next queue index to emit; reset at every measurement start.
    cursor: usize,
    /// Where the queue's zero sits on the sim timeline. Loads at run
    /// start anchor at 0; enabling or editing mid-run anchors at the
    /// current clock, so the block starts *now* at recorded spacing
    /// instead of bursting out frames dated in the past.
    anchor_us: u64,
}

impl ReplayBlock {
    pub fn new(
        id: u64,
        name: String,
        channel: u8,
        path: String,
        node_filter: Option<String>,
        ids: Vec<(u32, bool)>,
        enabled: bool,
    ) -> Self {
        ReplayBlock {
            id,
            name,
            channel,
            path,
            node_filter,
            ids,
            enabled,
            last_error: None,
            queue: Vec::new(),
            cursor: 0,
            anchor_us: 0,
        }
    }

    /// (Re)reads the log and rebuilds the queue, anchoring the queue's
    /// zero at `sim_t_us`: a block loaded at run start anchors at 0, one
    /// enabled mid-run starts at the current clock. The node filter
    /// resolves against the bus's current database -- which is why the
    /// queue is runtime state: a DBC swap changes what a node-filtered
    /// block sends. A failed load disables the block and records why.
    pub fn load_queue(&mut self, dbc: Option<&SymbolTable>, sim_t_us: u64) {
        self.queue.clear();
        self.cursor = 0;
        self.anchor_us = sim_t_us;
        let load = (|| -> Result<Vec<(u64, CanFrame)>, String> {
            if self.path.trim().is_empty() {
                return Err("no log file".to_string());
            }
            let mut stream = crate::log::open_stream(std::path::Path::new(&self.path))
                .map_err(|e| e.to_string())?;
            let node_ids: Option<Vec<u32>> = match &self.node_filter {
                Some(node) => Some(match dbc {
                    Some(db) if db.nodes.iter().any(|n| n == node) => db.node_tx_ids(node),
                    _ => {
                        return Err(format!("node `{node}` is not in the bus's database"));
                    }
                }),
                None => None,
            };
            let mut out: Vec<(u64, CanFrame)> = Vec::new();
            while let Some(t) = stream.peek_t() {
                let Some(f) = stream.next_frame() else {
                    break;
                };
                let _ = t;
                if let Some(ids) = &node_ids
                    && !ids.contains(&f.id)
                {
                    continue;
                }
                if !self.ids.is_empty() && !self.ids.contains(&(f.id, f.extended)) {
                    continue;
                }
                out.push((f.t_us, f));
            }
            if out.is_empty() {
                return Err("the log carries no matching frames".to_string());
            }
            let t0 = out[0].0;
            for (t, _) in &mut out {
                *t -= t0;
            }
            Ok(out)
        })();
        match load {
            Ok(queue) => {
                self.queue = queue;
                self.last_error = None;
            }
            Err(e) => {
                self.enabled = false;
                self.last_error = Some(e);
            }
        }
    }

    /// Emits every queued frame the sim clock has passed, up to `budget`,
    /// retargeted to the block's bus. Frames carry their recorded spacing
    /// past the anchor, so catch-up bursts keep the exact gaps the log
    /// captured.
    pub fn poll(&mut self, sim_t_us: u64, out: &mut Vec<CanFrame>, budget: usize) {
        if !self.enabled {
            return;
        }
        while self.cursor < self.queue.len() && budget > out.len() {
            let (rel_t, mut f) = self.queue[self.cursor];
            let due = self.anchor_us.saturating_add(rel_t);
            if due > sim_t_us {
                break;
            }
            f.t_us = due;
            f.channel = self.channel;
            f.dir = Direction::Tx;
            out.push(f);
            self.cursor += 1;
        }
    }

    /// Rewinds to the run start without re-reading the file. The anchor
    /// goes to 0 with the clock: this runs inside `reset_run`, where the
    /// sim timeline restarts.
    pub fn rewind(&mut self) {
        self.cursor = 0;
        self.anchor_us = 0;
    }

    /// How many frames the loaded queue carries (after the filters).
    pub fn queue_len(&self) -> usize {
        self.queue.len()
    }
}

#[cfg(test)]
#[path = "block_tests.rs"]
mod tests;
