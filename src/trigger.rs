//! Trigger conditions and their evaluation: the shared front end for
//! frame/signal/timeout rules and the reaction rules that send on an edge,
//! reusing the same matching and edge bookkeeping.
//!
//! A trigger watches incoming frames in the measurement loop and tracks
//! its condition's level; only the stored false->true edge fires the
//! action. Level semantics differ by condition kind in one deliberate
//! way: a CAN signal keeps its last value between its own message's
//! frames, so a crossing holds until the message says otherwise, while
//! presence conditions ("this ID showed up", "an error frame appeared")
//! latch for the whole run -- the interesting fact is that it happened,
//! not that it keeps happening.

use crate::app::App;

/// What a trigger watches.
#[derive(Clone, Debug, PartialEq)]
pub enum TriggerCond {
    /// A decoded signal's physical value at or past a threshold. The
    /// edge is the crossing; the level follows the signal's own frames
    /// and holds in between. `ext` picks the frame class, so a standard
    /// and an extended message sharing one numeric id are told apart.
    SignalCross {
        ch: u8,
        id: u32,
        ext: bool,
        signal: String,
        threshold: f64,
        rising: bool,
    },
    /// A frame with this (bus, id) arriving. Latches for the run.
    IdPresent { ch: u8, id: u32 },
    /// Any error frame on this bus. Latches for the run.
    ErrorFrame { ch: u8 },
    /// A message that has been seen going silent beyond the spec's
    /// grace window. Swept once per step against the aggregates, not
    /// per frame; the level clears when traffic resumes, so every new
    /// dropout is a fresh edge.
    CycleTimeout { ch: u8, id: u32 },
    /// A FlexRay frame arriving in `slot` on the cluster `bus`. Latches for
    /// the run like the other presence conditions. `bus` is a **FlexRay** bus
    /// index, not a CAN channel -- the two numbering spaces are separate, so
    /// nothing here may hand one to the other.
    FrFramePresent { bus: u8, slot: u16 },
    /// A decoded FlexRay signal at or past a threshold. The identity is the
    /// subscription key's own triple `(bus, slot, signal)`, so a trigger and a
    /// Graphics curve name the same value. Unlike `SignalCross`, whose level
    /// holds between its message's frames in silence, this one can only move
    /// when the slot carries a frame -- a quiet cluster keeps the last verdict.
    FrSignalCross {
        bus: u8,
        slot: u16,
        signal: String,
        threshold: f64,
        rising: bool,
    },
    /// A CAN frame whose payload byte at `offset`, masked by `mask`, equals
    /// `value`. Useful for undescribed frames, where no signal can be named.
    /// The level follows each frame of the address, so a payload change is a
    /// new edge.
    PayloadMatch {
        ch: u8,
        id: u32,
        ext: bool,
        offset: u8,
        mask: u8,
        value: u8,
    },
    /// A captured Ethernet flow: bus, EtherType and optionally a VLAN tag.
    /// Presence-style: once the flow is seen the level latches for the run.
    EthFlow {
        bus: u8,
        ethertype: u16,
        vlan: Option<u16>,
    },
    /// A FlexRay slot that has gone silent past the schedule's grace. The
    /// level clears when traffic resumes, so every new dropout is an edge.
    FrTimeout { bus: u8, slot: u16 },
    /// An E2E counter gap or jump on this CAN message. Event-style: it fires
    /// on the anomaly itself and needs no level, because two jumps in a row
    /// are two events.
    E2eIssue { ch: u8, id: u32, ext: bool },
    /// A system variable at or past a threshold (`rising`), or at/below
    /// it with `rising == false`. Swept once per step against the live
    /// registry -- sysvars are the user-input channel, so "the operator
    /// raised Setpoint" can arm a recording. The level follows the value
    /// and the edge is the crossing.
    SysVar {
        key: String,
        threshold: f64,
        rising: bool,
    },
    /// A derived channel at or past a threshold. The lane is sampled once per
    /// step, so the level is swept after that sampling and the edge can trail
    /// the source frame by one lap.
    DerivedSignal {
        id: String,
        threshold: f64,
        rising: bool,
    },
    /// A decoded SOME/IP message from one service on an Ethernet bus. The
    /// condition is an event, like E2E; every message of the service fires.
    SomeIp {
        bus: u8,
        service: u16,
        method: Option<u16>,
        message_type: Option<u8>,
    },
}

