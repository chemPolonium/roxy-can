use std::cmp::Ordering;
use std::sync::Mutex;

use crate::app::{App, PopupTarget, SigScope, TraceRow, TOOLBAR_H};
use crate::can::frame::{CanFrame, Direction};
use crate::ui::flags_color;
use crate::ui::idfilter::scope_combo;
use dear_imgui_rs::{
    Condition, ListClipper, SortDirection, TableColumnFlags, TableFlags, TableOptions,
    TableSizingPolicy, Ui,
};

/// Window index and frame targeted by the row context menu; must survive
/// across frames while the popup is open.
static CTX: Mutex<Option<(usize, CanFrame)>> = Mutex::new(None);
/// Same, for FlexRay rows (their menu only copies address and payload).
static CTX_FR: Mutex<Option<(usize, crate::trace::FrRow)>> = Mutex::new(None);

pub fn render(app: &mut App, ui: &Ui) {
    let io = ui.io();
    let n = app.trace_windows.len();
    for i in 0..n {
        if !app.trace_windows[i].opened {
            continue;
        }
        let mut open = true;
        let raw = app.trace_windows[i].name.clone();
        let title = if raw.trim().is_empty() {
            format!("Trace {}", i + 1)
        } else {
            raw
        };
        let off = i as f32 * 30.0;
        let focus = app.focus_title.as_deref() == Some(title.as_str());
        if focus {
            app.focus_title = None;
        }
        let mut window = ui
            .window(format!("{title}###trace{i}"))
            .opened(&mut open)
            .position([off, TOOLBAR_H + off], Condition::FirstUseEver)
            .size(
                [
                    io.display_size()[0] * 0.55,
                    io.display_size()[1] * 0.5 - TOOLBAR_H,
                ],
                Condition::FirstUseEver,
            );
        if focus {
            window = window.focused(true);
        }
        window.build(|| window_content(app, ui, i));
        app.trace_windows[i].opened = open;
    }
}

fn fmt_id(f: &CanFrame) -> String {
    if f.extended {
        format!("{:08X}x", f.id)
    } else {
        format!("{:03X}", f.id)
    }
}

fn fmt_data(f: &CanFrame) -> String {
    f.payload().iter().map(|b| format!("{b:02X} ")).collect()
}

/// One trace row as plain text, used for "Copy row".
fn fmt_row(app: &App, f: &CanFrame) -> String {
    let tag = f.flags.tag();
    let flags = if tag.is_empty() { "-" } else { tag };
    format!(
        "{:.6}  {}  {}  {}  {}  {}  {}  {}",
        f.t_us as f64 / 1e6,
        app.channel_name(f.channel),
        fmt_id(f),
        app.message_name(f.channel, f.id).unwrap_or("-"),
        f.len,
        flags,
        fmt_data(f).trim_end(),
        match f.dir {
            Direction::Rx => "Rx",
            Direction::Tx => "Tx",
        }
    )
}

fn window_content(app: &mut App, ui: &Ui, i: usize) {
    // CAN and FlexRay rows share the one table, interleaved by time --
    // the row cache is a merged, filtered TraceRow list (see
    // `App::sync_trace_rows`), so no separate FR section exists here.
    can_table(app, ui, i);
}

