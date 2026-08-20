use crate::app::{App, Mode, STATUSBAR_H};
use imgui::{Condition, Ui, WindowFlags};

/// Fixed bottom bar showing measurement state; independent of any window.
pub fn render(app: &App, ui: &Ui) {
    let io = ui.io();
    let flags = WindowFlags::NO_TITLE_BAR
        | WindowFlags::NO_RESIZE
        | WindowFlags::NO_MOVE
        | WindowFlags::NO_COLLAPSE
        | WindowFlags::NO_SCROLLBAR
        | WindowFlags::NO_SAVED_SETTINGS
        | WindowFlags::NO_FOCUS_ON_APPEARING
        | WindowFlags::NO_NAV;
    ui.window("##statusbar")
        .flags(flags)
        .position([0.0, io.display_size[1] - STATUSBAR_H], Condition::Always)
        .size([io.display_size[0], STATUSBAR_H], Condition::Always)
        .build(|| {
            let (state, color): (&str, [f32; 4]) = if app.measuring {
                match app.mode {
                    Mode::Virtual => ("MEASURING (virtual)", [0.4, 0.95, 0.5, 1.0]),
                    Mode::Replay => ("REPLAYING", [0.3, 0.8, 1.0, 1.0]),
                }
            } else {
                ("STOPPED", [0.6, 0.6, 0.65, 1.0])
            };
            ui.text_colored(color, state);
            ui.same_line();
            ui.text(format!(
                "| frames: {}  | {:.0} f/s  | load: {:.1}% @500k  | trace: {}  | signals: {}",
                app.frame_counter,
                app.frame_rate,
                app.bus_load,
                app.trace.len(),
                app.subs.len()
            ));
            if app.recording {
                ui.same_line();
                ui.text_colored([1.0, 0.4, 0.4, 1.0], "| REC");
            }

            let p = ui.cursor_screen_pos();
            let msg = &app.status;
            let w = msg.chars().count() as f32 * 7.0;
            ui.get_window_draw_list().add_text(
                [io.display_size[0] - w - 12.0, p[1]],
                [0.7, 0.75, 0.85, 1.0],
                msg.clone(),
            );
        });
}
