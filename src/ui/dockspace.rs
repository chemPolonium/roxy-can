use crate::app::{App, STATUSBAR_H, TABSTRIP_H};
use dear_imgui_rs::{Condition, StyleVar, Ui, WindowFlags, sys};
use std::ffi::CString;

/// Invisible host window providing a dock space between the toolbar and the
/// desktop tab strip; other windows can be dragged onto its edges to dock.
///
/// The dock space id is derived exactly like the imgui-rs build derived it
/// (string id inside this host window), so dock layouts persisted in project
/// files keep resolving to the same nodes across the binding migration.
pub fn render(app: &mut App, ui: &Ui) {
    let io = ui.io();
    let w = io.display_size()[0];
    let toolbar_h = crate::ui::toolbar_height();
    let h = (io.display_size()[1] - toolbar_h - STATUSBAR_H - TABSTRIP_H).max(50.0);
    let dock_default = app.dock_default_pending;
    if app.dock_focus_pending {
        // The layout was built last frame, so the window now exists and can
        // claim its tab; docking alone leaves the last-drawn tab selected.
        unsafe { sys::igSetWindowFocus_Str(c"Measurement Setup".as_ptr()) };
        app.dock_focus_pending = false;
    }
    let names = DefaultWindows::from_app(app);
    let flags = WindowFlags::NO_TITLE_BAR
        | WindowFlags::NO_RESIZE
        | WindowFlags::NO_MOVE
        | WindowFlags::NO_COLLAPSE
        | WindowFlags::NO_SCROLLBAR
        | WindowFlags::NO_SCROLL_WITH_MOUSE
        | WindowFlags::NO_SAVED_SETTINGS
        | WindowFlags::NO_BACKGROUND
        | WindowFlags::NO_FOCUS_ON_APPEARING
        | WindowFlags::NO_BRING_TO_FRONT_ON_FOCUS
        | WindowFlags::NO_NAV
        | WindowFlags::NO_DOCKING;
    let empty = !any_window_open(app);
    let zero_pad = ui.push_style_var(StyleVar::WindowPadding([0.0, 0.0]));
    // The host's border would stack with the toolbar's and the desktop
    // strip's, leaving stray gray lines around the dock area.
    let no_border = ui.push_style_var(StyleVar::WindowBorderSize(0.0));
    let mut applied = false;
    ui.window("##dockspace_host")
        .flags(flags)
        .position([0.0, toolbar_h], Condition::Always)
        .size([w, h], Condition::Always)
        .build(|| unsafe {
            let id = dear_imgui_rs::sys::igGetID_Str(c"##dockspace".as_ptr());
            dear_imgui_rs::sys::igDockSpace(
                id,
                dear_imgui_rs::sys::ImVec2 { x: 0.0, y: 0.0 },
                dear_imgui_rs::sys::ImGuiDockNodeFlags_None,
                std::ptr::null(),
            );
            // With no windows open there is nothing to arrange; keep the
            // first-use layout pending so the first window opened from the
            // start overlay still lands in the default arrangement.
            if dock_default && !empty {
                apply_default_layout(id, w, h, toolbar_h, &names);
                applied = true;
            }
        });
    no_border.pop();
    zero_pad.pop();
    if applied {
        app.dock_default_pending = false;
        app.dock_focus_pending = true;
    }
    if empty {
        match empty_state(ui, toolbar_h, w, h) {
            Some(EmptyStateCmd::MeasurementSetup) => app.measurement_view.open = true,
            Some(EmptyStateCmd::NewTrace) => app.new_trace_window(),
            Some(EmptyStateCmd::Network) => app.network_view.open = true,
            Some(EmptyStateCmd::Buses) => app.buses_view.open = true,
            None => {}
        }
    }
}

enum EmptyStateCmd {
    MeasurementSetup,
    NewTrace,
    Network,
    Buses,
}

/// Whether this desktop has any content window to show. Popups (Message
/// Selection, Help) do not count: the empty workspace needs its own entry
/// points instead of a blank black dock.
fn any_window_open(app: &App) -> bool {
    app.trace_windows.iter().any(|w| w.opened)
        || app.msg_windows.iter().any(|w| w.opened)
        || app.stats_windows.iter().any(|w| w.opened)
        || app.graphics.iter().any(|w| w.opened)
        || app.data_windows.iter().any(|w| w.opened)
        || app.state_trackers.iter().any(|w| w.opened)
        || app.measurement_view.open
        || app.network_view.open
        || app.write_view.open
        || app.generator_view.open
        || app.buses_view.open
        || app.triggers_view.open
        || app.busstats_view.open
        || app.spec_view.open
        || app.sysvars_view.open
        || app.monitor_view.open
        || !app.script_editor.open.is_empty()
}

