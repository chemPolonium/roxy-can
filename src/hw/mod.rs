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
pub mod vector;

use crate::can::frame::CanFrame;
use std::collections::HashMap;

/// Which vendor driver an attachment talks through.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HwDriver {
    Kvaser,
    Vector,
}

impl HwDriver {
    /// Short tag for combo labels: `[K] ch0` vs `[V] ch0`.
    pub fn tag(self) -> &'static str {
        match self {
            HwDriver::Kvaser => "K",
            HwDriver::Vector => "V",
        }
    }

    /// The persisted word (profile `[[hw]]` driver field). Both the long
    /// name and the tag parse, so a hand-written profile accepts either.
    pub fn word(self) -> &'static str {
        match self {
            HwDriver::Kvaser => "Kvaser",
            HwDriver::Vector => "Vector",
        }
    }

    pub fn parse(word: &str) -> Option<Self> {
        match word {
            "Kvaser" | "K" => Some(HwDriver::Kvaser),
            "Vector" | "V" => Some(HwDriver::Vector),
            _ => None,
        }
    }
}

/// One discoverable channel from any supported driver, flattened for
/// the Buses window's attachment combo.
#[derive(Clone, Debug, PartialEq)]
pub struct AnyChannelInfo {
    pub driver: HwDriver,
    pub index: i32,
    pub name: String,
}

/// Enumerates channels across every supported driver. A driver whose
/// runtime is missing simply contributes no entries; `Err` only when
/// no driver is available at all.
pub fn enumerate_all() -> Result<Vec<AnyChannelInfo>, String> {
    let mut out = Vec::new();
    let mut errors: Vec<String> = Vec::new();
    match kvaser::enumerate() {
        Ok(list) => {
            for c in list {
                out.push(AnyChannelInfo {
                    driver: HwDriver::Kvaser,
                    index: c.index,
                    name: c.name,
                });
            }
        }
        Err(e) => errors.push(format!("Kvaser: {e}")),
    }
    match vector::enumerate() {
        Ok(list) => {
            for c in list {
                // A FlexRay port of a VN7640 must not surface as a CAN
                // attachment. Channels that report no capability bits at
                // all (every virtual channel so far) keep the legacy
                // behaviour and stay in the list.
                if c.flexray && !c.can {
                    continue;
                }
                out.push(AnyChannelInfo {
                    driver: HwDriver::Vector,
                    index: c.index,
                    name: c.name,
                });
            }
        }
        Err(e) => errors.push(format!("Vector: {e}")),
    }
    if out.is_empty() && !errors.is_empty() {
        Err(errors.join("；"))
    } else {
        Ok(out)
    }
}

/// One bus's attachment: which adapter port it talks through.
#[derive(Debug)]
pub struct BusHardware {
    /// Human-facing adapter identity (driver-specific channel index).
    pub adapter: i32,
    pub driver: HwDriver,
    pub kbps: u32,
    /// Whether the port holds init access (can transmit). A receive-only
    /// attachment (another program holds the channel) still feeds RX into
    /// every view, but node switches cannot direct traffic to the wire.
    pub can_tx: bool,
    port: HwPort,
}

/// The port kinds. The mock exists for headless tests -- the gating
/// logic around the hardware must be provable without an adapter.
#[derive(Debug)]
pub enum HwPort {
    Kvaser(kvaser::KvaserChannel),
    Vector(vector::VectorChannel),
    #[cfg(test)]
    Mock(MockPort),
}

impl HwPort {
    pub fn write_frame(&self, f: &CanFrame) -> Result<(), String> {
        match self {
            HwPort::Kvaser(ch) => ch.write_frame(f),
            HwPort::Vector(ch) => ch.write_frame(f),
            #[cfg(test)]
            HwPort::Mock(m) => m.write(f),
        }
    }

    pub fn try_read(&mut self) -> Option<CanFrame> {
        match self {
            HwPort::Kvaser(ch) => ch.try_read(),
            HwPort::Vector(ch) => ch.try_read(),
            #[cfg(test)]
            HwPort::Mock(m) => m.try_read(),
        }
    }

