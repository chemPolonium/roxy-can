//! Trigger conditions and their evaluation -- the shared front end for
//! the trigger layer (TODO item 8) and, later, reaction rules
//! (`on message X -> send Y`, TODO item 10), which reuse the same
//! matching and edge bookkeeping instead of growing a second evaluator.
//!
//! A trigger watches incoming frames in the measurement loop and tracks
//! its condition's level; only the stored false->true edge fires the
//! action. Level semantics differ by condition kind in one deliberate
//! way: a CAN signal keeps its last value between its own message's
//! frames, so a crossing holds until the message says otherwise, while
//! presence conditions ("this ID showed up", "an error frame appeared")
//! latch for the whole run -- the interesting fact is that it happened,
//! not that it keeps happening.
//!
//! The editing UI and project persistence land with the rest of item 8;
//! until then some of this is reachable only from tests, which
//! dead_code cannot see.

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
}

/// What a fired trigger does.
///
/// `Send` is the reaction rule of TODO item 10 riding the same evaluator:
/// on the edge it transmits **one** frame from the generator entry with
/// that (bus, id) -- payload assembled by the entry's own base bytes and
/// waveform sources, stamped with the triggering frame's own timestamp so
/// a reaction during replay lands on the log timeline like any injected
/// frame. Referencing the entry keeps the rule alive across generator
/// row edits; a missing entry is a no-op, not an error.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TriggerAction {
    StartRecording,
    StopRecording,
    Send {
        ch: u8,
        id: u32,
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
}

/// Everything the Triggers window can ask for. The window draws read-only and
/// returns these; [`App::apply_trigger_cmds`] is the one place that writes.
#[derive(Clone, Debug, PartialEq)]
pub enum TriggerCmd {
    Add(TriggerKind),
    Remove(usize),
    SetEnabled {
        index: usize,
        on: bool,
    },
    Edit {
        index: usize,
        cond: TriggerCond,
        action: TriggerAction,
    },
    Rearm,
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
        }
    }

    /// Clears the level latch of appearance-type conditions so the next
    /// occurrence fires again. A no-op for real-level conditions.
    pub fn rearm(&mut self) {
        if self.cond.latches_once() {
            self.level = false;
        }
    }
}

impl TriggerCond {
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
            | TriggerCond::CycleTimeout { ch, .. } => Some(*ch),
            // System variables and derived channels are global; a FlexRay
            // rule names a cluster.
            TriggerCond::SysVar { .. }
            | TriggerCond::DerivedSignal { .. }
            | TriggerCond::FrFramePresent { .. }
            | TriggerCond::FrSignalCross { .. } => None,
        }
    }

    /// The FlexRay bus a rule watches, whichever FlexRay condition it is.
    pub fn fr_bus(&self) -> Option<u8> {
        match self {
            TriggerCond::FrFramePresent { bus, .. } | TriggerCond::FrSignalCross { bus, .. } => {
                Some(*bus)
            }
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
            TriggerCond::ErrorFrame { .. } => "error frames".to_string(),
            TriggerCond::CycleTimeout { id, .. } => format!("0x{id:X} timeout"),
            TriggerCond::FrFramePresent { slot, .. } => format!("slot {slot} frame"),
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
                },
                TriggerCmd::Remove(index) => self.remove_trigger(index),
                TriggerCmd::SetEnabled { index, on } => {
                    self.send(crate::bus::BusCommand::SetTriggerEnabled { index, on });
                }
                TriggerCmd::Edit {
                    index,
                    cond,
                    action,
                } => {
                    self.send(crate::bus::BusCommand::EditTrigger {
                        index,
                        cond,
                        action,
                    });
                }
                TriggerCmd::Rearm => {
                    self.send(crate::bus::BusCommand::RearmTriggers);
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
            if let Some((slot, _, names)) = cfg.db.slot_signals().into_iter().next() {
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

    fn push_trigger(&mut self, cond: TriggerCond) {
        let action = TriggerAction::StartRecording;
        self.send(crate::bus::BusCommand::AddTrigger {
            cond: cond.clone(),
            action,
        });
    }

    pub fn remove_trigger(&mut self, i: usize) {
        if i < self.snap.triggers.len() {
            self.send(crate::bus::BusCommand::RemoveTrigger { index: i });
        }
    }
}
