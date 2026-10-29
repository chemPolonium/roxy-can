//! A tiny line debugger shared by the script VM and the web front end.
//!
//! The core thread parks inside the VM while paused; the HTTP thread talks
//! to this control directly, so Continue/Step do not need the command queue
//! (which the parked core cannot service).

use std::collections::BTreeSet;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex, OnceLock};
use std::time::{Duration, Instant};

#[derive(Clone, Debug, Default)]
pub struct DebugValue {
    pub name: String,
    pub value: String,
}

#[derive(Clone, Debug, Default)]
pub struct DebugFrame {
    pub function: String,
    pub line: u32,
}

#[derive(Clone, Debug, Default)]
pub struct DebugView {
    pub enabled: bool,
    pub node: Option<u64>,
    pub paused: bool,
    pub line: Option<u32>,
    pub function: Option<String>,
    pub breakpoints: Vec<u32>,
    pub locals: Vec<DebugValue>,
    pub globals: Vec<DebugValue>,
    pub stack: Vec<DebugFrame>,
    pub note: Option<String>,
}

/// A paused browser is expected to keep polling; if it disappears we let the
/// core continue rather than parking the whole measurement forever.
const CLIENT_TIMEOUT: Duration = Duration::from_secs(30);

struct Inner {
    enabled: bool,
    node: Option<u64>,
    breakpoints: BTreeSet<u32>,
    /// Resume a line step from this source line.
    step_from: Option<u32>,
    pause_requested: bool,
    paused: bool,
    /// The VM has built and published locals/stack for the current pause.
    /// `should_pause` knows the line before the VM snapshot exists, so the
    /// public view must not claim `paused` until `wait_paused` installs it.
    view_ready: bool,
    /// A breakpoint on this line already fired; do not fire again until the
    /// line changes.
    skip_line: Option<u32>,
    view: DebugView,
    last_poll: Instant,
}

pub struct DebugControl {
    active: AtomicBool,
    /// Node id plus one; zero means nobody is armed.
    target: AtomicU64,
    inner: Mutex<Inner>,
    cv: Condvar,
}