/// What a fired trigger does.
///
/// `Send` transmits **one** frame from the generator entry with that
/// (bus, id) on the edge -- payload assembled by the entry's own base bytes
/// and waveform sources, stamped with the triggering frame's own timestamp
/// so a reaction during replay lands on the log timeline like any injected
/// frame. Referencing the entry keeps the rule alive across generator
/// row edits; a missing entry is a no-op, not an error.
#[derive(Clone, Debug, PartialEq)]
pub enum TriggerAction {
    StartRecording,
    StopRecording,
    Send {
        ch: u8,
        id: u32,
    },
    /// Sends one whole FlexRay generator entry (bus + slot), values encoded
    /// from the entry's current signals at the firing instant.
    SendFr {
        bus: u8,
        slot: u16,
    },
    /// Injects one whole Ethernet frame from the bus's matrix: the frame name
    /// supplies VLAN/EtherType/length, the bytes are the payload as typed.
    SendEth {
        bus: u8,
        frame: String,
        data: Vec<u8>,
    },
    /// Injects one SOME/IP UDP notification/request into the session.
    SendSomeIp {
        bus: u8,
        service: u16,
        method: u16,
        client: u16,
        session: u16,
        message_type: u8,
        data: Vec<u8>,
        /// ARXML field values; when non-empty the sender encodes `data` from
        /// the service catalog instead of using the bytes above.
        values: std::collections::BTreeMap<String, f64>,
        /// `udp` or `tcp`; TCP uses host/port.
        #[allow(dead_code)]
        transport: String,
        #[allow(dead_code)]
        host: String,
        #[allow(dead_code)]
        port: u16,
    },
    /// Blanks the trace ring on the edge: a clean window at the moment
    /// something interesting happened, and the manual way to re-arm a
    /// latched appearance watch. Aggregates, spec memory and the
    /// recorder are deliberately untouched.
    ClearTrace,
    /// Drops a timestamped marker on the edge; Graphics draws the markers
    /// as vertical lines so a curve can be read against the moment the
    /// condition fired.
    InsertMarker,
    /// Writes a system variable, clamped against its declaration. This is
    /// the same write path as a Panel Toggle/Setpoint, so scripts, panels
    /// and other triggers see the value immediately.
    SetSysVar {
        key: String,
        value: f64,
    },
    /// Queues one `on panel_action("…")` handler with an optional value.
    /// The handler runs through the node dispatch, exactly like a Panel
    /// button press.
    RunScript {
        name: String,
        value: Option<f64>,
    },
}

impl TriggerAction {
    /// Stable code shared with the web command and the project file.
    pub fn kind(&self) -> &'static str {
        match self {
            TriggerAction::StartRecording => "start_recording",
            TriggerAction::StopRecording => "stop_recording",
            TriggerAction::Send { .. } => "send",
            TriggerAction::SendFr { .. } => "send_fr",
            TriggerAction::SendEth { .. } => "send_eth",
            TriggerAction::SendSomeIp { .. } => "send_someip",
            TriggerAction::ClearTrace => "clear_trace",
            TriggerAction::InsertMarker => "insert_marker",
            TriggerAction::SetSysVar { .. } => "set_sysvar",
            TriggerAction::RunScript { .. } => "script",
        }
    }

    /// The label the list, the fire log and the Write line show.
    pub fn label(&self) -> String {
        match self {
            TriggerAction::StartRecording => "start recording".to_string(),
            TriggerAction::StopRecording => "stop recording".to_string(),
            TriggerAction::Send { id, .. } => format!("send 0x{id:X}"),
            TriggerAction::SendFr { bus, slot } => format!("send FR{bus} slot {slot}"),
            TriggerAction::SendEth { bus, frame, .. } => format!("send ETH{bus} {frame}"),
            TriggerAction::SendSomeIp {
                service, method, ..
            } => {
                format!("send SOME/IP 0x{service:04X}.0x{method:04X}")
            }
            TriggerAction::ClearTrace => "clear trace".to_string(),
            TriggerAction::InsertMarker => "insert marker".to_string(),
            TriggerAction::SetSysVar { key, value } => {
                format!("set {key} = {value}")
            }
            TriggerAction::RunScript { name, .. } => format!("run \"{name}\""),
        }
    }
}

