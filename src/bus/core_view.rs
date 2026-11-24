//! Read-only views and snapshots handed to the web layer.

use super::*;

impl BusCore {
    /// One frame-shaped read of the bus for the frontend.
    pub(super) fn cluster_ids_view(&self) -> WebClusterIds {
        WebClusterIds {
            next: self.cluster_ids.next(),
            can: self.can_cluster_ids.iter().map(|id| id.0).collect(),
            fr: self
                .fr_cluster_ids
                .iter()
                .map(|(bus, id)| (*bus, id.0))
                .collect(),
            eth: self
                .eth_cluster_ids
                .iter()
                .map(|(bus, id)| (*bus, id.0))
                .collect(),
        }
    }

    pub(super) fn model_frames_view(
        &self,
    ) -> std::collections::BTreeMap<(u8, u32, bool), ModelFrameView> {
        use crate::model::FrameAddress;
        let mut out = std::collections::BTreeMap::new();
        for (ch, comm) in self.model_comms.iter().enumerate() {
            let (Some(comm), Some(plans)) = (
                comm.as_ref(),
                self.model_plans.get(ch).and_then(Option::as_ref),
            ) else {
                continue;
            };
            for frame in comm.frames.values() {
                let FrameAddress::Can { id, extended, .. } = frame.address else {
                    continue;
                };
                let signal_count = plans
                    .get(frame.id)
                    .map(|plan| plan.reads.len())
                    .unwrap_or(0);
                out.insert(
                    (ch as u8, id, extended),
                    ModelFrameView {
                        name: frame.name.clone(),
                        signal_count,
                    },
                );
            }
        }
        out
    }

    pub(super) fn fr_model_frames_view(
        &self,
    ) -> std::collections::BTreeMap<(u8, u16), ModelFrameView> {
        use crate::model::FrameAddress;
        let mut out = std::collections::BTreeMap::new();
        for (bus, runtime) in &self.fr_models {
            let plans = Some(&runtime.plans);
            for frame in runtime.frames.values() {
                let FrameAddress::FlexRay { slot, .. } = frame.address else {
                    continue;
                };
                let signal_count = plans
                    .and_then(|plans| plans.get(frame.id))
                    .map(|plan| plan.reads.len())
                    .unwrap_or(0);
                out.insert(
                    (*bus, slot),
                    ModelFrameView {
                        name: frame.name.clone(),
                        signal_count,
                    },
                );
            }
        }
        out
    }

    pub(super) fn eth_model_frames_view(
        &self,
    ) -> std::collections::BTreeMap<String, ModelFrameView> {
        use crate::model::FrameAddress;
        let mut out = std::collections::BTreeMap::new();
        for runtime in self.eth_models.values() {
            let plans = Some(&runtime.plans);
            for frame in runtime.frames.values() {
                if !matches!(frame.address, FrameAddress::Ethernet { .. }) {
                    continue;
                }
                let signal_count = plans
                    .and_then(|plans| plans.get(frame.id))
                    .map(|plan| plan.reads.len())
                    .unwrap_or(0);
                out.insert(
                    frame.name.clone(),
                    ModelFrameView {
                        name: frame.name.clone(),
                        signal_count,
                    },
                );
            }
        }
        out
    }

