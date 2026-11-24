//! Replay blocks and log replay control.

use super::*;

impl BusCore {
    /// Adds a replay block. New blocks start disabled: nothing transmits
    /// until the user says so, whichever way the toggle flips later.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn add_replay_block(
        &mut self,
        name: String,
        channel: u8,
        path: String,
        node_filter: Option<String>,
        attached: Option<(u8, String)>,
        ids: Vec<(u32, bool)>,
        status: &mut String,
    ) {
        self.block_counter += 1;
        let mut block = crate::block::ReplayBlock::new(
            self.block_counter,
            name,
            channel,
            path,
            node_filter,
            attached,
            ids,
            false,
        );
        if block.enabled {
            let dbc = self
                .model_dbs
                .get(block.channel as usize)
                .and_then(|db| db.clone());
            let node_ids = block
                .node_filter
                .as_deref()
                .and_then(|node| self.model_node_tx_ids(block.channel, node));
            block.load_queue_with(dbc.as_deref(), node_ids, self.sim_t_us);
        }
        *status = format!("replay block `{}` added (disabled)", block.name);
        self.replay_blocks.push(block);
        self.nodes_dirty = true;
    }

    /// Rewrites one block's declaration. An enabled block reloads its
    /// queue against the current database, so the edit lands immediately;
    /// a disabled one just carries the new text until it is enabled.
    // The signature mirrors the `SetReplayBlock` command one-to-one; a
    // payload struct would just relocate the field list.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn set_replay_block(
        &mut self,
        id: u64,
        name: String,
        channel: u8,
        path: String,
        node_filter: Option<String>,
        attached: Option<(u8, String)>,
        ids: Vec<(u32, bool)>,
        status: &mut String,
    ) {
        let Some(block) = self.replay_blocks.iter_mut().find(|b| b.id == id) else {
            return;
        };
        block.name = name;
        block.channel = channel;
        block.path = path;
        block.node_filter = node_filter;
        block.attached = attached;
        block.ids = ids;
        if block.enabled {
            let dbc = self
                .model_dbs
                .get(block.channel as usize)
                .and_then(|db| db.clone());
            block.load_queue(dbc.as_deref(), self.sim_t_us);
        }
        *status = format!("replay block `{}` updated", block.name);
        self.nodes_dirty = true;
    }

    /// Toggles a block. Enabling loads the queue right away regardless of
    /// the measurement state, so a broken path surfaces now, not silently
    /// at the next start.
    pub(super) fn set_replay_block_enabled(&mut self, id: u64, on: bool, status: &mut String) {
        // The model lookup needs `&self`, the queue edit needs `&mut self`;
        // resolve the node's ids between the two borrows.
        let Some((channel, node)) = self
            .replay_blocks
            .iter()
            .find(|b| b.id == id)
            .map(|b| (b.channel, b.node_filter.clone()))
        else {
            return;
        };
        let node_ids = node
            .as_deref()
            .and_then(|name| self.model_node_tx_ids(channel, name));
        let Some(block) = self.replay_blocks.iter_mut().find(|b| b.id == id) else {
            return;
        };
        block.enabled = on;
        if on {
            let dbc = self
                .model_dbs
                .get(block.channel as usize)
                .and_then(|db| db.clone());
            block.load_queue_with(dbc.as_deref(), node_ids, self.sim_t_us);
            *status = match (&block.last_error, block.enabled) {
                (Some(e), _) => format!("replay block `{}`: {e}", block.name),
                (None, true) => format!(
                    "replay block `{}` on ({} frame(s))",
                    block.name,
                    block.queue_len()
                ),
                (None, false) => format!("replay block `{}` off", block.name),
            };
        } else {
            *status = format!("replay block `{}` off", block.name);
        }
        self.nodes_dirty = true;
    }
    /// 按当前映射把"日志里有什么"换成工程编号，供生成器静音那两份表用。
    /// 不回放的那些（`target` 给 `None`）不进表：它们的帧也进不了环，留着
    /// 只会让一个条目莫名被静音。
    pub(super) fn remap_log_ids(&mut self) {
        use crate::replay_map::LogBusKind;
        let ids: std::collections::HashSet<(u8, u32)> = self
            .log_ids_file
            .iter()
            .filter_map(|(ch, id)| self.replay_bus(LogBusKind::Can, *ch).map(|t| (t, *id)))
            .collect();
        let fr: std::collections::HashSet<(u8, u16)> = self
            .log_fr_ids_file
            .iter()
            .filter_map(|(bus, slot)| self.replay_bus(LogBusKind::Fr, *bus).map(|t| (t, *slot)))
            .collect();
        self.replay_ids = ids;
        self.replay_fr_ids = fr;
    }

    /// Opens `path` and starts replaying it. File open and the silence-set
    /// scan are the two deliberate blocking costs of a replay start; they
    /// run once per start, never per frame.
    pub(super) fn start_replay(&mut self, path: &str, speed: f64, status: &mut String) {
        let stream = match crate::log::open_stream(std::path::Path::new(path)) {
            Ok(stream) => stream,
            Err(e) => {
                *status = format!("replay failed [{path}]: {e}");
                return;
            }
        };
        // Collect the log's ids once at open, from a temporary second
        // stream, so the generators can stand down for the traffic the replay
        // itself covers. Draining here, never per frame.
        let (ids, fr_ids) = scan_log_ids(std::path::Path::new(path)).unwrap_or_default();
        // 两份集合存的是**文件自己的**编号；喂给生成器静音的那两份按当前映射
        // 推导，于是中途改映射只重算集合，不必把整个文件再读一遍。
        self.log_ids_file = ids;
        self.log_fr_ids_file = fr_ids;
        self.remap_log_ids();
        let info = stream.describe();
        self.recorder.close();
        // Replay just re-emits an existing log; recording it would only
        // duplicate the file, so drop the Record state.
        self.recorder.recording = false;
        let mut source = crate::source::replay::ReplaySource::new(stream);
        source.set_speed(speed);
        self.source = Box::new(source);
        self.mode = Mode::Replay;
        self.run_mode = Mode::Replay;
        self.reset_run();
        self.measuring = true;
        self.nodes_start();
        let tag = if info.is_empty() {
            String::new()
        } else {
            format!(" [{info}]")
        };
        *status = format!("replaying{tag} at {speed}x");
    }
}
