use crate::app::App;
use crate::load::{BusLoad, FrameClass};
use dear_imgui_rs::{Condition, TableColumnFlags, TableFlags, TableSizingPolicy, TableOptions, Ui};

/// The CAN statistics window, laid out like the reference: one row per
/// statistic, columns Current/Last, Min, Max, Avg, one section per bus.
/// All numbers come from `BusLoad`, which folds the same frame stream the
/// aggregates read.
///
/// The hardware-only rows of the reference (chip state is shown as
/// Simulated; transmit/receive error counters and transceiver
/// errors/delay) have no meaning on a simulated bus and are not listed.
pub fn render(app: &mut App, ui: &Ui) {
    if !app.show_bus_stats {
        return;
    }
    let io = ui.io();
    let mut open = app.show_bus_stats;
    ui.window("Bus Statistics###busstats")
        .opened(&mut open)
        .position(
            [
                io.display_size()[0] * 0.55,
                io.display_size()[1] * 0.12,
            ],
            Condition::FirstUseEver,
        )
        .size([520.0, 420.0], Condition::FirstUseEver)
        .build(|| content(app, ui));
    app.show_bus_stats = open;
}

fn content(app: &mut App, ui: &Ui) {
    load_history_section(app, ui);
    ui.separator();

    // NO_BORDERS_IN_BODY restricts column-resize dragging to the header row.
    let flags = TableFlags::BORDERS_INNER
        | TableFlags::ROW_BG
        | TableFlags::RESIZABLE
        | TableFlags::NO_BORDERS_IN_BODY
        | TableFlags::SCROLL_Y;
    let opts = TableOptions::from(flags).sizing_policy(TableSizingPolicy::StretchProp);
    let Some(_table) = ui.begin_table_with_flags("bus_stats_table", 5, opts) else {
        return;
    };
    ui.table_setup_column_stretch_weight("Statistic", TableColumnFlags::NONE, 1.0);
    for (label, w) in [
        ("Current / Last", 90.0),
        ("Min", 70.0),
        ("Max", 70.0),
        ("Avg", 70.0),
    ] {
        ui.table_setup_column_fixed_width(label, TableColumnFlags::NONE, w);
    }
    ui.table_setup_scroll_freeze(0, 1);
    ui.table_headers_row();

    for (i, ch) in app.snap.channels.iter().enumerate() {
        let Some(load) = app.snap.bus_loads.get(i) else {
            continue;
        };
        ui.table_next_row();
        if !ui.table_next_column() {
            continue;
        }
        ui.text_colored(
            [0.35, 0.65, 1.0, 1.0],
            format!("{} ({} k)", ch.name, ch.bitrate_kbps),
        );
        for _ in 1..5 {
            ui.table_next_column();
        }

        let (l_min, l_max, l_avg) = load.load_stats();
        stat_row(
            ui,
            "Busload [%]",
            pct(load.load()),
            opt_pct(l_min),
            opt_pct(l_max),
            opt_pct(l_avg),
        );
        let (d_last, d_min, d_max, d_avg) = load.send_dist_us();
        stat_row(
            ui,
            "Min. Send Dist. [ms]",
            opt_ms(d_last),
            opt_ms(d_min),
            opt_ms(d_max),
            opt_ms_f(d_avg),
        );
        stat_row(
            ui,
            "Bursts [total]",
            load.bursts_total().to_string(),
            "-".to_string(),
            "-".to_string(),
            "-".to_string(),
        );
        let (bt_last, bt_min, bt_max, bt_avg) = load.burst_time_us();
        stat_row(
            ui,
            "Burst Time [ms]",
            us_ms(bt_last),
            opt_ms(bt_min),
            opt_ms(bt_max),
            opt_ms_f(bt_avg),
        );
        let (f_cur, f_min, f_max, f_avg) = load.frames_per_burst();
        stat_row(
            ui,
            "Frames per Burst",
            f_cur.to_string(),
            opt_num_u(f_min),
            opt_num_u(f_max),
            opt_num(f_avg),
        );
        class_rows(ui, load, "Std. Data", FrameClass::StdData);
        class_rows(ui, load, "Ext. Data", FrameClass::ExtData);
        class_rows(ui, load, "Std. Remote", FrameClass::StdRemote);
        class_rows(ui, load, "Ext. Remote", FrameClass::ExtRemote);
        let (e_min, e_max, e_avg) = load.rate_stats(FrameClass::Error);
        stat_row(
            ui,
            "Errorframes [n/s]",
            num(load.class_rate(FrameClass::Error)),
            opt_num(e_min),
            opt_num(e_max),
            opt_num(e_avg),
        );
        stat_row(
            ui,
            "Errorframes [total]",
            load.errors.to_string(),
            "-".to_string(),
            "-".to_string(),
            "-".to_string(),
        );
        stat_row(
            ui,
            "Chip State",
            "Simulated".to_string(),
            "-".to_string(),
            "-".to_string(),
            "-".to_string(),
        );
    }
}

