//! CAN channel configuration, subscriptions and derived-signal wiring.

use super::*;

impl BusCore {
    /// Starts caching one signal: a fresh [`Subscription`] gets the next
    /// palette color and the database's display type. An existing
    /// subscription for the key is left untouched.
    /// How many CAN channels exist. The cluster-id list *is* the channel
    /// list: there is no parallel configuration vector.
    pub(crate) fn can_count(&self) -> usize {
        self.can_cluster_ids.len()
    }

    /// The cluster a CAN channel names in the model.
    pub(crate) fn can_cluster(&self, ch: usize) -> Option<&crate::model::Cluster> {
        let comm = self.model_comms.get(ch)?.as_deref()?;
        comm.clusters.get(self.can_cluster_ids.get(ch)?)
    }

    /// The channel's model, empty when no database is attached.
    pub(crate) fn can_comm(&self, ch: usize) -> Option<&crate::model::Comm> {
        self.model_comms.get(ch)?.as_deref()
    }

    /// The user-facing channel name, from its cluster.
    pub(crate) fn can_name(&self, ch: usize) -> String {
        self.can_cluster(ch)
            .map(|cluster| cluster.name.clone())
            .unwrap_or_else(|| format!("CAN{}", ch + 1))
    }

    /// The channel's attached DBC paths, primary first. ARXML/FIBEX sources
    /// are model provenance, not database files to parse as DBC.
    pub(crate) fn can_paths(&self, ch: usize) -> Vec<String> {
        self.can_cluster(ch)
            .map(|cluster| {
                cluster
                    .sources
                    .iter()
                    .filter_map(|source| match source {
                        crate::model::ClusterSource::Dbc { path } => Some(path.clone()),
                        _ => None,
                    })
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Arbitration and FD data-phase bitrates in kbit/s.
    pub(crate) fn can_bitrates(&self, ch: usize) -> (u32, u32) {
        self.can_comm(ch)
            .and_then(|comm| {
                comm.networks
                    .values()
                    .find_map(|network| match &network.params {
                        crate::model::NetworkParams::Can(params) => {
                            Some((params.bitrate_kbps, params.fd_data_kbps))
                        }
                        _ => None,
                    })
            })
            .unwrap_or((
                crate::model::CanParams::DEFAULT_BITRATE_KBPS,
                crate::model::CanParams::DEFAULT_FD_DATA_KBPS,
            ))
    }

    /// The node-role declarations of one channel; empty means restbus.
    pub(crate) fn can_node_roles(&self, ch: usize) -> std::collections::BTreeMap<String, NodeRole> {
        self.can_cluster_ids
            .get(ch)
            .and_then(|cluster| self.can_node_roles.get(cluster))
            .cloned()
            .unwrap_or_default()
    }

    /// The role declared for `node` on one channel; every node without an
    /// entry is `Absent` -- the restbus default, not a stored state.
    pub(crate) fn can_role_of(&self, ch: usize, node: &str) -> NodeRole {
        Self::role_in(&self.can_cluster_ids, &self.can_node_roles, ch, node)
    }

    /// Test-only setters: fixtures that used to poke the removed bus
    /// configuration struct now write the model the same way the commands do.
    #[cfg(test)]
    pub(crate) fn set_can_name_for_test(&mut self, ch: usize, name: &str) {
        self.update_can_comm(ch, |comm| {
            for cluster in comm.clusters.values_mut() {
                cluster.name = name.to_string();
            }
            for network in comm.networks.values_mut() {
                network.name = name.to_string();
            }
        });
    }

    #[cfg(test)]
    pub(crate) fn set_can_bitrates_for_test(
        &mut self,
        ch: usize,
        bitrate: Option<u32>,
        fd: Option<u32>,
    ) {
        self.update_can_comm(ch, |comm| {
            for network in comm.networks.values_mut() {
                if let crate::model::NetworkParams::Can(params) = &mut network.params {
                    if let Some(k) = bitrate {
                        params.bitrate_kbps = k;
                    }
                    if let Some(k) = fd {
                        params.fd_data_kbps = k;
                    }
                }
            }
        });
    }

    #[cfg(test)]
    pub(crate) fn set_role_for_test(&mut self, ch: usize, node: &str, role: NodeRole) {
        let Some(cluster) = self.can_cluster_ids.get(ch).copied() else {
            return;
        };
        match role {
            NodeRole::Absent => {
                if let Some(roles) = self.can_node_roles.get_mut(&cluster) {
                    roles.remove(node);
                }
            }
            role => {
                self.can_node_roles
                    .entry(cluster)
                    .or_default()
                    .insert(node.to_string(), role);
            }
        }
    }

    /// Field-level form of [`Self::can_role_of`] for call sites that already
    /// hold a mutable borrow of another `BusCore` field (the node loop).
    pub(crate) fn role_in(
        ids: &[crate::model::ClusterId],
        roles: &std::collections::BTreeMap<
            crate::model::ClusterId,
            std::collections::BTreeMap<String, NodeRole>,
        >,
        ch: usize,
        node: &str,
    ) -> NodeRole {
        ids.get(ch)
            .and_then(|cluster| roles.get(cluster))
            .and_then(|roles| roles.get(node))
            .copied()
            .unwrap_or(NodeRole::Absent)
    }

    /// Replaces the channel's description paths in its cluster's sources.
    pub(super) fn set_can_paths(&mut self, ch: usize, paths: Vec<String>) {
        let Some(cluster) = self.can_cluster_ids.get(ch).copied() else {
            return;
        };
        let Some(comm) = self.model_comms.get_mut(ch).and_then(Option::as_mut) else {
            return;
        };
        if let Some(cluster) = Arc::make_mut(comm).clusters.get_mut(&cluster) {
            let keep: Vec<_> = cluster
                .sources
                .iter()
                .filter(|source| !matches!(source, crate::model::ClusterSource::Dbc { .. }))
                .cloned()
                .collect();
            cluster.sources = paths
                .iter()
                .filter(|path| !path.trim().is_empty())
                .map(|path| crate::model::ClusterSource::Dbc { path: path.clone() })
                .chain(keep)
                .collect();
        }
    }

    /// Applies a settings change to the channel's cluster/network in place.
    pub(super) fn update_can_comm(
        &mut self,
        ch: usize,
        update: impl FnOnce(&mut crate::model::Comm),
    ) {
        if ch >= self.can_cluster_ids.len() {
            return;
        }
        while self.model_comms.len() <= ch {
            self.model_comms.push(None);
            self.model_plans.push(None);
            self.model_dbs.push(None);
            self.can_xml_models.push(None);
        }
        if self.model_comms[ch].is_none() {
            let (bitrate, fd) = self.can_bitrates(ch);
            let name = self.can_name(ch);
            let cluster = self.can_cluster_ids[ch];
            self.model_comms[ch] = Some(Arc::new(crate::model::empty_can_comm(
                &name,
                cluster,
                &[],
                bitrate,
                fd,
            )));
        }
        if let Some(comm) = self.model_comms[ch].as_mut() {
            update(Arc::make_mut(comm));
        }
    }

    /// A CAN key carrying the channel's cluster id, for canonical wire text.
    /// The model a node's buffer builtins encode/decode through, when the
    /// channel has one built from its current table.
    pub(crate) fn can_codec(&self, ch: u8) -> Option<crate::node::CanCodec> {
        let index = ch as usize;
        let comm = self.model_comms.get(index)?.as_ref()?.clone();
        let plans = self.model_plans.get(index)?.as_ref()?.clone();
        Some((comm, plans, *self.can_cluster_ids.get(index)?))
    }

    pub(super) fn can_codec_map(&self) -> std::collections::HashMap<u8, crate::node::CanCodec> {
        (0..self.can_count() as u8)
            .filter_map(|ch| self.can_codec(ch).map(|codec| (ch, codec)))
            .collect()
    }

    /// The source file a FlexRay bus's model came from.
    pub(crate) fn fr_description_path(&self, bus: u8) -> Option<String> {
        self.fr_models
            .get(&bus)
            .and_then(|comm| comm.clusters.values().next())
            .and_then(|cluster| cluster.sources.first())
            .and_then(|source| source.path())
            .map(str::to_string)
            .or_else(|| {
                self.fr_sources
                    .get(&bus)
                    .and_then(|sources| sources.first())
                    .and_then(|source| source.path())
                    .map(str::to_string)
            })
    }

    /// The source file an Ethernet bus's model came from.
    pub(crate) fn eth_description_path(&self, bus: u8) -> Option<String> {
        self.eth_models
            .get(&bus)
            .and_then(|comm| comm.clusters.values().next())
            .and_then(|cluster| cluster.sources.first())
            .and_then(|source| source.path())
            .map(str::to_string)
            .or_else(|| {
                self.eth_sources
                    .get(&bus)
                    .and_then(|sources| sources.first())
                    .and_then(|source| source.path())
                    .map(str::to_string)
            })
    }

    pub(crate) fn can_key(&self, ch: u8, id: u32, ext: bool, name: String) -> SigKey {
        let cluster = self
            .can_cluster_ids
            .get(ch as usize)
            .copied()
            .unwrap_or(crate::model::ClusterId(0));
        SigKey::can_in(cluster, id, ext, name)
    }

    pub(crate) fn fr_key(&self, bus: u8, slot: u16, name: String) -> SigKey {
        let cluster = self
            .fr_cluster_ids
            .get(&bus)
            .copied()
            .unwrap_or(crate::model::ClusterId(0));
        SigKey::fr_in(cluster, slot, name)
    }

    pub(crate) fn eth_key(&self, bus: u8, frame: String, name: String) -> SigKey {
        let cluster = self
            .eth_cluster_ids
            .get(&bus)
            .copied()
            .unwrap_or(crate::model::ClusterId(0));
        SigKey::eth_in(cluster, frame, name)
    }

    /// The bus a key routes on, resolved from its cluster. The key itself
    /// carries no bus index: one cluster names exactly one bus, so the map is
    /// the only place that pairing lives.
    pub(crate) fn key_bus(&self, key: &SigKey) -> Option<u8> {
        let cluster = key.cluster()?;
        match key {
            SigKey::Can { .. } => self
                .can_cluster_ids
                .iter()
                .position(|id| *id == cluster)
                .map(|index| index as u8),
            SigKey::Fr { .. } => self
                .fr_cluster_ids
                .iter()
                .find(|(_, id)| **id == cluster)
                .map(|(bus, _)| *bus),
            SigKey::Eth { .. } => self
                .eth_cluster_ids
                .iter()
                .find(|(_, id)| **id == cluster)
                .map(|(bus, _)| *bus),
            SigKey::Derived { .. } => None,
        }
    }

    /// The cluster the bus at `bus` names in one protocol space.
    pub(crate) fn cluster_of_bus(
        &self,
        kind: crate::replay_map::LogBusKind,
        bus: u8,
    ) -> Option<crate::model::ClusterId> {
        match kind {
            crate::replay_map::LogBusKind::Can => self.can_cluster_ids.get(bus as usize).copied(),
            crate::replay_map::LogBusKind::Fr => self.fr_cluster_ids.get(&bus).copied(),
            crate::replay_map::LogBusKind::Eth => self.eth_cluster_ids.get(&bus).copied(),
        }
    }

    /// The bus one log channel replays on, after the cluster-targeted map and
    /// the live bus list are both consulted. `None` = this channel is dropped.
    pub(crate) fn replay_bus(&self, kind: crate::replay_map::LogBusKind, raw: u8) -> Option<u8> {
        replay_bus_in(
            &self.replay_map,
            kind,
            raw,
            &self.can_cluster_ids,
            &self.fr_cluster_ids,
            &self.eth_cluster_ids,
        )
    }

    /// Confirms a key names a live cluster. A key naming a cluster that is
    /// gone is dropped rather than written to whichever bus happens to be zero.
    pub(crate) fn bind_key(&self, key: SigKey) -> Option<SigKey> {
        // A computed channel has no bus to resolve; it rides the core itself.
        if matches!(key, SigKey::Derived { .. }) {
            return Some(key);
        }
        self.key_bus(&key).map(|_| key)
    }

    /// Text from a panel/script/derived source: a canonical `sig:` key, with
    /// the cluster resolved against the live bus list.
    pub(crate) fn resolve_key_text(&self, text: &str) -> Option<SigKey> {
        let text = text.trim();
        SigKey::from_text(text).and_then(|key| self.bind_key(key))
    }

    pub(crate) fn subscribe_signal(&mut self, key: SigKey) {
        let Some(key) = self.bind_key(key) else {
            return;
        };
        if !self.subs.contains_key(&key) {
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
                    history: crate::observe::SampleCache::default(),
                    published: std::sync::Arc::new(crate::observe::SampleCache::default()),
                    history_dirty: false,
                },
            );
        }
    }

    /// The display type the database declares for a subscribed signal, used
    /// until the first frame refreshes it -- and forever when no database
    /// names it.
    pub(super) fn signal_meta(&self, key: &SigKey) -> String {
        match key {
            // The FlexRay database carries no display type and
            // `decode_signals` leaves it empty; the Ethernet matrix states
            // signedness and width, the same pair its decoder tags.
            SigKey::Fr { .. } => String::new(),
            SigKey::Eth { frame, name, .. } => self
                .key_bus(key)
                .and_then(|bus| self.eth_dbs.get(&bus))
                .and_then(|db| db.frames.iter().find(|f| &f.name == frame))
                .and_then(|f| {
                    f.pdus
                        .iter()
                        .flat_map(|p| p.signals.iter())
                        .find(|s| &s.name == name)
                })
                .map(|s| format!("{}{}", if s.signed { "i" } else { "u" }, s.size))
                .unwrap_or_default(),
            SigKey::Can { id, ext, name, .. } => self
                .key_bus(key)
                .and_then(|ch| self.model_dbs.get(ch as usize))
                .and_then(|db| db.as_ref())
                .and_then(|db| db.messages.get(&(*id, *ext)))
                .and_then(|m| m.signals.iter().find(|s| &s.name == name))
                .map(|s| s.type_tag.clone())
                .unwrap_or_default(),
            SigKey::Derived { .. } => String::new(),
        }
    }

    /// Replaces the derived definitions. A definition whose expression or
    /// name changed starts a fresh curve; an unchanged one keeps sampling
    /// from where it was, so editing only its State bands costs no history.
    pub(super) fn set_derived(&mut self, defs: Vec<crate::derived::DerivedDef>) {
        let mut previous: HashMap<String, (Option<u64>, crate::derived::DerivedDef)> = self
            .derived
            .drain(..)
            .map(|runtime| (runtime.def.id.clone(), (runtime.last_t_us, runtime.def)))
            .collect();
        let new_keys: std::collections::HashSet<SigKey> = defs
            .iter()
            .map(|def| SigKey::derived(&def.id, &def.name))
            .collect();
        let stale: Vec<SigKey> = previous
            .values()
            .map(|(_, def)| SigKey::derived(&def.id, &def.name))
            .filter(|key| !new_keys.contains(key))
            .collect();
        for key in stale {
            self.subs.remove(&key);
        }
        // A threshold rule whose lane no longer exists can never fire again;
        // leaving it in the list (and in the project) would read as active
        // monitoring. The lane is the rule's subject, so it goes with it.
        let ids: std::collections::HashSet<String> =
            defs.iter().map(|def| def.id.clone()).collect();
        self.triggers.retain(|trigger| {
            !matches!(
                &trigger.cond,
                TriggerCond::DerivedSignal { id, .. } if !ids.contains(id)
            )
        });
        let names_by_id: HashMap<String, String> = defs
            .iter()
            .map(|def| (def.id.clone(), def.name.clone()))
            .collect();
        let mut runtimes = Vec::with_capacity(defs.len());
        for def in defs {
            let key = SigKey::derived(&def.id, &def.name);
            let pinned: Vec<(String, SigKey)> = def
                .sources
                .iter()
                .filter_map(|source| {
                    let key = self.resolve_key_text(&source.key)?;
                    // A derived source's wire form carries only its id; the
                    // definition list supplies the name half of the key.
                    let key = match key {
                        SigKey::Derived { id, .. } => SigKey::derived(&id, names_by_id.get(&id)?),
                        key => key,
                    };
                    Some((source.name.clone(), key))
                })
                .collect();
            self.subscribe_signal(key.clone());
            let reused = previous.remove(&def.id);
            let (last, reset) = match &reused {
                Some((last, old)) if old.expression == def.expression && old.name == def.name => {
                    (*last, false)
                }
                _ => (None, true),
            };
            if let Some(sub) = self.subs.get_mut(&key) {
                sub.unit = def.unit.clone();
                sub.type_tag.clear();
                if reset {
                    sub.reset_measurement();
                }
            }
            let parsed = crate::derived::parse(&def.expression);
            let (expr, error) = match parsed {
                Ok(expr) => (Some(expr), None),
                Err(error) => (None, Some(error)),
            };
            let refs = expr
                .as_ref()
                .map(crate::derived::references)
                .unwrap_or_default()
                .into_iter()
                .map(str::to_string)
                .collect();
            runtimes.push(DerivedRuntime {
                def,
                key,
                expr,
                refs,
                pinned,
                error,
                last_t_us: last,
            });
        }
        self.derived = runtimes;
    }

    /// Samples every derived channel from its sources' histories, oldest new
    /// point first. Definitions are evaluated in list order, so a channel may
    /// reference an earlier one; the union of all source timestamps is used,
    /// with the newest sample at or before each instant (sample-and-hold),
    /// exactly how the observers read a signal.
    pub(super) fn evaluate_derived(&mut self) {
        if self.derived.is_empty() {
            return;
        }
        // Sources are resolved against the subscriptions, never the whole
        // database: a derived lane follows what the run is actually watching.
        let all_keys: Vec<SigKey> = self.subs.keys().cloned().collect();
        for index in 0..self.derived.len() {
            let (key, refs, pinned, last) = {
                let runtime = &self.derived[index];
                if runtime.expr.is_none() {
                    continue;
                }
                (
                    runtime.key.clone(),
                    runtime.refs.clone(),
                    runtime.pinned.clone(),
                    runtime.last_t_us,
                )
            };
            let mut sources = Vec::with_capacity(refs.len());
            let mut unresolved = false;
            for text in &refs {
                if let Some((_, source)) = pinned.iter().find(|(name, _)| name == text) {
                    sources.push(source.clone());
                    continue;
                }
                match resolve_derived_ref(text, &all_keys, self, &key) {
                    Some(source) => sources.push(source),
                    None => unresolved = true,
                }
            }
            if unresolved {
                self.derived[index].error = Some(format!(
                    "unknown or ambiguous reference in [{}]",
                    refs.join("], [")
                ));
                continue;
            }
            // Only the points newer than the previous sweep are new work.
            // Walking the whole hour-long cache here would turn every step
            // into a full-history scan once a session has run for a while.
            let since = last.map_or(0, |last| last.saturating_add(1));
            let mut times: std::collections::BTreeSet<u64> = std::collections::BTreeSet::new();
            for source in &sources {
                let Some(sub) = self.subs.get(source) else {
                    continue;
                };
                for (t, _) in sub.history.range(since, u64::MAX) {
                    times.insert(*t);
                }
            }
            if times.is_empty() {
                continue;
            }
            let produced = {
                let expr = self.derived[index].expr.as_ref().expect("checked above");
                let mut produced = Vec::with_capacity(times.len());
                for &t in &times {
                    let value = crate::derived::eval(expr, &mut |name| {
                        let position = refs.iter().position(|candidate| candidate == name)?;
                        self.subs
                            .get(&sources[position])
                            .and_then(|sub| sub.history.at(t))
                    });
                    if let Some(value) = value {
                        produced.push((t, value));
                    }
                }
                produced
            };
            let end = times.into_iter().next_back().unwrap_or(0);
            let stride = self.applied_stride_us;
            if let Some(sub) = self.subs.get_mut(&key) {
                for (t, value) in produced {
                    sub.latest = value;
                    sub.last_raw = value.round() as i64;
                    sub.last_update_us = t;
                    sub.push_sample(t, value, stride);
                }
            }
            let runtime = &mut self.derived[index];
            runtime.last_t_us = Some(end);
            runtime.error = None;
        }
    }

    /// The next free "CAN{n}" for a newly added bus. An empty workspace starts
    /// from CAN1 even if a restored project carried a stale counter from
    /// buses that no longer exist.
    pub(super) fn next_can_name(&mut self) -> String {
        if self.can_count() == 0 {
            self.bus_counter = 0;
        }
        self.bus_counter += 1;
        format!("CAN{}", self.bus_counter)
    }

    /// Adds a bus with the sample DBC path, loads it and pre-populates its
    /// generator from the database.
    pub(super) fn add_channel(&mut self, status: &mut String) {
        let name = self.next_can_name();
        let cluster = self.cluster_ids.allocate();
        self.can_cluster_ids.push(cluster);
        let sources: Vec<crate::model::ClusterSource> = vec![crate::model::ClusterSource::Dbc {
            path: "builtin:sample.dbc".to_string(),
        }];
        self.model_comms
            .push(Some(Arc::new(crate::model::empty_can_comm(
                &name,
                cluster,
                &sources,
                crate::model::CanParams::DEFAULT_BITRATE_KBPS,
                crate::model::CanParams::DEFAULT_FD_DATA_KBPS,
            ))));
        self.model_plans.push(None);
        self.model_dbs.push(None);
        self.can_xml_models.push(None);
        self.bus_loads.push(crate::load::BusLoad::new());
        self.loads_dirty = true;
        let ch = self.can_count() - 1;
        self.load_channel(ch, status);
        let ids: Vec<u32> = self.model_dbs[ch]
            .as_ref()
            .map(|db| db.order.iter().map(|&(id, _)| id).collect())
            .unwrap_or_default();
        for id in ids {
            self.add_entry(ch as u8, id);
        }
    }

    /// Adds a CAN bus with no description at all: raw monitoring, and the
    /// Buses detail binds DBC/ARXML files afterwards.
    pub(super) fn add_empty_channel(&mut self, status: &mut String) {
        let name = self.next_can_name();
        let cluster = self.cluster_ids.allocate();
        self.can_cluster_ids.push(cluster);
        self.model_comms
            .push(Some(Arc::new(crate::model::empty_can_comm(
                &name,
                cluster,
                &[],
                crate::model::CanParams::DEFAULT_BITRATE_KBPS,
                crate::model::CanParams::DEFAULT_FD_DATA_KBPS,
            ))));
        self.model_plans.push(None);
        self.model_dbs.push(None);
        self.can_xml_models.push(None);
        self.bus_loads.push(crate::load::BusLoad::new());
        self.loads_dirty = true;
        self.nodes_dirty = true;
        *status = format!("{name} added");
    }

    /// Test hook: append a bus from a parsed table, the shape fixtures still
    /// use to seed a specific CAN database.
    #[cfg(test)]
    pub(super) fn add_channel_with_dbc(
        &mut self,
        name: String,
        dbc: Arc<crate::dbc::SymbolTable>,
        bitrate_kbps: u32,
        status: &mut String,
    ) {
        self.bus_counter += 1;
        let cluster = self.cluster_ids.allocate();
        self.can_cluster_ids.push(cluster);
        self.model_comms
            .push(Some(Arc::new(crate::model::empty_can_comm(
                &name,
                cluster,
                &[],
                bitrate_kbps,
                crate::model::CanParams::DEFAULT_FD_DATA_KBPS,
            ))));
        self.model_plans.push(None);
        self.model_dbs.push(Some(Arc::clone(&dbc)));
        self.can_xml_models.push(None);
        self.bus_loads.push(crate::load::BusLoad::new());
        self.loads_dirty = true;
        self.nodes_dirty = true;
        let ch = self.can_count() - 1;
        self.rebuild_can_model(ch);
        self.seed_entries_for_simulated(ch);
        let ids: Vec<u32> = dbc.order.iter().map(|&(id, _)| id).collect();
        for id in ids {
            self.add_entry(ch as u8, id);
        }
        *status = format!("{name} ARXML loaded: {} message(s)", dbc.order.len());
    }

    /// (Re)loads every database attached to the bus (primary first, then
    /// extras) and merges them into one lookup table: on a duplicate
    /// message id the earlier database wins. Files that fail to parse are
    /// skipped and reported; a bus whose files all fail ends up without a
    /// database. An empty path list is silent: a bus deliberately
    /// configured without a database is not an error.
    pub(super) fn load_channel(&mut self, ch: usize, status: &mut String) -> bool {
        if ch >= self.can_count() {
            return false;
        }
        let cluster = self.can_cluster_ids[ch];
        let paths = self.can_paths(ch);
        if paths.iter().all(|p| p.trim().is_empty()) {
            self.can_sums.remove(&cluster);
            self.set_can_table(ch, None);
            return false;
        }
        let name = self.can_name(ch);
        let mut tables: Vec<crate::dbc::SymbolTable> = Vec::new();
        let mut sums: Vec<u64> = Vec::new();
        let mut first_error: Option<String> = None;
        for path in &paths {
            let path = path.trim();
            if path.is_empty() {
                continue;
            }
            let content = match path.strip_prefix("builtin:") {
                Some(name) => match crate::sample_data::text(name) {
                    Some(text) => Ok(text.to_string()),
                    None => Err(format!("{path}: unknown built-in database")),
                },
                None => {
                    crate::dbc::read_file(std::path::Path::new(path)).map_err(|e| e.to_string())
                }
            };
            match content {
                Ok(content) => match crate::dbc::load_dbc_str(&content) {
                    Ok(table) => {
                        sums.push(content_sum(content.as_bytes()));
                        tables.push(table);
                    }
                    Err(e) => {
                        first_error.get_or_insert_with(|| format!("{path}: {e}"));
                    }
                },
                Err(e) => {
                    first_error.get_or_insert_with(|| format!("{path}: {e}"));
                }
            }
        }
        self.can_sums.insert(cluster, sums);
        let loaded = tables.len();
        let total: usize = tables.iter().map(|t| t.order.len()).sum();
        if tables.is_empty() {
            self.set_can_table(ch, None);
            *status = match first_error {
                Some(e) => format!("{name} DBC error: {e}"),
                None => format!("{name} DBC error: no database file"),
            };
            return false;
        }
        let mut merged = tables.remove(0);
        for extra in tables {
            crate::dbc::absorb(&mut merged, extra);
        }
        let merged = std::sync::Arc::new(merged);
        // Transmitter lists feed the role cards.
        self.nodes_dirty = true;
        self.set_can_table(ch, Some(merged.clone()));
        // `$报文::信号` 的名字→id 映射随库刷新：报文换了 id 或名字没了，
        // 节点的映射原地更新，变化进节点日志（静默错位由此可解释）。
        for node in &mut self.nodes {
            if node.channel as usize == ch {
                node.refresh_named_signals(&merged);
            }
        }
        // The role declarations are already in place on a reload, so the
        // fresh table has to meet them: Simulated nodes get entries for
        // messages the database newly declares. The model is rebuilt first
        // because the node table now reads the frame ports from it.
        self.rebuild_can_model(ch);
        self.seed_entries_for_simulated(ch);
        *status = match first_error {
            Some(e) => format!("{name} DBC partial: {loaded} file(s), {total} messages ({e})"),
            None => format!("{name} DBC loaded: {total} messages"),
        };
        true
    }

    /// Test hook for fixtures that used to assign a table directly.
    #[cfg(test)]
    pub(crate) fn set_can_table_for_test(
        &mut self,
        ch: usize,
        db: Option<Arc<crate::dbc::SymbolTable>>,
    ) {
        self.set_can_table(ch, db);
    }

    #[cfg(test)]
    pub(crate) fn copy_can_table_for_test(&mut self, from: usize, to: usize) {
        let db = self.model_dbs.get(from).and_then(|db| db.clone());
        self.set_can_table(to, db);
    }

    #[cfg(test)]
    pub(crate) fn move_can_table_for_test(&mut self, from: usize, to: usize) {
        let db = self.model_dbs.get_mut(from).and_then(|db| db.take());
        self.set_can_table(to, db);
        self.rebuild_can_model(from);
    }

    /// Replaces the parsed table behind a CAN channel and rebuilds its model.
    /// The table is the model adapter's guard, not separate bus state.
    pub(super) fn set_can_table(&mut self, ch: usize, db: Option<Arc<crate::dbc::SymbolTable>>) {
        while self.model_dbs.len() <= ch {
            self.model_dbs.push(None);
            self.can_xml_models.push(None);
        }
        self.model_dbs[ch] = db;
        self.rebuild_can_model(ch);
    }

    /// Rebuilds the unified model and its decode plans for one CAN channel.
    /// The DBC table (when present) and every bound ARXML aggregate are
    /// merged into the one cluster id the bus owns.
    pub(crate) fn rebuild_can_model(&mut self, ch: usize) {
        while self.model_comms.len() <= ch {
            self.model_comms.push(None);
            self.model_plans.push(None);
            self.model_dbs.push(None);
            self.can_xml_models.push(None);
        }
        if ch >= self.can_cluster_ids.len() {
            return;
        }
        let cluster_id = self.can_cluster_ids[ch];
        let name = self.can_name(ch);
        let (bitrate, fd) = self.can_bitrates(ch);
        let sources = self
            .can_cluster(ch)
            .map(|cluster| cluster.sources.clone())
            .unwrap_or_default();
        let db = self.model_dbs.get(ch).and_then(|db| db.clone());
        let mut comm = match &db {
            Some(db) => {
                let mut ids = crate::model::ElementIds::new();
                let outcome = crate::model::import_symbol_table_sources(
                    db, &name, cluster_id, &sources, &mut ids, bitrate, fd,
                );
                self.report_model_issues(&name, &outcome.issues);
                outcome.comm
            }
            None => crate::model::empty_can_comm(&name, cluster_id, &sources, bitrate, fd),
        };
        if let Some(extra) = self.can_xml_models.get(ch).and_then(|model| model.clone()) {
            let mut extra = (*extra).clone();
            extra.rebind_cluster(cluster_id);
            comm.merge_from(extra, cluster_id);
        }
        let (plans, plan_issues) = crate::model::PlanCache::build(&comm, 1);
        self.report_model_issues(&name, &comm.issues);
        self.report_model_issues(&name, &plan_issues);
        self.model_comms[ch] = Some(Arc::new(comm));
        self.model_plans[ch] = Some(Arc::new(plans));
        self.model_dbs[ch] = db;
    }

    /// Writes model and plan issues once each into the Write log. The status
    /// bar is for the action the user just took; an unsupported element in
    /// the matrix has to stay somewhere they can still read.
    pub(crate) fn report_model_issues(&mut self, label: &str, issues: &[crate::model::Issue]) {
        for issue in issues {
            if issue.severity == crate::model::IssueSeverity::Info {
                continue;
            }
            let text = format!("{label}: {}", issue.message);
            if !self.model_issues_seen.insert(text.clone()) {
                continue;
            }
            let kind = if issue.severity == crate::model::IssueSeverity::Error {
                WriteKind::Error
            } else {
                WriteKind::Warning
            };
            self.write_push(kind, text);
        }
    }

    pub(super) fn set_channel_model(
        &mut self,
        ch: u8,
        comm: Option<Arc<crate::model::Comm>>,
        sources: Vec<crate::model::ClusterSource>,
    ) {
        let ch = ch as usize;
        if ch >= self.can_count() {
            return;
        }
        if self.model_comms.len() <= ch {
            self.model_comms.resize(ch + 1, None);
            self.model_plans.resize(ch + 1, None);
            self.model_dbs.resize(ch + 1, None);
            self.can_xml_models.resize(ch + 1, None);
        }
        self.can_xml_models[ch] = comm;
        let cluster_id = self.can_cluster_ids[ch];
        let (bitrate, fd) = self.can_bitrates(ch);
        let name = self.can_name(ch);
        // Keep the DBC sources, replace the document-derived ones, so the
        // cluster lists exactly the descriptions bound to this bus.
        let dbc: Vec<_> = self
            .can_cluster(ch)
            .map(|cluster| {
                cluster
                    .sources
                    .iter()
                    .filter(|source| matches!(source, crate::model::ClusterSource::Dbc { .. }))
                    .cloned()
                    .collect()
            })
            .unwrap_or_default();
        let mut combined = dbc;
        combined.extend(sources);
        if self.model_comms[ch].is_none() {
            self.model_comms[ch] = Some(Arc::new(crate::model::empty_can_comm(
                &name, cluster_id, &combined, bitrate, fd,
            )));
        } else if let Some(current) = self.model_comms[ch].as_mut()
            && let Some(cluster) = Arc::make_mut(current).clusters.get_mut(&cluster_id)
        {
            cluster.sources = combined;
        }
        self.rebuild_can_model(ch);
        if let Some(current) = self.model_comms[ch].as_ref() {
            for node in &mut self.nodes {
                if node.channel as usize == ch {
                    node.refresh_named_signals_from_model(current, cluster_id);
                }
            }
        }
    }

    #[cfg(test)]
    pub(super) fn set_fr_model(&mut self, bus: u8, comm: Arc<crate::model::Comm>) {
        if !self.fr_dbs.contains_key(&bus) {
            return;
        }
        let id = match self.fr_cluster_ids.get(&bus) {
            Some(id) => *id,
            None => {
                let id = self.cluster_ids.allocate();
                self.fr_cluster_ids.insert(bus, id);
                id
            }
        };
        let mut comm = (*comm).clone();
        comm.rebind_cluster(id);
        let (plans, plan_issues) = crate::model::PlanCache::build(&comm, 1);
        self.report_model_issues(&format!("FR{bus}"), &comm.issues);
        self.report_model_issues(&format!("FR{bus}"), &plan_issues);
        self.fr_models.insert(
            bus,
            Arc::new(FrRuntime {
                comm: Arc::new(comm),
                plans: Arc::new(plans),
                db: Arc::clone(&self.fr_dbs[&bus]),
            }),
        );
    }

    pub(super) fn rebuild_fr_models(&mut self) {
        self.fr_models.clear();
        let buses: Vec<u8> = self.fr_dbs.keys().copied().collect();
        self.fr_cluster_ids.retain(|bus, _| buses.contains(bus));
        for bus in &buses {
            if !self.fr_cluster_ids.contains_key(bus) {
                let id = self.cluster_ids.allocate();
                self.fr_cluster_ids.insert(*bus, id);
            }
        }
        let dbs: Vec<(u8, Arc<crate::fr_db::FrDb>)> = self
            .fr_dbs
            .iter()
            .map(|(bus, db)| (*bus, Arc::clone(db)))
            .collect();
        for (bus, db) in dbs {
            let mut ids = crate::model::ElementIds::new();
            let cluster_id = self
                .fr_cluster_ids
                .get(&bus)
                .copied()
                .unwrap_or(crate::model::ClusterId(1000 + u32::from(bus)));
            let sources = self.fr_sources.get(&bus).cloned().unwrap_or_default();
            let source = sources
                .first()
                .cloned()
                .unwrap_or(crate::model::ClusterSource::Manual);
            let outcome =
                crate::model::import_fr_db(&db, &format!("FR{bus}"), cluster_id, &source, &mut ids);
            let mut comm = outcome.comm;
            if let Some(cluster) = comm.clusters.get_mut(&cluster_id) {
                cluster.sources = sources;
            }
            let (plans, plan_issues) = crate::model::PlanCache::build(&comm, 1);
            let label = format!("FR{bus}");
            self.report_model_issues(&label, &outcome.issues);
            self.report_model_issues(&label, &plan_issues);
            self.fr_models.insert(
                bus,
                Arc::new(FrRuntime {
                    comm: Arc::new(comm),
                    plans: Arc::new(plans),
                    db,
                }),
            );
        }
    }

    pub(super) fn rebuild_eth_models(&mut self) {
        self.eth_models.clear();
        let buses: Vec<u8> = self.eth_dbs.keys().copied().collect();
        self.eth_cluster_ids.retain(|bus, _| buses.contains(bus));
        for bus in &buses {
            if !self.eth_cluster_ids.contains_key(bus) {
                let id = self.cluster_ids.allocate();
                self.eth_cluster_ids.insert(*bus, id);
            }
        }
        let dbs: Vec<(u8, Arc<crate::eth::db::EthDb>)> = self
            .eth_dbs
            .iter()
            .map(|(bus, db)| (*bus, Arc::clone(db)))
            .collect();
        for (bus, db) in dbs {
            let mut ids = crate::model::ElementIds::new();
            let cluster_id = self
                .eth_cluster_ids
                .get(&bus)
                .copied()
                .unwrap_or(crate::model::ClusterId(2000 + u32::from(bus)));
            let sources = self.eth_sources.get(&bus).cloned().unwrap_or_default();
            let source = sources
                .first()
                .cloned()
                .unwrap_or(crate::model::ClusterSource::Manual);
            let outcome = crate::model::import_eth_db_obj(
                &db,
                &format!("ETH{bus}"),
                cluster_id,
                &source,
                &mut ids,
            );
            let mut comm = outcome.comm;
            let issues = outcome.issues;
            if let Some(cluster) = comm.clusters.get_mut(&cluster_id) {
                cluster.sources = sources;
            }
            let (plans, plan_issues) = crate::model::PlanCache::build(&comm, 1);
            let label = format!("ETH{bus}");
            self.report_model_issues(&label, &issues);
            self.report_model_issues(&label, &plan_issues);
            self.eth_models.insert(
                bus,
                Arc::new(EthRuntime {
                    comm: Arc::new(comm),
                    plans: Arc::new(plans),
                    db,
                }),
            );
        }
        // A service catalog imported before its matrices must survive a later
        // ETH model rebuild as Comm entities, not just in the flat catalog.
        if !self.someip_services.services.is_empty() {
            let catalog = self.someip_services.clone();
            self.adopt_someip_services(catalog);
        }
    }
}
