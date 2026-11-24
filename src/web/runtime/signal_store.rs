/// Incremental cache of the chosen signals. Unlike a Graphics window this is
/// a session-level selection: a project load starts from its Graphics rows
/// and the Signals panel adds session-only picks. The frontend asks for a seed
/// and then only for samples newer than its last timestamp.
#[derive(Default)]
struct SignalStore {
    epoch: u64,
    primed: bool,
    selected: Vec<SelectedSignal>,
    buffers: Vec<SignalBuffer>,
    last_t: Vec<u64>,
    /// Fingerprint of the derived definitions appended after the selected
    /// signals; a change rebuilds the lane list (and its epoch).
    derived_meta: String,
}

impl SignalStore {
    fn is_selected(&self, key: &SigKey) -> bool {
        self.selected.iter().any(|signal| &signal.key == key)
    }

    fn is_read(&self, key: &SigKey) -> bool {
        self.selected
            .iter()
            .any(|signal| &signal.key == key && signal.read)
    }

    /// Every subscribed key: plotted lanes and read-only signals alike.
    fn keys(&self) -> Vec<SigKey> {
        self.selected
            .iter()
            .map(|signal| signal.key.clone())
            .collect()
    }

    /// Plotted lanes only, in lane order. Read-only signals never draw.
    fn chart_keys(&self) -> Vec<SigKey> {
        self.selected
            .iter()
            .filter(|signal| !signal.read)
            .map(|signal| signal.key.clone())
            .collect()
    }

    /// Adds a selection without subscribing; the command handler owns the
    /// bus subscription so this stays a pure cache operation.
    #[cfg(test)]
    fn select(&mut self, app: &App, ch: u8, id: u32, ext: bool, name: &str) {
        self.select_key(app, app.can_key(ch, id, ext, name.to_string()));
    }

    fn select_key(&mut self, app: &App, key: SigKey) {
        self.select_key_mode(app, key, false);
    }

    /// Selects a signal as a plotted lane (`read = false`) or a cursor-only
    /// read signal. Changing the mode keeps the entry and its history; only
    /// the lane/readout placement changes.
    fn select_key_mode(&mut self, app: &App, key: SigKey, read: bool) {
        let Some(key) = app.bind_key(key) else {
            return;
        };
        if let Some(pos) = self.selected.iter().position(|signal| signal.key == key) {
            if self.selected[pos].read == read {
                return;
            }
            let mut signal = self.selected.remove(pos);
            signal.read = read;
            if !read {
                // A signal coming back to the chart gets a lane colour that
                // is not already in use.
                signal.color = self.free_chart_color();
            }
            let at = if read {
                self.selected.len()
            } else {
                self.selected
                    .iter()
                    .position(|signal| signal.read)
                    .unwrap_or(self.selected.len())
            };
            self.selected.insert(at, signal);
            self.primed = false;
            return;
        }
        let name = match &key {
            SigKey::Can { name, .. }
            | SigKey::Fr { name, .. }
            | SigKey::Eth { name, .. }
            | SigKey::Derived { name, .. } => name.clone(),
        };
        let bus = signal_bus(app, &key);
        let unit = match &key {
            SigKey::Can { id, ext, name, .. } => app
                .key_bus(&key)
                .map(|ch| signal_unit(app, ch, *id, *ext, name))
                .unwrap_or_default(),
            SigKey::Derived { id, .. } => app
                .derived_defs
                .iter()
                .find(|def| &def.id == id)
                .map(|def| def.unit.clone())
                .unwrap_or_default(),
            // FR/Eth units arrive with the first subscription publish.
            SigKey::Fr { .. } | SigKey::Eth { .. } => String::new(),
        };
        let color = if read { 0 } else { self.free_chart_color() };
        let signal = SelectedSignal {
            key,
            name,
            bus,
            unit,
            color,
            read,
        };
        if read {
            self.selected.push(signal);
        } else {
            let at = self
                .selected
                .iter()
                .position(|signal| signal.read)
                .unwrap_or(self.selected.len());
            self.selected.insert(at, signal);
        }
        self.primed = false;
    }

