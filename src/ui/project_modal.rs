use crate::app::{App, PendingAction};
use imgui::Ui;

enum Choice {
    Save,
    Discard,
    Cancel,
}

fn perform(app: &mut App, action: PendingAction) {
    match action {
        PendingAction::Quit => app.quit = true,
        PendingAction::NewProject => app.new_project(),
        PendingAction::OpenProject => app.open_project_dialog(),
    }
}

/// Confirmation shown before anything discards an untitled workspace.
pub fn render(app: &mut App, ui: &Ui) {
    if app.pending_action.is_none() {
        return;
    }
    const ID: &str = "Unsaved Project##projmodal";
    let popup_open = unsafe {
        let id = std::ffi::CString::new(ID).unwrap();
        imgui::sys::igIsPopupOpen_Str(id.as_ptr(), imgui::sys::ImGuiPopupFlags_None as i32)
    };
    if !popup_open {
        ui.open_popup(ID);
    }
    let mut open = true;
    let mut choice: Option<Choice> = None;
    ui.modal_popup_config(ID).opened(&mut open).build(|| {
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
                    perform(app, action);
                }
            }
            Choice::Discard => perform(app, action),
            Choice::Cancel => {}
        }
    }
}