fn can_table(app: &mut App, ui: &Ui, i: usize) {
    // The filtered row cache rebuilds on the text gate, like every number
    // readout; the clipper then submits only the visible slice per frame.
    app.sync_trace_rows(i);
    let matching = app.trace_windows[i].rows.len();
    ui.text(format!(
        "{matching} matching frames · ring holds {}",
        app.snap.trace_len
    ));
    // The FlexRay watch's share of the table, when any FR frames exist:
    // ring depth plus the head-trim count the FR ring dropped.
    if !app.snap.fr_trace.is_empty() {
        ui.same_line();
        let fr_note = if app.snap.fr_dropped > 0 {
            format!(
                "· FlexRay {} 帧（早期裁掉 {}）",
                app.snap.fr_trace.len(),
                app.snap.fr_dropped
            )
        } else {
            format!("· FlexRay {} 帧", app.snap.fr_trace.len())
        };
        ui.text_colored([0.55, 0.8, 1.0, 1.0], fr_note);
    }
    // Head trims are accounted either way: with the archive they are
    // merely moved to disk (the export still covers them), without it
    // they are real loss.
    let (dropped, first_dropped_t_us) = app.snap.trace.head_loss();
    if dropped > 0 {
        ui.same_line();
        let since = first_dropped_t_us
            .map(|t| format!("since {:.3} s", t as f64 / 1e6))
            .unwrap_or_default();
        let archived = app.snap.trace.archive().is_some();
        let msg = if archived {
            format!("· 已归档 {dropped} 帧（导出包含）{since}")
        } else {
            format!("· head trimmed: {dropped} frame(s) {since}")
        };
        ui.text_colored([1.0, 0.8, 0.4, 1.0], msg);
        if ui.is_item_hovered() {
            ui.tooltip_text(if archived {
                "超出容量的旧帧已写入临时归档文件，Export Trace 会包含它们"
            } else {
                "超出容量的旧帧被裁掉且归档不可用"
            });
        }
    }
    let new_scope = scope_combo(
        app,
        ui,
        &format!("##tscope{i}"),
        app.trace_windows[i].scope,
        PopupTarget::Trace(i),
    );
    app.trace_windows[i].scope = new_scope;
    ui.same_line();
    ui.set_next_item_width(120.0);
    ui.input_text(format!("##tfilter{i}"), &mut app.trace_windows[i].filter)
        .build();
    ui.same_line();
    ui.set_next_item_width(52.0);
    ui.combo_simple_string(
        format!("##tdir{i}"),
        &mut app.trace_windows[i].dir,
        &["All", "Rx", "Tx"],
    );
    // The payload / kind / DBC-only / time-range filters collapse behind
    // this toggle: the main row stays short enough for a half-width
    // window, and the extras only take space when actually wanted.
    ui.same_line();
    if ui.button(format!("筛选##tf{i}")) {
        app.trace_windows[i].filters_open = !app.trace_windows[i].filters_open;
    }
    if app.trace_windows[i].filters_open {
        ui.same_line();
        ui.set_next_item_width(84.0);
        ui.input_text(format!("##tpayload{i}"), &mut app.trace_windows[i].payload)
            .hint("payload 11 22")
            .build();
        if ui.is_item_hovered() {
            ui.tooltip_text("payload 字节搜索：hex 对、空格可选；匹配含此序列的帧。无法解析时不过滤");
        }
        ui.same_line();
        ui.set_next_item_width(58.0);
        ui.combo_simple_string(
            format!("##tflags{i}"),
            &mut app.trace_windows[i].flags_kind,
            &["Any", "Data", "FD", "RTR", "Error"],
        );
        ui.same_line();
        ui.checkbox(
            format!("DBC##tdbc{i}"),
            &mut app.trace_windows[i].dbc_only,
        );
        ui.same_line();
        // Expand FlexRay rows into their decoded signal children (needs
        // the watch's description database).
        ui.checkbox(
            format!("FR 信号##tfrx{i}"),
            &mut app.trace_windows[i].fr_expand,
        );
        if ui.is_item_hovered() {
            ui.tooltip_text("FlexRay 帧下方展开解码后的信号值（需挂接带数据库的 FR 监听）");
        }
        // Time window: two small numeric boxes in seconds -- empty means
        // unbounded on that side. Same parse semantics as the filter's
        // time-range check (blank/invalid = no bound).
        ui.same_line();
        ui.set_next_item_width(52.0);
        ui.input_text(format!("##tfrom{i}"), &mut app.trace_windows[i].time_from)
            .hint("从 s")
            .build();
        if ui.is_item_hovered() {
            ui.tooltip_text("时间范围下界（秒）：早于此的帧不显示；留空 = 不限");
        }
        ui.same_line();
        ui.set_next_item_width(52.0);
        ui.input_text(format!("##tto{i}"), &mut app.trace_windows[i].time_to)
            .hint("到 s")
            .build();
        if ui.is_item_hovered() {
            ui.tooltip_text("时间范围上界（秒）：晚于此的帧不显示；留空 = 不限");
        }
    }
    ui.same_line();
    // Clear empties the display (ring + archive); the filter controls
    // keep their settings -- they are the viewer's lens, not its content.
    if ui.button(format!("Clear##tf{i}")) {
        app.send(crate::bus::BusCommand::ClearTrace);
    }
    ui.same_line();
    if ui.button(format!("Export##tx{i}")) {
        app.export_trace_dialog(i);
    }
    ui.separator();

    // NO_BORDERS_IN_BODY restricts column-resize dragging to the header row.
    let tbl_flags = TableFlags::BORDERS_INNER
        | TableFlags::ROW_BG
        | TableFlags::RESIZABLE
        | TableFlags::NO_BORDERS_IN_BODY
        | TableFlags::SCROLL_Y
        | TableFlags::SORTABLE
        | TableFlags::SORT_TRISTATE;
    let opts = TableOptions::from(tbl_flags).sizing_policy(TableSizingPolicy::StretchProp);
    let Some(_table) = ui.begin_table_with_flags(format!("trace_table{i}"), 8, opts) else {
        return;
    };
    // "{:.6}" timestamp: up to ~10 chars
    ui.table_setup_column(
        "Time",
        TableColumnFlags::NONE,
        Some(dear_imgui_rs::TableColumnWidth::fixed(76.0)),
    );
    ui.table_setup_column(
        "Bus",
        TableColumnFlags::NONE,
        Some(dear_imgui_rs::TableColumnWidth::fixed(60.0)),
    );
    // extended IDs render as "1FFFFFFFx" (9 chars)
    ui.table_setup_column(
        "ID",
        TableColumnFlags::NONE,
        Some(dear_imgui_rs::TableColumnWidth::fixed(68.0)),
    );
    ui.table_setup_column("Name", TableColumnFlags::NONE, None);
    ui.table_setup_column(
        "Len",
        TableColumnFlags::NONE,
        Some(dear_imgui_rs::TableColumnWidth::fixed(36.0)),
    );
    ui.table_setup_column(
        "Flags",
        TableColumnFlags::NONE,
        Some(dear_imgui_rs::TableColumnWidth::fixed(44.0)),
    );
    // A full 64-byte FD payload needs room; let it stretch with the window.
    ui.table_setup_column(
        "Data",
        TableColumnFlags::NONE,
        Some(dear_imgui_rs::TableColumnWidth::stretch(1.4)),
    );
    ui.table_setup_column(
        "Dir",
        TableColumnFlags::NONE,
        Some(dear_imgui_rs::TableColumnWidth::fixed(34.0)),
    );
    // Freeze the header row so it stays visible while scrolling.
    ui.table_setup_scroll_freeze(0, 1);
    ui.table_headers_row();

    // Take the row cache out: the sort needs `app` for names while the
    // clipper below needs the rows as an owned list, and the right-click
    // popup afterwards needs `app` mutably again.
    let mut rows = std::mem::take(&mut app.trace_windows[i].rows);

    // Newest first (the default order); sorted when a column header is
    // clicked, back to default on the third click (tri-state). The row
    // cache is the app's; a sort re-orders it in place.
    if let Some(mut specs) = ui.table_get_sort_specs()
        && let Some(s) = specs.iter().next()
    {
        let col = s.column_index.get();
        let asc = s.sort_direction == SortDirection::Ascending;
        rows.sort_by(|a, b| sort_frame(app, col, a, b, asc));
        specs.clear_dirty(ui);
    }

    // Virtual scrolling: the clipper submits only the visible slice of
    // the (possibly very long) filtered row list.
    let clip = ListClipper::new(rows.len()).begin(ui);
    for r in clip.iter() {
        let row = &rows[r];
        let mut hovered = false;
        ui.table_next_row();
        #[expect(clippy::needless_late_init)]
        let can_ctx: Option<CanFrame>;
        match row {
            TraceRow::Can(f) => {
                if f.is_error() {
                    ui.table_set_row_bg1_color([0.55, 0.12, 0.12, 0.35]);
                } else if f.is_remote() {
                    ui.table_set_row_bg1_color([0.35, 0.22, 0.55, 0.25]);
                }
                if !ui.table_next_column() {
                    continue;
                }
                ui.text(format!("{:.6}", f.t_us as f64 / 1e6));
                hovered |= ui.is_item_hovered();
                ui.table_next_column();
                ui.text(app.channel_name(f.channel));
                hovered |= ui.is_item_hovered();
                ui.table_next_column();
                ui.text(fmt_id(f));
                hovered |= ui.is_item_hovered();
                ui.table_next_column();
                match app.message_name(f.channel, f.id) {
                    Some(name) => ui.text(name),
                    None => ui.text("-"),
                }
                hovered |= ui.is_item_hovered();
                ui.table_next_column();
                ui.text(format!("{}", f.len));
                hovered |= ui.is_item_hovered();
                ui.table_next_column();
                ui.text_colored(flags_color(f.flags), f.flags.tag());
                hovered |= ui.is_item_hovered();
                ui.table_next_column();
                ui.text(fmt_data(f));
                hovered |= ui.is_item_hovered();
                ui.table_next_column();
                match f.dir {
                    Direction::Rx => ui.text_colored([0.6, 0.65, 0.7, 1.0], "Rx"),
                    Direction::Tx => ui.text_colored([1.0, 0.65, 0.2, 1.0], "Tx"),
                }
                hovered |= ui.is_item_hovered();
                can_ctx = if hovered
                    && ui.is_mouse_released(dear_imgui_rs::MouseButton::Right)
                {
                    Some(*f)
                } else {
                    None
                };
            }
            TraceRow::Fr(fr) => {
                // A distinct cool tint keeps the FR stream readable inside
                // the CAN rows; the reception channel rides the Bus cell.
                ui.table_set_row_bg1_color([0.15, 0.35, 0.55, 0.20]);
                if !ui.table_next_column() {
                    continue;
                }
                ui.text(format!("{:.6}", fr.t_us as f64 / 1e6));
                hovered |= ui.is_item_hovered();
                ui.table_next_column();
                ui.text_colored(
                    [0.55, 0.8, 1.0, 1.0],
                    match fr.ab {
                        0 => "FR A",
                        1 => "FR B",
                        _ => "FR",
                    },
                );
                hovered |= ui.is_item_hovered();
                ui.table_next_column();
                ui.text_colored([0.55, 0.8, 1.0, 1.0], format!("{}.{}", fr.slot, fr.cycle));
                hovered |= ui.is_item_hovered();
                ui.table_next_column();
                // The description database wins; a name carried by the
                // log itself (CANoe ASC) is the fallback.
                match app
                    .fr_db
                    .as_ref()
                    .and_then(|db| db.frame_at(fr.slot, fr.cycle, fr.ab))
                    .map(|f| f.name.as_str())
                    .or(fr.name.as_deref())
                {
                    Some(name) => ui.text(name),
                    None => ui.text("-"),
                }
                hovered |= ui.is_item_hovered();
                ui.table_next_column();
                ui.text(format!("{}", fr.payload.len()));
                ui.table_next_column();
                ui.text("-");
                ui.table_next_column();
                let hex: String =
                    fr.payload.iter().map(|b| format!("{b:02X} ")).collect();
                ui.text(hex.trim_end());
                ui.table_next_column();
                ui.text_colored([0.6, 0.65, 0.7, 1.0], "Rx");
                if hovered && ui.is_mouse_released(dear_imgui_rs::MouseButton::Right) {
                    *CTX_FR.lock().unwrap() = Some((i, (*fr).clone()));
                    ui.open_popup(format!("trace_fr_ctx{i}"));
                }
                can_ctx = None;
            }
            TraceRow::FrSig {
                signal, value, ..
            } => {
                // A decoded signal child under its FR frame row.
                ui.table_next_row();
                if !ui.table_next_column() {
                    continue;
                }
                ui.text("-");
                ui.table_next_column();
                ui.text("-");
                ui.table_next_column();
                ui.text("  └");
                ui.same_line();
                ui.text_colored([0.55, 0.8, 1.0, 1.0], signal);
                ui.table_next_column();
                ui.text("-");
                ui.table_next_column();
                ui.text("-");
                ui.table_next_column();
                ui.text("-");
                ui.table_next_column();
                ui.text_colored([0.75, 0.92, 1.0, 1.0], value);
                ui.table_next_column();
                ui.text("-");
                can_ctx = None;
            }
        }
        if let Some(f) = can_ctx {
            *CTX.lock().unwrap() = Some((i, f));
            ui.open_popup(format!("trace_row_ctx{i}"));
        }
    }
    // The rows go back before the popup: its menu mutates the window's
    // filter state.
    app.trace_windows[i].rows = rows;

    // The FlexRay row menu: the slot address and the payload, copied.
    if let Some(_p) = ui.begin_popup(format!("trace_fr_ctx{i}"))
        && let Some((pi, r)) = CTX_FR.lock().unwrap().clone()
        && pi == i
    {
        ui.text(format!("FR slot {}.{}", r.slot, r.cycle));
        ui.separator();
        let hex: String = r.payload.iter().map(|b| format!("{b:02X} ")).collect();
        if ui.menu_item("Copy payload") {
            crate::clipboard::Clipboard.set(hex.trim_end());
        }
        if ui.menu_item("Copy slot.cycle") {
            crate::clipboard::Clipboard.set(&format!("{}.{}", r.slot, r.cycle));
        }
    }

    if let Some(_p) = ui.begin_popup(format!("trace_row_ctx{i}"))
        && let Some((pi, f)) = *CTX.lock().unwrap()
        && pi == i
    {
        let name = app.message_name(f.channel, f.id).unwrap_or("-");
        ui.text(format!(
            "{}  {}  {name}",
            app.channel_name(f.channel),
            fmt_id(&f)
        ));
        ui.separator();
        if ui.menu_item(format!("Filter this ID ({})", fmt_id(&f))) {
            app.trace_windows[i].filter = format!("{:03X}", f.id);
        }
        if ui.menu_item("Clear filter") {
            let w = &mut app.trace_windows[i];
            w.filter.clear();
            w.dir = 0;
            w.dbc_only = false;
            w.scope = SigScope::All;
        }
        let addable = !f.is_error() && !f.is_remote();
        if ui.menu_item_enabled_selected("Add to Interactive Generator", None::<&str>, false, addable)
        {
            // Add, then re-shape the fresh row via command: keep the flags
            // it was seen with, and for an id no database declares, keep
            // the observed payload. `add_tx` skips a duplicate row, so the
            // length compare tells a fresh add from an existing one.
            let was_len = app.snap.tx.len();
            app.add_tx(f.channel, f.id);
            if app.snap.tx.len() > was_len {
                let known = app
                    .channel_dbc(f.channel)
                    .is_some_and(|db| db.messages.contains_key(&(f.id, f.extended)));
                let data_text = if known {
                    None
                } else {
                    Some(
                        f.data[..f.len as usize]
                            .iter()
                            .map(|b| format!("{b:02X}"))
                            .collect::<Vec<_>>()
                            .join(" "),
                    )
                };
                app.send(crate::bus::BusCommand::SetEntryConfig {
                    ch: f.channel,
                    id: f.id,
                    active: true,
                    cycle_us: app
                        .dbc_cycle_us(f.channel, f.id)
                        .unwrap_or(crate::app::DEFAULT_TX_CYCLE_US),
                    fd: f.flags.contains(crate::can::frame::FrameFlags::FD),
                    flags: Some(f.flags),
                    data_text,
                    srcs: Vec::new(),
                });
            }
        }
        ui.separator();
        // R1-3 跳转时刻：回放模式下可跳到该帧的时间点。
        if ui.menu_item(format!(
            "跳转到此时刻 ({:.3}s)",
            f.t_us as f64 / 1e6
        )) {
            app.seek_replay_seconds(f.t_us as f64 / 1e6);
        }
        if ui.menu_item("Copy row") {
            crate::clipboard::Clipboard.set(&fmt_row(app, &f));
        }
        if ui.menu_item("Copy ID") {
            crate::clipboard::Clipboard.set(&fmt_id(&f));
        }
    }
}

