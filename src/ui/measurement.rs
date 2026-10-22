use crate::app::{App, PopupTarget, SigScope};
use crate::ui::idfilter::{scope_combo, target_name};
use crate::ui::siglist::ListKind;
use dear_imgui_rs::{Condition, TableColumnFlags, TableFlags, TableOptions, TableSizingPolicy, Ui};

/// The Measurement Setup window's own state: open flag only. Every window it
/// lists belongs to `App`; edits to them are commands.
pub struct MeasurementView {
    pub open: bool,
}

impl Default for MeasurementView {
    fn default() -> Self {
        Self { open: true }
    }
}

pub enum MeasurementCmd {
    NewTrace,
    NewMessages,
    NewStats,
    NewGraphics,
    NewData,
    NewState,
    SetTraceLimit(usize),
    OpenPicker(PopupTarget),
    Focus(PopupTarget),
    Remove(PopupTarget),
    Export(PopupTarget),
    SetName {
        target: PopupTarget,
        name: String,
    },
    SetScope {
        target: PopupTarget,
        scope: SigScope,
    },
}

/// "Go to" button: opens the window if hidden and always brings it to the
/// front. There is no close action — windows close via their own title
/// bar. `###` keeps the widget ID stable.
fn goto_button(ui: &Ui, id: &str) -> bool {
    ui.button(format!("->###goto_{id}"))
}

/// Single-table overview of every observer (Trace, Messages, Statistics,
/// Graphics, Data): open state, Signals scope, and per-observer export.
/// A flat list rather than a node graph: there is no measurement topology to
/// wire here, only a table of what exists.
pub fn render(app: &mut App, ui: &Ui) {
    let mut view = std::mem::take(&mut app.measurement_view);
    let cmds = view.show(ui, app);
    app.measurement_view = view;
    app.apply_measurement_cmds(cmds);
}

impl MeasurementView {
    pub fn show(&mut self, ui: &Ui, app: &App) -> Vec<MeasurementCmd> {
        let mut cmds = Vec::new();
        let io = ui.io();
        let mut open = self.open;
        if open {
            let (pos, size) = crate::ui::fit_first_use(
                io.display_size(),
                [io.display_size()[0] * 0.04, io.display_size()[1] * 0.30],
                [760.0, 340.0],
            );
            ui.window("Measurement Setup")
                .opened(&mut open)
                .position(pos, Condition::FirstUseEver)
                .size(size, Condition::FirstUseEver)
                .build(|| content(app, ui, &mut cmds));
        }
        self.open = open;
        cmds
    }
}

