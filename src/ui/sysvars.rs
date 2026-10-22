use crate::app::App;
use crate::bus::SysVarDef;
use crate::ui::help::popup_is_open;
use dear_imgui_rs::{Condition, Key, NumericFormat, StyleVar, TableColumnFlags, TableFlags, Ui};

/// The System Variables window's own state: open flag and the editor draft.
#[derive(Default)]
pub struct SysVarsView {
    pub open: bool,
    pub(crate) draft: Option<SysVarDraft>,
}

/// What the manager asks the bus/core to do. Value edits, deletes and the
/// add/edit Apply all cross this one boundary.
pub enum SysVarsCmd {
    SetValue {
        namespace: String,
        name: String,
        value: f64,
    },
    Delete {
        namespace: String,
        name: String,
    },
    Apply {
        def: SysVarDef,
        old: Option<(String, String)>,
    },
}

/// Draft state of the variable editor popup: which definition it edits
/// (None = a new one) plus the not-yet-applied shape as text buffers.
/// Nothing reaches the bus until Apply.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct SysVarDraft {
    /// The `(namespace, name)` being edited, when editing.
    pub editing: Option<(String, String)>,
    pub namespace: String,
    pub name: String,
    pub init: String,
    pub min: String,
    pub max: String,
    pub unit: String,
    pub comment: String,
    /// Validation failure from the last Apply, shown until it succeeds.
    pub error: Option<String>,
    /// The validated definition of a successful Apply, taken by the
    /// window code on the next pass.
    pub built: Option<SysVarDef>,
}

impl SysVarDraft {
    pub(crate) fn for_add() -> Self {
        Self {
            editing: None,
            namespace: "SysVar".to_string(),
            name: String::new(),
            init: "0".to_string(),
            min: String::new(),
            max: String::new(),
            unit: String::new(),
            comment: String::new(),
            error: None,
            built: None,
        }
    }

    pub(crate) fn for_edit(def: &SysVarDef) -> Self {
        Self {
            editing: Some((def.namespace.clone(), def.name.clone())),
            namespace: def.namespace.clone(),
            name: def.name.clone(),
            init: fmt_f64(def.init),
            min: def.min.map(fmt_f64).unwrap_or_default(),
            max: def.max.map(fmt_f64).unwrap_or_default(),
            unit: def.unit.clone(),
            comment: def.comment.clone(),
            error: None,
            built: None,
        }
    }

    /// Validates the buffers into a definition. `Err` carries the reason,
    /// shown in the dialog.
    fn build(&self) -> Result<SysVarDef, String> {
        let ns = self.namespace.trim();
        let name = self.name.trim();
        if ns.is_empty() || name.is_empty() {
            return Err("namespace and name must not be empty".into());
        }
        if ns.contains("::") || name.contains("::") {
            return Err("namespace and name must not contain \"::\"".into());
        }
        let init: f64 = self
            .init
            .trim()
            .parse()
            .map_err(|_| "initial value must be a number".to_string())?;
        let opt = |s: &str| -> Result<Option<f64>, String> {
            let t = s.trim();
            if t.is_empty() {
                return Ok(None);
            }
            t.parse::<f64>()
                .map(Some)
                .map_err(|_| "bounds must be numbers".to_string())
        };
        let min = opt(&self.min)?;
        let max = opt(&self.max)?;
        if let (Some(lo), Some(hi)) = (min, max)
            && lo > hi
        {
            return Err(format!("lower bound {lo} is greater than upper bound {hi}"));
        }
        Ok(SysVarDef {
            namespace: ns.to_string(),
            name: name.to_string(),
            init,
            min,
            max,
            unit: self.unit.trim().to_string(),
            comment: self.comment.trim().to_string(),
        })
    }
}

fn fmt_f64(v: f64) -> String {
    format!("{v}")
}

/// The System Variables manager: one flat table (a namespace column
/// stands in for the tree until namespaces multiply), live values, and
/// an add/edit dialog in the CANoe mold. Definitions and values live on
/// the bus; everything here reads the snapshot and returns commands.
pub fn render(app: &mut App, ui: &Ui) {
    let mut view = std::mem::take(&mut app.sysvars_view);
    let cmds = view.show(ui, app);
    app.sysvars_view = view;
    app.apply_sysvars_cmds(cmds);
}

impl SysVarsView {
    pub fn show(&mut self, ui: &Ui, app: &App) -> Vec<SysVarsCmd> {
        let mut cmds = Vec::new();
        if !self.open {
            return cmds;
        }
        let io = ui.io();
        let mut open = self.open;
        let min = ui.push_style_var(StyleVar::WindowMinSize([520.0, 220.0]));
        let (pos, size) = crate::ui::fit_first_use(
            io.display_size(),
            [io.display_size()[0] * 0.25, io.display_size()[1] * 0.25],
            [680.0, 360.0],
        );
        ui.window("System Variables")
            .opened(&mut open)
            .position(pos, Condition::FirstUseEver)
            .size(size, Condition::FirstUseEver)
            .build(|| self.content(app, ui, &mut cmds));
        min.pop();
        self.open = open;
        cmds
    }

