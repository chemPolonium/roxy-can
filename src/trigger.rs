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
use crate::can::frame::CanFrame;

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

/// What a fired trigger does. Recording actions today; `Send` joins
/// here with the reaction rules, on the same edge.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum TriggerAction {
    StartRecording,
    StopRecording,
}

/// One armed condition plus its edge state and fire history.
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
    /// Folds one frame into every enabled trigger and acts on edges.
    /// Runs as the first thing that happens to a received frame, so a
    /// trigger that starts a recording still captures the frame that
    /// fired it.
    pub fn eval_triggers(&mut self, f: &CanFrame) {
        if self.triggers.is_empty() {
            return;
        }
        let mut fired: Vec<TriggerAction> = Vec::new();
        for i in 0..self.triggers.len() {
            // Observe under shared borrows first: acting on an edge
            // needs `self` exclusively, and the two cannot overlap.
            let now = {
                let t = &self.triggers[i];
                if !t.enabled {
                    continue;
                }
                match &t.cond {
                    TriggerCond::SignalCross {
                        ch,
                        id,
                        signal,
                        threshold,
                        rising,
                    } => {
                        if f.channel != *ch || f.id != *id || f.is_error() {
                            // Not this message's frame: the level holds.
                            continue;
                        }
                        let Some(db) = self.channel_dbc(*ch) else {
                            continue; // no database on the bus, no opinion
                        };
                        match db.decode_signals(f).into_iter().find(|d| d.name == *signal) {
                            Some(d) => {
                                if *rising {
                                    d.phys >= *threshold
                                } else {
                                    d.phys <= *threshold
                                }
                            }
                            // The condition names a signal the database
                            // lacks -- same courtesy as a missing db.
                            None => continue,
                        }
                    }
                    TriggerCond::IdPresent { ch, id } => {
                        if f.channel != *ch || f.id != *id || f.is_error() {
                            continue;
                        }
                        true // latch: once seen, it stays seen
                    }
                    TriggerCond::ErrorFrame { ch } => {
                        if f.channel != *ch || !f.is_error() {
                            continue;
                        }
                        true
                    }
                    // Not a frame condition: swept once per step against
                    // the aggregates in `eval_timeout_triggers`.
                    TriggerCond::CycleTimeout { .. } => continue,
                }
            };
            let t = &mut self.triggers[i];
            let was = t.level;
            t.level = now;
            if now && !was {
                t.fired += 1;
                t.last_fire_t_us = f.t_us;
                fired.push(t.action);
            }
        }
        self.run_actions(fired);
    }

    /// Sweep conditions: evaluated once per measurement step against the
    /// aggregates, not per frame. A message only convicts after it has
    /// been seen once (the Missing verdict takes the same stance), and
    /// the level clears when traffic resumes, so every new dropout is a
    /// fresh edge.
    pub fn eval_timeout_triggers(&mut self, now_us: u64) {
        if self.triggers.is_empty() {
            return;
        }
        let mut fired: Vec<TriggerAction> = Vec::new();
        for i in 0..self.triggers.len() {
            let (ch, id) = match &self.triggers[i].cond {
                TriggerCond::CycleTimeout { ch, id } => (*ch, *id),
                _ => continue,
            };
            if !self.triggers[i].enabled {
                continue;
            }
            let silent = self.timeout_silent(ch, id, now_us);
            let t = &mut self.triggers[i];
            let was = t.level;
            t.level = silent;
            if silent && !was {
                t.fired += 1;
                t.last_fire_t_us = now_us;
                fired.push(t.action);
            }
        }
        self.run_actions(fired);
    }

    /// The spec's own grace comparison decides silence, so a trigger and
    /// the Dropped verdict can never disagree about the same message.
    fn timeout_silent(&self, ch: u8, id: u32, now_us: u64) -> bool {
        let Some(agg) = self.aggs.get(&(ch, id)) else {
            return false; // never seen: no opinion, not a dropout
        };
        let Some(declared) = self.dbc_cycle_us(ch, id) else {
            return false; // no database, message, or declared period
        };
        crate::spec::missing_offender(now_us, agg.last_t_us, declared, self.spec_grace)
    }

    fn run_actions(&mut self, fired: Vec<TriggerAction>) {
        for action in fired {
            match action {
                TriggerAction::StartRecording => {
                    if self.measuring && !self.recorder.recording {
                        self.recorder.recording = true;
                        let opened = self.recorder.open();
                        self.recorder.recording = opened.is_ok();
                        self.status = match opened {
                            Ok(path) => format!("trigger started recording to {path}"),
                            Err(e) => format!("trigger record failed: {e}"),
                        };
                    }
                }
                TriggerAction::StopRecording => {
                    if self.recorder.recording {
                        self.recorder.close();
                        self.recorder.recording = false;
                        self.status = "trigger stopped recording".to_string();
                    }
                }
            }
        }
    }

    /// One-line description of trigger `i` for the list: bus name plus
    /// the condition's own summary.
    pub fn trigger_summary(&self, i: usize) -> String {
        match self.triggers.get(i) {
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
        let db = self.channels.first().and_then(|c| c.dbc.as_ref());
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
        self.triggers
            .push(Trigger::new(cond, TriggerAction::StartRecording));
        self.trigger_sel = Some(self.triggers.len() - 1);
        self.show_triggers = true;
    }

    pub fn remove_trigger(&mut self, i: usize) {
        if i < self.triggers.len() {
            self.triggers.remove(i);
        }
        if self.trigger_sel.is_some_and(|s| s >= self.triggers.len()) {
            self.trigger_sel = None;
        }
        // The editor's hex buffer names a trigger that may be gone.
        self.trig_edit_sel = None;
    }
}
