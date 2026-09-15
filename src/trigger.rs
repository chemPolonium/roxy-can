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
#![allow(dead_code)]

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
    /// The bus the condition watches.
    pub fn bus(&self) -> u8 {
        match self {
            TriggerCond::SignalCross { ch, .. }
            | TriggerCond::IdPresent { ch, .. }
            | TriggerCond::ErrorFrame { ch }
            | TriggerCond::CycleTimeout { ch, .. } => *ch,
            // System variables are global: no bus to watch.
            TriggerCond::SysVar { .. } => 0,
        }
    }

    /// True for conditions whose level is a **latch** -- once the event is
    /// seen the level stays true for the run. Re-arming means clearing
    /// exactly these; signal-cross and cycle-timeout levels are real
    /// levels and must never be touched.
    pub fn latches_once(&self) -> bool {
        matches!(
            self,
            TriggerCond::IdPresent { .. } | TriggerCond::ErrorFrame { .. }
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
            TriggerCond::SysVar {
                key,
                threshold,
                rising,
            } => format!(
                "@{key} {} {threshold}",
                if *rising { ">=" } else { "<=" }
            ),
        }
    }
}

impl App {
    /// One-line description of trigger `i` for the list: bus name plus
    /// the condition's own summary. Reads the snapshot: the rules live
    /// on the bus, the window only shapes them.
    pub fn trigger_summary(&self, i: usize) -> String {
        match self.snap.triggers.get(i) {
            Some(t) => format!("{}  {}", self.channel_name(t.cond.bus()), t.cond.short()),
            None => String::new(),
        }
    }

    /// Signal names the database declares on `(ch, id)`, for the editor's
    /// signal picker; empty when there is no database or message.
    pub fn signal_names(&self, ch: u8, id: u32) -> Vec<String> {
        self.channel_dbc(ch)
            .and_then(|db| db.message_of(id))
            .map(|m| m.signals.iter().map(|s| s.name.clone()).collect())
            .unwrap_or_default()
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

    fn push_trigger(&mut self, cond: TriggerCond) {
        // The new rule's index is the list's length *before* the append;
        // its editor opens straight away.
        let index = self.snap.triggers.len();
        let action = TriggerAction::StartRecording;
        self.send(crate::bus::BusCommand::AddTrigger {
            cond: cond.clone(),
            action,
        });
        self.trig_draft = Some(crate::ui::triggers::TrigDraft::new(index, cond, action));
        self.show_triggers = true;
    }

    pub fn remove_trigger(&mut self, i: usize) {
        if i < self.snap.triggers.len() {
            self.send(crate::bus::BusCommand::RemoveTrigger { index: i });
        }
        // An editor pointing at the removed row closes; one pointing past
        // it follows the row up.
        match &mut self.trig_draft {
            Some(d) if d.index == i => self.trig_draft = None,
            Some(d) if d.index > i => d.index -= 1,
            _ => {}
        }
    }
}
