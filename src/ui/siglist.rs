use crate::app::{App, GfxSignal, PALETTE};
use crate::observe::YMode;
use dear_imgui_rs::{MouseButton, Ui};

#[derive(Clone, Copy, PartialEq)]
pub enum ListKind {
    Graphics(usize),
    Data(usize),
    State(usize),
}

/// What the shared signal list asks the window/App to do. Visibility, order
/// and y-mode are the list's own state; these three cross windows.
pub enum SigListCmd {
    AddStateSignal {
        window: usize,
        key: crate::observe::SigKey,
    },
    NewStateWith(crate::observe::SigKey),
    EditStateRule {
        window: usize,
        key: crate::observe::SigKey,
    },
}

static DRAG: std::sync::Mutex<Option<(ListKind, usize)>> = std::sync::Mutex::new(None);

const ROW_H: f32 = 20.0;

/// Legacy wrapper for windows not yet migrated: take the signal list and
/// (for Graphics) the y-locks out, draw read-only, put them back, then apply
/// the commands. New windows call `draw_view` directly.
#[allow(dead_code)] // the headless drag test still calls the legacy wrapper
pub fn draw(app: &mut App, ui: &Ui, kind: ListKind) {
    let mut signals = match kind {
        ListKind::Graphics(i) => std::mem::take(&mut app.graphics[i].signals),
        ListKind::Data(i) => std::mem::take(&mut app.data_windows[i].signals),
        ListKind::State(i) => std::mem::take(&mut app.state_trackers[i].signals),
    };
    let mut y_locks = match kind {
        ListKind::Graphics(i) => Some(std::mem::take(&mut app.graphics[i].y_locks)),
        _ => None,
    };
    let mut cmds = Vec::new();
    draw_view(app, ui, kind, &mut signals, y_locks.as_mut(), &mut cmds);
    match kind {
        ListKind::Graphics(i) => {
            app.graphics[i].signals = signals;
            if let Some(locks) = y_locks {
                app.graphics[i].y_locks = locks;
            }
        }
        ListKind::Data(i) => app.data_windows[i].signals = signals,
        ListKind::State(i) => app.state_trackers[i].signals = signals,
    }
    app.apply_siglist_cmds(cmds);
}

