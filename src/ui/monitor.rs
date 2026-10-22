//! The Monitor window: the lightweight big-screen panel. One text row per
//! watched signal -- label, live physical value, optional coloring rule
//! (`>=`/`<=` a threshold repaints the row in a palette color). No plots,
//! no tables: rows a user glances at from across the lab.

use crate::app::{App, MonitorRow, PALETTE};
use crate::observe::SigKey;
use dear_imgui_rs::{Condition, StyleVar, Ui};

/// The Monitor's rows and open flag. Row edits (rule, threshold, direction,
/// digits) are UI/project state and happen in place; only adding/removing a
/// signal touches the bus, so those come back as commands.
#[derive(Default)]
pub struct MonitorView {
    pub open: bool,
    pub rows: Vec<MonitorRow>,
}

pub enum MonitorCmd {
    OpenPicker,
    Clear,
    Remove(SigKey),
}

pub fn render(app: &mut App, ui: &Ui) {
    let mut view = std::mem::take(&mut app.monitor_view);
    let cmds = view.show(ui, app);
    app.monitor_view = view;
    app.apply_monitor_cmds(cmds);
}

impl MonitorView {
    pub fn show(&mut self, ui: &Ui, app: &App) -> Vec<MonitorCmd> {
        let mut cmds = Vec::new();
        if !self.open {
            return cmds;
        }
        let io = ui.io();
        let mut open = self.open;
        let min = ui.push_style_var(StyleVar::WindowMinSize([360.0, 160.0]));
        // Rule controls reserve 268 px on the right; 420 left too little for
        // a signal name plus its value before the controls began.
        let (pos, size) = crate::ui::fit_first_use(
            io.display_size(),
            [io.display_size()[0] * 0.45, io.display_size()[1] * 0.12],
            [520.0, 360.0],
        );
        ui.window("Monitor")
            .opened(&mut open)
            .position(pos, Condition::FirstUseEver)
            .size(size, Condition::FirstUseEver)
            .build(|| self.content(app, ui, &mut cmds));
        min.pop();
        self.open = open;
        cmds
    }

    fn content(&mut self, app: &App, ui: &Ui, cmds: &mut Vec<MonitorCmd>) {
        if ui.button("+ Add signal") {
            cmds.push(MonitorCmd::OpenPicker);
        }
        if ui.is_item_hovered() {
            ui.tooltip_text(
                "Pick a row from the signal picker (database, derived, or system variable)",
            );
        }
        ui.same_line();
        if ui.button("Clear") {
            cmds.push(MonitorCmd::Clear);
        }
        ui.separator();

        for i in 0..self.rows.len() {
            let key = self.rows[i].key.clone();
            let sub = app.sub_view(&key);
            let (value, unit) = match sub {
                Some(s) => (s.latest, s.unit.clone()),
                None => (0.0, String::new()),
            };
            let (label, digits, rule_on, rising, threshold, color) = {
                let row = &self.rows[i];
                (
                    row.label.clone(),
                    row.digits,
                    row.rule_on,
                    row.rising,
                    row.threshold,
                    row.color,
                )
            };
            // Coloring: a rule that holds repaints the whole row in its
            // palette color; the default text color otherwise.
            let hot =
                rule_on && ((rising && value >= threshold) || (!rising && value <= threshold));
            let text_color = if hot {
                PALETTE[color % PALETTE.len()]
            } else {
                [1.0, 1.0, 1.0, 1.0]
            };
            let chip = if hot {
                PALETTE[color % PALETTE.len()]
            } else {
                [0.35, 0.35, 0.4, 1.0]
            };
            let dl = ui.get_window_draw_list();
            let p = ui.cursor_screen_pos();
            dl.add_rect([p[0], p[1] + 3.0], [p[0] + 6.0, p[1] + 17.0], chip)
                .filled(true)
                .build();
            ui.dummy([10.0, 0.0]);
            ui.same_line();
            ui.text_colored(text_color, &label);
            ui.same_line();
            ui.text_colored(text_color, format!("{:.*} {}", digits, value, unit));

            self.rule_controls(ui, i, cmds);
        }
        if self.rows.is_empty() {
            ui.text_disabled("no rows -- add a signal to watch");
        }
    }

    /// The per-row right-aligned controls: rule on/off, threshold, direction,
    /// color cycle, digits, remove.
    fn rule_controls(&mut self, ui: &Ui, i: usize, cmds: &mut Vec<MonitorCmd>) {
        let width = ui.content_region_avail()[0];
        ui.same_line_with_pos(width - 268.0);
        // The label is a state, so two rows agree on it -- the suffix keeps them on
        // separate IDs.
        let toggle = if self.rows[i].rule_on { "on" } else { "off" };
        if ui.button(format!("{toggle}##mrule{i}")) {
            self.rows[i].rule_on = !self.rows[i].rule_on;
        }
        if ui.is_item_hovered() {
            ui.tooltip_text("Toggle the coloring rule");
        }
        ui.same_line();
        ui.set_next_item_width(66.0);
        let mut th = self.rows[i].threshold as f32;
        if ui
            .input_float_config(format!("##mthr{i}"))
            .display_format(dear_imgui_rs::NumericFormat::new("%g").expect("static format"))
            .build(&mut th)
        {
            self.rows[i].threshold = th as f64;
        }
        if ui.is_item_hovered() {
            ui.tooltip_text("Color threshold");
        }
        ui.same_line();
        let dir = if self.rows[i].rising { ">=" } else { "<=" };
        if ui.button(format!("{dir}##mdir{i}")) {
            self.rows[i].rising = !self.rows[i].rising;
        }
        if ui.is_item_hovered() {
            ui.tooltip_text("Flip the comparison direction");
        }
        ui.same_line();
        let digits = self.rows[i].digits;
        if ui.button(format!("{digits}##mdig{i}")) {
            self.rows[i].digits = (digits + 1) % 4;
        }
        if ui.is_item_hovered() {
            ui.tooltip_text("Decimal places");
        }
        ui.same_line();
        if ui.button(format!("x##mrm{i}")) {
            cmds.push(MonitorCmd::Remove(self.rows[i].key.clone()));
        }
    }
}

impl App {
    pub(crate) fn apply_monitor_cmds(&mut self, cmds: Vec<MonitorCmd>) {
        for cmd in cmds {
            match cmd {
                MonitorCmd::OpenPicker => {
                    self.id_filter
                        .open_for(crate::workspace::PopupTarget::Monitor);
                }
                MonitorCmd::Clear => {
                    let keys: Vec<SigKey> = self
                        .monitor_view
                        .rows
                        .iter()
                        .map(|r| r.key.clone())
                        .collect();
                    for key in keys {
                        self.set_win_signal(crate::workspace::PopupTarget::Monitor, key, false);
                    }
                }
                MonitorCmd::Remove(key) => {
                    self.set_win_signal(crate::workspace::PopupTarget::Monitor, key, false);
                }
            }
        }
    }
}
