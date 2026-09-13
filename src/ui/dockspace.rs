use crate::app::{STATUSBAR_H, TABSTRIP_H, TOOLBAR_H};
use dear_imgui_rs::{Condition, StyleVar, Ui, WindowFlags};

/// Invisible host window providing a dock space between the toolbar and the
/// desktop tab strip; other windows can be dragged onto its edges to dock.
///
/// The dock space id is derived exactly like the imgui-rs build derived it
/// (string id inside this host window), so dock layouts persisted in project
/// files keep resolving to the same nodes across the binding migration.
pub fn render(ui: &Ui) {
    let io = ui.io();
    let w = io.display_size()[0];
    let h = (io.display_size()[1] - TOOLBAR_H - STATUSBAR_H - TABSTRIP_H).max(50.0);
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
    ui.window("##dockspace_host")
        .flags(flags)
        .position([0.0, TOOLBAR_H], Condition::Always)
        .size([w, h], Condition::Always)
        .build(|| unsafe {
            let id = dear_imgui_rs::sys::igGetID_Str(c"##dockspace".as_ptr());
            dear_imgui_rs::sys::igDockSpace(
                id,
                dear_imgui_rs::sys::ImVec2 { x: 0.0, y: 0.0 },
                dear_imgui_rs::sys::ImGuiDockNodeFlags_None,
                std::ptr::null(),
            );
        });
    zero_pad.pop();
}