    fn content(&mut self, app: &App, ui: &Ui, cmds: &mut Vec<SysVarsCmd>) {
        if ui.button("+ Variable") {
            self.draft = Some(SysVarDraft::for_add());
        }
        ui.same_line();
        ui.align_text_to_frame_padding();
        ui.text_disabled(
            "Read with sys_get(\"ns::name\"), write with sys_set(\"ns::name\", v); reset to the initial value when measurement starts",
        );

        let n = app.snap.sysvars.len();
        let Some(_tbl) = ui.begin_table_with_flags(
            "##sysvartable",
            8,
            TableFlags::RESIZABLE | TableFlags::BORDERS | TableFlags::ROW_BG | TableFlags::SCROLL_Y,
        ) else {
            return;
        };
        // Freeze the header row so it stays put while the list scrolls.
        ui.table_setup_scroll_freeze(0, 1);
        ui.table_setup_column_stretch_weight("Namespace", TableColumnFlags::NONE, 1.4);
        ui.table_setup_column_stretch_weight("Name", TableColumnFlags::NONE, 1.4);
        ui.table_setup_column_fixed_width("Value", TableColumnFlags::NONE, 92.0);
        ui.table_setup_column_fixed_width("Init", TableColumnFlags::NONE, 72.0);
        ui.table_setup_column_fixed_width("Min", TableColumnFlags::NONE, 64.0);
        ui.table_setup_column_fixed_width("Max", TableColumnFlags::NONE, 64.0);
        ui.table_setup_column_stretch_weight("Unit / Comment", TableColumnFlags::NONE, 0.9);
        ui.table_setup_column_fixed_width("", TableColumnFlags::NONE, 84.0);
        ui.table_headers_row();

        for i in 0..n {
            ui.table_next_row();
            let Some(v) = app.snap.sysvars.get(i) else {
                continue;
            };
            let (ns, name, init, unit, comment) = (
                v.def.namespace.clone(),
                v.def.name.clone(),
                v.def.init,
                v.def.unit.clone(),
                v.def.comment.clone(),
            );
            let (min_t, max_t) = (
                v.def.min.map(fmt_f64).unwrap_or_default(),
                v.def.max.map(fmt_f64).unwrap_or_default(),
            );
            let bounded = v.def.min.is_some() || v.def.max.is_some();
            let bounds_tip = if bounded {
                Some(format!(
                    "writes are clamped to {} .. {}",
                    v.def.min.map(fmt_f64).unwrap_or_else(|| "-".into()),
                    v.def.max.map(fmt_f64).unwrap_or_else(|| "+".into())
                ))
            } else {
                None
            };
            ui.table_next_column();
            ui.align_text_to_frame_padding();
            ui.text(&ns);
            ui.table_next_column();
            ui.text(&name);
            ui.table_next_column();
            // Live value, editable in place: a manager edit is the same
            // SetSysVar command a script write takes.
            let mut val = v.value as f32;
            ui.set_next_item_width(-1.0);
            let val_fmt = NumericFormat::new("%g").expect("static format");
            if ui
                .input_float_config(format!("##svval{i}"))
                .display_format(val_fmt)
                .build(&mut val)
            {
                cmds.push(SysVarsCmd::SetValue {
                    namespace: ns.clone(),
                    name: name.clone(),
                    value: val as f64,
                });
            }
            if ui.is_item_hovered()
                && let Some(tip) = &bounds_tip
            {
                ui.tooltip_text(tip);
            }
            ui.table_next_column();
            ui.text(fmt_f64(init));
            ui.table_next_column();
            ui.text(&min_t);
            ui.table_next_column();
            ui.text(&max_t);
            ui.table_next_column();
            ui.align_text_to_frame_padding();
            if unit.is_empty() && comment.is_empty() {
                ui.text_disabled("-");
            } else if unit.is_empty() {
                ui.text_disabled(&comment);
            } else {
                ui.text(format!("{unit} {comment}"));
            }
            ui.table_next_column();
            if ui.button(format!("edit##sved{i}"))
                && let Some(d) = app.snap.sysvars.get(i)
            {
                self.draft = Some(SysVarDraft::for_edit(&d.def));
            }
            ui.same_line();
            if ui.button(format!("x##svrm{i}")) {
                cmds.push(SysVarsCmd::Delete {
                    namespace: ns,
                    name,
                });
            }
        }

        self.editor_modal(ui, cmds);
    }

