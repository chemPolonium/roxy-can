pub mod buses;
pub mod data;
pub mod desktops;
pub mod dockspace;
pub mod graphics;
pub mod help;
pub mod idfilter;
pub mod measurement;
pub mod messages;
pub mod network;
pub mod project_modal;
pub mod siglist;
pub mod stats;
pub mod statusbar;
pub mod toolbar;
pub mod trace;
pub mod tx;

use crate::app::App;
use crate::can::frame::FrameFlags;
use imgui::Ui;

/// Consistent RGBA tint for the frame-type tag rendered in Trace / Messages /
/// Statistics. Error frames are red, remote requests violet, FD cyan (orange
/// when ESI is set — the sender is reporting error-passive), and classic data
/// frames are a muted grey so they recede into the row.
pub(crate) fn flags_color(flags: FrameFlags) -> [f32; 4] {
    if flags.contains(FrameFlags::ERROR) {
        [1.0, 0.35, 0.35, 1.0]
    } else if flags.contains(FrameFlags::RTR) {
        [0.75, 0.55, 1.0, 1.0]
    } else if !flags.contains(FrameFlags::FD) {
        [0.4, 0.45, 0.5, 1.0]
    } else if flags.contains(FrameFlags::ESI) {
        [1.0, 0.55, 0.2, 1.0]
    } else {
        [0.35, 0.85, 1.0, 1.0]
    }
}

pub fn render(app: &mut App, ui: &Ui) {
    dockspace::render(ui);
    toolbar::render(app, ui);
    trace::render(app, ui);
    messages::render(app, ui);
    stats::render(app, ui);
    measurement::render(app, ui);
    idfilter::render(app, ui);
    buses::render(app, ui);
    tx::render(app, ui);
    network::render(app, ui);
    data::render(app, ui);
    graphics::render(app, ui);
    statusbar::render(app, ui);
    desktops::render(app, ui);
    project_modal::render(app, ui);
    help::render(app, ui);
}