/// The kinds of rule the Triggers window's "+" buttons create. The window
/// names the kind; the default condition (which bus, id, signal) is still
/// chosen here, against the live snapshot, so the UI never guesses.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TriggerKind {
    Signal,
    Id,
    ErrorFrame,
    FrFrame,
    FrSignal,
    Timeout,
    SysVar,
    Derived,
    Payload,
    EthFlow,
    FrTimeout,
    E2e,
    SomeIp,
}

/// Everything the Triggers window can ask for. The window draws read-only and
/// returns these; [`App::apply_trigger_cmds`] is the one place that writes.
#[derive(Clone, Debug, PartialEq)]
pub enum TriggerCmd {
    Add(TriggerKind),
    /// Append a rule with a fully-formed condition (the Trace/chart context
    /// menus and the web seed command). Uses the default action.
    AddCond(TriggerCond),
    Remove(usize),
    SetEnabled {
        index: usize,
        on: bool,
    },
    Edit {
        index: usize,
        cond: TriggerCond,
        action: TriggerAction,
        cooldown_us: u64,
        max_fires: u32,
        pre_frames: Option<usize>,
        post_frames: Option<u32>,
    },
    Rearm,
    RearmOne(usize),
    SetPreFrames(usize),
    SetPostFrames(u32),
    SetMarkerCap(usize),
}

/// One armed condition plus its edge state and fire history.
#[derive(Clone, Debug, PartialEq)]
pub struct Trigger {
    pub cond: TriggerCond,
    pub action: TriggerAction,
    pub enabled: bool,
    /// The condition's current level. Only frames the condition watches
    /// can move it.
    pub level: bool,
    pub fired: u64,
    pub last_fire_t_us: u64,
    /// Minimum gap between two edges of this rule; 0 disables the gate.
    /// Set per rule so an oscillating signal cannot flood the action.
    pub cooldown_us: u64,
    /// Stop after this many fires; 0 means unlimited. The rule stays
    /// enabled (its row shows "done") so raising the number resumes it.
    pub max_fires: u32,
    /// Recording context for this rule: how many pre-trigger frames to keep
    /// and how long the post-roll runs. `None` inherits the global limits.
    pub pre_frames: Option<usize>,
    pub post_frames: Option<u32>,
}

impl Trigger {
    pub fn new(cond: TriggerCond, action: TriggerAction) -> Self {
        Trigger {
            cond,
            action,
            enabled: true,
            level: false,
            fired: 0,
            last_fire_t_us: 0,
            cooldown_us: 0,
            max_fires: 0,
            pre_frames: None,
            post_frames: None,
        }
    }

    /// Clears the level latch of appearance-type conditions so the next
    /// occurrence fires again. A no-op for real-level conditions.
    pub fn rearm(&mut self) {
        if self.cond.latches_once() {
            self.level = false;
        }
    }

    /// Per-rule re-arm: drops the latch and the fire count so a one-shot
    /// rule can run again without touching its neighbours.
    pub fn rearm_rule(&mut self) {
        self.level = false;
        self.fired = 0;
    }

    /// Whether an edge at `at_us` should run this rule's action. A single
    /// rule can fire at most `max_fires` times per run, and no two edges
    /// land closer than `cooldown_us`.
    pub fn can_fire(&self, at_us: u64) -> bool {
        if !self.enabled {
            return false;
        }
        if self.max_fires > 0 && self.fired >= u64::from(self.max_fires) {
            return false;
        }
        if self.cooldown_us == 0 || self.fired == 0 {
            return true;
        }
        at_us.saturating_sub(self.last_fire_t_us) >= self.cooldown_us
    }