/// The [n/s] and [total] pair of one identifier class.
fn class_rows(ui: &Ui, load: &BusLoad, label: &str, class: FrameClass) {
    let (r_min, r_max, r_avg) = load.rate_stats(class);
    stat_row(
        ui,
        format!("{label} [n/s]"),
        num(load.class_rate(class)),
        opt_num(r_min),
        opt_num(r_max),
        opt_num(r_avg),
    );
    stat_row(
        ui,
        format!("{label} [total]"),
        load.class_total(class).to_string(),
        "-".to_string(),
        "-".to_string(),
        "-".to_string(),
    );
}

fn stat_row(
    ui: &Ui,
    label: impl AsRef<str>,
    current: String,
    min: String,
    max: String,
    avg: String,
) {
    ui.table_next_row();
    if !ui.table_next_column() {
        return;
    }
    ui.text(label);
    ui.table_next_column();
    ui.text(current);
    ui.table_next_column();
    ui.text(min);
    ui.table_next_column();
    ui.text(max);
    ui.table_next_column();
    ui.text(avg);
}

fn pct(v: f64) -> String {
    format!("{:.2}", v * 100.0)
}

fn opt_pct(v: Option<f64>) -> String {
    v.map(pct).unwrap_or_else(|| "-".to_string())
}

fn num(v: f64) -> String {
    format!("{v:.0}")
}

fn opt_num(v: Option<f64>) -> String {
    v.map(num).unwrap_or_else(|| "-".to_string())
}

fn us_ms(us: u64) -> String {
    format!("{:.3}", us as f64 / 1000.0)
}

fn opt_ms(us: Option<u64>) -> String {
    us.map(us_ms).unwrap_or_else(|| "-".to_string())
}

fn opt_ms_f(us: Option<f64>) -> String {
    us.map(|v| format!("{:.3}", v / 1000.0))
        .unwrap_or_else(|| "-".to_string())
}

fn opt_num_u(v: Option<u64>) -> String {
    v.map(|x| x.to_string()).unwrap_or_else(|| "-".to_string())
}

/// One sparkline per bus: the load history BusLoad already keeps (60 x
/// 100 ms buckets) drawn as a polyline. Y scales to the window's own
/// peak (floor 25%) so light loads stay readable; the 100% overload line
/// appears only when the peak reaches it.
fn load_history_section(app: &mut App, ui: &Ui) {
    for (i, ch) in app.snap.channels.iter().enumerate() {
        let Some(load) = app.snap.bus_loads.get(i) else {
            continue;
        };
        let pts: Vec<(u64, f64)> = load.history().collect();
        if pts.len() < 2 {
            continue;
        }
        ui.text_colored(
            [0.35, 0.65, 1.0, 1.0],
            format!("{} 负载历史（近 1 分钟）", ch.name),
        );
        ui.same_line();
        ui.text_disabled(format!("当前 {:.2} %", load.load() * 100.0));

        let avail = ui.content_region_avail();
        let rect_min = ui.cursor_screen_pos();
        let rect_max = [rect_min[0] + avail[0], rect_min[1] + 64.0];
        ui.dummy([avail[0], 64.0]);
        let dl = ui.get_window_draw_list();
        dl.add_rect(rect_min, rect_max, [0.08, 0.08, 0.10, 1.0])
            .filled(true)
            .build();
        dl.add_rect(rect_min, rect_max, [0.20, 0.20, 0.25, 1.0])
            .build();

        let peak = pts.iter().map(|&(_, v)| v).fold(0.0f64, f64::max).max(0.25);
        let t0 = pts[0].0;
        let t1 = pts[pts.len() - 1].0 + crate::load::BUCKET_US;
        let span = (t1 - t0).max(1) as f32;
        let w = rect_max[0] - rect_min[0];
        let h = rect_max[1] - rect_min[1];
        let xy: Vec<[f32; 2]> = pts
            .iter()
            .map(|&(t, v)| {
                [
                    rect_min[0] + ((t - t0) as f32 / span) * w,
                    rect_max[1] - (v as f32 / peak as f32) * h,
                ]
            })
            .collect();
        // The 100% overload reference, drawn only when the scale shows it.
        if peak >= 1.0 {
            let y = rect_max[1] - (1.0 / peak as f32) * h;
            dl.add_line([rect_min[0], y], [rect_max[0], y], [0.90, 0.30, 0.30, 0.55])
                .build();
        }
        dl.add_polyline(xy, [0.35, 0.80, 1.00, 1.0])
            .thickness(1.5)
            .build();
    }
}
