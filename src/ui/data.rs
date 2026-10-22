use crate::app::{App, TOOLBAR_H};
use crate::ui::siglist::{self, ListKind, SigListCmd};
use dear_imgui_rs::{Condition, TableColumnFlags, TableFlags, Ui};

const PANEL_W: f32 = 190.0;

pub enum DataCmd {
    ClearFocus,
    Sig(SigListCmd),
}

pub fn render(app: &mut App, ui: &Ui) {
    let mut wins = std::mem::take(&mut app.data_windows);
    let mut cmds = Vec::new();
    for (i, win) in wins.iter_mut().enumerate() {
        win.show(ui, app, i, &mut cmds);
    }
    app.data_windows = wins;
    app.apply_data_cmds(cmds);
}

impl crate::observe::DataWindow {
    pub fn show(&mut self, ui: &Ui, app: &App, i: usize, cmds: &mut Vec<DataCmd>) {
        if !self.opened {
            return;
        }
        let io = ui.io();
        let mut open = true;
        let raw = self.name.clone();
        let name = if raw.trim().is_empty() {
            format!("Data {}", i + 1)
        } else {
            raw
        };
        let focus = app.focus_title.as_deref() == Some(name.as_str());
        if focus {
            cmds.push(DataCmd::ClearFocus);
        }
        // The value table's fixed columns add up to ~540 px beside the
        // 190 px signal panel; the old 480 px default could not show them.
        let (pos, size) = crate::ui::fit_first_use(
            io.display_size(),
            [io.display_size()[0] - 840.0, TOOLBAR_H + i as f32 * 28.0],
            [840.0, 380.0],
        );
        let mut window = ui
            .window(format!("{name}###data{i}"))
            .opened(&mut open)
            .position(pos, Condition::FirstUseEver)
            .size(size, Condition::FirstUseEver);
        if focus {
            window = window.focused(true);
        }
        window.build(|| self.window_content(app, ui, i, cmds));
        self.opened = open;
    }

    fn window_content(&mut self, app: &App, ui: &Ui, i: usize, cmds: &mut Vec<DataCmd>) {
        let avail = ui.content_region_avail();

        // The same draggable split as the Graphics window: the panel's right border
        // resizes it, and the width is remembered with the layout. Per-window id so
        // two Data windows can differ.
        ui.child_window(format!("data_sig_panel{i}"))
            .size([PANEL_W, avail[1]])
            .child_flags(dear_imgui_rs::ChildFlags::RESIZE_X | dear_imgui_rs::ChildFlags::BORDERS)
            .flags(dear_imgui_rs::WindowFlags::HORIZONTAL_SCROLLBAR)
            .build(ui, || self.left_panel(app, ui, i, cmds));

        ui.same_line();

        ui.child_window("values_area")
            .size([0.0, avail[1]])
            .build(ui, || self.values_area(app, ui, i));
    }

    /// Left panel: the window's selected signal list; each signal can be
    /// toggled for display. Bus, node identity, and signal selection live in
    /// Measurement Setup.
    fn left_panel(&mut self, app: &App, ui: &Ui, i: usize, cmds: &mut Vec<DataCmd>) {
        ui.text("Signals");
        let mut sig_cmds = Vec::new();
        siglist::draw_view(
            app,
            ui,
            ListKind::Data(i),
            &mut self.signals,
            None,
            &mut sig_cmds,
        );
        cmds.extend(sig_cmds.into_iter().map(DataCmd::Sig));
    }

    /// Right area: value table for the visible signals, the reference Data
    /// window's column set -- physical value, unit, and raw wire value in
    /// their own columns, then a bar. The bar draws the latest value's place
    /// in the database's declared min..max; a signal without a declared range
    /// falls back to its observed one.
    ///
    /// The value/unit/raw columns draw a throttled snapshot (`sync_data_text`)
    /// so digits hold still long enough to read, while the bar -- drawn as a
    /// plain filled rect, no text inside -- animates at full frame rate.
    fn values_area(&self, app: &App, ui: &Ui, i: usize) {
        let (keys, cache) = (&self.text_keys, &self.text_cache);
        if keys.is_empty() {
            ui.text("add signals via Measurement Setup (…)");
            return;
        }
        let tbl_flags = TableFlags::BORDERS_INNER | TableFlags::ROW_BG | TableFlags::SCROLL_Y;
        if let Some(_table) = ui.begin_table_with_flags("data_table", 8, tbl_flags) {
            // Fixed widths for the text columns; the Bar column stretches and
            // takes whatever is left.
            ui.table_setup_column_fixed_width("Name", TableColumnFlags::NONE, 130.0);
            ui.table_setup_column_fixed_width("Value", TableColumnFlags::NONE, 70.0);
            ui.table_setup_column_fixed_width("Unit", TableColumnFlags::NONE, 56.0);
            ui.table_setup_column_fixed_width("Raw", TableColumnFlags::NONE, 76.0);
            ui.table_setup_column_fixed_width("Min", TableColumnFlags::NONE, 70.0);
            ui.table_setup_column_fixed_width("Avg", TableColumnFlags::NONE, 70.0);
            ui.table_setup_column_fixed_width("Max", TableColumnFlags::NONE, 70.0);
            ui.table_setup_column_stretch_weight("Bar", TableColumnFlags::NONE, 1.0);
            ui.table_setup_scroll_freeze(0, 1);
            ui.table_headers_row();
            let _ = i;
            for (key, text) in keys.iter().zip(cache.iter()) {
                let Some(sub) = app.sub_view(key) else {
                    continue;
                };
                ui.table_next_row();
                if !ui.table_next_column() {
                    continue;
                }
                ui.text(key.name());
                ui.table_next_column();
                ui.text(&text[0]);
                ui.table_next_column();
                ui.text(&text[1]);
                ui.table_next_column();
                ui.text(&text[2]);
                ui.table_next_column();
                ui.text(&text[3]);
                ui.table_next_column();
                ui.text(&text[4]);
                ui.table_next_column();
                ui.text(&text[5]);
                ui.table_next_column();
                let frac = match app.declared_range(key) {
                    Some((lo, hi)) => ((sub.latest - lo) / (hi - lo)).clamp(0.0, 1.0),
                    None if sub.max > sub.min => {
                        ((sub.latest - sub.min) / (sub.max - sub.min)).clamp(0.0, 1.0)
                    }
                    None => 0.0,
                };
                // The widget's default size is what this cell wants: the width
                // fills the column and the height is the standard frame height.
                // The overlay percentage is silenced -- the value column next
                // to it is throttled, so a live number on the bar would read
                // as truth while the text column lags behind.
                ui.progress_bar(frac as f32)
                    .size([-1.0, 13.0])
                    .overlay_text("")
                    .build();
            }
        }
    }
}

impl App {
    pub(crate) fn apply_data_cmds(&mut self, cmds: Vec<DataCmd>) {
        for cmd in cmds {
            match cmd {
                DataCmd::ClearFocus => self.focus_title = None,
                DataCmd::Sig(cmd) => self.apply_siglist_cmds(vec![cmd]),
            }
        }
    }
}
