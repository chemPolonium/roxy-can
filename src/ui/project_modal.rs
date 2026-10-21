use crate::app::App;
use dear_imgui_rs::Ui;

/// What the operator chose in the unsaved-project modal. The pending action
/// itself stays on `App`; this is only the decision.
pub enum ProjectModalCmd {
    Save,
    Discard,
    Cancel,
}

/// Confirmation shown before anything discards an untitled workspace.
pub fn render(app: &mut App, ui: &Ui) {
    if app.pending_action.is_none() {
        return;
    }
    let cmds = show(ui);
    app.apply_project_modal_cmds(cmds);
}

fn show(ui: &Ui) -> Vec<ProjectModalCmd> {
    let mut cmds = Vec::new();
    const ID: &str = "Unsaved Project##projmodal";
    if !ui.is_popup_open(ID) {
        ui.open_popup(ID);
    }
    let mut open = true;
    ui.modal_popup_with_opened(ID, &mut open, || {
        ui.text("The current workspace is not saved as a project.");
        ui.text("Save it before continuing?");
        ui.separator();
        if ui.button("Save Changes") {
            cmds.push(ProjectModalCmd::Save);
        }
        ui.same_line();
        if ui.button("Don't Save") {
            cmds.push(ProjectModalCmd::Discard);
        }
        ui.same_line();
        if ui.button("Cancel") {
            cmds.push(ProjectModalCmd::Cancel);
        }
    });
    if !open {
        cmds.push(ProjectModalCmd::Cancel);
    }
    cmds
}

impl App {
    pub(crate) fn apply_project_modal_cmds(&mut self, cmds: Vec<ProjectModalCmd>) {
        for cmd in cmds {
            let Some(action) = self.pending_action.take() else {
                return;
            };
            match cmd {
                ProjectModalCmd::Save => {
                    if self.save_project(None) {
                        self.run_action(action);
                    }
                }
                ProjectModalCmd::Discard => self.run_action(action),
                ProjectModalCmd::Cancel => {}
            }
        }
    }
}