fn content(app: &App, ui: &Ui, cmds: &mut Vec<MeasurementCmd>) {
    if ui.button("+ Trace") {
        cmds.push(MeasurementCmd::NewTrace);
    }
    ui.same_line();
    if ui.button("+ Messages") {
        cmds.push(MeasurementCmd::NewMessages);
    }
    ui.same_line();
    if ui.button("+ Statistics") {
        cmds.push(MeasurementCmd::NewStats);
    }
    ui.same_line();
    if ui.button("+ Graphics") {
        cmds.push(MeasurementCmd::NewGraphics);
    }
    ui.same_line();
    if ui.button("+ Data") {
        cmds.push(MeasurementCmd::NewData);
    }
    ui.same_line();
    if ui.button("+ State Tracker") {
        cmds.push(MeasurementCmd::NewState);
    }
    ui.same_line();
    // The trace ring's retention is a global capacity, not a window
    // property: one combo here, project-persisted.
    ui.set_next_item_width(90.0);
    const TRACE_CAPS: [usize; 5] = [50_000, 200_000, 500_000, 1_000_000, 2_000_000];
    const TRACE_LABELS: [&str; 5] = ["50k", "200k", "500k", "1M", "2M"];
    let mut cap_idx = TRACE_CAPS
        .iter()
        .position(|&c| c == app.trace_limit)
        .unwrap_or(0);
    if ui.combo_simple_string("##tracecap", &mut cap_idx, &TRACE_LABELS) {
        cmds.push(MeasurementCmd::SetTraceLimit(TRACE_CAPS[cap_idx]));
    }
    if ui.is_item_hovered() {
        ui.tooltip_text("Trace capacity (frames): past this the oldest frames are dropped and the Trace window says so");
    }
    ui.separator();

    // NO_BORDERS_IN_BODY restricts column-resize dragging to the header row.
    let flags = TableFlags::BORDERS_INNER
        | TableFlags::ROW_BG
        | TableFlags::RESIZABLE
        | TableFlags::NO_BORDERS_IN_BODY
        | TableFlags::SCROLL_Y;
    let opts = TableOptions::from(flags).sizing_policy(TableSizingPolicy::StretchProp);
    let Some(_table) = ui.begin_table_with_flags("meas_table", 6, opts) else {
        return;
    };
    // default-size "->" button
    ui.table_setup_column_fixed_width("Open", TableColumnFlags::NONE, 32.0);
    // longest type label is "State Tracker"
    ui.table_setup_column_fixed_width("Type", TableColumnFlags::NONE, 96.0);
    ui.table_setup_column_stretch_weight("Name", TableColumnFlags::NONE, 1.0);
    ui.table_setup_column_fixed_width("Filter", TableColumnFlags::NONE, 160.0);
    ui.table_setup_column_fixed_width("Export", TableColumnFlags::NONE, 60.0);
    ui.table_setup_column_fixed_width("", TableColumnFlags::NONE, 26.0);
    ui.table_headers_row();

    for i in 0..app.trace_windows.len() {
        trace_row(app, ui, i, cmds);
    }
    for i in 0..app.msg_windows.len() {
        messages_row(app, ui, i, cmds);
    }
    for i in 0..app.stats_windows.len() {
        stats_row(app, ui, i, cmds);
    }
    for i in 0..app.graphics.len() {
        graphics_row(app, ui, i, cmds);
    }
    for i in 0..app.data_windows.len() {
        data_row(app, ui, i, cmds);
    }
    for i in 0..app.state_trackers.len() {
        state_row(app, ui, i, cmds);
    }
}

fn row_open(ui: &Ui, target: PopupTarget, id: &str, cmds: &mut Vec<MeasurementCmd>) {
    if goto_button(ui, id) {
        cmds.push(MeasurementCmd::Focus(target));
    }
    if ui.is_item_hovered() {
        ui.tooltip_text("Show this window and bring it to the front");
    }
}

fn row_name(app: &App, ui: &Ui, target: PopupTarget, id: String, cmds: &mut Vec<MeasurementCmd>) {
    let mut name = win_name(app, target).to_string();
    ui.set_next_item_width(-1.0);
    if ui.input_text(id, &mut name).build() {
        cmds.push(MeasurementCmd::SetName { target, name });
    }
}

fn row_scope(app: &App, ui: &Ui, target: PopupTarget, id: String, cmds: &mut Vec<MeasurementCmd>) {
    let mut open_picker = false;
    let new_scope = scope_combo(
        app,
        ui,
        &id,
        win_scope(app, target),
        target,
        &mut open_picker,
    );
    if open_picker {
        cmds.push(MeasurementCmd::OpenPicker(target));
    }
    if new_scope != win_scope(app, target) {
        cmds.push(MeasurementCmd::SetScope {
            target,
            scope: new_scope,
        });
    }
}

fn row_remove(ui: &Ui, id: String, target: PopupTarget, cmds: &mut Vec<MeasurementCmd>) {
    if ui.button(id) {
        cmds.push(MeasurementCmd::Remove(target));
    }
    if ui.is_item_hovered() {
        ui.tooltip_text(
            "Remove this window from Measurement Setup; it closes and its filter, scope and signal selection are discarded",
        );
    }
}

