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

/// Windows resolves condvar waits (and sleeps) on the system timer
/// interrupt, 15.625 ms apart by default -- every `recv_timeout` here
/// would overshoot its target by up to one such tick, and a 10 ms cyclic
/// timer would fire every ~15.6 ms. `timeBeginPeriod(1)` is the OS's own
/// switch for 1 ms wait resolution (per-process since Windows 10 2004);
/// the guard holds it for the thread's lifetime and restores on drop.
#[cfg(windows)]
struct WaitResolution;

#[cfg(windows)]
impl WaitResolution {
    fn raise() -> Self {
        #[link(name = "winmm")]
        unsafe extern "system" {
            fn timeBeginPeriod(ms: u32) -> u32;
        }
        // TIMERR_NOERROR == 0; a failure only means coarser wakes.
        unsafe { timeBeginPeriod(1) };
        Self
    }
}

#[cfg(windows)]
impl Drop for WaitResolution {
    fn drop(&mut self) {
        #[link(name = "winmm")]
        unsafe extern "system" {
            fn timeEndPeriod(ms: u32) -> u32;
        }
        unsafe { timeEndPeriod(1) };
    }
}

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
            // Command news also lands in the Write window (the status bar
            // shows each line once; the Write ring keeps them).
            self.core
                .write_push(crate::bus::WriteKind::Info, status.clone());
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
        self.core.publish_loads();
        self.core.publish_nodes();
        self.core.publish_write();
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
            // Deadline waits only mean something at 1 ms resolution (see
            // `WaitResolution`); held until the thread exits.
            #[cfg(windows)]
            let _wait_resolution = WaitResolution::raise();
            let mut clock_zero = Instant::now();
            // Stamp for the current pause, fed to `advance_clock` on the
            // first post-resume lap so replay shifts its log clock by the
            // pause and sim time stays frozen across it -- the same
            // contract the UI loop used to implement.
            let mut paused_at: Option<u64> = None;
            // Rate limit for the DBC auto-reload sweep: cheap stats, but
            // there is no reason to stat attached files every lap.
            let mut last_dbc_sweep = Instant::now();
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
                        let (reset, panicked) = catch_core_panic("命令处理", || lane.apply(cmd));
                        if panicked {
                            lane.halt_after_internal_error("命令处理");
                        } else if let Some(reset) = reset {
                            clock_reset = reset;
                        }
                        let (drained, panicked) =
                            catch_core_panic("命令队列排空", || lane.drain());
                        if panicked {
                            lane.halt_after_internal_error("命令队列排空");
                        } else if let Some((more, more_reset)) = drained {
                            clock_reset |= more_reset;
                            any |= more;
                        }
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
                // External DBC edits flow back in without a manual reload:
                // checksums are cheap, the sweep runs at most every 2 s.
                if last_dbc_sweep.elapsed() >= Duration::from_secs(2) {
                    last_dbc_sweep = Instant::now();
                    if let Some(status) = lane.core.maybe_reload_changed_dbcs() {
                        lane.pending_status = Some(status);
                        any = true;
                    }
                }
                if lane.core.measuring && !lane.core.trace_paused {
                    let now = elapsed_us(clock_zero);
                    if let Some(at) = paused_at.take() {
                        lane.core.paused_at_us = Some(at);
                    }
                    let mut status = String::new();
                    let (_, panicked) = catch_core_panic("测量步进", || {
                        lane.core.step_to(
                            now,
                            knobs.stride_us(),
                            knobs.tol_pct(),
                            knobs.grace(),
                            &mut status,
                        );
                    });
                    if panicked {
                        lane.halt_after_internal_error("测量步进");
                    }
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

/// Runs `f` behind `catch_unwind`: a bug in the core (never scripts --
/// those carry their own budget and fuse) is caught here instead of
/// silently killing the "bus-core" thread. The caller turns the second
/// element into a halt via [`CoreLoop::halt_after_internal_error`].
fn catch_core_panic<T>(what: &'static str, f: impl FnOnce() -> T) -> (Option<T>, bool) {
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(f)) {
        Ok(v) => (Some(v), false),
        Err(_) => {
            eprintln!("roxy-can core: panicked during {what}");
            (None, true)
        }
    }
}

impl CoreLoop {
    /// The panic path: a loud error on the Write/status surfaces, the
    /// measurement stops, and the recording is closed -- the loop keeps
    /// servicing commands and idles rather than hot-looping on the
    /// state that tripped it.
    fn halt_after_internal_error(&mut self, what: &str) {
        self.pending_status = Some(format!(
            "[core] 内部错误（panic）：{what}——测量已停止；请保存现场并反馈复现步骤"
        ));
        self.core.measuring = false;
        self.core.recorder.close();
        self.core.recorder.recording = false;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A panicking lap halts the core loudly: measuring stops, the status
    /// names the panic, and the halt is reachable from the loop's own
    /// catch site.
    #[test]
    fn a_panicking_lap_halts_instead_of_killing_the_thread() {
        let (_tx, rx) = std::sync::mpsc::channel();
        let core = crate::bus::BusCore::new(vec![]);
        let mut lane = CoreLoop::new(core, rx, crate::bus::new_mailbox());
        lane.core.measuring = true;

        let (_, panicked) = catch_core_panic("测试", || panic!("injected"));
        assert!(panicked);
        lane.halt_after_internal_error("测试");

        assert!(!lane.core.measuring, "the measurement stops");
        let status = lane.pending_status.clone().expect("a status line");
        assert!(
            status.contains("panic") && status.contains("测试"),
            "the status names the fault site: {status}"
        );
    }

    /// The normal path is transparent: no panic, the result passes
    /// through untouched.
    #[test]
    fn catch_core_panic_passes_results_through() {
        let (v, panicked) = catch_core_panic("测试", || 42);
        assert!(!panicked);
        assert_eq!(v, Some(42));
    }
}
