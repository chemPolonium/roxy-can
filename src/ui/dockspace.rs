use crate::app::{STATUSBAR_H, TOOLBAR_H};
use imgui::{Condition, Ui, WindowFlags};

/// Invisible host window providing a dock space between the toolbar and the
/// status bar; other windows can be dragged onto its edges to snap/dock.
pub fn render(ui: &Ui) {
    let io = ui.io();
    let w = io.display_size[0];
    let h = (io.display_size[1] - TOOLBAR_H - STATUSBAR_H).max(50.0);
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
    let zero_pad = ui.push_style_var(imgui::StyleVar::WindowPadding([0.0, 0.0]));
    ui.window("##dockspace_host")
        .flags(flags)
        .position([0.0, TOOLBAR_H], Condition::Always)
        .size([w, h], Condition::Always)
        .build(|| unsafe {
            let id = imgui::sys::igGetID_Str(c"##dockspace".as_ptr());
            imgui::sys::igDockSpace(
                id,
                imgui::sys::ImVec2::new(0.0, 0.0),
                imgui::sys::ImGuiDockNodeFlags_None as i32,
                std::ptr::null(),
            );
        });
    zero_pad.pop();
}
