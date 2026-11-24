//! SOME-IP service adoption, replies and Ethernet sends.

use super::*;

impl BusCore {
    /// Mirrors the ARXML service catalog into each Ethernet cluster's unified
    /// `Comm`, so service/method/event/instance have the same stable identity
    /// as frames, PDUs and ECUs. Field layouts stay in the catalog.
    pub(super) fn adopt_someip_services(
        &mut self,
        catalog: crate::eth::arxml_service::ServiceCatalog,
    ) {
        let services = catalog.services.clone();
        for (bus, runtime) in &mut self.eth_models {
            let mut comm = (*runtime.comm).clone();
            comm.services.clear();
            let cluster = comm
                .clusters
                .keys()
                .next()
                .copied()
                .unwrap_or(crate::model::ClusterId(0));
            for service in &services {
                if !service.buses.contains(bus) {
                    continue;
                }
                comm.services.push(crate::model::Service {
                    id: crate::model::ServiceId(u32::from(service.id)),
                    cluster,
                    name: service.name.clone(),
                    major: service.major,
                    methods: service
                        .methods
                        .iter()
                        .map(|method| crate::model::ServiceMember {
                            id: method.id,
                            name: method.name.clone(),
                        })
                        .collect(),
                    events: service
                        .events
                        .iter()
                        .map(|event| crate::model::ServiceMember {
                            id: event.id,
                            name: event.name.clone(),
                        })
                        .collect(),
                    instances: service
                        .instances
                        .iter()
                        .map(|instance| crate::model::ServiceInstance {
                            id: instance.instance,
                            major: instance.major,
                            minor: instance.minor,
                            eventgroups: instance.eventgroups.clone(),
                        })
                        .collect(),
                });
            }
            *runtime = std::sync::Arc::new(ProtocolRuntime {
                comm: std::sync::Arc::new(comm),
                plans: runtime.plans.clone(),
                db: runtime.db.clone(),
            });
        }
        self.someip_services = catalog;
    }

    pub(super) fn note_someip_request(
        &mut self,
        bus: u8,
        service: u16,
        method: u16,
        client: u16,
        session: u16,
        message_type: u8,
    ) {
        if message_type != 0x00 {
            return;
        }
        self.someip_pending.push(SomeIpPending {
            bus,
            service,
            method,
            client,
            session,
            deadline_us: self.sim_t_us.saturating_add(5_000_000),
        });
    }

    pub(super) fn auto_reply_someip_method(&mut self, event: &crate::eth::someip::SomeIpEvent) {
        if event.message_type != 0x00 {
            return;
        }
        if !self
            .someip_offers
            .keys()
            .any(|(bus, service, _)| *bus == event.bus && *service == event.service)
        {
            return;
        }
        let configured = self
            .someip_replies
            .get(&(event.service, event.method))
            .cloned();
        let (configured_return, payload) = if let Some((return_code, payload)) = configured {
            (return_code, payload)
        } else {
            let Some(service) = self.someip_services.service(event.service) else {
                return;
            };
            let Some(method) = service
                .methods
                .iter()
                .find(|method| method.id == event.method)
            else {
                return;
            };
            let Ok(payload) =
                crate::eth::arxml_service::encode_values(&method.output, &Default::default())
            else {
                return;
            };
            (0, payload)
        };
        let dst = event
            .src
            .parse::<std::net::Ipv4Addr>()
            .map(|ip| ip.octets())
            .unwrap_or([10, 0, 0, 2]);
        if event.transport == "TCP" {
            if let Err(error) = send_someip_tcp_rc(
                &event.src,
                event.src_port.max(1),
                event.service,
                event.method,
                event.client,
                event.session,
                0x80,
                configured_return,
                &payload,
            ) {
                self.write_push(
                    WriteKind::Warning,
                    format!("SOME/IP TCP auto reply: {error}"),
                );
            } else {
                self.write_push(
                    WriteKind::Info,
                    format!(
                        "SOME/IP TCP auto reply 0x{:04X}.0x{:04X} session {}",
                        event.service, event.method, event.session
                    ),
                );
            }
            return;
        }
        let frame = crate::eth::someip::build_udp_frame_to_rc(
            self.sim_t_us,
            event.bus,
            [10, 0, 0, 1],
            dst,
            30501,
            event.src_port.max(1),
            event.service,
            event.method,
            event.client,
            event.session,
            0x80,
            configured_return,
            &payload,
        );
        self.someip_send_guard = true;
        self.emit_eth(frame).ok();
        self.someip_send_guard = false;
        self.write_push(
            WriteKind::Info,
            format!(
                "SOME/IP auto reply 0x{:04X}.0x{:04X} session {}",
                event.service, event.method, event.session
            ),
        );
    }