    /// Records one fire. The rule stays enabled even at the cap; callers
    /// gate the next edge through [`Trigger::can_fire`].
    pub fn note_fire(&mut self, at_us: u64) {
        self.fired += 1;
        self.last_fire_t_us = at_us;
    }
}

/// One edge that actually ran an action. Runtime-only: the Triggers list
/// shows the counters, the replay bar walks these timestamps, and every
/// entry becomes one Write line.
#[derive(Clone, Debug, PartialEq)]
pub struct TriggerFire {
    /// Rule index at the moment it fired (the list is index-addressed).
    pub index: usize,
    pub at_us: u64,
    /// Condition summary (`cond.short()`), without the bus name.
    pub text: String,
    pub action: String,
    pub result: String,
    /// False when the action could not do what it asked (missing entry,
    /// unknown variable, measurement not running, ...).
    pub ok: bool,
}

impl TriggerCond {
    /// Whether a decoded SOME/IP message satisfies this condition. Optional
    /// method/type are both `None` for old rules and mean "any".
    pub fn someip_matches(&self, bus: u8, service: u16, method: u16, message_type: u8) -> bool {
        match self {
            TriggerCond::SomeIp {
                bus: want_bus,
                service: want_service,
                method: want_method,
                message_type: want_type,
            } => {
                *want_bus == bus
                    && *want_service == service
                    && want_method.is_none_or(|want| want == method)
                    && want_type.is_none_or(|want| want == message_type)
            }
            _ => false,
        }
    }

    /// The CAN channel a condition watches, or `None` for the global and the
    /// FlexRay ones. Named rather than plain `bus` because the two bus spaces
    /// are separate: renumbering CAN channels on a bus removal must not reach
    /// into a FlexRay index, and a CAN channel name must not be printed for
    /// one.
    pub fn can_bus(&self) -> Option<u8> {
        match self {
            TriggerCond::SignalCross { ch, .. }
            | TriggerCond::IdPresent { ch, .. }
            | TriggerCond::ErrorFrame { ch }
            | TriggerCond::CycleTimeout { ch, .. }
            | TriggerCond::PayloadMatch { ch, .. }
            | TriggerCond::E2eIssue { ch, .. } => Some(*ch),
            // System variables and derived channels are global; a FlexRay
            // rule names a cluster, an Ethernet rule its own bus.
            TriggerCond::SysVar { .. }
            | TriggerCond::DerivedSignal { .. }
            | TriggerCond::FrFramePresent { .. }
            | TriggerCond::FrSignalCross { .. }
            | TriggerCond::FrTimeout { .. }
            | TriggerCond::EthFlow { .. }
            | TriggerCond::SomeIp { .. } => None,
        }
    }

    /// The FlexRay bus a rule watches, whichever FlexRay condition it is.
    pub fn fr_bus(&self) -> Option<u8> {
        match self {
            TriggerCond::FrFramePresent { bus, .. } | TriggerCond::FrSignalCross { bus, .. } => {
                Some(*bus)
            }
            TriggerCond::FrTimeout { bus, .. } => Some(*bus),
            _ => None,
        }
    }

    /// True for conditions whose level is a **latch** -- once the event is
    /// seen the level stays true for the run. Re-arming means clearing
    /// exactly these; the crossing and timeout conditions carry real levels,
    /// which a frame or a value moves back down, and must never be touched.
    pub fn latches_once(&self) -> bool {
        matches!(
            self,
            TriggerCond::IdPresent { .. }
                | TriggerCond::ErrorFrame { .. }
                | TriggerCond::FrFramePresent { .. }
                | TriggerCond::EthFlow { .. }
        )
    }

