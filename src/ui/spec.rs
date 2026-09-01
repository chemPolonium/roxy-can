use crate::app::App;
use crate::spec::{Kind, Latch};
use imgui::{Condition, TableColumnFlags, TableColumnSetup, TableFlags, Ui};

/// What arrived that the databases said must not, or did not arrive at all.
/// A report rather than a status light: rows latch and stay until cleared, or
/// until the next run replaces them.
pub fn render(app: &mut App, ui: &Ui) {
    if !app.show_spec {
        return;
    }
    let io = ui.io();
    let mut open = app.show_spec;
    ui.window("Specification")
        .opened(&mut open)
        .position(
            [io.display_size[0] * 0.30, io.display_size[1] * 0.55],
            Condition::FirstUseEver,
        )
        .size([620.0, 260.0], Condition::FirstUseEver)
        .build(|| content(app, ui));
    app.show_spec = open;
}

fn content(app: &mut App, ui: &Ui) {
    // The two rules that can only say something where a period was declared.
    // A database that defaults `GenMsgCycleTime` to 0 makes every message look
    // event-triggered, and this line is the difference between "the bus is
    // clean" and "there was nothing to check".
    let periodic: usize = app
        .channels
        .iter()
        .filter_map(|c| c.dbc.as_ref())
        .map(|db| {
            db.messages
                .values()
                .filter(|m| m.cycle_us.is_some_and(|d| d > 0))
                .count()
        })
        .sum();
    ui.text("tolerance");
    ui.same_line();
    app.spec_tol_pct = threshold(app, ui, "spec_tol", app.spec_tol_pct, 0.0, 100.0, "%.0f%%");
    ui.same_line();
    ui.text("grace");
    ui.same_line();
    app.spec_grace = threshold(app, ui, "spec_grace", app.spec_grace, 1.0, 20.0, "%.0fx");
    ui.same_line();
    ui.text(format!("{periodic} periodic messages declared"));

    // One checkbox per rule, session-only: real logs carry other people's
    // traffic, and being able to stop looking at it without editing the project
    // is what keeps the report usable.
    for (i, kind) in Kind::ALL.iter().enumerate() {
        if i > 0 {
            ui.same_line();
        }
        let mut shown = app.spec_show[i];
        if ui.checkbox(kind.label(), &mut shown) {
            app.spec_show[i] = shown;
        }
    }
    ui.same_line();
    if ui.small_button("Export##spec") {
        app.export_spec_dialog();
    }
    ui.same_line();
    if ui.small_button("Clear##spec") {
        app.spec.clear();
    }
    ui.separator();

    let rows: Vec<((u8, u32, Kind), Latch)> = app
        .spec
        .rows
        .iter()
        .filter(|((_, _, kind), _)| app.spec_show[kind.index()])
        .map(|(k, l)| (*k, *l))
        .collect();
    if rows.is_empty() {
        ui.text_colored([0.5, 0.55, 0.6, 1.0], "no violations");
        return;
    }
    ui.text(format!("{} violations", rows.len()));

    let flags = TableFlags::BORDERS_INNER
        | TableFlags::ROW_BG
        | TableFlags::RESIZABLE
        | TableFlags::NO_BORDERS_IN_BODY
        | TableFlags::SCROLL_Y
        | TableFlags::SIZING_STRETCH_PROP;
    let Some(_table) = ui.begin_table_with_flags("spec_table", 8, flags) else {
        return;
    };
    ui.table_setup_column_with(TableColumnSetup {
        flags: TableColumnFlags::WIDTH_STRETCH,
        init_width_or_weight: 2.0,
        ..TableColumnSetup::new("Message")
    });
    for (label, width) in [
        ("Bus", 56.0),
        ("Rule", 82.0),
        ("Declared", 78.0),
        ("Measured", 78.0),
        ("Count", 52.0),
        ("First", 76.0),
        ("Last", 76.0),
    ] {
        ui.table_setup_column_with(TableColumnSetup {
            flags: TableColumnFlags::WIDTH_FIXED,
            init_width_or_weight: width,
            ..TableColumnSetup::new(label)
        });
    }
    ui.table_headers_row();

    for ((ch, id, kind), l) in rows {
        ui.table_next_row();
        ui.table_next_column();
        let name = app.message_name(ch, id).unwrap_or("not in database");
        ui.text(format!("{name} ({id:X})"));
        for column in [
            app.channel_name(ch),
            kind.label().to_string(),
            crate::spec::qty(kind, l.declared),
            crate::spec::qty(kind, l.measured),
            format!("{}", l.count),
            secs(l.first_t_us),
            secs(l.last_t_us),
        ] {
            ui.table_next_column();
            ui.text(column);
        }
    }
}

/// One integer threshold. The drag previews while it moves and the model only
/// hears about it when the gesture ends: a monitor setting typed one digit at a
/// time would otherwise re-judge the bus at every keystroke.
fn threshold(app: &mut App, ui: &Ui, key: &str, model: u64, lo: f32, hi: f32, fmt: &str) -> u64 {
    let label = format!("##{key}");
    let mut v = app.num_draft.shown(key, model as f64) as f32;
    let moved = imgui::Drag::new(&label)
        .speed((hi - lo) / 100.0)
        .range(lo, hi)
        .display_format(fmt)
        .build(ui, &mut v);
    let ended = ui.is_item_deactivated();
    let committed = app.num_draft.step(
        key,
        v as f64,
        moved,
        ui.is_item_deactivated_after_edit(),
        ended,
    );
    match committed {
        Some(val) => val.clamp(lo as f64, hi as f64).round() as u64,
        None => model,
    }
}

fn secs(t_us: u64) -> String {
    format!("{:.3} s", t_us as f64 / 1e6)
}
