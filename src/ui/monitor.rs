//! The Monitor window: the lightweight big-screen panel. One text row per
//! watched signal -- label, live physical value, optional coloring rule
//! (`>=`/`<=` a threshold repaints the row in a palette color). No plots,
//! no tables: rows a user glances at from across the lab.

use crate::app::{App, PALETTE};
use dear_imgui_rs::{Condition, StyleVar, Ui};

pub fn render(app: &mut App, ui: &Ui) {
    if !app.show_monitor {
        return;
    }
    let io = ui.io();
    let mut open = app.show_monitor;
    let min = ui.push_style_var(StyleVar::WindowMinSize([360.0, 160.0]));
    ui.window("Monitor")
        .opened(&mut open)
        .position(
            [io.display_size()[0] * 0.45,
             io.display_size()[1] * 0.12],
            Condition::FirstUseEver,
        )
        .size([420.0, 360.0], Condition::FirstUseEver)
        .build(|| content(app, ui));
    min.pop();
    app.show_monitor = open;
}

fn content(app: &mut App, ui: &Ui) {
    if ui.button("+ Add signal") {
        app.popup_target = Some(crate::workspace::PopupTarget::Monitor);
        app.show_id_filter = true;
    }
    if ui.is_item_hovered() {
        ui.tooltip_text("从信号选择器挑一行监控（数据库信号 / 派生信号 / 系统变量）");
    }
    ui.same_line();
    if ui.button("Clear") {
        let keys: Vec<crate::observe::SigKey> =
            app.monitor_rows.iter().map(|r| r.key.clone()).collect();
        for key in keys {
            app.set_win_signal(crate::workspace::PopupTarget::Monitor, key, false);
        }
    }
    ui.separator();

    let rows = app.monitor_rows.clone();
    for (i, row) in rows.iter().enumerate() {
        let sub = app.sub_view(&row.key);
        let (value, unit) = match sub {
            Some(s) => (s.latest, s.unit.clone()),
            None => (0.0, String::new()),
        };
        // Coloring: a rule that holds repaints the whole row in its
        // palette color; the default text color otherwise.
        let hot = row.rule_on
            && ((row.rising && value >= row.threshold)
                || (!row.rising && value <= row.threshold));
        let text_color = if hot {
            PALETTE[row.color % PALETTE.len()]
        } else {
            [1.0, 1.0, 1.0, 1.0]
        };
        let chip = if hot { PALETTE[row.color % PALETTE.len()] } else { [0.35, 0.35, 0.4, 1.0] };
        let dl = ui.get_window_draw_list();
        let p = ui.cursor_screen_pos();
        dl.add_rect([p[0], p[1] + 3.0], [p[0] + 6.0, p[1] + 17.0], chip)
            .filled(true)
            .build();
        ui.dummy([10.0, 0.0]);
        ui.same_line();
        ui.text_colored(text_color, &row.label);
        ui.same_line();
        ui.text_colored(
            text_color,
            format!("{:.*} {}", row.digits, value, unit),
        );

        // Right-aligned row controls: rule toggle, threshold, direction,
        // color, remove.
        rule_controls(app, ui, i);
    }
    if app.monitor_rows.is_empty() {
        ui.text_disabled("no rows -- add a signal to watch");
    }
}

/// The per-row right-aligned controls: rule on/off, threshold, direction,
/// color cycle, remove.
fn rule_controls(app: &mut App, ui: &Ui, i: usize) {
    let width = ui.content_region_avail()[0];
    ui.same_line_with_pos(width - 240.0);
    let row = &mut app.monitor_rows[i];
    if ui.button(if row.rule_on { "on" } else { "off" }) {
        row.rule_on = !row.rule_on;
    }
    if ui.is_item_hovered() {
        ui.tooltip_text("开关着色规则");
    }
    ui.same_line();
    ui.set_next_item_width(66.0);
    let mut th = row.threshold as f32;
    if ui
        .input_float_config(format!("##mthr{i}"))
        .display_format(dear_imgui_rs::NumericFormat::new("%g").expect("static format"))
        .build(&mut th)
    {
        row.threshold = th as f64;
    }
    if ui.is_item_hovered() {
        ui.tooltip_text("着色阈值");
    }
    ui.same_line();
    let dir = if row.rising { ">=" } else { "<=" };
    if ui.button(format!("{dir}##mdir{i}")) {
        row.rising = !row.rising;
    }
    if ui.is_item_hovered() {
        ui.tooltip_text("切换方向");
    }
    ui.same_line();
    let slot = row.color % PALETTE.len();
    if ui.button(format!("{slot}##mcol{i}")) {
        row.color = (row.color + 1) % PALETTE.len();
    }
    if ui.is_item_hovered() {
        ui.tooltip_text("换颜色");
    }
    ui.same_line();
    if ui.button(format!("x##mrm{i}")) {
        let key = app.monitor_rows[i].key.clone();
        app.set_win_signal(crate::workspace::PopupTarget::Monitor, key, false);
    }
}