    fn free_chart_color(&self) -> usize {
        (0..SIGNAL_PALETTE)
            .find(|color| {
                !self
                    .selected
                    .iter()
                    .any(|signal| !signal.read && signal.color == *color)
            })
            .unwrap_or(self.selected.len() % SIGNAL_PALETTE)
    }

    fn unselect(&mut self, key: &SigKey) {
        if let Some(pos) = self.selected.iter().position(|signal| &signal.key == key) {
            self.selected.remove(pos);
        }
        self.primed = false;
    }

    /// Reorders one lane. Colors travel with the signal; the next sync
    /// rebuilds the per-lane buffers and bumps the epoch so the frontend
    /// replaces its lane list rather than merging against stale indexes.
    fn move_selected(&mut self, key: &SigKey, to: usize) -> bool {
        let Some(from) = self.selected.iter().position(|signal| &signal.key == key) else {
            return false;
        };
        if self.selected[from].read {
            return false;
        }
        let charted = self.selected.iter().filter(|signal| !signal.read).count();
        let to = to.min(charted.saturating_sub(1));
        if from == to {
            return false;
        }
        let signal = self.selected.remove(from);
        self.selected.insert(to, signal);
        self.primed = false;
        true
    }

    /// The removed channel's cluster takes its selections with it; the
    /// channels that stay keep their keys, so nothing is renumbered here.
    fn remap_after_remove(&mut self, removed: Option<crate::model::ClusterId>) {
        self.selected.retain(
            |signal| !matches!(removed, Some(cluster) if signal.key.cluster() == Some(cluster)),
        );
        self.primed = false;
    }

    /// A loaded database can replace the signals under a selection. Drop the
    /// ones it no longer declares and return their keys for unsubscription.
    #[cfg(test)]
    fn prune_missing(&mut self, app: &App) -> Vec<SigKey> {
        let mut dropped = Vec::new();
        self.selected.retain(|signal| {
            let exists = match &signal.key {
                SigKey::Can { id, ext, name, .. } => app
                    .key_bus(&signal.key)
                    .and_then(|ch| app.snap.channels.get(ch as usize))
                    .and_then(|channel| channel.dbc.as_ref())
                    .and_then(|db| db.messages.get(&(*id, *ext)))
                    .is_some_and(|message| {
                        message
                            .signals
                            .iter()
                            .any(|candidate| &candidate.name == name)
                    }),
                SigKey::Derived { id, .. } => app.derived_defs.iter().any(|def| &def.id == id),
                _ => true,
            };
            if !exists {
                dropped.push(signal.key.clone());
            }
            exists
        });
        if !dropped.is_empty() {
            self.primed = false;
        }
        dropped
    }

    /// Drops selected signals and buffers; used when the bus set changes.
    fn clear_selection(&mut self) {
        self.selected.clear();
        self.primed = false;
    }

    /// Keeps the selection but forgets sampled history; used when a new log
    /// starts on the same project.
    fn reset(&mut self) {
        self.primed = false;
    }