    /// One-line summary without the bus name; the trigger list shows the
    /// bus separately.
    pub fn short(&self) -> String {
        match self {
            TriggerCond::SignalCross {
                id,
                ext,
                signal,
                threshold,
                rising,
                ..
            } => format!(
                "{signal} {} {threshold} @ 0x{id:X}{}",
                if *rising { ">=" } else { "<=" },
                if *ext { "x" } else { "" }
            ),
            TriggerCond::IdPresent { id, .. } => format!("0x{id:X} present"),
            TriggerCond::PayloadMatch {
                id,
                offset,
                mask,
                value,
                ..
            } => format!("0x{id:X} byte[{offset}]&{mask:02X}={value:02X}"),
            TriggerCond::EthFlow {
                ethertype, vlan, ..
            } => match vlan {
                Some(vlan) => format!("ETH 0x{ethertype:04X} VLAN {vlan}"),
                None => format!("ETH 0x{ethertype:04X}"),
            },
            TriggerCond::ErrorFrame { .. } => "error frames".to_string(),
            TriggerCond::CycleTimeout { id, .. } => format!("0x{id:X} timeout"),
            TriggerCond::FrFramePresent { slot, .. } => format!("slot {slot} frame"),
            TriggerCond::FrTimeout { slot, .. } => format!("slot {slot} timeout"),
            TriggerCond::E2eIssue { id, .. } => format!("0x{id:X} E2E gap/jump"),
            TriggerCond::SomeIp {
                service,
                method,
                message_type,
                ..
            } => {
                let mut text = format!("SOME/IP 0x{service:04X}");
                if let Some(method) = method {
                    text.push_str(&format!(".0x{method:04X}"));
                }
                if let Some(message_type) = message_type {
                    text.push_str(&format!(" type 0x{message_type:02X}"));
                }
                text
            }
            TriggerCond::FrSignalCross {
                slot,
                signal,
                threshold,
                rising,
                ..
            } => format!(
                "{signal} {} {threshold} @ slot {slot}",
                if *rising { ">=" } else { "<=" }
            ),
            TriggerCond::SysVar {
                key,
                threshold,
                rising,
            } => format!("@{key} {} {threshold}", if *rising { ">=" } else { "<=" }),
            // The window replaces `id` with the definition's name; this form
            // only exists so the match is total.
            TriggerCond::DerivedSignal { id, .. } => format!("derived {id}"),
        }
    }
}

impl App {
    /// Applies the Triggers window's collected commands. This is the only
    /// place the window's edits reach the bus or the run limits.
    pub(crate) fn apply_trigger_cmds(&mut self, cmds: Vec<TriggerCmd>) {
        for cmd in cmds {
            match cmd {
                TriggerCmd::Add(kind) => match kind {
                    TriggerKind::Signal => self.add_signal_trigger(),
                    TriggerKind::Id => self.add_id_trigger(),
                    TriggerKind::ErrorFrame => self.add_error_trigger(),
                    TriggerKind::FrFrame => self.add_fr_trigger(),
                    TriggerKind::FrSignal => self.add_fr_signal_trigger(),
                    TriggerKind::Timeout => self.add_timeout_trigger(),
                    TriggerKind::SysVar => self.add_sysvar_trigger(),
                    TriggerKind::Derived => self.add_derived_trigger(),
                    TriggerKind::Payload => self.add_payload_trigger(),
                    TriggerKind::EthFlow => self.add_eth_flow_trigger(),
                    TriggerKind::FrTimeout => self.add_fr_timeout_trigger(),
                    TriggerKind::E2e => self.add_e2e_trigger(),
                    TriggerKind::SomeIp => self.add_someip_trigger(),
                },
                TriggerCmd::AddCond(cond) => self.add_trigger_cond(cond),
                TriggerCmd::Remove(index) => self.remove_trigger(index),
                TriggerCmd::SetEnabled { index, on } => {
                    self.send(crate::bus::BusCommand::SetTriggerEnabled { index, on });
                }
                TriggerCmd::Edit {
                    index,
                    cond,
                    action,
                    cooldown_us,
                    max_fires,
                    pre_frames,
                    post_frames,
                } => {
                    self.send(crate::bus::BusCommand::EditTrigger {
                        index,
                        cond,
                        action,
                        cooldown_us,
                        max_fires,
                        pre_frames,
                        post_frames,
                    });
                }
                TriggerCmd::Rearm => {
                    self.send(crate::bus::BusCommand::RearmTriggers);
                }
                TriggerCmd::RearmOne(index) => {
                    self.send(crate::bus::BusCommand::RearmTrigger { index });
                }
                TriggerCmd::SetPreFrames(pre_frames) => {
                    self.limits.pre_frames = pre_frames;
                    self.set_run_limits(
                        self.limits.pre_frames,
                        self.limits.post_frames,
                        self.limits.marker_cap,
                    );
                }
                TriggerCmd::SetPostFrames(post_frames) => {
                    self.limits.post_frames = post_frames;
                    self.set_run_limits(
                        self.limits.pre_frames,
                        self.limits.post_frames,
                        self.limits.marker_cap,
                    );
                }
                TriggerCmd::SetMarkerCap(marker_cap) => {
                    self.limits.marker_cap = marker_cap;
                    self.set_run_limits(
                        self.limits.pre_frames,
                        self.limits.post_frames,
                        self.limits.marker_cap,
                    );
                }
            }
        }
    }