/// Shared selected-signal list of Graphics/Data/State Tracker windows.
/// Supports visibility toggles, "show all", drag reordering with an
/// insertion line indicator, and -- for Graphics lists only -- each
/// signal's own value-axis policy badge. Adding/removing signals happens
/// in the Signal Selection popup (Measurement Setup).
#[allow(clippy::needless_range_loop)]
pub fn draw_view(
    app: &App,
    ui: &Ui,
    kind: ListKind,
    signals: &mut Vec<GfxSignal>,
    mut y_locks: Option<&mut std::collections::HashMap<String, (f64, f64)>>,
    cmds: &mut Vec<SigListCmd>,
) {
    if ui.button("Show all") {
        for s in signals.iter_mut() {
            s.visible = true;
        }
    }

    let n = signals.len();
    let dl = ui.get_window_draw_list();
    let mouse = ui.io().mouse_pos();
    let mouse_down = ui.is_mouse_down(MouseButton::Left);
    let mut tops = Vec::with_capacity(n);
    let mut list_x = 0.0;

    for j in 0..n {
        let key = signals[j].key.clone();
        // The chip shows the curve color for Graphics/Data. A State Tracker
        // colors by state value, not per signal, so its rows stay bare.
        let chip = !matches!(kind, ListKind::State(_));
        let color = if chip {
            app.sub_view(&key)
                .map(|s| PALETTE[s.color % PALETTE.len()])
                .unwrap_or([0.5, 0.5, 0.5, 1.0])
        } else {
            [0.0, 0.0, 0.0, 0.0]
        };
        let p = ui.cursor_screen_pos();
        tops.push(p[1]);
        if j == 0 {
            list_x = p[0];
        }
        // The drag grip leads the row: two columns of dots on an invisible
        // button, the classic `::` handle. Dragging is the grip's job now
        // -- the checkbox only toggles visibility.
        ui.invisible_button(format!("##grip{j}"), [14.0, ROW_H]);
        if ui.is_item_active() && ui.is_mouse_dragging(MouseButton::Left) {
            *DRAG.lock().unwrap() = Some((kind, j));
        }
        let dot = [0.42, 0.42, 0.48, 1.0];
        for gx in [4.0, 9.0] {
            for gy in [6.0, 10.0, 14.0] {
                dl.add_circle([p[0] + gx, p[1] + gy], 1.0, dot)
                    .filled(true)
                    .build();
            }
        }
        ui.same_line();
        let mut vis = signals[j].visible;
        if ui.checkbox(format!("##sigvis{j}"), &mut vis) {
            signals[j].visible = vis;
        }
        // The Y-axis policy sits right after the checkbox, before the
        // color chip: it is always at the same offset regardless of how
        // long the signal name below grows, so nothing can cover it.
        if let ListKind::Graphics(gi) = kind {
            ui.same_line();
            let mode = signals[j].y_mode;
            let menu_id = format!("Y scale##{gi}-{j}");
            if ui.button(format!("{}##ym{j}", mode.short())) {
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
                        signals[j].y_mode = m;
                        let changed_key = format!("{:?}", signals[j].key);
                        if let Some(locks) = &mut y_locks {
                            locks.remove(&changed_key);
                        }
                    }
                    // imgui-rs tooltips draw unconditionally; without this
                    // gate every entry's hint piles up at the mouse.
                    if ui.is_item_hovered() {
                        ui.tooltip_text(m.hint());
                    }
                }
            });
        }
        ui.same_line();
        // The color chip follows: a spacer reserves its cell and the rect
        // lands inside it, so name positions stay.
        let chip_x = if chip {
            let cx = ui.cursor_screen_pos()[0];
            ui.dummy([18.0, 0.0]);
            Some(cx)
        } else {
            None
        };
        ui.same_line();
        ui.align_text_to_frame_padding();
        ui.text(key.name());
        // Right-click a Graphics/Data row routes the signal into a State
        // Tracker -- one more way to start watching a signal's states
        // without visiting the Signal Selection popup.
        let text_hovered = ui.is_item_hovered();
        if !matches!(kind, ListKind::State(_)) && text_hovered {
            let menu_id = format!("##track-menu{j}");
            if ui.is_mouse_clicked(MouseButton::Right) {
                ui.open_popup(&menu_id);
            }
            ui.popup(&menu_id, || {
                ui.text_disabled("Add to State Tracker");
                let names: Vec<String> =
                    app.state_trackers.iter().map(|w| w.name.clone()).collect();
                for (ti, name) in names.iter().enumerate() {
                    if ui.menu_item(name) {
                        cmds.push(SigListCmd::AddStateSignal {
                            window: ti,
                            key: key.clone(),
                        });
                    }
                }
                if ui.menu_item("+ New State Tracker") {
                    cmds.push(SigListCmd::NewStateWith(key.clone()));
                }
            });
        }
        // A State Tracker row's badge opens its custom state-band editor
        // (CANoe's Value Definition). It rides inline after the name: the
        // list scrolls horizontally now, so nothing anchors to the panel
        // edge any more.
        if let ListKind::State(wi) = kind {
            ui.same_line();
            if ui.button(format!("S##srule{j}")) {
                cmds.push(SigListCmd::EditStateRule {
                    window: wi,
                    key: key.clone(),
                });
            }
            if ui.is_item_hovered() {
                ui.tooltip_text("Custom state bands: cuts, names, colors");
            }
        }
        if let Some(cx) = chip_x {
            dl.add_rect([cx + 3.0, p[1] + 4.0], [cx + 13.0, p[1] + 14.0], color)
                .filled(true)
                .build();
        }
    }

    let drag = *DRAG.lock().unwrap();
    if let Some((dk, from)) = drag {
        // Only the window that owns the drag may handle the drop or clear
        // the state; any other window must leave DRAG untouched, otherwise
        // an earlier-rendered window would cancel the drag on mouse release.
        if dk == kind && from < tops.len() {
            let label = signals[from].key.name().to_string();
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
                        let sig = signals.remove(from);
                        signals.insert(adj, sig);
                    }
                    *DRAG.lock().unwrap() = None;
                }
            } else if !mouse_down {
                *DRAG.lock().unwrap() = None;
            }
        }
    }
}

impl App {
    pub(crate) fn apply_siglist_cmds(&mut self, cmds: Vec<SigListCmd>) {
        for cmd in cmds {
            match cmd {
                SigListCmd::AddStateSignal { window, key } => {
                    self.set_win_signal(crate::app::PopupTarget::State(window), key, true);
                }
                SigListCmd::NewStateWith(key) => {
                    self.new_state_window();
                    let ti = self.state_trackers.len() - 1;
                    self.set_win_signal(crate::app::PopupTarget::State(ti), key, true);
                }
                SigListCmd::EditStateRule { window, key } => {
                    self.state_ui.rule_edit = Some((window, key));
                }
            }
        }
    }
}
