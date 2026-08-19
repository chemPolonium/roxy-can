use crate::app::App;
use imgui::Ui;

/// Renders the DBC message/signal tree with checkboxes reflecting `selected`.
/// Returns (key, wanted) toggles to be applied by the caller.
pub fn signal_tree(app: &App, ui: &Ui, selected: &[(u32, String)]) -> Vec<((u32, String), bool)> {
    let mut toggles = Vec::new();
    let Some(db) = &app.dbc else {
        ui.text("no DBC loaded");
        return toggles;
    };
    for &id in &db.order {
        let Some(msg) = db.messages.get(&id) else {
            continue;
        };
        if let Some(_node) = ui.tree_node(format!("{} ({:X})", msg.name, id)) {
            for s in &msg.signals {
                let key = (id, s.name.clone());
                let mut on = selected.contains(&key);
                ui.checkbox(&s.name, &mut on);
                toggles.push((key, on));
            }
        }
    }
    toggles
}