    pub(super) fn ack_someip_subscribes(&mut self, event: &crate::eth::someip::SomeIpEvent) {
        let Some(entries) = &event.sd else {
            return;
        };
        for entry in entries {
            if entry.kind != 0x06 {
                continue;
            }
            let Some(eventgroup) = entry.eventgroup else {
                continue;
            };
            if entry.ttl == 0 {
                self.someip_subscriptions.retain(|subscription| {
                    !(subscription.bus == event.bus
                        && subscription.service == entry.service
                        && subscription.instance == entry.instance
                        && subscription.eventgroup == eventgroup)
                });
                continue;
            }
            if !self
                .someip_offers
                .contains_key(&(event.bus, entry.service, entry.instance))
            {
                continue;
            }
            let (ip, port) = entry
                .endpoints
                .first()
                .map(|endpoint| (endpoint.address.clone(), endpoint.port))
                .unwrap_or_else(|| ("10.0.0.2".to_string(), 30490));
            if let Some(existing) = self.someip_subscriptions.iter_mut().find(|subscription| {
                subscription.bus == event.bus
                    && subscription.service == entry.service
                    && subscription.instance == entry.instance
                    && subscription.eventgroup == eventgroup
            }) {
                existing.ip = ip;
                existing.port = port;
                existing.ttl = entry.ttl;
            } else {
                self.someip_subscriptions.push(SomeIpSubscription {
                    bus: event.bus,
                    service: entry.service,
                    instance: entry.instance,
                    eventgroup,
                    ip,
                    port,
                    ttl: entry.ttl,
                });
            }
            let session = self.next_someip_session(event.bus, 0xFFFF, 0x8100, 0, 0);
            let frame = crate::eth::someip::build_sd_subscribe_ack_frame(
                self.sim_t_us,
                event.bus,
                entry.service,
                entry.instance,
                entry.major,
                eventgroup,
                3,
                [10, 0, 0, 1],
                30490,
                session,
            );
            self.someip_send_guard = true;
            self.emit_eth(frame).ok();
            self.someip_send_guard = false;
        }
    }

    pub(super) fn expire_someip_requests(&mut self, now_us: u64) {
        let mut keep = Vec::new();
        let mut expired = Vec::new();
        for request in self.someip_pending.drain(..) {
            if now_us <= request.deadline_us {
                keep.push(request);
            } else {
                expired.push(request);
            }
        }
        self.someip_pending = keep;
        for request in expired {
            self.write_push(
                WriteKind::Warning,
                format!(
                    "SOME/IP request 0x{:04X}.0x{:04X} session {} timed out",
                    request.service, request.method, request.session
                ),
            );
        }
    }

    pub(super) fn next_someip_session(
        &mut self,
        bus: u8,
        service: u16,
        method: u16,
        client: u16,
        requested: u16,
    ) -> u16 {
        if requested != 0 {
            return requested;
        }
        let entry = self
            .someip_sessions
            .entry((bus, service, method, client))
            .or_insert(0);
        *entry = entry.wrapping_add(1).max(1);
        *entry
    }

    pub(super) fn encode_someip_payload(
        &self,
        service: u16,
        method: u16,
        message_type: u8,
        values: &std::collections::BTreeMap<String, f64>,
    ) -> Result<Vec<u8>, String> {
        self.encode_someip_payload_with_text(
            service,
            method,
            message_type,
            values,
            &std::collections::BTreeMap::new(),
        )
    }

