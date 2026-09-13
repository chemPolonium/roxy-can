//! The Write window: the system-wide output panel in the CANoe mold.
//! Node prints, static checks, timer/sysvar advisories and command news
//! all land in one scrolling log on the bus core; this window only
//! shapes and displays it.

use crate::app::App;
use crate::bus::WriteKind;
use dear_imgui_rs::{Condition, StyleVar, Ui};

/// `wall_us` (local microseconds since midnight) as `HH:MM:SS.mmm`.
fn wall_stamp(us: u64) -> String {
    let s = us / 1_000_000;
    let ms = (us % 1_000_000) / 1_000;
    format!(
        "[{:02}:{:02}:{:02}.{:03}]",
        s / 3_600,
        (s % 3_600) / 60,
        s % 60,
        ms
    )
}

pub fn render(app: &mut App, ui: &Ui) {
    if !app.show_write {
        return;
    }
    let io = ui.io();
    let mut open = app.show_write;
    let min = ui.push_style_var(StyleVar::WindowMinSize([420.0, 160.0]));
    ui.window("Write")
        .opened(&mut open)
        .position(
            [
                io.display_size()[0] * 0.55,
                io.display_size()[1] * 0.35,
            ],
            Condition::FirstUseEver,
        )
        .size([640.0, 280.0], Condition::FirstUseEver)
        .build(|| content(app, ui));
    min.pop();
    app.show_write = open;
}

fn content(app: &mut App, ui: &Ui) {
    if ui.button("清空") {
        app.send(crate::bus::BusCommand::ClearWrite);
    }
    ui.same_line();
    ui.align_text_to_frame_padding();
    ui.text_disabled("按类别：");
    // Per-kind visibility; the order mirrors WriteKind's variants.
    const LABELS: [&str; 4] = ["脚本", "信息", "告警", "错误"];
    for (slot, label) in app.write_filter.iter_mut().zip(LABELS) {
        ui.same_line();
        ui.checkbox(label, slot);
    }
    ui.same_line();
    ui.align_text_to_frame_padding();
    ui.text_disabled("脚本输出、检查告警与系统事件");

    // Stick to the bottom while the user hasn't scrolled up to read
    // something: arrivals keep appending below otherwise.
    let at_bottom = ui.scroll_y() + 2.0 >= ui.scroll_max_y();
    let want = |k: &WriteKind| -> bool {
        let i = match k {
            WriteKind::Script => 0,
            WriteKind::Info => 1,
            WriteKind::Warning => 2,
            WriteKind::Error => 3,
        };
        app.write_filter[i]
    };
    let lines: Vec<&crate::bus::WriteLine> =
        app.snap.write.iter().filter(|l| want(&l.kind)).collect();
    for line in lines {
        let color = match line.kind {
            WriteKind::Error => [1.0, 0.55, 0.3, 1.0],
            WriteKind::Warning => [1.0, 0.8, 0.4, 1.0],
            WriteKind::Info => [0.55, 0.75, 1.0, 1.0],
            WriteKind::Script => [1.0, 1.0, 1.0, 1.0],
        };
        ui.text_colored([0.45, 0.45, 0.45, 1.0], wall_stamp(line.wall_us));
        ui.same_line();
        ui.text_colored(color, &line.text);
    }
    if at_bottom && ui.scroll_max_y() > 0.0 {
        ui.set_scroll_here_y(1.0);
    }
}
