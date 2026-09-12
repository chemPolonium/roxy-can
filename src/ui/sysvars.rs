use crate::app::App;
use crate::bus::SysVarDef;
use crate::ui::help::popup_is_open;
use imgui::{Condition, Key, StyleVar, TableColumnFlags, TableColumnSetup, TableFlags, Ui};

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
            return Err("namespace 和名称不能为空".into());
        }
        if ns.contains("::") || name.contains("::") {
            return Err("namespace 和名称不能包含 \"::\"".into());
        }
        let init: f64 = self
            .init
            .trim()
            .parse()
            .map_err(|_| "初值必须是数字".to_string())?;
        let opt = |s: &str| -> Result<Option<f64>, String> {
            let t = s.trim();
            if t.is_empty() {
                return Ok(None);
            }
            t.parse::<f64>().map(Some).map_err(|_| "界限必须是数字".to_string())
        };
        let min = opt(&self.min)?;
        let max = opt(&self.max)?;
        if let (Some(lo), Some(hi)) = (min, max)
            && lo > hi
        {
            return Err(format!("下界 {lo} 大于上界 {hi}"));
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
/// the bus; everything here reads the snapshot and edits via commands.
pub fn render(app: &mut App, ui: &Ui) {
    if !app.show_sysvars {
        return;
    }
    let io = ui.io();
    let mut open = app.show_sysvars;
    let min = ui.push_style_var(StyleVar::WindowMinSize([520.0, 220.0]));
    ui.window("System Variables")
        .opened(&mut open)
        .position(
            [io.display_size[0] * 0.25, io.display_size[1] * 0.25],
            Condition::FirstUseEver,
        )
        .size([680.0, 360.0], Condition::FirstUseEver)
        .build(|| content(app, ui));
    min.pop();
    app.show_sysvars = open;
}

fn content(app: &mut App, ui: &Ui) {
    if ui.button("+ Variable") {
        app.sysvar_draft = Some(SysVarDraft::for_add());
    }
    ui.same_line();
    ui.align_text_to_frame_padding();
    ui.text_disabled("sys_get(\"ns::name\") / sys_set(\"ns::name\", v) 访问；每次测量开始复位为初值");

    let n = app.snap.sysvars.len();
    let Some(_tbl) = ui.begin_table_with_flags(
        "##sysvartable",
        8,
        TableFlags::RESIZABLE
            | TableFlags::BORDERS
            | TableFlags::ROW_BG
            | TableFlags::SCROLL_Y,
    ) else {
        return;
    };
    // Freeze the header row so it stays put while the list scrolls.
    ui.table_setup_scroll_freeze(0, 1);
    ui.table_setup_column_with(TableColumnSetup {
        flags: TableColumnFlags::WIDTH_STRETCH,
        init_width_or_weight: 1.4,
        ..TableColumnSetup::new("Namespace")
    });
    ui.table_setup_column_with(TableColumnSetup {
        flags: TableColumnFlags::WIDTH_STRETCH,
        init_width_or_weight: 1.4,
        ..TableColumnSetup::new("Name")
    });
    ui.table_setup_column_with(TableColumnSetup {
        flags: TableColumnFlags::WIDTH_FIXED,
        init_width_or_weight: 92.0,
        ..TableColumnSetup::new("Value")
    });
    ui.table_setup_column_with(TableColumnSetup {
        flags: TableColumnFlags::WIDTH_FIXED,
        init_width_or_weight: 72.0,
        ..TableColumnSetup::new("Init")
    });
    ui.table_setup_column_with(TableColumnSetup {
        flags: TableColumnFlags::WIDTH_FIXED,
        init_width_or_weight: 64.0,
        ..TableColumnSetup::new("Min")
    });
    ui.table_setup_column_with(TableColumnSetup {
        flags: TableColumnFlags::WIDTH_FIXED,
        init_width_or_weight: 64.0,
        ..TableColumnSetup::new("Max")
    });
    ui.table_setup_column_with(TableColumnSetup {
        flags: TableColumnFlags::WIDTH_STRETCH,
        init_width_or_weight: 0.9,
        ..TableColumnSetup::new("Unit / Comment")
    });
    ui.table_setup_column_with(TableColumnSetup {
        flags: TableColumnFlags::WIDTH_FIXED,
        init_width_or_weight: 84.0,
        ..TableColumnSetup::new("")
    });
    ui.table_headers_row();

    for i in 0..n {
        ui.table_next_row();
        // Copy the row out first: the widgets below send commands, which
        // need `&mut App`, so nothing may borrow the snapshot across them.
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
                "写入会被夹到 {} .. {}",
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
        if ui
            .input_float(format!("##svval{i}"), &mut val)
            .display_format("%g")
            .build()
        {
            app.send(crate::bus::BusCommand::SetSysVar {
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
            app.sysvar_draft = Some(SysVarDraft::for_edit(&d.def));
        }
        ui.same_line();
        if ui.button(format!("x##svrm{i}")) {
            app.send(crate::bus::BusCommand::DeleteSysVar {
                namespace: ns,
                name,
            });
        }
    }
    editor_modal(app, ui);
}

/// The add/edit popup. Drafted like the trigger editor: widgets shape a
/// local copy, Apply crosses it in one command, Cancel or Escape throws
/// it away. Renaming an existing variable deletes the old key and
/// defines the new one.
fn editor_modal(app: &mut App, ui: &Ui) {
    const ID: &str = "Edit variable##svmodal";
    let Some(mut draft) = app.sysvar_draft.clone() else {
        return;
    };
    if !popup_is_open(ui, ID) {
        ui.open_popup(ID);
    }
    let mut open = true;
    let mut dismissed = false;
    let min = ui.push_style_var(StyleVar::WindowMinSize([380.0, 0.0]));
    ui.modal_popup_config(ID).opened(&mut open).build(|| {
        ui.text(match &draft.editing {
            Some((ns, name)) => format!("编辑 {ns}::{name}"),
            None => "新建 system variable".to_string(),
        });
        ui.separator();
        if ui.is_window_appearing() {
            ui.set_keyboard_focus_here();
        }
        let Some(_grid) = ui.begin_table_with_flags(
            "##svedit",
            2,
            TableFlags::BORDERS_INNER_V | TableFlags::SIZING_STRETCH_PROP,
        ) else {
            return;
        };
        ui.table_setup_column_with(TableColumnSetup {
            flags: TableColumnFlags::WIDTH_FIXED,
            init_width_or_weight: 84.0,
            ..TableColumnSetup::new("")
        });
        ui.table_setup_column_with(TableColumnSetup {
            flags: TableColumnFlags::WIDTH_STRETCH,
            init_width_or_weight: 1.0,
            ..TableColumnSetup::new("")
        });

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
                .hint("无限制")
                .build();
        });
        row(ui, "Max", |ui| {
            ui.set_next_item_width(-1.0);
            ui.input_text("##svmax", &mut draft.max)
                .hint("无限制")
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
    app.sysvar_draft = Some(draft);
    if let Some(def) = built {
        // A rename leaves the old key behind: delete it after defining
        // the new one so a script is never left pointing at nothing.
        if let Some((old_ns, old_name)) = &app.sysvar_draft.as_ref().unwrap().editing
            && (*old_ns != def.namespace || *old_name != def.name)
        {
            app.send(crate::bus::BusCommand::DeleteSysVar {
                namespace: old_ns.clone(),
                name: old_name.clone(),
            });
        }
        app.send(crate::bus::BusCommand::DefineSysVar(def));
        app.sysvar_draft = None;
    } else if dismissed || !open {
        app.sysvar_draft = None;
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