    /// The add/edit popup. Drafted like the trigger editor: widgets shape a
    /// local copy, Apply crosses it in one command, Cancel or Escape throws
    /// it away. Renaming an existing variable deletes the old key and
    /// defines the new one.
    fn editor_modal(&mut self, ui: &Ui, cmds: &mut Vec<SysVarsCmd>) {
        const ID: &str = "Edit variable##svmodal";
        let Some(mut draft) = self.draft.clone() else {
            return;
        };
        if !popup_is_open(ui, ID) {
            ui.open_popup(ID);
        }
        let mut open = true;
        let mut dismissed = false;
        let min = ui.push_style_var(StyleVar::WindowMinSize([380.0, 0.0]));
        ui.modal_popup_with_opened(ID, &mut open, || {
            ui.text(match &draft.editing {
                Some((ns, name)) => format!("Edit {ns}::{name}"),
                None => "New system variable".to_string(),
            });
            ui.separator();
            if ui.is_window_appearing() {
                ui.set_keyboard_focus_here();
            }
            let Some(_grid) = ui.begin_table_with_flags("##svedit", 2, TableFlags::BORDERS_INNER_V)
            else {
                return;
            };
            ui.table_setup_column_fixed_width("", TableColumnFlags::NONE, 84.0);
            ui.table_setup_column_stretch_weight("", TableColumnFlags::NONE, 1.0);

            row(ui, "Namespace", |ui| {
                ui.set_next_item_width(-1.0);
                ui.input_text("##svns", &mut draft.namespace).build();
            });
            row(ui, "Name", |ui| {
                ui.set_next_item_width(-1.0);
                ui.input_text("##svname", &mut draft.name).build();
            });
            row(ui, "Init", |ui| {
                ui.set_next_item_width(-1.0);
                ui.input_text("##svinit", &mut draft.init).build();
            });
            row(ui, "Min", |ui| {
                ui.set_next_item_width(-1.0);
                ui.input_text("##svmin", &mut draft.min)
                    .hint("no limit")
                    .build();
            });
            row(ui, "Max", |ui| {
                ui.set_next_item_width(-1.0);
                ui.input_text("##svmax", &mut draft.max)
                    .hint("no limit")
                    .build();
            });
            row(ui, "Unit", |ui| {
                ui.set_next_item_width(-1.0);
                ui.input_text("##svunit", &mut draft.unit).build();
            });
            row(ui, "Comment", |ui| {
                ui.set_next_item_width(-1.0);
                ui.input_text("##svcomment", &mut draft.comment).build();
            });
            ui.separator();
            if let Some(e) = &draft.error {
                ui.text_colored([1.0, 0.55, 0.3, 1.0], e);
            }
            if ui.is_key_pressed(Key::Escape) {
                dismissed = true;
            }
            if ui.button_with_size("Apply", [90.0, 0.0]) {
                match draft.build() {
                    Ok(def) => {
                        draft.built = Some(def);
                        draft.error = None;
                        ui.close_current_popup();
                    }
                    Err(e) => draft.error = Some(e),
                }
            }
            ui.same_line();
            if ui.button_with_size("Cancel", [90.0, 0.0]) {
                dismissed = true;
            }
        });
        min.pop();
        // Write the (possibly edited) draft back first: the widgets type
        // into this frame's clone, and without this the next frame would
        // restart from the stale copy -- typed text lost, Apply a no-op.
        let built = draft.built.take();
        let old = draft.editing.clone();
        self.draft = Some(draft);
        if let Some(def) = built {
            cmds.push(SysVarsCmd::Apply { def, old });
            self.draft = None;
        } else if dismissed || !open {
            self.draft = None;
        }
    }
}

impl App {
    pub(crate) fn apply_sysvars_cmds(&mut self, cmds: Vec<SysVarsCmd>) {
        for cmd in cmds {
            match cmd {
                SysVarsCmd::SetValue {
                    namespace,
                    name,
                    value,
                } => {
                    self.send(crate::bus::BusCommand::SetSysVar {
                        namespace,
                        name,
                        value,
                    });
                }
                SysVarsCmd::Delete { namespace, name } => {
                    self.send(crate::bus::BusCommand::DeleteSysVar { namespace, name });
                }
                SysVarsCmd::Apply { def, old } => {
                    // A rename leaves the old key behind: delete it after
                    // defining the new one so a script is never left pointing
                    // at nothing.
                    if let Some((old_ns, old_name)) = old
                        && (old_ns != def.namespace || old_name != def.name)
                    {
                        self.send(crate::bus::BusCommand::DeleteSysVar {
                            namespace: old_ns,
                            name: old_name,
                        });
                    }
                    self.send(crate::bus::BusCommand::DefineSysVar(def));
                }
            }
        }
    }
}

/// One label/widget line of the editor grid.
fn row(ui: &Ui, label: &str, body: impl FnOnce(&Ui)) {
    ui.table_next_row();
    ui.table_next_column();
    ui.text(label);
    ui.table_next_column();
    body(ui);
}