fn sort_frame(app: &App, col: usize, a: &TraceRow, b: &TraceRow, asc: bool) -> Ordering {
    // Bus sort keeps FR rows together after every CAN bus; the address
    // column is a CAN id or an FR slot, both u32s.
    let bus = |r: &TraceRow| match r {
        TraceRow::Can(f) => f.channel as u32,
        TraceRow::Fr(r) => 0x1000 + r.ab as u32,
        TraceRow::FrSig { .. } => 0x1000 + 3,
    };
    let addr = |r: &TraceRow| match r {
        TraceRow::Can(f) => f.id,
        TraceRow::Fr(r) => r.slot as u32,
        TraceRow::FrSig { slot, .. } => *slot as u32,
    };
    let name = |r: &TraceRow| match r {
        TraceRow::Can(f) => app
            .message_name(f.channel, f.id)
            .unwrap_or_default()
            .to_string(),
        TraceRow::Fr(_) => String::new(),
        TraceRow::FrSig { signal, .. } => signal.clone(),
    };
    let len = |r: &TraceRow| match r {
        TraceRow::Can(f) => f.len as usize,
        TraceRow::Fr(r) => r.payload.len(),
        TraceRow::FrSig { .. } => 0,
    };
    let flags_rank = |r: &TraceRow| match r {
        TraceRow::Can(f) => (f.is_fd(), f.esi(), f.brs()),
        TraceRow::Fr(_) | TraceRow::FrSig { .. } => (false, false, false),
    };
    let payload = |r: &TraceRow| match r {
        TraceRow::Can(f) => f.payload().to_vec(),
        TraceRow::Fr(r) => r.payload.clone(),
        TraceRow::FrSig { .. } => Vec::new(),
    };
    let dir = |r: &TraceRow| match r {
        TraceRow::Can(f) => f.dir as u8,
        TraceRow::Fr(_) => 0,
        TraceRow::FrSig { .. } => 3,
    };
    let ord = match col {
        0 => a.t_us().cmp(&b.t_us()),
        1 => bus(a).cmp(&bus(b)),
        2 => addr(a).cmp(&addr(b)),
        3 => name(a).cmp(&name(b)),
        4 => len(a).cmp(&len(b)),
        5 => flags_rank(a).cmp(&flags_rank(b)),
        6 => payload(a).cmp(&payload(b)),
        _ => dir(a).cmp(&dir(b)),
    };
    if asc { ord } else { ord.reverse() }
}