fn trace_row(app: &App, ui: &Ui, i: usize, cmds: &mut Vec<MeasurementCmd>) {
    ui.table_next_row();
    if !ui.table_next_column() {
        return;
    }
    row_open(ui, PopupTarget::Trace(i), &format!("t{i}"), cmds);
    ui.table_next_column();
    ui.text("Trace");
    ui.table_next_column();
    row_name(app, ui, PopupTarget::Trace(i), format!("##name_t{i}"), cmds);
    ui.table_next_column();
    row_scope(
        app,
        ui,
        PopupTarget::Trace(i),
        format!("##mscope_t{i}"),
        cmds,
    );
    ui.table_next_column();
    if ui.button(format!("Export##save_t{i}")) {
        cmds.push(MeasurementCmd::Export(PopupTarget::Trace(i)));
    }
    ui.table_next_column();
    row_remove(ui, format!("x##t{i}"), PopupTarget::Trace(i), cmds);
}

fn messages_row(app: &App, ui: &Ui, i: usize, cmds: &mut Vec<MeasurementCmd>) {
    ui.table_next_row();
    if !ui.table_next_column() {
        return;
    }
    row_open(ui, PopupTarget::Messages(i), &format!("m{i}"), cmds);
    ui.table_next_column();
    ui.text("Messages");
    ui.table_next_column();
    row_name(
        app,
        ui,
        PopupTarget::Messages(i),
        format!("##name_m{i}"),
        cmds,
    );
    ui.table_next_column();
    row_scope(
        app,
        ui,
        PopupTarget::Messages(i),
        format!("##mscope_m{i}"),
        cmds,
    );
    ui.table_next_column();
    if ui.button(format!("Export##save_m{i}")) {
        cmds.push(MeasurementCmd::Export(PopupTarget::Messages(i)));
    }
    ui.table_next_column();
    row_remove(ui, format!("x##m{i}"), PopupTarget::Messages(i), cmds);
}

fn stats_row(app: &App, ui: &Ui, i: usize, cmds: &mut Vec<MeasurementCmd>) {
    ui.table_next_row();
    if !ui.table_next_column() {
        return;
    }
    row_open(ui, PopupTarget::Stats(i), &format!("s{i}"), cmds);
    ui.table_next_column();
    ui.text("Statistics");
    ui.table_next_column();
    row_name(app, ui, PopupTarget::Stats(i), format!("##name_s{i}"), cmds);
    ui.table_next_column();
    row_scope(
        app,
        ui,
        PopupTarget::Stats(i),
        format!("##mscope_s{i}"),
        cmds,
    );
    ui.table_next_column();
    if ui.button(format!("Export##save_s{i}")) {
        cmds.push(MeasurementCmd::Export(PopupTarget::Stats(i)));
    }
    ui.table_next_column();
    row_remove(ui, format!("x##s{i}"), PopupTarget::Stats(i), cmds);
}

/// Filter cell of a Graphics/Data row: a "…" button opening the Signal
/// Selection popup and the visible/total signal count.
fn signal_cell(app: &App, ui: &Ui, kind: ListKind, cmds: &mut Vec<MeasurementCmd>) {
    let (target, prefix, i, vis, total) = match kind {
        ListKind::Graphics(i) => {
            let w = &app.graphics[i];
            (
                PopupTarget::Graphics(i),
                "g",
                i,
                w.signals.iter().filter(|s| s.visible).count(),
                w.signals.len(),
            )
        }
        ListKind::Data(i) => {
            let w = &app.data_windows[i];
            (
                PopupTarget::Data(i),
                "d",
                i,
                w.signals.iter().filter(|s| s.visible).count(),
                w.signals.len(),
            )
        }
        ListKind::State(i) => {
            let w = &app.state_trackers[i];
            (
                PopupTarget::State(i),
                "st",
                i,
                w.signals.iter().filter(|s| s.visible).count(),
                w.signals.len(),
            )
        }
    };
    if ui.button(format!("…##selsig{prefix}{i}")) {
        cmds.push(MeasurementCmd::OpenPicker(target));
    }
    ui.same_line();
    ui.text(format!("{vis}/{total}"));
}