    /// Whether FD data-phase params are active on this port.
    pub fn fd(&self) -> bool {
        match self {
            HwPort::Kvaser(ch) => ch.fd,
            HwPort::Vector(ch) => ch.fd,
            #[cfg(test)]
            HwPort::Mock(_) => false,
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

/// The FlexRay receive ports a watch can drain. Mirrors [`HwPort`]: one
/// adapter per kind, and a test double so the plumbing around a watch --
/// which bus it feeds, what it does to its siblings -- is provable without
/// a FlexRay-capable interface in the machine.
#[derive(Debug)]
pub enum FrPort {
    Vector(vector::flexray::FlexRayChannel),
    #[cfg(test)]
    Mock(MockFrPort),
}

impl FrPort {
    fn try_read(&mut self) -> Option<vector::flexray::FrFrame> {
        match self {
            FrPort::Vector(ch) => ch.try_read(),
            #[cfg(test)]
            FrPort::Mock(m) => m.try_read(),
        }
    }
}

/// The receive queue [`Hardware::attach_fr_mock`] hands back to a test.
#[cfg(test)]
pub type MockFrHandles = std::sync::Arc<
    std::sync::Mutex<std::collections::VecDeque<vector::flexray::FrFrame>>,
>;

/// Test double for a FlexRay port: hands back whatever the test queued.
#[cfg(test)]
#[derive(Debug)]
pub struct MockFrPort {
    pub incoming: MockFrHandles,
}

#[cfg(test)]
impl MockFrPort {
    fn try_read(&mut self) -> Option<vector::flexray::FrFrame> {
        self.incoming.lock().expect("mock lock").pop_front()
    }
}

/// All hardware attachments plus the CANoe-style bus mode.
#[derive(Debug)]
pub struct Hardware {
    /// Bus index → attachment. One adapter per bus.
    pub buses: HashMap<u8, BusHardware>,
    /// FlexRay bus index → its RX-only watch. Several clusters can be watched
    /// at once, and a slot number only means something inside its own one, so
    /// the watch -- and the description it was configured from -- belongs to
    /// the bus. Drained alongside the CAN adapters; rows land in the Trace
    /// window's FR section and nowhere else.
    pub fr_watches: HashMap<u8, FrWatch>,
    /// CANoe-style bus mode. `false` (Simulated) parks every attachment:
    /// received frames are discarded and wire writes are suppressed, while
    /// the attachments stay configured for the moment the mode flips
    /// back. `true` (Real bus) connects the attachments to their
    /// adapters -- simulated traffic for nodes in the Simulated role
    /// goes onto the wire, and real traffic arrives through the same
    /// attachment. Attaching is an explicit act, so the default keeps
    /// the attachment the user just made connected.
    pub live: bool,
}

/// The FlexRay RX-only watch: a Vector channel opened with a
/// FIBEX-parsed cluster configuration, identified by the Vector channel
/// index and the description file it was configured from.
#[derive(Debug)]
pub struct FrWatch {
    /// The FlexRay bus this watch feeds. One watch per bus index, so rows
    /// arriving from different ports stay separable downstream.
    pub bus: u8,
    pub channel_index: i32,
    pub fibex_path: String,
    pub port: FrPort,
}

impl Hardware {
    /// A fresh hardware map starts wire-connected: attaching is an
    /// explicit user act and they expect the adapter they just attached
    /// to be on the wire.
    pub fn new() -> Self {
        Self {
            buses: Default::default(),
            fr_watches: Default::default(),
            live: true,
        }
    }
}

impl Hardware {
    /// Whether a bus has any hardware attachment.
    #[cfg(test)]
    pub fn is_attached(&self, bus: u8) -> bool {
        self.buses.contains_key(&bus)
    }

    /// Whether the bus's attachment has FD data-phase params applied.
    pub fn fd(&self, bus: u8) -> bool {
        self.buses
            .get(&bus)
            .is_some_and(|bh| bh.port.fd())
    }

    /// Attaches a port to a bus, replacing any previous attachment.
    pub fn attach(&mut self, bus: u8, driver: HwDriver, adapter: i32, kbps: u32, can_tx: bool, port: HwPort) {
        self.buses.insert(
            bus,
            BusHardware {
                adapter,
                driver,
                kbps,
                can_tx,
                port,
            },
        );
    }

    pub fn detach(&mut self, bus: u8) {
        self.buses.remove(&bus);
    }

    /// Attaches the FlexRay RX-only watch from the cluster description the
    /// frontend already parsed. Reading and parsing the file again here doubled
    /// the load cost of a big FIBEX -- and `read_to_string` refused the
    /// GBK-encoded exports the frontend's tolerant read accepts, so the watch
    /// could not be opened at all for those.
    pub fn attach_fr(
        &mut self,
        bus: u8,
        channel_index: i32,
        fibex_path: &str,
        db: &crate::fr_db::FrDb,
    ) -> Result<(), String> {
        let port =
            crate::hw::vector::flexray::FlexRayChannel::open_rx_with_db(channel_index, db)?;
        self.fr_watches.insert(
            bus,
            FrWatch {
                bus,
                channel_index,
                fibex_path: fibex_path.to_string(),
                port: FrPort::Vector(port),
            },
        );
        Ok(())
    }

    /// Attaches a test FlexRay watch on a bus and returns the queue to feed.
    /// No driver call happens, which is the point: the multi-watch plumbing is
    /// testable on a machine with no FlexRay hardware.
    #[cfg(test)]
    pub fn attach_fr_mock(&mut self, bus: u8, channel_index: i32) -> MockFrHandles {
        let incoming = std::sync::Arc::new(std::sync::Mutex::new(
            std::collections::VecDeque::new(),
        ));
        self.fr_watches.insert(
            bus,
            FrWatch {
                bus,
                channel_index,
                fibex_path: String::new(),
                port: FrPort::Mock(MockFrPort {
                    incoming: incoming.clone(),
                }),
            },
        );
        incoming
    }

    /// Drops one bus's watch; its port closes with it. The other buses keep
    /// theirs.
    pub fn detach_fr(&mut self, bus: u8) {
        self.fr_watches.remove(&bus);
    }

    /// Drops the attachment of a bus being removed and shifts the
    /// survivors down -- same policy as the tx entries. The ports close
    /// with their bus.
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
    }

    /// Writes one frame out the bus's wire in Real bus mode. Everything
    /// the tool transmits for a Simulated node rides the same switch --
    /// CANoe's simulated/real bus distinction, not a per-node dial.
    /// 只收句柄写入失败时静默降级——帧已留在内部总线，视图不丢。
    /// Simulated 模式（`!live`）不写线。
    pub fn write_if_live(&mut self, bus: u8, f: &CanFrame) {
        if !self.live {
            return;
        }
        if let Some(bh) = self.buses.get_mut(&bus) {
            bh.port.write_frame(f).ok();
        }
    }

    /// Drains every attached adapter's receive queue into `out`, stamped
    /// against the sim clock and tagged with the bus they are mapped to.
    /// Simulated 模式（`!live`）照常抽干队列（防驱动缓冲塞满旧帧）但
    /// 把帧丢弃——不上内部总线。
    pub fn poll_rx(&mut self, sim_t_us: u64, out: &mut Vec<CanFrame>) {
        for (&bus, bh) in self.buses.iter_mut() {
            while let Some(mut f) = bh.port.try_read() {
                if !self.live {
                    continue;
                }
                f.t_us = sim_t_us;
                f.channel = bus;
                out.push(f);
            }
        }
    }

    /// Drains every FlexRay watch's receive queue into `out`, stamped against
    /// `sim_t_us` exactly as `poll_rx` stamps a CAN frame -- the row a trigger
    /// judges and the row the trace shows must carry the clock the arrival
    /// happened on. Discards when parked (`!live`) like the CAN side: the queue
    /// must not back up with stale frames, but parked traffic stays off the
    /// tool's timeline. Buses are drained in index order so a run is
    /// reproducible. The reception channel (A/B) reads as unknown until the
    /// event offset is probe-verified.
    pub fn poll_fr(&mut self, sim_t_us: u64, out: &mut Vec<crate::trace::FrRow>) {
        let mut buses: Vec<u8> = self.fr_watches.keys().copied().collect();
        buses.sort();
        for bus in buses {
            let Some(w) = self.fr_watches.get_mut(&bus) else {
                continue;
            };
            while let Some(f) = w.port.try_read() {
                if !self.live {
                    continue;
                }
                out.push(crate::trace::FrRow {
                    bus: w.bus,
                    t_us: sim_t_us,
                    ab: 2,
                    slot: f.slot,
                    cycle: f.cycle,
                    payload: f.payload,
                    header_crc: f.header_crc,
                    flags: f.flags,
                    name: None,
                });
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
            HwDriver::Kvaser,
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
