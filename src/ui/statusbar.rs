use crate::app::{App, Mode, STATUSBAR_H};
use dear_imgui_rs::{Condition, StyleVar, Ui, WindowFlags};

/// The longest prefix of `text` that fits `avail` pixels as measured by
/// `width_of`, with `…` marking a cut. `width_of` is a parameter so the rule
/// can be tested without a font atlas.
fn fit_width(text: &str, avail: f32, width_of: impl Fn(&str) -> f32) -> String {
    if avail <= 0.0 {
        return String::new();
    }
    if width_of(text) <= avail {
        return text.to_string();
    }
    let ellipsis_w = width_of("…");
    let chars: Vec<char> = text.chars().collect();
    // Binary search the prefix: a status line is drawn every frame, and the
    // messages that need cutting are the long ones.
    let mut lo = 0usize;
    let mut hi = chars.len();
    while lo < hi {
        let mid = lo + (hi - lo).div_ceil(2);
        let cand: String = chars[..mid].iter().copied().collect();
        if width_of(&cand) + ellipsis_w <= avail {
            lo = mid;
        } else {
            hi = mid - 1;
        }
    }
    let mut out: String = chars[..lo].iter().copied().collect();
    out.push('…');
    out
}

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
    let min = ui.push_style_var(StyleVar::WindowMinSize([0.0, 0.0]));
    let pad = ui.push_style_var(StyleVar::WindowPadding([8.0, 6.5]));
    ui.window("##statusbar")
        .flags(flags)
        .position([0.0, io.display_size()[1] - STATUSBAR_H], Condition::Always)
        .size([io.display_size()[0], STATUSBAR_H], Condition::Always)
        .build(|| {
            // Never wrap: on narrow windows the left-hand chain would wrap to
            // a second line that falls outside the bar.
            let wrap = ui.push_text_wrap_pos(-1.0);
            ui.text_colored([0.8, 0.85, 1.0, 1.0], app.display_name());
            ui.same_line();
            let (state, color): (String, [f32; 4]) = if app.snap.measuring {
                match app.snap.mode {
                    // The wire's story, in the same words the toolbar switch
                    // uses -- see [`crate::bus::wire_note`].
                    Mode::Virtual => (
                        format!("MEASURING ({})", crate::bus::wire_note(app.snap.hw.len(), app.snap.real_bus)),
                        if app.snap.hw.is_empty() || app.snap.real_bus {
                            [0.4, 0.95, 0.5, 1.0]
                        } else {
                            [1.0, 0.8, 0.4, 1.0]
                        },
                    ),
                    Mode::Replay => ("REPLAYING".to_string(), [0.3, 0.8, 1.0, 1.0]),
                }
            } else {
                ("STOPPED".to_string(), [0.6, 0.6, 0.65, 1.0])
            };
            ui.text_colored(color, state);
            ui.same_line();
            // Throttled counters, refreshed on the text gate by
            // `sync_status_text`, so the digits hold still long enough to
            // read instead of flickering with every frame.
            ui.text(&app.status_counters);
            if app.snap.recording {
                ui.same_line();
                ui.text_colored([1.0, 0.4, 0.4, 1.0], "| REC");
            }
            // Not gated on `measuring`: after a log runs out the source keeps
            // its timeline, so the readout holds at the end of the run.
            if matches!(app.snap.mode, Mode::Replay)
                && let Some((pos_s, dur_s)) = app.replay_position()
            {
                ui.same_line();
                let name = crate::ui::toolbar::file_name(&app.log_path);
                if name.is_empty() {
                    ui.text(format!("| {:.2} / {:.2}s", pos_s.min(dur_s), dur_s));
                } else {
                    ui.text(format!("| {name}  {:.2} / {:.2}s", pos_s.min(dur_s), dur_s));
                }
            }

            wrap.end();
            let msg = &app.status;
            let pad_y = unsafe { ui.style() }.window_padding()[1];
            // The message owns the space the left-hand chain leaves it and no
            // more. It used to be drawn right-aligned at its full width, so a
            // driver failure long enough to matter -- which channel refused
            // what, and why -- painted straight over the project name and the
            // counters. Cut it with an ellipsis; the whole line is one gesture
            // away on hover, and the Write window keeps every one of them.
            // The anchor is the last item's right edge: a text item leaves the
            // cursor at the *start* of the line, which would read as "the
            // whole bar is free".
            let left_x = ui.item_rect_max()[0] + 8.0;
            let right = io.display_size()[0] - 12.0;
            let shown = fit_width(msg, right - left_x, |s| ui.calc_text_size(s)[0]);
            let w = ui.calc_text_size(&shown)[0];
            ui.set_cursor_pos([right - w, pad_y]);
            ui.text_colored([0.7, 0.75, 0.85, 1.0], &shown);
            if shown != *msg && ui.is_item_hovered() {
                ui.tooltip_text(msg.clone());
            }
        });
    pad.pop();
    min.pop();
}

#[cfg(test)]
mod tests {
    use super::fit_width;

    /// One pixel per `char`: the ruler stands in for the font atlas, and the
    /// messages that need cutting are the multibyte ones -- a byte-indexed
    /// prefix would panic mid-character.
    fn ruler(s: &str) -> f32 {
        s.chars().count() as f32
    }

    fn fits(msg: &str, avail: f32) -> String {
        fit_width(msg, avail, ruler)
    }

    #[test]
    fn a_message_that_fits_is_left_alone() {
        assert_eq!(
            fits("hardware attached to 0", 999.0),
            "hardware attached to 0"
        );
    }

    #[test]
    fn a_long_message_is_cut_to_the_room_left_and_says_so() {
        let msg = "打开 Vector 通道 0 失败（status 204: XL_ERR_INVALID_CHANNEL_MASK）";
        let shown = fits(msg, 12.0);
        assert_eq!(ruler(&shown), 12.0, "the cut used the room it was given: {shown}");
        assert!(shown.ends_with('…'), "a cut is marked as one: {shown}");
        assert!(msg.starts_with(shown.trim_end_matches('…')));
    }

    #[test]
    fn every_room_from_nothing_to_the_whole_line_comes_back_measurable() {
        let msg = "FlexRay 监听挂接失败: FR0 集群配置被拒绝（status 112: XL_ERR_INVALID_ACCESS）";
        for px in 0..=ruler(msg) as u32 + 3 {
            let avail = px as f32;
            let shown = fits(msg, avail);
            assert!(
                ruler(&shown) <= avail,
                "asked for {avail}px of {msg:?} and got {shown:?}"
            );
        }
    }

    #[test]
    fn no_room_is_no_message() {
        assert_eq!(fits("anything at all", 0.0), "");
        assert_eq!(fits("anything at all", -5.0), "");
    }
}