    pub(super) fn encode_someip_payload_with_text(
        &self,
        service: u16,
        method: u16,
        message_type: u8,
        values: &std::collections::BTreeMap<String, f64>,
        texts: &std::collections::BTreeMap<String, String>,
    ) -> Result<Vec<u8>, String> {
        let definition = self
            .someip_services
            .service(service)
            .ok_or_else(|| format!("no ARXML service 0x{service:04X}"))?;
        let fields = definition
            .events
            .iter()
            .find(|entry| entry.id == method)
            .map(|entry| entry.fields.as_slice())
            .or_else(|| {
                definition
                    .methods
                    .iter()
                    .find(|entry| entry.id == method)
                    .map(|entry| {
                        if message_type == 0x80 {
                            entry.output.as_slice()
                        } else {
                            entry.input.as_slice()
                        }
                    })
            })
            .ok_or_else(|| format!("no ARXML fields for 0x{service:04X}.0x{method:04X}"))?;
        crate::eth::arxml_service::encode_values_with_text(fields, values, texts)
    }

    /// Injects one whole Ethernet frame through the matrix definition. The
    /// trigger and the Panel button share this: name gives VLAN/EtherType/
    /// length, bytes are the payload.
    /// Delivers one locally built Ethernet frame: wire first when a live port
    /// is attached, otherwise session-only injection.
    pub(super) fn apply_someip_replies(&mut self, replies: Vec<crate::script::SomeIpReply>) {
        for reply in replies {
            self.someip_replies.insert(
                (reply.service, reply.method),
                (reply.return_code, reply.payload),
            );
        }
    }

    pub(super) fn apply_script_someip_offers(&mut self, offers: Vec<crate::script::SomeIpOffer>) {
        for offer in offers {
            let key = (offer.bus, offer.service, offer.instance);
            if offer.ttl == 0 {
                self.someip_offers.remove(&key);
            } else {
                self.someip_offers.insert(key, offer.ttl);
            }
            let session = self.next_someip_session(offer.bus, 0xFFFF, 0x8100, 0, 0);
            let frame = crate::eth::someip::build_sd_offer_frame(
                self.sim_t_us,
                offer.bus,
                offer.service,
                offer.instance,
                offer.major,
                offer.minor,
                offer.ttl,
                [10, 0, 0, 1],
                30490,
                session,
            );
            self.emit_eth(frame).ok();
        }
    }

    pub(super) fn apply_script_someip_tcp(&mut self, sends: Vec<crate::script::SomeIpTcpSend>) {
        use std::io::Write;
        for send in sends {
            let session =
                self.next_someip_session(0, send.service, send.method, send.client, send.session);
            let message = crate::eth::someip::build_message(
                send.service,
                send.method,
                send.client,
                session,
                send.message_type,
                &send.payload,
            );
            let address = format!("{}:{}", send.host, send.port);
            match std::net::TcpStream::connect(&address) {
                Ok(mut stream) => {
                    let _ = stream.set_write_timeout(Some(std::time::Duration::from_millis(500)));
                    if let Err(error) = stream.write_all(&message) {
                        self.write_push(
                            WriteKind::Warning,
                            format!("SOME/IP TCP {address}: {error}"),
                        );
                    }
                }
                Err(error) => self.write_push(
                    WriteKind::Warning,
                    format!("SOME/IP TCP {address}: {error}"),
                ),
            }
        }
    }

    pub(super) fn apply_script_someip(&mut self, sends: Vec<crate::script::SomeIpSend>) {
        for send in sends {
            let session = self.next_someip_session(
                send.bus,
                send.service,
                send.method,
                send.client,
                send.session,
            );
            let frame = crate::eth::someip::build_udp_frame(
                self.sim_t_us,
                send.bus,
                send.service,
                send.method,
                send.client,
                session,
                send.message_type,
                &send.payload,
            );
            self.someip_send_guard = true;
            self.emit_eth(frame).ok();
            self.someip_send_guard = false;
            if send.message_type == 0x02 {
                let subscribers: Vec<SomeIpSubscription> = self
                    .someip_subscriptions
                    .iter()
                    .filter(|subscription| {
                        subscription.bus == send.bus && subscription.service == send.service
                    })
                    .cloned()
                    .collect();
                for subscription in subscribers {
                    let dst = subscription
                        .ip
                        .parse::<std::net::Ipv4Addr>()
                        .map(|ip| ip.octets())
                        .unwrap_or([10, 0, 0, 2]);
                    let reply = crate::eth::someip::build_udp_frame_to(
                        self.sim_t_us,
                        send.bus,
                        [10, 0, 0, 1],
                        dst,
                        30501,
                        subscription.port,
                        send.service,
                        send.method,
                        send.client,
                        session,
                        send.message_type,
                        &send.payload,
                    );
                    self.someip_send_guard = true;
                    self.emit_eth(reply).ok();
                    self.someip_send_guard = false;
                }
            }
        }
    }