    pub(super) fn expanded_signals_view(
        &self,
    ) -> std::collections::BTreeMap<String, Vec<ExpandedSignal>> {
        let mut out = std::collections::BTreeMap::new();
        if self.expanded_messages.is_empty() {
            return out;
        }
        for agg in self.aggs.values() {
            let key = format!("can:{}:{}:{}", agg.channel, agg.id, agg.extended);
            if !self.expanded_messages.contains(&key) {
                continue;
            }
            if agg.flags.contains(crate::can::frame::FrameFlags::ERROR)
                || agg.flags.contains(crate::can::frame::FrameFlags::RTR)
            {
                continue;
            }
            let frame = CanFrame {
                t_us: agg.last_t_us,
                channel: agg.channel,
                id: agg.id,
                extended: agg.extended,
                len: agg.len,
                data: agg.data,
                dir: agg.dir,
                flags: agg.flags,
            };
            let decoded = self.model_can_decoded(&frame, false).unwrap_or_default();
            out.insert(
                key,
                decoded
                    .into_iter()
                    .map(|signal| ExpandedSignal {
                        value: crate::dbc::fmt_decoded(&signal.type_tag, signal.phys),
                        name: signal.name,
                        raw: signal.raw,
                        unit: signal.unit,
                        label: signal.label,
                    })
                    .collect(),
            );
        }
        for ((bus, slot, _), agg) in &self.fr_aggs {
            let occupant = match &agg.occupant {
                crate::aggregate::FrOccupant::Frame(index) => format!("f{index}"),
                crate::aggregate::FrOccupant::Logged(name) => format!("l:{name}"),
                crate::aggregate::FrOccupant::Unknown => "u".to_string(),
            };
            let key = format!("fr:{bus}:{slot}:{occupant}");
            if !self.expanded_messages.contains(&key) {
                continue;
            }
            let row = crate::trace::FrRow {
                bus: *bus,
                t_us: agg.last_t_us,
                ab: agg.ab,
                slot: *slot,
                cycle: 0,
                payload: agg.payload.clone(),
                header_crc: 0,
                flags: 0,
                name: agg.name.clone(),
            };
            let decoded = agg
                .occupant
                .frame_ix()
                .and_then(|ix| self.model_fr_decoded(&row, ix, false))
                .unwrap_or_default();
            out.insert(
                key,
                decoded
                    .into_iter()
                    .map(|signal| ExpandedSignal {
                        value: crate::dbc::fmt_decoded(&signal.type_tag, signal.phys),
                        name: signal.name,
                        raw: signal.raw,
                        unit: signal.unit,
                        label: signal.label,
                    })
                    .collect(),
            );
        }
        for (flow_key, agg) in &self.eth_aggs {
            let key = flow_key.message_key();
            if !self.expanded_messages.contains(&key) {
                continue;
            }
            let Some(db) = self.eth_dbs.get(&flow_key.bus) else {
                continue;
            };
            let flow = flow_key.flow();
            let decoded = match db.match_flow(&flow, self.eth_length_includes_header) {
                crate::eth::db::EthMatch::Unique(index) => {
                    let name = &db.frames[index].name;
                    self.model_eth_decoded(flow_key.bus, name, &agg.payload, false)
                        .unwrap_or_default()
                }
                _ => Vec::new(),
            };
            out.insert(
                key,
                decoded
                    .into_iter()
                    .map(|signal| ExpandedSignal {
                        value: crate::dbc::fmt_decoded(&signal.type_tag, signal.phys),
                        name: signal.name,
                        raw: signal.raw,
                        unit: signal.unit,
                        label: signal.label,
                    })
                    .collect(),
            );
        }
        out
    }

