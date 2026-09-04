use crate::app::{App, GfxSignal, PALETTE};
use crate::observe::YMode;
use imgui::{MouseButton, Ui};

#[derive(Clone, Copy, PartialEq)]
pub enum ListKind {
    Graphics(usize),
    Data(usize),
    State(usize),
}

static DRAG: std::sync::Mutex<Option<(ListKind, usize)>> = std::sync::Mutex::new(None);

const ROW_H: f32 = 20.0;

fn signals_mut(app: &mut App, kind: ListKind) -> &mut Vec<GfxSignal> {
    match kind {
        ListKind::Graphics(i) => &mut app.graphics[i].signals,
        ListKind::Data(i) => &mut app.data_windows[i].signals,
        ListKind::State(i) => &mut app.state_trackers[i].signals,
    }
}

/// Shared selected-signal list of Graphics/Data/State Tracker windows.
/// Supports visibility toggles, "show all", drag reordering with an
/// insertion line indicator, and -- for Graphics lists only -- each
/// signal's own value-axis policy badge. Adding/removing signals happens
/// in the Signal Selection popup (Measurement Setup).
pub fn draw(app: &mut App, ui: &Ui, kind: ListKind) {
    if ui.small_button("Show all") {
        for s in signals_mut(app, kind).iter_mut() {
            s.visible = true;
        }
    }

    let n = signals_mut(app, kind).len();
    let dl = ui.get_window_draw_list();
    let mouse = ui.io().mouse_pos;
    let mouse_down = ui.io().mouse_down[0];
    let mut tops = Vec::with_capacity(n);
    let mut list_x = 0.0;

    for j in 0..n {
        let key = signals_mut(app, kind)[j].key.clone();
        let color = app
            .sub_view(&key)
            .map(|s| PALETTE[s.color % PALETTE.len()])
            .unwrap_or([0.5, 0.5, 0.5, 1.0]);
        let p = ui.cursor_screen_pos();
        tops.push(p[1]);
        if j == 0 {
            list_x = p[0];
        }
        ui.dummy([14.0, 0.0]);
        ui.same_line();
        let mut vis = signals_mut(app, kind)[j].visible;
        if ui.checkbox(format!("##sigvis{j}"), &mut vis) {
            signals_mut(app, kind)[j].visible = vis;
        }
        if ui.is_item_active() && ui.is_mouse_dragging(MouseButton::Left) {
            *DRAG.lock().unwrap() = Some((kind, j));
        }
        ui.same_line();
        ui.text(&key.2);
        // Each Graphics signal scales its own value axis; the badge shows
        // the policy's initial and opens the mode menu. A Data table has no
        // axis, so its rows stay bare.
        if let ListKind::Graphics(gi) = kind {
            let mode = signals_mut(app, kind)[j].y_mode;
            let menu_id = format!("Y scale##{gi}-{j}");
            ui.same_line_with_pos(super::graphics::PANEL_W - 44.0);
            if ui.small_button(format!("{}##ym{j}", mode.short())) {
                ui.open_popup(&menu_id);
            }
            let badge_hovered = ui.is_item_hovered();
            // The initials alone leave too much to guess: hovering explains
            // the current policy, and each menu entry explains itself.
            // Gated on the badge itself -- `popup` submits no item when it
            // is closed, and an ungated tooltip would latch onto whatever
            // the previous row left hovered.
            if badge_hovered {
                ui.tooltip(|| {
                    ui.text(format!("Y axis: {}", mode.label()));
                    ui.text_disabled(mode.hint());
                });
            }
            ui.popup(&menu_id, || {
                for m in YMode::ALL {
                    if ui.selectable_config(m.label()).selected(m == mode).build() {
                        // Re-entering Lock re-captures whatever the axis
                        // shows then, so any change drops the frozen range.
                        let changed_key = {
                            let sigs = signals_mut(app, kind);
                            sigs[j].y_mode = m;
                            format!("{:?}", sigs[j].key)
                        };
                        app.graphics[gi].y_locks.remove(&changed_key);
                    }
                    // imgui-rs tooltips draw unconditionally; without this
                    // gate every entry's hint piles up at the mouse.
                    if ui.is_item_hovered() {
                        ui.tooltip_text(m.hint());
                    }
                }
            });
        }
        dl.add_rect([p[0], p[1] + 4.0], [p[0] + 10.0, p[1] + 14.0], color)
            .filled(true)
            .build();
    }

    let drag = *DRAG.lock().unwrap();
    if let Some((dk, from)) = drag {
        // Only the window that owns the drag may handle the drop or clear
        // the state; any other window must leave DRAG untouched, otherwise
        // an earlier-rendered window would cancel the drag on mouse release.
        if dk == kind && from < tops.len() {
            let label = signals_mut(app, kind)[from].key.2.clone();
            dl.add_text(
                [mouse[0] + 12.0, mouse[1] + 12.0],
                [0.9, 0.9, 0.95, 1.0],
                label,
            );
            let over_list = !tops.is_empty()
                && mouse[1] >= tops[0] - 4.0
                && mouse[1] <= tops[tops.len() - 1] + ROW_H + 4.0;
            if over_list {
                // Insertion index = first row whose midpoint is below the mouse.
                let mut target = tops.len();
                for (j, &top) in tops.iter().enumerate() {
                    if mouse[1] < top + ROW_H / 2.0 {
                        target = j;
                        break;
                    }
                }
                let line_y = if target < tops.len() {
                    tops[target]
                } else {
                    tops[tops.len() - 1] + ROW_H
                };
                dl.add_line(
                    [list_x, line_y],
                    [list_x + 160.0, line_y],
                    [0.35, 0.85, 1.0, 1.0],
                )
                .thickness(2.0)
                .build();
                if !mouse_down {
                    let no_move = target == from || target == from + 1;
                    if !no_move {
                        let adj = if target > from { target - 1 } else { target };
                        let sig = signals_mut(app, kind).remove(from);
                        signals_mut(app, kind).insert(adj, sig);
                    }
                    *DRAG.lock().unwrap() = None;
                }
            } else if !mouse_down {
                *DRAG.lock().unwrap() = None;
            }
        }
    }
}
