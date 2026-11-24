//! ScriptNode methods: signals.

use super::*;

impl ScriptNode {
    /// Recomputes the named-read name→id map against `db` — a
    /// DBC reload may have moved a name onto a different id or dropped
    /// it. The map refreshes in place and every change lands in the node
    /// log as a `[reload]` line, so a stale read is explainable.
    pub fn refresh_named_signals(&mut self, db: &crate::dbc::SymbolTable) {
        self.refresh_named_with(&|message| db.message_id_by_name(message), &|message| {
            db.message_key_by_name(message)
        });
    }

    /// The same refresh against the unified model, for ARXML clusters that
    /// no longer have a DBC-shaped table.
    pub fn refresh_named_signals_from_model(
        &mut self,
        comm: &crate::model::Comm,
        cluster: crate::model::ClusterId,
    ) {
        let frame = |name: &str| comm.frame_by_name(cluster, name);
        self.refresh_named_with(
            &|message| frame(message).and_then(|frame| frame.address.can_id()),
            &|message| {
                frame(message).and_then(|frame| match frame.address {
                    crate::model::FrameAddress::Can { id, extended, .. } => Some((id, extended)),
                    _ => None,
                })
            },
        );
    }

    pub(super) fn refresh_named_with(
        &mut self,
        id_of: &dyn Fn(&str) -> Option<u32>,
        key_of: &dyn Fn(&str) -> Option<(u32, bool)>,
    ) {
        let Some(rt) = self.runtime.as_mut() else {
            return;
        };
        for (msg, sig) in rt.vm.named_signal_refs().to_vec() {
            let new_id = id_of(&msg);
            let old = rt.vm.named_messages.get(&msg).copied();
            match (old, new_id) {
                (Some(o), Some(n)) if o == n => {}
                (None, None) => {}
                (old, new) => {
                    if let Some(n) = new {
                        rt.vm.named_messages.insert(msg.clone(), n);
                    } else {
                        rt.vm.named_messages.remove(&msg);
                    }
                    let what = match (old, new) {
                        (Some(o), Some(n)) => {
                            format!("\"{msg}\".{sig} id 0x{o:X} -> 0x{n:X}")
                        }
                        (Some(o), None) => {
                            format!("\"{msg}\".{sig} (0x{o:X}) was removed from the database")
                        }
                        (None, Some(n)) => format!("\"{msg}\".{sig} added (0x{n:X})"),
                        (None, None) => continue,
                    };
                    Self::push_log_into(
                        &mut self.log,
                        &mut self.log_dirty,
                        &mut self.pending_lines,
                        format!("[reload] {what}"),
                    );
                }
            }
        }
        // Handlers named after a DBC message follow the same reload: a name
        // that moved to another id (or its frame class) re-points in place.
        let mut taken: std::collections::HashSet<(u32, bool)> = rt
            .handlers
            .iter()
            .filter_map(|handler| match handler.kind {
                HandlerKind::Message { id, .. } => Some((id, false)),
                HandlerKind::ExtendedMessage { id } => Some((id, true)),
                _ => None,
            })
            .collect();
        let mut names: Vec<String> = Vec::new();
        for handler in &rt.handlers {
            if let HandlerKind::NamedMessage { name } = &handler.kind
                && !names.contains(name)
            {
                names.push(name.clone());
            }
        }
        let mut resolved = HashMap::new();
        for name in &names {
            if let Some(key) = key_of(name)
                && taken.insert(key)
            {
                resolved.insert(name.clone(), key);
            }
        }
        for name in &names {
            let old = rt.named_handler_ids.get(name).copied();
            let new = resolved.get(name).copied();
            if old == new {
                continue;
            }
            let what = match (old, new) {
                (Some((o, oe)), Some((n, ne))) => format!(
                    "handler \"{name}\" 0x{o:X}{} -> 0x{n:X}{}",
                    if oe { "x" } else { "" },
                    if ne { "x" } else { "" }
                ),
                (Some((o, oe)), None) => format!(
                    "handler \"{name}\" (0x{o:X}{}) lost its message in the database",
                    if oe { "x" } else { "" }
                ),
                (None, Some((n, ne))) => format!(
                    "handler \"{name}\" added at 0x{n:X}{}",
                    if ne { "x" } else { "" }
                ),
                (None, None) => continue,
            };
            Self::push_log_into(
                &mut self.log,
                &mut self.log_dirty,
                &mut self.pending_lines,
                format!("[reload] {what}"),
            );
        }
        rt.named_handler_ids = resolved;
    }