fn graphics_row(app: &App, ui: &Ui, i: usize, cmds: &mut Vec<MeasurementCmd>) {
    ui.table_next_row();
    if !ui.table_next_column() {
        return;
    }
    row_open(ui, PopupTarget::Graphics(i), &format!("g{i}"), cmds);
    ui.table_next_column();
    ui.text("Graphics");
    ui.table_next_column();
    row_name(
        app,
        ui,
        PopupTarget::Graphics(i),
        format!("##name_g{i}"),
        cmds,
    );
    ui.table_next_column();
    signal_cell(app, ui, ListKind::Graphics(i), cmds);
    ui.table_next_column();
    if ui.button(format!("Export##save_g{i}")) {
        cmds.push(MeasurementCmd::Export(PopupTarget::Graphics(i)));
    }
    ui.table_next_column();
    row_remove(ui, format!("x##g{i}"), PopupTarget::Graphics(i), cmds);
}

fn data_row(app: &App, ui: &Ui, i: usize, cmds: &mut Vec<MeasurementCmd>) {
    ui.table_next_row();
    if !ui.table_next_column() {
        return;
    }
    row_open(ui, PopupTarget::Data(i), &format!("d{i}"), cmds);
    ui.table_next_column();
    ui.text("Data");
    ui.table_next_column();
    row_name(app, ui, PopupTarget::Data(i), format!("##name_d{i}"), cmds);
    ui.table_next_column();
    signal_cell(app, ui, ListKind::Data(i), cmds);
    ui.table_next_column();
    if ui.button(format!("Export##save_d{i}")) {
        cmds.push(MeasurementCmd::Export(PopupTarget::Data(i)));
    }
    ui.table_next_column();
    row_remove(ui, format!("x##d{i}"), PopupTarget::Data(i), cmds);
}

/// The State Tracker row mirrors Data: signal-level selection, no CSV
/// export yet (the band view has nothing tabular to write).
fn state_row(app: &App, ui: &Ui, i: usize, cmds: &mut Vec<MeasurementCmd>) {
    ui.table_next_row();
    if !ui.table_next_column() {
        return;
    }
    row_open(ui, PopupTarget::State(i), &format!("st{i}"), cmds);
    ui.table_next_column();
    ui.text("State Tracker");
    ui.table_next_column();
    row_name(
        app,
        ui,
        PopupTarget::State(i),
        format!("##name_st{i}"),
        cmds,
    );
    ui.table_next_column();
    signal_cell(app, ui, ListKind::State(i), cmds);
    ui.table_next_column();
    ui.table_next_column();
    row_remove(ui, format!("x##st{i}"), PopupTarget::State(i), cmds);
}

fn win_name(app: &App, t: PopupTarget) -> &str {
    match t {
        PopupTarget::Trace(i) => app.trace_windows.get(i).map(|w| w.name.as_str()),
        PopupTarget::Messages(i) => app.msg_windows.get(i).map(|w| w.name.as_str()),
        PopupTarget::Stats(i) => app.stats_windows.get(i).map(|w| w.name.as_str()),
        PopupTarget::Graphics(i) => app.graphics.get(i).map(|w| w.name.as_str()),
        PopupTarget::Data(i) => app.data_windows.get(i).map(|w| w.name.as_str()),
        PopupTarget::State(i) => app.state_trackers.get(i).map(|w| w.name.as_str()),
        PopupTarget::Monitor => None,
    }
    .unwrap_or("")
}

fn win_scope(app: &App, t: PopupTarget) -> SigScope {
    match t {
        PopupTarget::Trace(i) => app.trace_windows.get(i).map(|w| w.scope),
        PopupTarget::Messages(i) => app.msg_windows.get(i).map(|w| w.scope),
        PopupTarget::Stats(i) => app.stats_windows.get(i).map(|w| w.scope),
        PopupTarget::Graphics(_) | PopupTarget::Data(_) | PopupTarget::State(_) => {
            Some(SigScope::All)
        }
        PopupTarget::Monitor => Some(SigScope::All),
    }
    .unwrap_or(SigScope::All)
}

