pub mod data;
pub mod dockspace;
pub mod graphics;
pub mod messages;
pub mod siglist;
pub mod statusbar;
pub mod symbols;
pub mod toolbar;
pub mod trace;

use crate::app::App;
use imgui::Ui;

pub fn render(app: &mut App, ui: &Ui) {
    dockspace::render(ui);
    toolbar::render(app, ui);
    trace::render(app, ui);
    messages::render(app, ui);
    data::render(app, ui);
    graphics::render(app, ui);
    statusbar::render(app, ui);
}