/// A closed desktop is not an error: offer the windows an operator needs to
/// get back to work instead of leaving an empty dock with no explanation.
/// This is a separate overlay window because widgets submitted after
/// `DockSpace` inside the host are clipped by the dock node's draw pass.
fn empty_state(ui: &Ui, toolbar_h: f32, w: f32, h: f32) -> Option<EmptyStateCmd> {
    const BOX_W: f32 = 520.0;
    let mut cmd = None;
    let x = ((w - BOX_W) * 0.5).max(20.0);
    let y = toolbar_h + (h * 0.5 - 44.0).max(20.0);
    let flags = WindowFlags::NO_TITLE_BAR
        | WindowFlags::NO_RESIZE
        | WindowFlags::NO_MOVE
        | WindowFlags::NO_COLLAPSE
        | WindowFlags::NO_SCROLLBAR
        | WindowFlags::NO_SCROLL_WITH_MOUSE
        | WindowFlags::NO_SAVED_SETTINGS
        | WindowFlags::NO_BACKGROUND
        | WindowFlags::NO_FOCUS_ON_APPEARING
        | WindowFlags::NO_NAV
        | WindowFlags::NO_DOCKING;
    let no_border = ui.push_style_var(StyleVar::WindowBorderSize(0.0));
    ui.window("##empty_state")
        .flags(flags)
        .position([x, y], Condition::Always)
        .size([BOX_W, 96.0], Condition::Always)
        .build(|| {
            ui.text_colored([0.55, 0.75, 1.0, 1.0], "No windows open on this desktop");
            ui.text_disabled("Open one from the View menu, or start here:");
            if ui.button("Measurement Setup") {
                cmd = Some(EmptyStateCmd::MeasurementSetup);
            }
            ui.same_line();
            if ui.button("New Trace") {
                cmd = Some(EmptyStateCmd::NewTrace);
            }
            ui.same_line();
            if ui.button("Network") {
                cmd = Some(EmptyStateCmd::Network);
            }
            ui.same_line();
            if ui.button("Buses") {
                cmd = Some(EmptyStateCmd::Buses);
            }
        });
    no_border.pop();
    cmd
}

/// Native names (`Title###id`) of the windows a fresh workspace seeds. Only
/// these are docked; windows the user later opens stay floating until the
/// user arranges them, and project layouts never reach this path.
struct DefaultWindows {
    trace: Option<CString>,
    messages: Option<CString>,
}

impl DefaultWindows {
    fn from_app(app: &App) -> Self {
        let trace = app
            .trace_windows
            .iter()
            .enumerate()
            .find(|(_, w)| w.opened)
            .map(|(i, w)| {
                let title = if w.name.trim().is_empty() {
                    format!("Trace {}", i + 1)
                } else {
                    w.name.clone()
                };
                native(&title, &format!("trace{i}"))
            });
        let messages = app
            .msg_windows
            .iter()
            .enumerate()
            .find(|(_, w)| w.opened)
            .map(|(i, w)| {
                let title = if w.name.trim().is_empty() {
                    format!("Messages {}", i + 1)
                } else {
                    w.name.clone()
                };
                native(&title, &format!("msgs{i}"))
            });
        Self { trace, messages }
    }
}

fn native(title: &str, id: &str) -> CString {
    CString::new(format!("{title}###{id}")).expect("window titles cannot contain NUL")
}

/// First-use arrangement: observers across the top, the topology and consoles
/// tabbed along the bottom. The previous defaults were six floating rectangles
/// whose percentage positions overlapped on a 1280x800 display, so Trace opened
/// underneath Network and Measurement's + buttons were clipped. Network keeps a
/// full-width tab because its tree pane is a fixed 320px: a side column would
/// leave nothing for the node details.
unsafe fn apply_default_layout(
    root: sys::ImGuiID,
    w: f32,
    h: f32,
    toolbar_h: f32,
    names: &DefaultWindows,
) {
    let bottom_h = (h * 0.30).clamp(180.0, 340.0).min(h * 0.5);
    unsafe {
        sys::igDockBuilderSetNodePos(
            root,
            sys::ImVec2 {
                x: 0.0,
                y: toolbar_h,
            },
        );
        sys::igDockBuilderSetNodeSize(root, sys::ImVec2 { x: w, y: h });
        let mut bottom = 0;
        let mut top = 0;
        sys::igDockBuilderSplitNode(
            root,
            sys::ImGuiDir_Down,
            bottom_h / h,
            &mut bottom,
            &mut top,
        );
        for name in [
            c"Measurement Setup",
            c"Network",
            c"Write",
            c"Interactive Generator",
        ] {
            sys::igDockBuilderDockWindow(name.as_ptr(), bottom);
        }
        if let Some(name) = &names.messages {
            sys::igDockBuilderDockWindow(name.as_ptr(), top);
        }
        if let Some(name) = &names.trace {
            sys::igDockBuilderDockWindow(name.as_ptr(), top);
        }
        sys::igDockBuilderFinish(root);
    }
}
