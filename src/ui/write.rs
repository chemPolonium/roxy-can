//! The Write window: the system-wide output panel in the CANoe mold.
//! Node prints, static checks, timer/sysvar advisories and command news
//! all land in one scrolling log on the bus core; this window only
//! shapes and displays it.

use crate::app::App;
use crate::bus::WriteKind;
use dear_imgui_rs::{Condition, StyleColor, StyleVar, Ui};

/// The Write window's own state: open flag and per-kind visibility. Drawing
/// is read-only against [`App`]; the two actions that touch the core or the
/// filesystem come back as commands.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WriteView {
    pub open: bool,
    pub filter: [bool; 4],
}

impl Default for WriteView {
    fn default() -> Self {
        Self {
            open: true,
            filter: [true; 4],
        }
    }
}

/// What the Write window asks for. `Clear` reaches the core's log ring;
/// `Export` opens the native save dialog.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WriteCmd {
    Clear,
    Export,
}

/// `wall_us` (local microseconds since midnight) as `HH:MM:SS.mmm`.
/// Shared with the Write log's text export so the file lines match the
/// window's.
pub(crate) fn wall_stamp(us: u64) -> String {
    let s = us / 1_000_000;
    let ms = (us % 1_000_000) / 1_000;
    format!(
        "[{:02}:{:02}:{:02}.{:03}]",
        s / 3_600,
        (s % 3_600) / 60,
        s % 60,
        ms
    )
}

pub fn render(app: &mut App, ui: &Ui) {
    let mut view = std::mem::take(&mut app.write_view);
    let cmds = view.show(ui, app);
    app.write_view = view;
    app.apply_write_cmds(cmds);
}

impl WriteView {
    /// Draw the window and collect its two actions. `app` is read-only: the
    /// filter checkboxes live in `self`, and nothing here mutates the bus.
    pub fn show(&mut self, ui: &Ui, app: &App) -> Vec<WriteCmd> {
        let mut cmds = Vec::new();
        if !self.open {
            return cmds;
        }
        let io = ui.io();
        let mut open = self.open;
        let min = ui.push_style_var(StyleVar::WindowMinSize([420.0, 160.0]));
        ui.window("Write")
            .opened(&mut open)
            .position(
                [io.display_size()[0] * 0.55, io.display_size()[1] * 0.35],
                Condition::FirstUseEver,
            )
            .size([640.0, 280.0], Condition::FirstUseEver)
            .build(|| content(self, app, ui, &mut cmds));
        min.pop();
        self.open = open;
        cmds
    }
}

impl App {
    pub(crate) fn apply_write_cmds(&mut self, cmds: Vec<WriteCmd>) {
        for cmd in cmds {
            match cmd {
                WriteCmd::Clear => self.send(crate::bus::BusCommand::ClearWrite),
                WriteCmd::Export => self.export_write_dialog(),
            }
        }
    }
}

fn content(view: &mut WriteView, app: &App, ui: &Ui, cmds: &mut Vec<WriteCmd>) {
    if ui.button("Clear") {
        cmds.push(WriteCmd::Clear);
    }
    ui.same_line();
    if ui.button("Export") {
        cmds.push(WriteCmd::Export);
    }
    if ui.is_item_hovered() {
        ui.tooltip_text("Export the current filtered view as a text file");
    }
    ui.same_line();
    ui.align_text_to_frame_padding();
    ui.text_disabled("Kind:");
    // Per-kind visibility; the order mirrors WriteKind's variants.
    const LABELS: [&str; 4] = ["Script", "Info", "Warning", "Error"];
    for (slot, label) in view.filter.iter_mut().zip(LABELS) {
        ui.same_line();
        ui.checkbox(label, slot);
    }
    ui.same_line();
    ui.align_text_to_frame_padding();
    ui.text_disabled("script output, checks, system events");

    // Stick to the bottom while the user hasn't scrolled up to read
    // something: arrivals keep appending below otherwise.
    let at_bottom = ui.scroll_y() + 2.0 >= ui.scroll_max_y();
    let want = |k: &WriteKind| -> bool {
        let i = match k {
            WriteKind::Script => 0,
            WriteKind::Info => 1,
            WriteKind::Warning => 2,
            WriteKind::Error => 3,
        };
        view.filter[i]
    };
    let lines: Vec<&crate::bus::WriteLine> =
        app.snap.write.iter().filter(|l| want(&l.kind)).collect();
    for line in lines {
        let color = match line.kind {
            WriteKind::Error => [1.0, 0.55, 0.3, 1.0],
            WriteKind::Warning => [1.0, 0.8, 0.4, 1.0],
            WriteKind::Info => [0.55, 0.75, 1.0, 1.0],
            WriteKind::Script => [1.0, 1.0, 1.0, 1.0],
        };
        ui.text_colored([0.45, 0.45, 0.45, 1.0], wall_stamp(line.wall_us));
        ui.same_line();
        // `text_wrapped` is the API for this: it pushes the wrap position at
        // the window's work-rect edge itself. Hand-computing one from the
        // cursor and the available width put a screen coordinate where the
        // binding wants a window-local one, and the middle of the line was
        // drawn off-screen -- the message looked truncated mid-word while the
        // tail sat on a second line. Colour rides a style push because there is
        // no coloured variant of the wrapped call.
        let color_tok = ui.push_style_color(StyleColor::Text, color);
        ui.text_wrapped(&line.text);
        color_tok.pop();
    }
    if at_bottom && ui.scroll_max_y() > 0.0 {
        ui.set_scroll_here_y(1.0);
    }
}
