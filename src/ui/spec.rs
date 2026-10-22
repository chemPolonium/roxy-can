use crate::app::App;
use crate::spec::Kind;
use dear_imgui_rs::{
    Condition, Drag, NumericFormat, TableColumnFlags, TableFlags, TableOptions, TableSizingPolicy,
    Ui,
};

/// The Specification window's own state: open flag, the four rule-kind
/// filters, and the drag draft shared by its two threshold fields. The
/// tolerance/grace values and the violation rows stay on `App`; changes go
/// back as commands.
pub struct SpecView {
    pub open: bool,
    show: [bool; 4],
    draft: crate::ui::Draft,
}

impl Default for SpecView {
    fn default() -> Self {
        Self {
            open: false,
            show: [true; 4],
            draft: crate::ui::Draft::default(),
        }
    }
}

pub enum SpecCmd {
    SetTolerance(u64),
    SetGrace(u64),
    Export,
    Clear,
}

/// What arrived that the databases said must not, or did not arrive at all.
/// A report rather than a status light: rows latch and stay until cleared, or
/// until the next run replaces them.
pub fn render(app: &mut App, ui: &Ui) {
    let mut view = std::mem::take(&mut app.spec_view);
    let cmds = view.show(ui, app);
    app.spec_view = view;
    app.apply_spec_cmds(cmds);
}

impl SpecView {
    pub fn show(&mut self, ui: &Ui, app: &App) -> Vec<SpecCmd> {
        let mut cmds = Vec::new();
        if !self.open {
            return cmds;
        }
        let io = ui.io();
        let mut open = self.open;
        let (pos, size) = crate::ui::fit_first_use(
            io.display_size(),
            [io.display_size()[0] * 0.30, io.display_size()[1] * 0.55],
            [620.0, 260.0],
        );
        ui.window("Specification")
            .opened(&mut open)
            .position(pos, Condition::FirstUseEver)
            .size(size, Condition::FirstUseEver)
            .build(|| self.content(app, ui, &mut cmds));
        self.open = open;
        cmds
    }

