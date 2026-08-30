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
pub mod spec;
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
    spec::render(app, ui);
    data::render(app, ui);
    graphics::render(app, ui);
    statusbar::render(app, ui);
    desktops::render(app, ui);
    project_modal::render(app, ui);
    help::render(app, ui);
}

/// A generator number held while its widget is being edited, so the model is
/// written once at the end instead of on every frame.
///
/// An imgui drag reports a change on each keystroke of the inline text box its
/// double-click opens, and every one of those used to go straight into the
/// transmit schedule: dialing in 100 sent at 1 ms, then 10 ms. Only one widget
/// can be edited at a time, so a single slot serves the whole tool; `key` keeps
/// the edits from leaking into each other.
#[derive(Default)]
pub(crate) struct Draft {
    key: String,
    value: f64,
    open: bool,
}

impl Draft {
    /// What the widget should display: the draft while it owns `key`, so a
    /// dragged handle does not snap back to the untouched model.
    pub(crate) fn shown(&self, key: &str, model: f64) -> f64 {
        if self.open && self.key == key {
            self.value
        } else {
            model
        }
    }

    /// One frame of the machine. `changed` is what the widget returned;
    /// `ended_edited` and `ended` are imgui's two deactivate flags for it.
    /// Returns the value to write into the model, at most once per edit.
    pub(crate) fn step(
        &mut self,
        key: &str,
        value: f64,
        changed: bool,
        ended_edited: bool,
        ended: bool,
    ) -> Option<f64> {
        let mine = self.open && self.key == key;
        if mine && ended_edited {
            self.open = false;
            // A release can both change and end in the same frame; take that
            // one over the parked value from the frame before.
            return Some(if changed { value } else { self.value });
        }
        if changed {
            self.open = true;
            self.key = key.to_string();
            self.value = value;
        }
        if self.open && self.key == key && ended {
            // Escape, or a press that moved nothing: nothing was edited, so
            // nothing reaches the model.
            self.open = false;
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::Draft;

    /// The whole point of the draft: a moving handle must not touch the model,
    /// or a half-typed number would go out on the bus.
    #[test]
    fn editing_previews_without_touching_the_model() {
        let mut d = Draft::default();
        assert_eq!(d.step("sig", 1.0, true, false, false), None);
        assert_eq!(d.step("sig", 10.0, true, false, false), None);
        assert_eq!(d.shown("sig", 133.0), 10.0, "the handle rides the draft");
        assert_eq!(d.shown("other", 133.0), 133.0, "not this widget's business");
    }

    /// imgui broadcasts the deactivate-after-edit flag on the frame after the
    /// last change, and that single frame is what writes the model.
    #[test]
    fn the_number_lands_once_when_the_edit_ends() {
        let mut d = Draft::default();
        d.step("sig", 100.0, true, false, false);
        assert_eq!(
            d.step("sig", 100.0, false, true, true),
            Some(100.0),
            "committed on the end frame"
        );
        assert_eq!(
            d.step("sig", 100.0, false, true, true),
            None,
            "and never a second time"
        );
        assert_eq!(d.shown("sig", 133.0), 133.0, "released back to the model");
    }

    /// A release that both moved and ended still has to commit, exactly once.
    #[test]
    fn a_drag_released_mid_change_still_commits() {
        let mut d = Draft::default();
        d.step("sig", 42.0, true, false, false);
        assert_eq!(d.step("sig", 43.0, true, true, true), Some(43.0));
    }

    /// Escape restores the widget without editing it, so the model must not
    /// move at all -- the value stays exactly where it was.
    #[test]
    fn an_abandoned_edit_writes_nothing() {
        let mut d = Draft::default();
        d.step("sig", 7.0, true, false, false);
        assert_eq!(
            d.step("sig", 7.0, false, false, true),
            None,
            "ended, not edited"
        );
        assert_eq!(d.shown("sig", 133.0), 133.0);
    }

    /// Merely clicking a slider is not an edit: it must not pin a driven
    /// signal, which is what writing on press used to do.
    #[test]
    fn a_press_that_changed_nothing_writes_nothing() {
        let mut d = Draft::default();
        assert_eq!(d.step("sig", 5.0, false, false, true), None);
        assert!(d.shown("sig", 5.0) == 5.0);
        assert_eq!(d.step("other", 5.0, true, false, false), None);
        assert_eq!(
            d.step("sig", 5.0, false, true, true),
            None,
            "not its edit to end"
        );
    }
}