/// Keeps the Filter Selection popup pointing at the right window after an
/// observer row is deleted. `which`: 0 = Trace, 1 = Messages, 2 = Statistics,
/// 3 = Graphics, 4 = Data, 5 = State Tracker.
fn popup_after_remove(t: PopupTarget, removed: usize, which: u8) -> Option<PopupTarget> {
    match t {
        PopupTarget::Trace(i) if which == 0 => match i.cmp(&removed) {
            std::cmp::Ordering::Equal => None,
            std::cmp::Ordering::Greater => Some(PopupTarget::Trace(i - 1)),
            std::cmp::Ordering::Less => Some(t),
        },
        PopupTarget::Messages(i) if which == 1 => match i.cmp(&removed) {
            std::cmp::Ordering::Equal => None,
            std::cmp::Ordering::Greater => Some(PopupTarget::Messages(i - 1)),
            std::cmp::Ordering::Less => Some(t),
        },
        PopupTarget::Stats(i) if which == 2 => match i.cmp(&removed) {
            std::cmp::Ordering::Equal => None,
            std::cmp::Ordering::Greater => Some(PopupTarget::Stats(i - 1)),
            std::cmp::Ordering::Less => Some(t),
        },
        PopupTarget::Graphics(i) if which == 3 => match i.cmp(&removed) {
            std::cmp::Ordering::Equal => None,
            std::cmp::Ordering::Greater => Some(PopupTarget::Graphics(i - 1)),
            std::cmp::Ordering::Less => Some(t),
        },
        PopupTarget::Data(i) if which == 4 => match i.cmp(&removed) {
            std::cmp::Ordering::Equal => None,
            std::cmp::Ordering::Greater => Some(PopupTarget::Data(i - 1)),
            std::cmp::Ordering::Less => Some(t),
        },
        PopupTarget::State(i) if which == 5 => match i.cmp(&removed) {
            std::cmp::Ordering::Equal => None,
            std::cmp::Ordering::Greater => Some(PopupTarget::State(i - 1)),
            std::cmp::Ordering::Less => Some(t),
        },
        _ => Some(t),
    }
}

impl App {
    /// Shows one observer window and brings it to the front. Shared by the
    /// Measurement Setup `->` button and the View > Observers menu.
    pub fn focus_observer(&mut self, target: PopupTarget) {
        match target {
            PopupTarget::Trace(i) => {
                if let Some(w) = self.trace_windows.get_mut(i) {
                    w.opened = true;
                }
            }
            PopupTarget::Messages(i) => {
                if let Some(w) = self.msg_windows.get_mut(i) {
                    w.opened = true;
                }
            }
            PopupTarget::Stats(i) => {
                if let Some(w) = self.stats_windows.get_mut(i) {
                    w.opened = true;
                }
            }
            PopupTarget::Graphics(i) => {
                if let Some(w) = self.graphics.get_mut(i) {
                    w.opened = true;
                }
            }
            PopupTarget::Data(i) => {
                if let Some(w) = self.data_windows.get_mut(i) {
                    w.opened = true;
                }
            }
            PopupTarget::State(i) => {
                if let Some(w) = self.state_trackers.get_mut(i) {
                    w.opened = true;
                }
            }
            PopupTarget::Monitor => self.monitor_view.open = true,
        }
        self.focus_title = Some(target_name(self, target));
    }

    fn remove_observer(&mut self, target: PopupTarget) {
        match target {
            PopupTarget::Trace(i) if i < self.trace_windows.len() => {
                self.trace_windows.remove(i);
                self.id_filter.target = self
                    .id_filter
                    .target
                    .and_then(|t| popup_after_remove(t, i, 0));
            }
            PopupTarget::Messages(i) if i < self.msg_windows.len() => {
                self.msg_windows.remove(i);
                self.id_filter.target = self
                    .id_filter
                    .target
                    .and_then(|t| popup_after_remove(t, i, 1));
            }
            PopupTarget::Stats(i) if i < self.stats_windows.len() => {
                self.stats_windows.remove(i);
                self.id_filter.target = self
                    .id_filter
                    .target
                    .and_then(|t| popup_after_remove(t, i, 2));
            }
            PopupTarget::Graphics(i) if i < self.graphics.len() => {
                let keys: Vec<_> = self.graphics[i]
                    .signals
                    .iter()
                    .map(|s| s.key.clone())
                    .collect();
                self.graphics.remove(i);
                for k in keys {
                    self.prune_signal(&k);
                }
                self.id_filter.target = self
                    .id_filter
                    .target
                    .and_then(|t| popup_after_remove(t, i, 3));
            }
            PopupTarget::Data(i) if i < self.data_windows.len() => {
                let keys: Vec<_> = self.data_windows[i]
                    .signals
                    .iter()
                    .map(|s| s.key.clone())
                    .collect();
                self.data_windows.remove(i);
                for k in keys {
                    self.prune_signal(&k);
                }
                self.id_filter.target = self
                    .id_filter
                    .target
                    .and_then(|t| popup_after_remove(t, i, 4));
            }
            PopupTarget::State(i) if i < self.state_trackers.len() => {
                let keys: Vec<_> = self.state_trackers[i]
                    .signals
                    .iter()
                    .map(|s| s.key.clone())
                    .collect();
                self.state_trackers.remove(i);
                for k in keys {
                    self.prune_signal(&k);
                }
                self.id_filter.target = self
                    .id_filter
                    .target
                    .and_then(|t| popup_after_remove(t, i, 5));
            }
            _ => {}
        }
    }

