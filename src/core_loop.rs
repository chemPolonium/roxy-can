//! The core side of the stage-3 split: what runs "next to the bus", on
//! the bus's own thread. [`CoreLoop`] owns the core plus its two pipes
//! (the command inbox and the snapshot mailbox) and knows how to run one
//! lap of work; [`spawn_lane`] is the real thread that waits on the next
//! deadline and laps. Tests and the future headless CLI run the same
//! laps by hand instead.

use std::sync::Arc;
use std::sync::mpsc::{Receiver, RecvTimeoutError};
use std::time::{Duration, Instant};

use crate::bus::{BusCommand, BusCore, SnapshotMailbox};

/// Longest the core thread sleeps when no deadline is pending. Bounded so
/// a stopped bus still answers commands promptly; deadlines (generator
/// slots, replay frames) usually wake it well before this.
const IDLE_SLEEP_US: u64 = 10_000;

/// The frontend's continuously-tuned stepping policy, handed to the core
/// thread through atomics rather than commands: these are knobs, not
/// events -- the frontend writes them every frame, the core reads them
/// every lap, and a command per drag-frame would flood the inbox.
#[derive(Default)]
pub struct BusKnobs {
    stride_us: std::sync::atomic::AtomicU64,
    tol_pct: std::sync::atomic::AtomicU64,
    grace: std::sync::atomic::AtomicU64,
}

impl BusKnobs {
    pub(crate) fn set(&self, stride_us: u64, tol_pct: u64, grace: u64) {
        use std::sync::atomic::Ordering::Relaxed;
        self.stride_us.store(stride_us, Relaxed);
        self.tol_pct.store(tol_pct, Relaxed);
        self.grace.store(grace, Relaxed);
    }

    pub(crate) fn stride_us(&self) -> u64 {
        self.stride_us.load(std::sync::atomic::Ordering::Relaxed)
    }

    pub(crate) fn tol_pct(&self) -> u64 {
        self.tol_pct.load(std::sync::atomic::Ordering::Relaxed)
    }

    pub(crate) fn grace(&self) -> u64 {
        self.grace.load(std::sync::atomic::Ordering::Relaxed)
    }
}

/// The core as a runnable unit: bus state plus the receiving end of both
/// pipes. Manual drives keep one on the UI thread; the threaded drive
/// hands one to [`spawn_lane`].
pub(crate) struct CoreLoop {
    pub(crate) core: BusCore,
    inbox: Receiver<BusCommand>,
    mail: SnapshotMailbox,
    /// Status text from applied commands, riding the next publish.
    pending_status: Option<String>,
}

impl CoreLoop {
    pub(crate) fn new(core: BusCore, inbox: Receiver<BusCommand>, mail: SnapshotMailbox) -> Self {
        CoreLoop {
            core,
            inbox,
            mail,
            pending_status: None,
        }
    }

    /// Applies one command; `true` when it restarted the run's clock
    /// (the threaded lap re-anchors its wall-clock zero on that).
    fn apply(&mut self, cmd: BusCommand) -> bool {
        let clock_reset = matches!(cmd, BusCommand::StartVirtual);
        let mut status = String::new();
        self.core.handle(cmd, &mut status);
        if !status.is_empty() {
            self.pending_status = Some(status);
        }
        clock_reset
    }

    /// Applies every queued command. Reports whether anything ran and
    /// whether the run clock restarted.
    pub(crate) fn drain(&mut self) -> (bool, bool) {
        let mut any = false;
        let mut clock_reset = false;
        while let Ok(cmd) = self.inbox.try_recv() {
            clock_reset |= self.apply(cmd);
            any = true;
        }
        (any, clock_reset)
    }

    /// Publishes the current frame into the mailbox; pending status rides
    /// along once and clears. Status is news, not state.
    pub(crate) fn publish(&mut self) {
        let status = self.pending_status.take();
        let snap = Arc::new(self.core.snapshot_with_status(status));
        *self.mail.lock().expect("snapshot mailbox poisoned") = snap;
    }

    /// One hand-cranked lap, exactly what the UI loop's `tick` always
    /// meant: drain, step the bus to `now_us`, publish. No gating -- the
    /// caller decides whether this lap should step at all.
    pub(crate) fn step_lap(&mut self, now_us: u64, stride: u64, tol_pct: u64, grace: u64) {
        self.drain();
        let mut status = String::new();
        self.core.step(now_us, stride, tol_pct, grace, &mut status);
        if !status.is_empty() {
            self.pending_status = Some(status);
        }
        self.publish();
    }
}

/// The core thread: wait for the next command or bus deadline, lap,
/// publish. Exits when every sender is gone (the frontend dropped). The
/// clock is the thread's own -- a fresh run (`StartVirtual`) re-anchors
/// it, which is what keeps sim time and wall time in step.
pub(crate) fn spawn_lane(mut lane: CoreLoop, knobs: Arc<BusKnobs>) {
    let _ = std::thread::Builder::new()
        .name("bus-core".to_string())
        .spawn(move || {
            let mut clock_zero = Instant::now();
            // Stamp for the current pause, fed to `advance_clock` on the
            // first post-resume lap so replay shifts its log clock by the
            // pause and sim time stays frozen across it -- the same
            // contract the UI loop used to implement.
            let mut paused_at: Option<u64> = None;
            loop {
                let now = elapsed_us(clock_zero);
                let wait_us = lane
                    .core
                    .next_deadline(now)
                    .map(|deadline| deadline.saturating_sub(now))
                    .unwrap_or(IDLE_SLEEP_US)
                    .min(IDLE_SLEEP_US);
                let (mut any, mut clock_reset) = (false, false);
                match lane.inbox.recv_timeout(Duration::from_micros(wait_us)) {
                    Ok(cmd) => {
                        any = true;
                        clock_reset = lane.apply(cmd);
                        let (more, more_reset) = lane.drain();
                        clock_reset |= more_reset;
                        any |= more;
                    }
                    Err(RecvTimeoutError::Timeout) => {}
                    Err(RecvTimeoutError::Disconnected) => break,
                }
                if clock_reset {
                    // The run restarted: sim time zeroed by the command,
                    // wall anchor re-set here, in the same stroke.
                    clock_zero = Instant::now();
                    paused_at = None;
                }
                if lane.core.measuring && !lane.core.trace_paused {
                    let now = elapsed_us(clock_zero);
                    if let Some(at) = paused_at.take() {
                        lane.core.paused_at_us = Some(at);
                    }
                    let mut status = String::new();
                    lane.core.step_to(
                        now,
                        knobs.stride_us(),
                        knobs.tol_pct(),
                        knobs.grace(),
                        &mut status,
                    );
                    if !status.is_empty() {
                        lane.pending_status = Some(status);
                    }
                    lane.publish();
                } else if lane.core.measuring {
                    if paused_at.is_none() {
                        paused_at = Some(elapsed_us(clock_zero));
                    }
                    if any {
                        lane.publish();
                    }
                } else if any {
                    // A stopped bus still owes the frontend its answers:
                    // command results ride the next published snapshot.
                    lane.publish();
                }
            }
        });
}

fn elapsed_us(since: Instant) -> u64 {
    since.elapsed().as_micros() as u64
}