impl DebugControl {
    fn new() -> Self {
        Self {
            active: AtomicBool::new(false),
            target: AtomicU64::new(0),
            inner: Mutex::new(Inner {
                enabled: false,
                node: None,
                breakpoints: BTreeSet::new(),
                step_from: None,
                pause_requested: false,
                paused: false,
                view_ready: false,
                skip_line: None,
                view: DebugView::default(),
                last_poll: Instant::now(),
            }),
            cv: Condvar::new(),
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn armed_for(&self, node: u64) -> bool {
        self.active.load(Ordering::Acquire)
            && self.target.load(Ordering::Acquire) == node.wrapping_add(1)
    }

    fn publish(&self, inner: &mut Inner) -> DebugView {
        inner.last_poll = Instant::now();
        inner.view.enabled = inner.enabled;
        inner.view.node = inner.node;
        inner.view.paused = inner.paused && inner.view_ready;
        inner.view.breakpoints = inner.breakpoints.iter().copied().collect();
        inner.view.clone()
    }

    fn arm(&self, node: u64, lines: Vec<u32>) -> DebugView {
        let mut inner = self.lock();
        inner.enabled = true;
        inner.node = Some(node);
        inner.breakpoints = lines.into_iter().collect();
        inner.step_from = None;
        inner.pause_requested = false;
        inner.paused = false;
        inner.view_ready = false;
        inner.skip_line = None;
        inner.view = DebugView {
            enabled: true,
            node: Some(node),
            breakpoints: inner.breakpoints.iter().copied().collect(),
            ..DebugView::default()
        };
        self.target.store(node.wrapping_add(1), Ordering::Release);
        self.active.store(true, Ordering::Release);
        let view = self.publish(&mut inner);
        self.cv.notify_all();
        view
    }

    fn set_breakpoints(&self, lines: Vec<u32>) -> DebugView {
        let mut inner = self.lock();
        inner.breakpoints = lines.into_iter().collect();
        self.publish(&mut inner)
    }

    fn pause(&self) -> DebugView {
        let mut inner = self.lock();
        if inner.enabled {
            inner.pause_requested = true;
        }
        self.publish(&mut inner)
    }

    fn resume(&self, step: bool) -> DebugView {
        let mut inner = self.lock();
        inner.paused = false;
        inner.view_ready = false;
        inner.pause_requested = false;
        inner.step_from = if step { inner.view.line } else { None };
        let view = self.publish(&mut inner);
        self.cv.notify_all();
        view
    }

    fn stop(&self) -> DebugView {
        let mut inner = self.lock();
        inner.enabled = false;
        inner.paused = false;
        inner.view_ready = false;
        inner.pause_requested = false;
        inner.step_from = None;
        inner.skip_line = None;
        inner.view.note = None;
        self.active.store(false, Ordering::Release);
        self.target.store(0, Ordering::Release);
        let view = self.publish(&mut inner);
        self.cv.notify_all();
        view
    }

    fn view(&self) -> DebugView {
        let mut inner = self.lock();
        self.publish(&mut inner)
    }

    fn should_pause(&self, node: u64, line: u32) -> bool {
        if !self.armed_for(node) {
            return false;
        }
        let mut inner = self.lock();
        if !inner.enabled {
            return false;
        }
        let mut hit = false;
        if let Some(from) = inner.step_from
            && line != from
        {
            inner.step_from = None;
            hit = true;
        }
        if !hit && inner.pause_requested {
            inner.pause_requested = false;
            hit = true;
        }
        if !hit && inner.breakpoints.contains(&line) && inner.skip_line != Some(line) {
            hit = true;
        }
        if hit {
            inner.paused = true;
            inner.view_ready = false;
            inner.skip_line = Some(line);
            inner.view.line = Some(line);
            inner.view.paused = true;
            inner.last_poll = Instant::now();
        } else if inner.skip_line.is_some_and(|skip| skip != line) {
            inner.skip_line = None;
        }
        hit
    }

    /// Stops the VM before `line` until the browser continues, steps or
    /// stops. The caller keeps its frames and stack intact while blocked.
    fn wait_paused(&self, mut view: DebugView) {
        let mut inner = self.lock();
        if !inner.enabled || !inner.paused {
            return;
        }
        view.enabled = true;
        view.node = inner.node;
        view.paused = true;
        view.breakpoints = inner.breakpoints.iter().copied().collect();
        inner.view = view;
        inner.view_ready = true;
        inner.last_poll = Instant::now();
        while inner.enabled && inner.paused {
            let (next, _) = self
                .cv
                .wait_timeout(inner, Duration::from_millis(200))
                .unwrap_or_else(|e| e.into_inner());
            inner = next;
            if inner.enabled && inner.paused && inner.last_poll.elapsed() > CLIENT_TIMEOUT {
                inner.enabled = false;
                inner.paused = false;
                inner.view_ready = false;
                inner.view.paused = false;
                inner.view.note = Some("debugger timed out (no browser poll)".to_string());
                self.active.store(false, Ordering::Release);
                self.target.store(0, Ordering::Release);
                break;
            }
        }
    }
}

fn controller() -> &'static Arc<DebugControl> {
    static CONTROL: OnceLock<Arc<DebugControl>> = OnceLock::new();
    CONTROL.get_or_init(|| Arc::new(DebugControl::new()))
}

/// Whether the VM for `node` should consider debug checks at all.
pub fn enabled_for(node: u64) -> bool {
    controller().armed_for(node)
}

/// True when the VM should park before `line`.
pub fn should_pause(node: u64, line: u32) -> bool {
    controller().should_pause(node, line)
}

/// Park the VM; returns after Continue/Step/Stop.
pub fn wait_paused(view: DebugView) {
    controller().wait_paused(view);
}

pub fn arm(node: u64, lines: Vec<u32>) -> DebugView {
    controller().arm(node, lines)
}

pub fn set_breakpoints(lines: Vec<u32>) -> DebugView {
    controller().set_breakpoints(lines)
}

pub fn pause() -> DebugView {
    controller().pause()
}

pub fn resume(step: bool) -> DebugView {
    controller().resume(step)
}

pub fn stop() -> DebugView {
    controller().stop()
}

pub fn view() -> DebugView {
    controller().view()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn unique_id() -> u64 {
        9_000_000 + std::process::id() as u64
    }

    #[test]
    fn breakpoints_skip_the_rest_of_a_hit_line() {
        let id = unique_id();
        let control = DebugControl::new();
        control.arm(id, vec![3]);
        assert!(control.should_pause(id, 3));
        assert!(!control.should_pause(id, 3), "same line only fires once");
        assert!(!control.should_pause(id, 4));
        assert!(
            control.should_pause(id, 3),
            "a later visit fires the breakpoint again"
        );
        control.stop();
    }

    #[test]
    fn stepping_waits_for_the_next_line() {
        let id = unique_id() + 1;
        let control = DebugControl::new();
        control.arm(id, Vec::new());
        control.pause();
        assert!(control.should_pause(id, 2));
        let view = control.view();
        assert_eq!(view.line, Some(2));
        control.resume(true);
        assert!(!control.should_pause(id, 2));
        assert!(control.should_pause(id, 3));
        let pending = control.view();
        assert_eq!(pending.line, Some(3));
        assert!(
            !pending.paused,
            "a hit is not public until the VM publishes locals and stack"
        );
        control.stop();
    }

    fn paused_at(line: u32) -> DebugView {
        for _ in 0..400 {
            let view = super::view();
            if view.paused && view.line == Some(line) {
                return view;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        panic!("no pause at line {line}: {:?}", super::view());
    }

    #[test]
    fn vm_pauses_steps_and_reports_locals() {
        let id = unique_id() + 3;
        stop();
        let script =
            crate::script::compile("fn add(a) {\n    let b = a + 1;\n    print(b);\n}\nadd(4);\n")
                .expect("compile");
        let mut vm = crate::script::Vm::new(script);
        vm.node_id = id;
        arm(id, vec![2]);
        let runner = std::thread::spawn(move || {
            vm.run().map_err(|error| error.to_string())?;
            Ok::<_, String>(vm.output)
        });

        let view = paused_at(2);
        assert_eq!(view.function.as_deref(), Some("add"));
        let stack: Vec<&str> = view.stack.iter().map(|f| f.function.as_str()).collect();
        assert_eq!(stack, vec!["main", "add"]);
        assert!(view.locals.iter().any(|v| v.name == "a" && v.value == "4"));
        assert!(!view.locals.iter().any(|v| v.name == "b"));

        resume(true);
        let view = paused_at(3);
        assert!(view.locals.iter().any(|v| v.name == "b" && v.value == "5"));

        resume(false);
        let output = runner.join().expect("vm thread").expect("run");
        assert_eq!(output, vec!["5".to_string()]);
        stop();
        assert!(!super::view().enabled);
    }

    #[test]
    fn stop_releases_a_parked_vm() {
        let id = unique_id() + 2;
        let control = Arc::new(DebugControl::new());
        control.arm(id, vec![1]);
        assert!(control.should_pause(id, 1));
        let parked = control.clone();
        let waiter = std::thread::spawn(move || {
            parked.wait_paused(DebugView {
                line: Some(1),
                ..DebugView::default()
            });
        });
        std::thread::sleep(Duration::from_millis(50));
        control.stop();
        waiter.join().unwrap();
        assert!(!control.view().enabled);
    }
}