    pub(crate) fn snapshot(&self) -> Snapshot {
        let replay = match (self.source.position(), self.source.duration()) {
            (Some(p), Some(d)) => Some((p, d)),
            _ => None,
        };
        Snapshot {
            frame_counter: self.frame_counter,
            #[cfg(test)]
            trace_len: self.trace.len(),
            trace: Arc::clone(&self.published_trace),
            fr_trace: Arc::clone(&self.published_fr),
            eth_trace: Arc::clone(&self.published_eth),
            someip: self.someip_events.iter().cloned().collect(),
            someip_subscriptions: self
                .someip_subscriptions
                .iter()
                .map(|subscription| SomeIpSubscriptionView {
                    bus: subscription.bus,
                    service: subscription.service,
                    instance: subscription.instance,
                    eventgroup: subscription.eventgroup,
                    ip: subscription.ip.clone(),
                    port: subscription.port,
                    ttl: subscription.ttl,
                })
                .collect(),
            eth_attached: self
                .eth_dbs
                .keys()
                .filter_map(|bus| {
                    self.hw
                        .eth_device(*bus)
                        .map(|device| (*bus, device.to_string()))
                })
                .collect(),
            #[cfg(test)]
            fr_dropped: self.fr_trace.dropped(),
            fr_watches: {
                let mut watches: Vec<FrWatchView> = self
                    .hw
                    .fr_watches
                    .values()
                    .map(|w| FrWatchView {
                        bus: w.bus,
                        channel_index: w.channel_index,
                        fibex_path: w.fibex_path.clone(),
                    })
                    .collect();
                watches.sort_by_key(|w| w.bus);
                watches
            },
            #[cfg(test)]
            sub_count: self.subs.len(),
            replay,
            replay_map: self.replay_map.clone(),
            replay_census_can: self.log_census.rows(crate::replay_map::LogBusKind::Can),
            replay_census_fr: self.log_census.rows(crate::replay_map::LogBusKind::Fr),
            replay_census_eth: self.log_census.rows(crate::replay_map::LogBusKind::Eth),
            channel_count: self.can_count(),
            cluster_ids: self.cluster_ids_view(),
            model_frames: self.model_frames_view(),
            e2e: self.e2e_tracks.clone(),
            fr_e2e: self.fr_e2e_tracks.clone(),
            eth_e2e: self.eth_e2e_tracks.clone(),
            fr_model_frames: self.fr_model_frames_view(),
            eth_model_frames: self.eth_model_frames_view(),
            expanded_signals: self.expanded_signals_view(),
            laps: self.laps,
            hw_refresh: self.hw.refresh_seq,
            bus_counter: self.bus_counter,
            sim_t_us: self.sim_t_us,
            spec: self.spec.clone(),
            bus_loads: Arc::clone(&self.published_loads),
            #[cfg(test)]
            fr_loads: Arc::clone(&self.published_fr_loads),
            nodes: Arc::clone(&self.published_nodes),
            blocks: (*self.published_blocks).clone(),
            #[cfg(test)]
            emitted: self.emitted_streams.clone(),
            sysvars: self
                .sysvars
                .iter()
                .map(|(d, v)| SysVarView {
                    def: d.clone(),
                    value: *v,
                })
                .collect(),
            write: Arc::clone(&self.published_write),
            hw: self
                .hw
                .buses
                .iter()
                .map(|(&bus, bh)| HwBusView {
                    bus,
                    adapter: bh.adapter,
                    driver: bh.driver,
                    kbps: bh.kbps,
                    can_tx: bh.can_tx,
                    fd: self.hw.fd(bus),
                    tx_fail: self.hw.tx_fail(bus),
                })
                .collect(),
            real_bus: self.hw.live,
            triggers: self.triggers.clone(),
            trigger_fires: self.trigger_fires.iter().cloned().collect(),
            derived_errors: self
                .derived
                .iter()
                .filter_map(|runtime| {
                    runtime
                        .error
                        .clone()
                        .map(|error| (runtime.def.id.clone(), error))
                })
                .collect(),
            last_record: self.recorder.last_record.clone(),
            last_eth_record: self.recorder.last_eth_record.clone(),
            channels: (0..self.can_count())
                .map(|index| {
                    let (bitrate_kbps, fd_data_kbps) = self.can_bitrates(index);
                    ChannelView {
                        name: self.can_name(index),
                        dbc_paths: self.can_paths(index),
                        bitrate_kbps,
                        fd_data_kbps,
                        node_roles: self.can_node_roles(index),
                        node_ecus: self
                            .model_comms
                            .get(index)
                            .and_then(Option::as_ref)
                            .map(|comm| {
                                comm.ecus
                                    .values()
                                    .map(|ecu| (ecu.name.clone(), ecu.id.0))
                                    .collect()
                            })
                            .unwrap_or_default(),
                        dbc: self.model_dbs.get(index).and_then(|db| db.clone()),
                    }
                })
                .collect(),
            mode: self.mode,
            run_mode: self.run_mode,
            measuring: self.measuring,
            trace_paused: self.trace_paused,
            recording: self.recorder.recording,
            status: None,
            aggs: self.aggs.values().copied().collect(),
            fr_aggs: {
                let mut rows: Vec<crate::aggregate::FrFrameAgg> =
                    self.fr_aggs.values().cloned().collect();
                // Frame order within a slot, so two occupants of one slot keep
                // the same positions between publishes: a HashMap's iteration
                // order alone would shuffle the rows under the reader's eyes.
                rows.sort_by(|a, b| {
                    (a.bus, a.slot, a.name.as_deref().unwrap_or("")).cmp(&(
                        b.bus,
                        b.slot,
                        b.name.as_deref().unwrap_or(""),
                    ))
                });
                rows
            },
            eth_aggs: {
                let mut rows: Vec<crate::aggregate::EthAgg> =
                    self.eth_aggs.values().cloned().collect();
                rows.sort_by(|a, b| {
                    (
                        a.key.bus,
                        a.key.vlan,
                        a.key.src,
                        a.key.dst,
                        a.key.ethertype,
                        a.key.len,
                    )
                        .cmp(&(
                            b.key.bus,
                            b.key.vlan,
                            b.key.src,
                            b.key.dst,
                            b.key.ethertype,
                            b.key.len,
                        ))
                });
                rows
            },
            subs: self
                .subs
                .iter()
                .map(|(key, s)| SubView {
                    key: key.clone(),
                    latest: s.latest,
                    last_raw: s.last_raw,
                    unit: s.unit.clone(),
                    label: s.label.clone(),
                    type_tag: s.type_tag.clone(),
                    min: s.min,
                    max: s.max,
                    avg: s.avg,
                    history: s.published.clone(),
                })
                .collect(),
            markers: self.markers.clone(),
            tx: self
                .tx_list
                .iter()
                .map(|t| {
                    // 显示值读发射时存储的载荷（随消息周期跳变）；
                    // 从未发送过的条目回退显示 base 字节。
                    let (sent_data, sent_len) = if t.last_len > 0 {
                        (t.last_sent, t.last_len)
                    } else {
                        (t.data, t.len)
                    };
                    TxView {
                        channel: t.channel,
                        id: t.id,
                        name: t.name.clone(),
                        node: t.node.clone(),
                        active: t.active,
                        gate_open: t.node.is_empty()
                            || self.can_role_of(t.channel as usize, &t.node)
                                == crate::app::NodeRole::Simulated,
                        fd: t.flags.contains(crate::can::frame::FrameFlags::FD),
                        brs: t.flags.contains(crate::can::frame::FrameFlags::BRS),
                        cycle_us: t.cycle_us,
                        data_text: t.data_text.clone(),
                        sent_text: crate::generator::hex_text(&sent_data, sent_len),
                        srcs: t.srcs.clone(),
                        muted: matches!(self.mode, Mode::Replay)
                            && self.replay_ids.contains(&(t.channel, t.id)),
                    }
                })
                .collect(),
            fr_tx: self
                .fr_tx_list
                .iter()
                .map(|t| {
                    // The row reads back what went out; before the first
                    // emission, what the base bytes say.
                    let sent = if t.last_sent.is_empty() {
                        &t.data
                    } else {
                        &t.last_sent
                    };
                    FrTxView {
                        bus: t.bus,
                        slot: t.slot,
                        name: t.name.clone(),
                        node: t.node.clone(),
                        active: t.active,
                        cycle_us: t.cycle_us,
                        data_text: t.data_text.clone(),
                        sent_text: crate::generator::hex_of(sent),
                        srcs: t.srcs.clone(),
                        muted: matches!(self.mode, Mode::Replay)
                            && self.replay_fr_ids.contains(&(t.bus, t.slot)),
                        undescribed: !self
                            .fr_dbs
                            .get(&t.bus)
                            .is_some_and(|db| db.frame_ix_of_slot(t.slot).is_some()),
                    }
                })
                .collect(),
        }
    }
}