    /// Re-resolves dotted FlexRay reads against a replaced cluster
    /// description. The slot map updates in place and every change lands
    /// in the log as a `[reload]` line, like a DBC reload does for named
    /// message reads.
    pub fn refresh_fr_named(
        &mut self,
        dbs: &std::collections::BTreeMap<u8, std::sync::Arc<crate::fr_db::FrDb>>,
    ) {
        let Some(rt) = self.runtime.as_mut() else {
            return;
        };
        let mut resolved = HashMap::new();
        for (bus, sig) in rt.vm.fr_named_refs().to_vec() {
            let Some(db) = dbs.get(&bus) else {
                continue;
            };
            if let Ok(slot) = fr_slot_of(db, bus, &sig) {
                resolved.insert((bus, sig), slot);
            }
        }
        if rt.vm.fr_named_slots != resolved {
            let old = std::mem::replace(&mut rt.vm.fr_named_slots, resolved.clone());
            for ((bus, sig), slot) in &resolved {
                match old.get(&(*bus, sig.clone())) {
                    Some(previous) if previous == slot => {}
                    Some(previous) => Self::push_log_into(
                        &mut self.log,
                        &mut self.log_dirty,
                        &mut self.pending_lines,
                        format!("[reload] FR{bus}.{sig} slot {previous} -> {slot}"),
                    ),
                    None => Self::push_log_into(
                        &mut self.log,
                        &mut self.log_dirty,
                        &mut self.pending_lines,
                        format!("[reload] FR{bus}.{sig} resolved to slot {slot}"),
                    ),
                }
            }
            for ((bus, sig), slot) in &old {
                if !resolved.contains_key(&(*bus, sig.clone())) {
                    Self::push_log_into(
                        &mut self.log,
                        &mut self.log_dirty,
                        &mut self.pending_lines,
                        format!("[reload] FR{bus}.{sig} unresolved (was slot {slot})"),
                    );
                }
            }
        }
        // The named-cluster reads move with the same description: the name
        // comes from the cluster file, so a replacement can point it at a
        // different bus/slot or make it unknown.
        let mut named = HashMap::new();
        for (bus_name, sig) in rt.vm.fr_bus_refs().to_vec() {
            if let Ok(bus) = fr_bus_by_name(dbs, &bus_name)
                && let Some(db) = dbs.get(&bus)
                && let Ok(slot) = fr_slot_of(db, bus, &sig)
            {
                named.insert((bus_name, sig), (bus, slot));
            }
        }
        if rt.vm.fr_bus_named_slots != named {
            let old_named = std::mem::replace(&mut rt.vm.fr_bus_named_slots, named.clone());
            for ((bus_name, sig), (bus, slot)) in &named {
                match old_named.get(&(bus_name.clone(), sig.clone())) {
                    Some(previous) if previous == &(*bus, *slot) => {}
                    Some(previous) => Self::push_log_into(
                        &mut self.log,
                        &mut self.log_dirty,
                        &mut self.pending_lines,
                        format!(
                            "[reload] FR.{bus_name}.{sig} FR{} slot {} -> FR{bus} slot {slot}",
                            previous.0, previous.1
                        ),
                    ),
                    None => Self::push_log_into(
                        &mut self.log,
                        &mut self.log_dirty,
                        &mut self.pending_lines,
                        format!("[reload] FR.{bus_name}.{sig} resolved to FR{bus} slot {slot}"),
                    ),
                }
            }
            for ((bus_name, sig), (bus, slot)) in &old_named {
                if !named.contains_key(&(bus_name.clone(), sig.clone())) {
                    Self::push_log_into(
                        &mut self.log,
                        &mut self.log_dirty,
                        &mut self.pending_lines,
                        format!(
                            "[reload] FR.{bus_name}.{sig} unresolved (was FR{bus} slot {slot})"
                        ),
                    );
                }
            }
        }
    }

    /// Re-resolves named-cluster Ethernet reads against replaced matrices.
    pub fn refresh_eth_named(
        &mut self,
        dbs: &std::collections::BTreeMap<u8, std::sync::Arc<crate::eth::db::EthDb>>,
    ) {
        let Some(rt) = self.runtime.as_mut() else {
            return;
        };
        let mut resolved = HashMap::new();
        for (bus_name, frame, sig) in rt.vm.eth_bus_refs().to_vec() {
            if let Ok(bus) = eth_bus_by_name(dbs, &bus_name)
                && let Some(db) = dbs.get(&bus)
                && eth_signal_check(db, bus, &frame, &sig).is_ok()
            {
                resolved.insert((bus_name, frame, sig), bus);
            }
        }
        if rt.vm.eth_bus_named_ids == resolved {
            return;
        }
        let old = std::mem::replace(&mut rt.vm.eth_bus_named_ids, resolved.clone());
        for ((bus_name, frame, sig), bus) in &resolved {
            match old.get(&(bus_name.clone(), frame.clone(), sig.clone())) {
                Some(previous) if previous == bus => {}
                Some(previous) => Self::push_log_into(
                    &mut self.log,
                    &mut self.log_dirty,
                    &mut self.pending_lines,
                    format!("[reload] ETH.{bus_name}.{frame}.{sig} ETH{previous} -> ETH{bus}"),
                ),
                None => Self::push_log_into(
                    &mut self.log,
                    &mut self.log_dirty,
                    &mut self.pending_lines,
                    format!("[reload] ETH.{bus_name}.{frame}.{sig} resolved to ETH{bus}"),
                ),
            }
        }
        for ((bus_name, frame, sig), bus) in &old {
            if !resolved.contains_key(&(bus_name.clone(), frame.clone(), sig.clone())) {
                Self::push_log_into(
                    &mut self.log,
                    &mut self.log_dirty,
                    &mut self.pending_lines,
                    format!("[reload] ETH.{bus_name}.{frame}.{sig} unresolved (was ETH{bus})"),
                );
            }
        }
    }
}
