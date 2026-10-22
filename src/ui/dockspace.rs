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
    let zero_pad = ui.push_style_var(StyleVar::WindowPadding([0.0, 0.0]));
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
            if dock_default {
                apply_default_layout(id, w, h, toolbar_h, &names);
                applied = true;
            }
        });
    zero_pad.pop();
    if applied {
        app.dock_default_pending = false;
        app.dock_focus_pending = true;
    }
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
