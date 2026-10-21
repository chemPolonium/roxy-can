use crate::app::{App, PopupTarget};
use crate::ui::flags_color;
use crate::ui::idfilter::scope_combo;
use crate::workspace::StatsWin;
use dear_imgui_rs::{Condition, TableColumnFlags, TableFlags, TableSizingPolicy, Ui};

pub enum StatsCmd {
    Export(usize),
    OpenPicker(usize),
    ClearFocus,
}

pub fn render(app: &mut App, ui: &Ui) {
    let mut wins = std::mem::take(&mut app.stats_windows);
    let mut cmds = Vec::new();
    for (i, win) in wins.iter_mut().enumerate() {
        win.show(ui, app, i, &mut cmds);
    }
    app.stats_windows = wins;
    app.apply_stats_cmds(cmds);
}

impl StatsWin {
    pub fn show(&mut self, ui: &Ui, app: &App, i: usize, cmds: &mut Vec<StatsCmd>) {
        if !self.opened {
            return;
        }
        let io = ui.io();
        let mut open = true;
        let raw = self.name.clone();
        let title = if raw.trim().is_empty() {
            format!("Message Statistics {}", i + 1)
        } else {
            raw
        };
        let off = i as f32 * 30.0;
        let focus = app.focus_title.as_deref() == Some(title.as_str());
        if focus {
            cmds.push(StatsCmd::ClearFocus);
        }
        let mut window = ui
            .window(format!("{title}###stats{i}"))
            .opened(&mut open)
            .position(
                [
                    io.display_size()[0] * 0.6 + off,
                    io.display_size()[1] * 0.45 + off,
                ],
                Condition::FirstUseEver,
            )
            .size(
                [640.0, io.display_size()[1] * 0.42],
                Condition::FirstUseEver,
            );
        if focus {
            window = window.focused(true);
        }
        window.build(|| self.window_content(app, ui, i, cmds));
        self.opened = open;
    }

    fn window_content(&mut self, app: &App, ui: &Ui, i: usize, cmds: &mut Vec<StatsCmd>) {
        let mut open_picker = false;
        let new_scope = scope_combo(
            app,
            ui,
            &format!("##sscope{i}"),
            self.scope,
            PopupTarget::Stats(i),
            &mut open_picker,
        );
        if open_picker {
            cmds.push(StatsCmd::OpenPicker(i));
        }
        self.scope = new_scope;
        ui.same_line();
        if ui.button(format!("Export##sx{i}")) {
            cmds.push(StatsCmd::Export(i));
        }
        ui.separator();

        // The rows were refreshed on the text gate before the draw; this path
        // only reads them.
        ui.text(&self.text_header);
        ui.separator();

        // NO_BORDERS_IN_BODY restricts column-resize dragging to the header row.
        let tbl_flags = TableFlags::BORDERS_INNER
            | TableFlags::ROW_BG
            | TableFlags::RESIZABLE
            | TableFlags::NO_BORDERS_IN_BODY
            | TableFlags::SCROLL_Y;
        let opts = dear_imgui_rs::TableOptions::from(tbl_flags)
            .sizing_policy(TableSizingPolicy::StretchProp);
        let Some(_table) = ui.begin_table_with_flags(format!("stats_table{i}"), 9, opts) else {
            return;
        };
        ui.table_setup_column_stretch_weight("Message", TableColumnFlags::NONE, 1.0);
        for (label, w) in [
            ("Bus", 60.0),
            ("Count", 55.0),
            // "{:.2}" milliseconds: ~7 chars
            ("Min(ms)", 62.0),
            ("Avg(ms)", 62.0),
            ("Max(ms)", 62.0),
            ("Len", 36.0),
            ("Flags", 44.0),
            ("Share", 52.0),
        ] {
            ui.table_setup_column_fixed_width(label, TableColumnFlags::NONE, w);
        }
        ui.table_setup_scroll_freeze(0, 1);
        ui.table_headers_row();

        for row in &self.text_rows {
            ui.table_next_row();
            if !ui.table_next_column() {
                continue;
            }
            ui.text(&row.label);
            ui.table_next_column();
            ui.text(&row.bus);
            ui.table_next_column();
            ui.text(&row.count);
            ui.table_next_column();
            ui.text(&row.min);
            ui.table_next_column();
            ui.text(&row.avg);
            ui.table_next_column();
            ui.text(&row.max);
            ui.table_next_column();
            ui.text(&row.len);
            ui.table_next_column();
            ui.text_colored(flags_color(row.flags), row.flags.tag());
            ui.table_next_column();
            ui.text(&row.share);
        }
    }
}

impl App {
    pub(crate) fn apply_stats_cmds(&mut self, cmds: Vec<StatsCmd>) {
        for cmd in cmds {
            match cmd {
                StatsCmd::Export(i) => self.export_stats_dialog(i),
                StatsCmd::OpenPicker(i) => {
                    self.id_filter.open_for(PopupTarget::Stats(i));
                }
                StatsCmd::ClearFocus => self.focus_title = None,
            }
        }
    }
}