    /// One-line description of trigger `i` for the list: bus name plus
    /// the condition's own summary. Reads the snapshot: the rules live
    /// on the bus, the window only shapes them.
    pub fn trigger_summary(&self, i: usize) -> String {
        match self.snap.triggers.get(i) {
            Some(t) => {
                // The bus spaces name differently: a FlexRay rule reads its
                // 路's name (`FR1` until the user renames it), a CAN one keeps
                // its channel name, and a derived lane has no bus at all.
                let bus = match &t.cond {
                    TriggerCond::DerivedSignal { .. } => "Derived".to_string(),
                    TriggerCond::SomeIp { bus, .. } | TriggerCond::EthFlow { bus, .. } => {
                        self.eth_bus_label(*bus)
                    }
                    _ => match t.cond.fr_bus() {
                        Some(b) => self.fr_bus_name(b),
                        None => self.channel_name(t.cond.can_bus().unwrap_or(0)),
                    },
                };
                let text = match &t.cond {
                    TriggerCond::DerivedSignal {
                        id,
                        threshold,
                        rising,
                    } => {
                        let name = self
                            .derived_defs
                            .iter()
                            .find(|def| &def.id == id)
                            .map(|def| def.name.as_str())
                            .unwrap_or(id);
                        format!(
                            "{name} {} {threshold} (derived)",
                            if *rising { ">=" } else { "<=" }
                        )
                    }
                    _ => t.cond.short(),
                };
                format!("{bus}  {text}")
            }
            None => String::new(),
        }
    }

    pub fn add_signal_trigger(&mut self) {
        // Default to the database's first message and signal so the row
        // starts watching something real instead of a blind id.
        let db = self.snap.channels.first().and_then(|c| c.dbc.as_deref());
        let (id, ext) = db
            .and_then(|db| db.order.first())
            .copied()
            .unwrap_or((0x100, false));
        let signal = db
            .and_then(|db| db.messages.get(&(id, ext)))
            .and_then(|m| m.signals.first())
            .map(|s| s.name.clone())
            .unwrap_or_else(|| "Signal".to_string());
        self.push_trigger(TriggerCond::SignalCross {
            ch: 0,
            id,
            ext,
            signal,
            threshold: 0.0,
            rising: true,
        });
    }

    pub fn add_id_trigger(&mut self) {
        self.push_trigger(TriggerCond::IdPresent { ch: 0, id: 0x100 });
    }