    fn content(&mut self, app: &App, ui: &Ui, cmds: &mut Vec<SpecCmd>) {
        // The two rules that can only say something where a period was declared.
        // A database that defaults `GenMsgCycleTime` to 0 makes every message look
        // event-triggered, and this line is the difference between "the bus is
        // clean" and "there was nothing to check".
        let periodic: usize = app
            .snap
            .channels
            .iter()
            .filter_map(|c| c.dbc.as_ref())
            .map(|db| {
                db.messages
                    .values()
                    .filter(|m| m.cycle_us.is_some_and(|d| d > 0))
                    .count()
            })
            .sum();
        // These labels sit beside framed drags/checkboxes; ImGui draws plain
        // text from the line top, so align it to the frame padding first.
        ui.align_text_to_frame_padding();
        ui.text("tolerance");
        ui.same_line();
        if let Some(v) = self.threshold(ui, "spec_tol", app.spec_tol_pct, 0.0, 100.0, "%.0f%%") {
            cmds.push(SpecCmd::SetTolerance(v));
        }
        ui.same_line();
        ui.align_text_to_frame_padding();
        ui.text("grace");
        ui.same_line();
        if let Some(v) = self.threshold(ui, "spec_grace", app.spec_grace, 1.0, 20.0, "%.0fx") {
            cmds.push(SpecCmd::SetGrace(v));
        }
        ui.same_line();
        ui.align_text_to_frame_padding();
        ui.text(format!("{periodic} periodic messages declared"));
        // The four rule checkboxes and the two buttons need their own row:
        // together with the tolerance pair they overflow the default width
        // and the last rule's label was cut off. The count already ended
        // this row; a NewLine here would add a blank line.

        // One checkbox per rule, session-only: real logs carry other people's
        // traffic, and being able to stop looking at it without editing the project
        // is what keeps the report usable.
        for (i, kind) in Kind::ALL.iter().enumerate() {
            if i > 0 {
                ui.same_line();
            }
            if ui.checkbox(kind.label(), &mut self.show[i]) {
                // The filter is the window's own state; nothing on the bus
                // changes with it.
            }
        }
        ui.same_line();
        if ui.button("Export##spec") {
            cmds.push(SpecCmd::Export);
        }
        ui.same_line();
        if ui.button("Clear##spec") {
            cmds.push(SpecCmd::Clear);
        }
        ui.separator();

        let rows: Vec<crate::spec::SpecRow> = app
            .spec_rows()
            .into_iter()
            .filter(|r| self.show[r.kind.index()])
            .collect();
        if rows.is_empty() {
            ui.text_colored([0.5, 0.55, 0.6, 1.0], "no violations");
            return;
        }
        ui.text(format!("{} violations", rows.len()));

        let flags = TableFlags::BORDERS_INNER
            | TableFlags::ROW_BG
            | TableFlags::RESIZABLE
            | TableFlags::NO_BORDERS_IN_BODY
            | TableFlags::SCROLL_Y;
        let opts = TableOptions::from(flags).sizing_policy(TableSizingPolicy::StretchProp);
        let Some(_table) = ui.begin_table_with_flags("spec_table", 8, opts) else {
            return;
        };
        ui.table_setup_column_stretch_weight("Message", TableColumnFlags::NONE, 1.0);
        for (label, width) in [
            ("Bus", 56.0),
            ("Rule", 82.0),
            ("Declared", 78.0),
            ("Measured", 78.0),
            ("Count", 52.0),
            ("First", 76.0),
            ("Last", 76.0),
        ] {
            ui.table_setup_column_fixed_width(label, TableColumnFlags::NONE, width);
        }
        ui.table_headers_row();

        for r in &rows {
            ui.table_next_row();
            ui.table_next_column();
            ui.text(format!("{} ({})", r.name, r.addr));
            for column in [
                r.bus.clone(),
                r.kind.label().to_string(),
                crate::spec::qty(r.kind, r.declared),
                crate::spec::qty(r.kind, r.measured),
                format!("{}", r.count),
                secs(r.first_t_us),
                secs(r.last_t_us),
            ] {
                ui.table_next_column();
                ui.text(column);
            }
        }
    }

    /// One integer threshold. The drag previews while it moves and the model only
    /// hears about it when the gesture ends: a monitor setting typed one digit at a
    /// time would otherwise re-judge the bus at every keystroke.
    fn threshold(
        &mut self,
        ui: &Ui,
        key: &str,
        model: u64,
        lo: f32,
        hi: f32,
        fmt: &str,
    ) -> Option<u64> {
        let label = format!("##{key}");
        let mut v = self.draft.shown(key, model as f64) as f32;
        let format = NumericFormat::new(fmt).expect("static format string");
        // Without a width a DragFloat takes the whole remaining line: the
        // tolerance drag swallowed the row and pushed the grace drag (and the
        // declared-period count) out of the window.
        ui.set_next_item_width(80.0);
        let moved = Drag::new(&label)
            .speed((hi - lo) / 100.0)
            .range(lo, hi)
            .display_format(format)
            .build(ui, &mut v);
        let ended = ui.is_item_deactivated();
        let committed = self.draft.step(
            key,
            v as f64,
            moved,
            ui.is_item_deactivated_after_edit(),
            ended,
        );
        committed.map(|val| val.clamp(lo as f64, hi as f64).round() as u64)
    }
}

impl App {
    pub(crate) fn apply_spec_cmds(&mut self, cmds: Vec<SpecCmd>) {
        for cmd in cmds {
            match cmd {
                SpecCmd::SetTolerance(v) => self.spec_tol_pct = v,
                SpecCmd::SetGrace(v) => self.spec_grace = v,
                SpecCmd::Export => self.export_spec_dialog(),
                SpecCmd::Clear => self.send(crate::bus::BusCommand::ClearSpec),
            }
        }
    }
}

fn secs(t_us: u64) -> String {
    format!("{:.3} s", t_us as f64 / 1e6)
}