    fn sync(&mut self, app: &App) {
        // A derived rename replaces its series key; keep the lane/read choice
        // on the same definition id.
        let defs: std::collections::HashMap<&str, &crate::derived::DerivedDef> = app
            .derived_defs
            .iter()
            .map(|def| (def.id.as_str(), def))
            .collect();
        self.selected.retain(|signal| match &signal.key {
            SigKey::Derived { id, .. } => defs.contains_key(id.as_str()),
            _ => true,
        });
        for signal in &mut self.selected {
            if let SigKey::Derived { id, name } = &mut signal.key
                && let Some(def) = defs.get(id.as_str())
            {
                *name = def.name.clone();
                signal.name = def.name.clone();
                signal.unit = def.unit.clone();
            }
        }
        for signal in &mut self.selected {
            signal.bus = signal_bus(app, &signal.key);
            if let Some(sub) = signal_sub(app, &signal.key) {
                signal.unit = sub.unit.clone();
            }
        }
        // Computed channels are ordinary selectable series: definitions only
        // make them available, Plot/Read decides whether they appear.
        let signals = self.selected.clone();
        let derived_meta = app
            .derived_defs
            .iter()
            .map(|def| format!("{}|{}|{}", def.id, def.name, def.unit))
            .collect::<Vec<_>>()
            .join(";");
        let mut reset = !self.primed || derived_meta != self.derived_meta;
        self.derived_meta = derived_meta;
        if self.primed {
            for (i, signal) in signals.iter().enumerate() {
                let Some(sub) = signal_sub(app, &signal.key) else {
                    continue;
                };
                let last = sub.history.iter().last().map(|(t, _)| *t).unwrap_or(0);
                if self.last_t.get(i).copied().unwrap_or(0) > last {
                    reset = true;
                }
            }
        }
        if reset {
            self.primed = true;
            self.epoch = self.epoch.wrapping_add(1).max(1);
            self.buffers.clear();
            self.last_t = vec![0; signals.len()];
            for (i, signal) in signals.iter().enumerate() {
                let sub = signal_sub(app, &signal.key);
                let mut meta = signal_meta(signal);
                if let Some(sub) = sub {
                    meta.label = sub.label.clone();
                }
                let mut points = VecDeque::new();
                if let Some(sub) = sub {
                    let all: Vec<(u64, f64)> = sub.history.iter().copied().collect();
                    let start = all.len().saturating_sub(SIGNAL_CAP);
                    points.extend(all[start..].iter().copied());
                    if let Some((t, _)) = points.back() {
                        self.last_t[i] = *t;
                    }
                }
                self.buffers.push(SignalBuffer { meta, points });
            }
            return;
        }

        for (i, signal) in signals.iter().enumerate() {
            let Some(sub) = signal_sub(app, &signal.key) else {
                continue;
            };
            let Some(buffer) = self.buffers.get_mut(i) else {
                continue;
            };
            buffer.meta = signal_meta(signal);
            buffer.meta.label = sub.label.clone();
            let last = self.last_t.get(i).copied().unwrap_or(0);
            for &(t, value) in sub.history.iter() {
                if t > last {
                    buffer.points.push_back((t, value));
                }
            }
            while buffer.points.len() > SIGNAL_CAP {
                buffer.points.pop_front();
            }
            if let Some((t, _)) = buffer.points.back() {
                self.last_t[i] = *t;
            }
        }
    }

    fn max_t(&self) -> u64 {
        self.last_t.iter().copied().max().unwrap_or(0)
    }

    fn earliest_t(&self) -> u64 {
        self.buffers
            .iter()
            .filter_map(|buffer| buffer.points.front().map(|(t, _)| *t))
            .min()
            .unwrap_or(self.max_t())
    }

    /// Includes selected signals with no samples yet; the lane has to exist
    /// before the first frame arrives.
    fn seed(&self) -> Vec<SignalSeries> {
        self.buffers
            .iter()
            .map(|buffer| Self::dto(buffer, 0))
            .collect()
    }

    fn after(&self, since: u64) -> Vec<SignalSeries> {
        self.buffers
            .iter()
            .filter(|buffer| buffer.points.back().is_some_and(|(t, _)| *t > since))
            .map(|buffer| Self::dto(buffer, since))
            .collect()
    }

    fn dto(buffer: &SignalBuffer, since: u64) -> SignalSeries {
        let mut series = buffer.meta.clone();
        series.points = buffer
            .points
            .iter()
            .filter(|(t, _)| *t > since)
            .map(|(t, value)| [*t as f64 / 1e6, *value])
            .collect();
        series
    }
}

