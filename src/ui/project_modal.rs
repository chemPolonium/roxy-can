use crate::app::App;
use dear_imgui_rs::Ui;

enum Choice {
    Save,
    Discard,
    Cancel,
}

/// Confirmation shown before anything discards an untitled workspace.
pub fn render(app: &mut App, ui: &Ui) {
    if app.pending_action.is_none() {
        return;
    }
    const ID: &str = "Unsaved Project##projmodal";
    if !ui.is_popup_open(ID) {
        ui.open_popup(ID);
    }
    let mut open = true;
    let mut choice: Option<Choice> = None;
    ui.modal_popup_with_opened(ID, &mut open, || {
        ui.text("The current workspace is not saved as a project.");
        ui.text("Save it before continuing?");
        ui.separator();
        if ui.button("Save Changes") {
            choice = Some(Choice::Save);
        }
        ui.same_line();
        if ui.button("Don't Save") {
            choice = Some(Choice::Discard);
        }
        ui.same_line();
        if ui.button("Cancel") {
            choice = Some(Choice::Cancel);
        }
    });
    if !open {
        choice = Some(Choice::Cancel);
    }
    if let Some(c) = choice {
        let action = app.pending_action.take().unwrap();
        match c {
            Choice::Save => {
                if app.save_project(None) {
                    app.run_action(action);
                }
            }
            Choice::Discard => app.run_action(action),
            Choice::Cancel => {}
        }
    }
}
