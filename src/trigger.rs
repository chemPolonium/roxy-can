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
    /// and holds in between.
    SignalCross {
        ch: u8,
        id: u32,
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
    Send { ch: u8, id: u32 },
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
}

impl TriggerCond {
    /// The bus the condition watches.
    pub fn bus(&self) -> u8 {
        match self {
            TriggerCond::SignalCross { ch, .. }
            | TriggerCond::IdPresent { ch, .. }
            | TriggerCond::ErrorFrame { ch }
            | TriggerCond::CycleTimeout { ch, .. } => *ch,
        }
    }

    /// One-line summary without the bus name; the trigger list shows the
    /// bus separately.
    pub fn short(&self) -> String {
        match self {
            TriggerCond::SignalCross {
                id,
                signal,
                threshold,
                rising,
                ..
            } => format!(
                "{signal} {} {threshold} @ 0x{id:X}",
                if *rising { ">=" } else { "<=" }
            ),
            TriggerCond::IdPresent { id, .. } => format!("0x{id:X} present"),
            TriggerCond::ErrorFrame { .. } => "error frames".to_string(),
            TriggerCond::CycleTimeout { id, .. } => format!("0x{id:X} timeout"),
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
            .and_then(|db| db.messages.get(&id))
            .map(|m| m.signals.iter().map(|s| s.name.clone()).collect())
            .unwrap_or_default()
    }

    pub fn add_signal_trigger(&mut self) {
        // Default to the database's first message and signal so the row
        // starts watching something real instead of a blind id.
        let db = self.snap.channels.first().and_then(|c| c.dbc.as_deref());
        let id = db.and_then(|db| db.order.first()).copied().unwrap_or(0x100);
        let signal = db
            .and_then(|db| db.messages.get(&id))
            .and_then(|m| m.signals.first())
            .map(|s| s.name.clone())
            .unwrap_or_else(|| "Signal".to_string());
        self.push_trigger(TriggerCond::SignalCross {
            ch: 0,
            id,
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

    fn push_trigger(&mut self, cond: TriggerCond) {
        // The new rule's index is the list's length *before* the append.
        let index = self.snap.triggers.len();
        self.send(crate::bus::BusCommand::AddTrigger {
            cond,
            action: TriggerAction::StartRecording,
        });
        self.trigger_sel = Some(index);
        self.show_triggers = true;
    }

    pub fn remove_trigger(&mut self, i: usize) {
        if i < self.snap.triggers.len() {
            self.send(crate::bus::BusCommand::RemoveTrigger { index: i });
        }
        if self
            .trigger_sel
            .is_some_and(|s| s >= self.snap.triggers.len())
        {
            self.trigger_sel = None;
        }
        // The editor's hex buffer names a trigger that may be gone.
        self.trig_edit_sel = None;
    }
}
