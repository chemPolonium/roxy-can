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
        | WindowFlags::NO_NAV
        | WindowFlags::NO_DOCKING;
    // ImGui's default window_min_size (32) would inflate this 26px bar past
    // the bottom of the screen.
    let min = ui.push_style_var(imgui::StyleVar::WindowMinSize([0.0, 0.0]));
    let pad = ui.push_style_var(imgui::StyleVar::WindowPadding([8.0, 6.5]));
    ui.window("##statusbar")
        .flags(flags)
        .position([0.0, io.display_size[1] - STATUSBAR_H], Condition::Always)
        .size([io.display_size[0], STATUSBAR_H], Condition::Always)
        .build(|| {
            // Never wrap: on narrow windows the left-hand chain would wrap to
            // a second line that falls outside the bar.
            let wrap = ui.push_text_wrap_pos_with_pos(-1.0);
            ui.text_colored([0.8, 0.85, 1.0, 1.0], app.display_name());
            ui.same_line();
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
                "| frames: {:>8}  | {:7.0} f/s  | trace: {:>6}  | signals: {:>4}",
                app.frame_counter,
                app.frame_rate,
                app.trace.len(),
                app.subs.len()
            ));
            if app.recording {
                ui.same_line();
                ui.text_colored([1.0, 0.4, 0.4, 1.0], "| REC");
            }
            if app.measuring
                && matches!(app.mode, Mode::Replay)
                && let Some((pos_s, dur_s)) = app.replay_position()
            {
                ui.same_line();
                ui.text(format!("| {:.2} / {:.2}s", pos_s.min(dur_s), dur_s));
            }

            wrap.end();
            let msg = &app.status;
            let w = ui.calc_text_size(msg)[0];
            let pad_y = unsafe { ui.style() }.window_padding[1];
            ui.get_window_draw_list().add_text(
                [
                    io.display_size[0] - w - 12.0,
                    io.display_size[1] - STATUSBAR_H + pad_y,
                ],
                [0.7, 0.75, 0.85, 1.0],
                msg.clone(),
            );
        });
    pad.pop();
    min.pop();
}