    /// The first FlexRay cluster's first slot that declares signals, with those
    /// signal names, so a new FlexRay rule starts watching something a curve
    /// could actually be drawn from. `(0, 1, [])` when no description is loaded
    /// to ask -- the popup still lets the user type a slot.
    fn fr_default_slot(&self) -> (u8, u16, Vec<String>) {
        for (bus, cfg) in self.fr_buses.iter() {
            if let Some((slot, _, names)) = cfg.slot_signals().into_iter().next() {
                return (*bus, slot, names);
            }
        }
        (0, 1, Vec::new())
    }

    pub fn add_fr_trigger(&mut self) {
        let (bus, slot, _) = self.fr_default_slot();
        self.push_trigger(TriggerCond::FrFramePresent { bus, slot });
    }

    pub fn add_fr_signal_trigger(&mut self) {
        let (bus, slot, names) = self.fr_default_slot();
        let signal = names
            .first()
            .cloned()
            .unwrap_or_else(|| "Signal".to_string());
        self.push_trigger(TriggerCond::FrSignalCross {
            bus,
            slot,
            signal,
            threshold: 0.0,
            rising: true,
        });
    }

    pub fn add_error_trigger(&mut self) {
        self.push_trigger(TriggerCond::ErrorFrame { ch: 0 });
    }

    pub fn add_timeout_trigger(&mut self) {
        self.push_trigger(TriggerCond::CycleTimeout { ch: 0, id: 0x100 });
    }

    pub fn add_sysvar_trigger(&mut self) {
        // Default to the first defined variable so the row starts
        // watching something real instead of a blank key.
        let key = self
            .snap
            .sysvars
            .first()
            .map(|v| format!("{}::{}", v.def.namespace, v.def.name))
            .unwrap_or_default();
        self.push_trigger(TriggerCond::SysVar {
            key,
            threshold: 0.0,
            rising: true,
        });
    }

    pub fn add_derived_trigger(&mut self) {
        // A rule needs a lane to read; the Derived dialog is where one is
        // created, so an empty project gets an explanation instead of a rule
        // that can never fire.
        let Some(def) = self.derived_defs.first() else {
            self.status = "add a derived channel before watching one".to_string();
            return;
        };
        self.push_trigger(TriggerCond::DerivedSignal {
            id: def.id.clone(),
            threshold: 0.0,
            rising: true,
        });
    }

    pub fn add_payload_trigger(&mut self) {
        self.push_trigger(TriggerCond::PayloadMatch {
            ch: 0,
            id: 0x100,
            ext: false,
            offset: 0,
            mask: 0xFF,
            value: 0,
        });
    }

    pub fn add_eth_flow_trigger(&mut self) {
        self.push_trigger(TriggerCond::EthFlow {
            bus: 0,
            ethertype: crate::eth::EthFrame::SESSION_ETHERTYPE,
            vlan: None,
        });
    }

    pub fn add_fr_timeout_trigger(&mut self) {
        let (bus, slot, _) = self.fr_default_slot();
        self.push_trigger(TriggerCond::FrTimeout { bus, slot });
    }

    pub fn add_e2e_trigger(&mut self) {
        self.push_trigger(TriggerCond::E2eIssue {
            ch: 0,
            id: 0x100,
            ext: false,
        });
    }

    pub fn add_someip_trigger(&mut self) {
        self.push_trigger(TriggerCond::SomeIp {
            bus: 0,
            service: 0x1234,
            method: None,
            message_type: None,
        });
    }

    /// Adds a rule with a condition the caller already chose (Trace/chart
    /// context menus, web seed commands). The default action is recording,
    /// like the "+" buttons; the editor changes it.
    pub fn add_trigger_cond(&mut self, cond: TriggerCond) {
        self.push_trigger(cond);
    }

    fn push_trigger(&mut self, cond: TriggerCond) {
        let action = TriggerAction::StartRecording;
        self.send(crate::bus::BusCommand::AddTrigger {
            cond: cond.clone(),
            action,
            enabled: false,
        });
    }

    pub fn remove_trigger(&mut self, i: usize) {
        if i < self.snap.triggers.len() {
            self.send(crate::bus::BusCommand::RemoveTrigger { index: i });
        }
    }
}
