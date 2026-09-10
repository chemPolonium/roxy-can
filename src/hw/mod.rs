//! Hardware attachments: real CAN adapters (Kvaser today) bound to
//! virtual buses. The model is deliberately manual, per the acceptance
//! feedback -- the tool never decides on its own whether a node's
//! simulated traffic belongs on the wire:
//!
//! - **RX**: everything an attached adapter receives is ingested like
//!   any bus traffic -- a real node's frames arrive this way.
//! - **TX**: nodes carry a per-node "经硬件发送" switch (off by
//!   default). With it on, that node's generator frames go to the wire
//!   *and* stay on the internal bus so every view sees them; with it
//!   off they are virtual-only. A real node that must not be doubled is
//!   declared Monitor (our traffic stays off) -- the user's call.
//!
//! The mapping is session state; the Profile overlay (phase 5) will
//! carry it into projects.

pub mod kvaser;

use crate::can::frame::CanFrame;
use std::collections::{BTreeSet, HashMap};

/// One bus's attachment: which adapter port it talks through.
#[derive(Debug)]
pub struct BusHardware {
    /// Human-facing adapter identity (Kvaser channel index for now).
    pub adapter: i32,
    pub kbps: u32,
    /// Whether the port holds init access (can transmit). A receive-only
    /// attachment (another program holds the channel) still feeds RX into
    /// every view, but node switches cannot direct traffic to the wire.
    pub can_tx: bool,
    port: HwPort,
}

/// The two port kinds. The mock exists for headless tests -- the gating
/// logic around the hardware must be provable without an adapter.
#[derive(Debug)]
pub enum HwPort {
    Kvaser(kvaser::KvaserChannel),
    #[cfg(test)]
    Mock(MockPort),
}

impl HwPort {
    pub fn write_frame(&self, f: &CanFrame) -> Result<(), String> {
        match self {
            HwPort::Kvaser(ch) => ch.write_frame(f),
            #[cfg(test)]
            HwPort::Mock(m) => m.write(f),
        }
    }

    pub fn try_read(&mut self) -> Option<CanFrame> {
        match self {
            HwPort::Kvaser(ch) => ch.try_read(),
            #[cfg(test)]
            HwPort::Mock(m) => m.try_read(),
        }
    }
}

/// Test double: records everything written, hands back whatever the test
/// queued as received. `Mutex` keeps the shared state `Send` (the core
/// may live on a thread).
#[cfg(test)]
#[derive(Debug, Default)]
pub struct MockPort {
    pub written: std::sync::Arc<std::sync::Mutex<Vec<CanFrame>>>,
    pub incoming: std::sync::Arc<std::sync::Mutex<std::collections::VecDeque<CanFrame>>>,
}

/// The shared handles [`Hardware::attach_mock`] hands back to a test:
/// everything written on the wire, and the receive queue to feed.
#[cfg(test)]
pub type MockHandles = (
    std::sync::Arc<std::sync::Mutex<Vec<CanFrame>>>,
    std::sync::Arc<std::sync::Mutex<std::collections::VecDeque<CanFrame>>>,
);

#[cfg(test)]
impl MockPort {
    pub fn write(&self, f: &CanFrame) -> Result<(), String> {
        self.written.lock().expect("mock lock").push(*f);
        Ok(())
    }

    pub fn try_read(&mut self) -> Option<CanFrame> {
        self.incoming.lock().expect("mock lock").pop_front()
    }
}

/// All hardware attachments plus the per-node wire-egress switches.
#[derive(Debug, Default)]
pub struct Hardware {
    /// Bus index → attachment. One adapter per bus.
    pub buses: HashMap<u8, BusHardware>,
    /// (bus, DBC node name) pairs whose generator frames also go on the
    /// wire. `BTreeSet` keeps the snapshot listing stable.
    pub node_tx: BTreeSet<(u8, String)>,
}

impl Hardware {
    /// Whether a bus has any hardware attachment.
    #[cfg(test)]
    pub fn is_attached(&self, bus: u8) -> bool {
        self.buses.contains_key(&bus)
    }

    pub fn node_sends_via_hw(&self, bus: u8, node: &str) -> bool {
        self.node_tx.contains(&(bus, node.to_string()))
    }

    /// Attaches a port to a bus, replacing any previous attachment.
    pub fn attach(&mut self, bus: u8, adapter: i32, kbps: u32, can_tx: bool, port: HwPort) {
        self.buses.insert(
            bus,
            BusHardware {
                adapter,
                kbps,
                can_tx,
                port,
            },
        );
    }

    pub fn detach(&mut self, bus: u8) {
        self.buses.remove(&bus);
        // A bus without hardware has no wire to send on: the per-node
        // switches become meaningless and are cleared with it.
        self.node_tx.retain(|(b, _)| *b != bus);
    }

    /// Drops the attachment (and switches) of a bus being removed and
    /// shifts the survivors down -- same policy as the tx entries. The
    /// ports close with their bus.
    pub fn remove_bus(&mut self, ch: usize) {
        self.buses.retain(|&b, _| b as usize != ch);
        self.buses = self
            .buses
            .drain()
            .map(|(mut b, bh)| {
                if b as usize > ch {
                    b -= 1;
                }
                (b, bh)
            })
            .collect();
        self.node_tx.retain(|(b, _)| *b as usize != ch);
        self.node_tx = self
            .node_tx
            .iter()
            .map(|&(b, ref n)| {
                let nb = if b as usize > ch { b - 1 } else { b };
                (nb, n.clone())
            })
            .collect();
    }

    pub fn set_node_tx(&mut self, bus: u8, node: &str, on: bool) {
        if on {
            if self.buses.contains_key(&bus) {
                self.node_tx.insert((bus, node.to_string()));
            }
        } else {
            self.node_tx.remove(&(bus, node.to_string()));
        }
    }

    /// Writes one frame out the bus's wire, if a node switch directs it
    /// there. Kvaser 虚拟通道的只收句柄同样能发车；物理适配器的只收
    /// 句柄写入失败时静默降级——帧已留在内部总线，视图不丢。
    pub fn write_if_directed(&mut self, bus: u8, node: &str, f: &CanFrame) {
        if node.is_empty() || !self.node_sends_via_hw(bus, node) {
            return;
        }
        if let Some(bh) = self.buses.get_mut(&bus) {
            bh.port.write_frame(f).ok();
        }
    }

    /// Drains every attached adapter's receive queue into `out`, stamped
    /// against the sim clock and tagged with the bus they are mapped to.
    pub fn poll_rx(&mut self, sim_t_us: u64, out: &mut Vec<CanFrame>) {
        for (&bus, bh) in self.buses.iter_mut() {
            while let Some(mut f) = bh.port.try_read() {
                f.t_us = sim_t_us;
                f.channel = bus;
                out.push(f);
            }
        }
    }

    /// Attaches a mock port to a bus and returns shared handles to what
    /// was written and what is queued as received. Tests only.
    #[cfg(test)]
    pub fn attach_mock(
        &mut self,
        bus: u8,
    ) -> MockHandles {
        let written = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let incoming = std::sync::Arc::new(std::sync::Mutex::new(
            std::collections::VecDeque::new(),
        ));
        self.attach(
            bus,
            -1,
            500,
            true,
            HwPort::Mock(MockPort {
                written: written.clone(),
                incoming: incoming.clone(),
            }),
        );
        (written, incoming)
    }
}
