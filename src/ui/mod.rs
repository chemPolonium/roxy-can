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
use imgui::Ui;

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