    pub(crate) fn apply_measurement_cmds(&mut self, cmds: Vec<MeasurementCmd>) {
        for cmd in cmds {
            match cmd {
                MeasurementCmd::NewTrace => self.new_trace_window(),
                MeasurementCmd::NewMessages => self.new_msg_window(),
                MeasurementCmd::NewStats => self.new_stats_window(),
                MeasurementCmd::NewGraphics => self.new_graphics_window(),
                MeasurementCmd::NewData => self.new_data_window(),
                MeasurementCmd::NewState => self.new_state_window(),
                MeasurementCmd::SetTraceLimit(cap) => {
                    self.trace_limit = cap;
                    self.set_trace_limit(cap);
                }
                MeasurementCmd::OpenPicker(target) => {
                    self.id_filter.open_for(target);
                }
                MeasurementCmd::Focus(target) => self.focus_observer(target),
                MeasurementCmd::Remove(target) => self.remove_observer(target),
                MeasurementCmd::Export(target) => match target {
                    PopupTarget::Trace(i) => self.export_trace_dialog(i),
                    PopupTarget::Messages(i) => self.export_messages_dialog(i),
                    PopupTarget::Stats(i) => self.export_stats_dialog(i),
                    PopupTarget::Graphics(i) => self.export_graphics_dialog(i),
                    PopupTarget::Data(i) => self.export_data_dialog(i),
                    PopupTarget::State(_) | PopupTarget::Monitor => {}
                },
                MeasurementCmd::SetName { target, name } => match target {
                    PopupTarget::Trace(i) => {
                        if let Some(w) = self.trace_windows.get_mut(i) {
                            w.name = name;
                        }
                    }
                    PopupTarget::Messages(i) => {
                        if let Some(w) = self.msg_windows.get_mut(i) {
                            w.name = name;
                        }
                    }
                    PopupTarget::Stats(i) => {
                        if let Some(w) = self.stats_windows.get_mut(i) {
                            w.name = name;
                        }
                    }
                    PopupTarget::Graphics(i) => {
                        if let Some(w) = self.graphics.get_mut(i) {
                            w.name = name;
                        }
                    }
                    PopupTarget::Data(i) => {
                        if let Some(w) = self.data_windows.get_mut(i) {
                            w.name = name;
                        }
                    }
                    PopupTarget::State(i) => {
                        if let Some(w) = self.state_trackers.get_mut(i) {
                            w.name = name;
                        }
                    }
                    PopupTarget::Monitor => {}
                },
                MeasurementCmd::SetScope { target, scope } => match target {
                    PopupTarget::Trace(i) => {
                        if let Some(w) = self.trace_windows.get_mut(i) {
                            w.scope = scope;
                        }
                    }
                    PopupTarget::Messages(i) => {
                        if let Some(w) = self.msg_windows.get_mut(i) {
                            w.scope = scope;
                        }
                    }
                    PopupTarget::Stats(i) => {
                        if let Some(w) = self.stats_windows.get_mut(i) {
                            w.scope = scope;
                        }
                    }
                    _ => {}
                },
            }
        }
    }
}