    pub(super) fn emit_eth(&mut self, frame: crate::eth::EthFrame) -> Result<(), String> {
        if self.hw.eth_device(frame.bus).is_some() {
            self.hw.send_eth(frame.bus, &frame)
        } else {
            self.ingest_eth(frame);
            Ok(())
        }
    }

    pub(super) fn send_eth_frame(
        &mut self,
        bus: u8,
        frame: &str,
        data: &[u8],
    ) -> Result<(), String> {
        if !self.measuring {
            return Err("sending a frame needs a running measurement".to_string());
        }
        let Some(db) = self.eth_dbs.get(&bus).cloned() else {
            return Err(format!("ETH{bus} has no matrix description"));
        };
        let Some(def) = db.frames.iter().find(|entry| entry.name == frame) else {
            return Err(format!("unknown Ethernet frame \"{frame}\""));
        };
        let expected = if self.eth_length_includes_header {
            def.length.saturating_sub(14)
        } else {
            def.length
        };
        if data.len() != expected {
            return Err(format!(
                "frame \"{frame}\" carries {expected} payload bytes"
            ));
        }
        let injected = crate::eth::EthFrame::session(
            self.sim_t_us,
            bus,
            def.vlan,
            def.ethertype_hint
                .unwrap_or(crate::eth::EthFrame::SESSION_ETHERTYPE),
            data.to_vec(),
        );
        self.ingest_eth(injected);
        Ok(())
    }

    pub(super) fn send_one_shot(
        &mut self,
        ch: u8,
        id: u32,
        at_us: u64,
        mirror: &HashMap<String, f64>,
    ) -> bool {
        let Some(i) = self
            .tx_list
            .iter()
            .position(|t| t.channel == ch && t.id == id)
        else {
            return false; // the generator row is gone: the rule idles
        };
        let (tch, tid, ext) = (
            self.tx_list[i].channel,
            self.tx_list[i].id,
            self.tx_list[i].extended,
        );
        // The reaction send goes through the same model encoder as the
        // periodic generator.
        let model = self
            .can_cluster_ids
            .get(tch as usize)
            .copied()
            .and_then(|cluster| {
                let comm = self.model_comms.get(tch as usize)?.as_ref()?.clone();
                let plans = self.model_plans.get(tch as usize)?.as_ref()?.clone();
                Some((cluster, comm, plans))
            });
        let (mut data, len, flags) = model
            .as_ref()
            .and_then(|(cluster, comm, plans)| {
                crate::generator::tx_payload_with_model(
                    *cluster,
                    comm,
                    plans,
                    &self.tx_list[i],
                    at_us,
                )
            })
            // A frame the model does not name is a raw send: its stored bytes
            // are the payload, and there is no signal to write into it.
            .unwrap_or((
                self.tx_list[i].data,
                self.tx_list[i].len,
                self.tx_list[i].flags,
            ));
        if !mirror.is_empty()
            && let Some((cluster, comm, plans)) = &model
        {
            let frame = comm.frames_in_cluster(*cluster).find(|frame| {
                matches!(
                    frame.address,
                    crate::model::FrameAddress::Can {
                        id,
                        extended,
                        ..
                    } if id == tid && extended == ext
                )
            });
            if let Some(frame) = frame
                && let Some(plan) = plans.get(frame.id)
            {
                let values: Vec<_> = comm
                    .signals_in_frame(frame)
                    .into_iter()
                    .filter_map(|signal| mirror.get(&signal.name).map(|value| (signal.id, *value)))
                    .collect();
                crate::model::encode_payload(comm, plan, &values, &mut data[..len as usize]);
            }
        }
        // 发送反应帧同样更新条目的最近发送载荷，显示与发送一致。
        self.tx_list[i].last_sent = data;
        self.tx_list[i].last_len = len;
        self.buf.push(CanFrame {
            t_us: at_us,
            channel: tch,
            id: tid,
            extended: ext,
            len,
            data,
            dir: crate::can::frame::Direction::Tx,
            flags,
        });
        true
    }
}
