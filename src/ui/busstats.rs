use crate::app::App;
use crate::load::{BusLoad, FrameClass};
use imgui::{Condition, TableColumnFlags, TableColumnSetup, TableFlags, Ui};

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
            [io.display_size[0] * 0.55, io.display_size[1] * 0.12],
            Condition::FirstUseEver,
        )
        .size([520.0, 420.0], Condition::FirstUseEver)
        .build(|| content(app, ui));
    app.show_bus_stats = open;
}

fn content(app: &mut App, ui: &Ui) {
    // NO_BORDERS_IN_BODY restricts column-resize dragging to the header row.
    let flags = TableFlags::BORDERS_INNER
        | TableFlags::ROW_BG
        | TableFlags::RESIZABLE
        | TableFlags::NO_BORDERS_IN_BODY
        | TableFlags::SCROLL_Y
        | TableFlags::SIZING_STRETCH_PROP;
    let Some(_table) = ui.begin_table_with_flags("bus_stats_table", 5, flags) else {
        return;
    };
    ui.table_setup_column_with(TableColumnSetup {
        flags: TableColumnFlags::WIDTH_STRETCH,
        init_width_or_weight: 1.0,
        ..TableColumnSetup::new("Statistic")
    });
    ui.table_setup_column_with(TableColumnSetup {
        flags: TableColumnFlags::WIDTH_FIXED,
        init_width_or_weight: 90.0,
        ..TableColumnSetup::new("Current / Last")
    });
    ui.table_setup_column_with(TableColumnSetup {
        flags: TableColumnFlags::WIDTH_FIXED,
        init_width_or_weight: 70.0,
        ..TableColumnSetup::new("Min")
    });
    ui.table_setup_column_with(TableColumnSetup {
        flags: TableColumnFlags::WIDTH_FIXED,
        init_width_or_weight: 70.0,
        ..TableColumnSetup::new("Max")
    });
    ui.table_setup_column_with(TableColumnSetup {
        flags: TableColumnFlags::WIDTH_FIXED,
        init_width_or_weight: 70.0,
        ..TableColumnSetup::new("Avg")
    });
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
