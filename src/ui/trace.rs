use std::cmp::Ordering;
use std::sync::Mutex;

use crate::app::{App, PopupTarget, SigScope, TOOLBAR_H, TracePick, TraceRow};
use crate::can::frame::{CanFrame, Direction};
use crate::ui::flags_color;
use crate::ui::idfilter::scope_combo;
use crate::workspace::TraceWin;
use dear_imgui_rs::{
    Condition, ListClipper, SortDirection, TableColumnFlags, TableFlags, TableOptions,
    TableSizingPolicy, Ui,
};

/// Window index and frame targeted by the row context menu; must survive
/// across frames while the popup is open.
static CTX: Mutex<Option<(usize, CanFrame)>> = Mutex::new(None);
/// Same, for FlexRay rows (their menu only copies address and payload).
static CTX_FR: Mutex<Option<(usize, crate::trace::FrRow)>> = Mutex::new(None);
/// Same, for Ethernet rows.
static CTX_ETH: Mutex<Option<(usize, crate::eth::EthFrame)>> = Mutex::new(None);

pub enum TraceCmd {
    ClearTrace,
    Export(usize),
    OpenPicker(usize),
    ClearFocus,
    AddEntry {
        ch: u8,
        id: u32,
        fd: bool,
        cycle_us: u64,
        flags: crate::can::frame::FrameFlags,
        data_text: Option<String>,
    },
}

pub fn render(app: &mut App, ui: &Ui) {
    let mut wins = std::mem::take(&mut app.trace_windows);
    let mut cmds = Vec::new();
    for (i, win) in wins.iter_mut().enumerate() {
        win.show(ui, app, i, &mut cmds);
    }
    app.trace_windows = wins;
    app.apply_trace_cmds(cmds);
}

impl TraceWin {
    pub fn show(&mut self, ui: &Ui, app: &App, i: usize, cmds: &mut Vec<TraceCmd>) {
        if !self.opened {
            return;
        }
        let io = ui.io();
        let mut open = true;
        let raw = self.name.clone();
        let title = if raw.trim().is_empty() {
            format!("Trace {}", i + 1)
        } else {
            raw
        };
        let off = i as f32 * 30.0;
        let focus = app.focus_title.as_deref() == Some(title.as_str());
        if focus {
            cmds.push(TraceCmd::ClearFocus);
        }
        let (pos, size) = crate::ui::fit_first_use(
            io.display_size(),
            [off, TOOLBAR_H + off],
            [
                io.display_size()[0] * 0.55,
                io.display_size()[1] * 0.5 - TOOLBAR_H,
            ],
        );
        let mut window = ui
            .window(format!("{title}###trace{i}"))
            .opened(&mut open)
            .position(pos, Condition::FirstUseEver)
            .size(size, Condition::FirstUseEver);
        if focus {
            window = window.focused(true);
        }
        window.build(|| self.content(app, ui, i, cmds));
        self.opened = open;
    }
}

fn fmt_id(f: &CanFrame) -> String {
    if f.extended {
        format!("{:08X}x", f.id)
    } else {
        format!("{:03X}", f.id)
    }
}

/// The row menu's "Filter this ID": the window is left showing the row the menu
/// was opened on, and only that row. The plain text is a substring search, so
/// `1AB` also matches `1AB0` and `0x1AB...`; the `id:` prefix is the exact form,
/// and the `x` suffix has to travel with a 29-bit id because the frame class is
/// part of the address.
#[allow(dead_code)] // the headless tests drive these helpers directly
pub(crate) fn filter_to_id(app: &mut App, win: usize, f: &CanFrame) {
    filter_to_id_win(&mut app.trace_windows[win], f);
}

fn filter_to_id_win(win: &mut TraceWin, f: &CanFrame) {
    win.filter = format!("id:{}", fmt_id(f));
}

/// The cursor pair's rows in every row menu, offered identically for CAN and
/// FlexRay: a cursor is an instant, and an instant does not care which cable the
/// row came off. Setting one that exists moves it; the clear only appears once
/// there is something to clear.
fn cursor_menu(win: &mut TraceWin, ui: &Ui, t_us: u64) {
    for (n, label) in [(0, "Set cursor A"), (1, "Set cursor B")] {
        if ui.menu_item(label) {
            win.mark_us[n] = Some(t_us);
        }
        if ui.is_item_hovered() {
            ui.tooltip_text("The header shows A, B and their Δt; this row is tinted");
        }
    }
    if win.mark_us.iter().any(Option::is_some) && ui.menu_item("Clear cursors") {
        win.mark_us = [None, None];
    }
}

/// "Clear filter" as both row menus offer it, in one place so the CAN and
/// FlexRay menus cannot drift again. Every condition the operator typed or
/// ticked goes: the text box, the payload search, the frame kind, 仅 DBC, the
/// time range, the direction, and the scope back to All. The window's Manual
/// pick set stays -- that is a curated list, not a condition typed on a whim,
/// and a one-key clear must not quietly eat it.
#[allow(dead_code)] // the headless tests drive these helpers directly
pub(crate) fn clear_filter(app: &mut App, win: usize) {
    clear_filter_win(&mut app.trace_windows[win]);
}

fn clear_filter_win(w: &mut TraceWin) {
    w.filter.clear();
    w.payload.clear();
    w.time_from.clear();
    w.time_to.clear();
    w.dir = 0;
    w.flags_kind = 0;
    w.dbc_only = false;
    w.scope = SigScope::All;
}

/// The cursor pair's row tint, in the plot's two colours at background
/// strength: a marked row is findable at a glance, and the row is still
/// readable through it.
const MARK_COLOR: [[f32; 4]; 2] = [[0.95, 0.85, 0.40, 0.22], [0.45, 0.80, 0.95, 0.22]];

/// Which cursor tints this row, if any. A cursor is an instant, so a row that
/// carries both (only possible at the same microsecond) reads as A.
fn mark_tint(mark_us: [Option<u64>; 2], t_us: u64) -> Option<[f32; 4]> {
    mark_us
        .iter()
        .position(|m| *m == Some(t_us))
        .map(|n| MARK_COLOR[n])
}

fn fmt_data(f: &CanFrame) -> String {
    f.payload().iter().map(|b| format!("{b:02X} ")).collect()
}

/// The picked row's background. It rides the table's *alternating* slot (bg0)
/// while the error / remote / cursor-mark tints ride bg1, so one row can be
/// picked and marked at once and still say both; it is stronger than a mark
/// because it is the row the keyboard is about to act on.
const PICK_COLOR: [f32; 4] = [0.25, 0.42, 0.68, 0.45];

/// One FlexRay frame's Bus cell: the 路 as the user named it, with the reception
/// channel appended when the row knows it. The table and "Copy row" print this
/// through one function, so a copied row reads like the row it was copied from.
fn fr_bus_cell(app: &App, bus: u8, ab: u8) -> String {
    format!(
        "{}{}",
        app.fr_bus_name(bus),
        match ab {
            0 => " A",
            1 => " B",
            _ => "",
        }
    )
}

/// One FlexRay frame row as plain text -- [`fmt_row`]'s twin with the same
/// columns in the same order, so a CAN row and a FlexRay row paste out
/// comparable. The flags column has nothing to report for a FlexRay frame, so it
/// reads `-` exactly as the table's cell does.
fn fmt_fr_row(app: &App, r: &crate::trace::FrRow) -> String {
    let hex: String = r.payload.iter().map(|b| format!("{b:02X} ")).collect();
    format!(
        "{:.6}  {}  {}.{}  {}  {}  -  {}  Rx",
        r.t_us as f64 / 1e6,
        fr_bus_cell(app, r.bus, r.ab),
        r.slot,
        r.cycle,
        app.fr_row_name(r).unwrap_or("-"),
        r.payload.len(),
        hex.trim_end()
    )
}

/// One Ethernet frame as plain text, [`fmt_row`]'s L2 twin.
fn fmt_eth_row(app: &App, f: &crate::eth::EthFrame) -> String {
    let hex: String = f.payload.iter().map(|b| format!("{b:02X} ")).collect();
    format!(
        "{:.6}  {}  {} > {}  vlan {}  0x{:04X}  {}  {}  {}",
        f.t_us as f64 / 1e6,
        app.eth_bus_label(f.bus),
        crate::eth::db::mac_text(&f.src),
        crate::eth::db::mac_text(&f.dst),
        f.vlan
            .map(|v| v.to_string())
            .unwrap_or_else(|| "-".to_string()),
        f.ethertype,
        app.eth_row_name(f),
        f.len(),
        hex.trim_end()
    )
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

impl TraceWin {
    fn content(&mut self, app: &App, ui: &Ui, i: usize, cmds: &mut Vec<TraceCmd>) {
        // CAN, FlexRay and Ethernet rows share the one table, interleaved by
        // time -- the row cache is a merged, filtered TraceRow list (see
        // `App::sync_trace_rows`), so no separate Ethernet/FR section exists.
        can_table(self, app, ui, i, cmds);
    }
}

fn can_table(win: &mut TraceWin, app: &App, ui: &Ui, i: usize, cmds: &mut Vec<TraceCmd>) {
    // The filtered row cache was refreshed before the draw (see
    // `App::sync_observer_text`); the clipper then submits only the visible
    // slice per frame.
    let matching = win.rows.len();
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
                "· FlexRay {} frames ({} head-trimmed)",
                app.snap.fr_trace.len(),
                app.snap.fr_dropped
            )
        } else {
            format!("· FlexRay {} frames", app.snap.fr_trace.len())
        };
        ui.text_colored([0.55, 0.8, 1.0, 1.0], fr_note);
    }
    if !app.snap.eth_trace.is_empty() {
        ui.same_line();
        ui.text_colored(
            [0.55, 0.9, 0.7, 1.0],
            format!("· Ethernet {} frames", app.snap.eth_trace.len()),
        );
    }
    // The cursor pair, worded as the plot words it -- to the microsecond here,
    // because that is what separates two rows. Only once something is marked:
    // an empty readout next to the frame count reads as a broken counter.
    let mark_us = win.mark_us;
    if mark_us.iter().any(Option::is_some) {
        ui.same_line();
        ui.text_colored(
            [0.95, 0.85, 0.4, 1.0],
            crate::ui::cursor_head(
                mark_us[0].map(|t| t as f64 / 1e6),
                mark_us[1].map(|t| t as f64 / 1e6),
                6,
            ),
        );
    }
    // The picked row's own line, and the only place its two keys are written
    // down: a highlight does not teach that Up/Down walk the list or that Ctrl+C
    // copies the row. It appears only once there is a pick, so the header never
    // carries a hint about a state the window is not in.
    if win.pick.is_some() {
        ui.same_line();
        ui.text_colored(
            [0.55, 0.8, 1.0, 1.0],
            "Row selected · ↑/↓ move · Ctrl+C copies it",
        );
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
            format!("· {dropped} frame(s) archived (export includes them) {since}")
        } else {
            format!("· head trimmed: {dropped} frame(s) {since}")
        };
        ui.text_colored([1.0, 0.8, 0.4, 1.0], msg);
        if ui.is_item_hovered() {
            ui.tooltip_text(if archived {
                "Frames past capacity were written to a temporary archive file; Export Trace includes them"
            } else {
                "Frames past capacity were trimmed and no archive is available"
            });
        }
    }
    let mut open_picker = false;
    let new_scope = scope_combo(
        app,
        ui,
        &format!("##tscope{i}"),
        win.scope,
        PopupTarget::Trace(i),
        &mut open_picker,
    );
    if open_picker {
        cmds.push(TraceCmd::OpenPicker(i));
    }
    win.scope = new_scope;
    ui.same_line();
    const FILTER_HINT: &str = "name / hex substring · id:1AB · slot:13";
    ui.set_next_item_width(crate::ui::hint_width(ui, FILTER_HINT, 120.0));
    ui.input_text(format!("##tfilter{i}"), &mut win.filter)
        .hint(FILTER_HINT)
        .build();
    ui.same_line();
    ui.set_next_item_width(52.0);
    ui.combo_simple_string(format!("##tdir{i}"), &mut win.dir, &["All", "Rx", "Tx"]);
    // The payload / kind / DBC-only / time-range filters collapse behind
    // this toggle: the main row stays short enough for a half-width
    // window, and the extras only take space when actually wanted.
    ui.same_line();
    // Closed and still filtering: say so on the button, and name the
    // conditions on hover. Otherwise the row is invisible while it works.
    let hidden = if win.filters_open {
        Vec::new()
    } else {
        win.hidden_conds()
    };
    let filter_label = if hidden.is_empty() {
        "Filter".to_string()
    } else {
        format!("Filter ·{}", hidden.len())
    };
    if ui.button(format!("{filter_label}##tf{i}")) {
        win.filters_open = !win.filters_open;
    }
    if !hidden.is_empty() && ui.is_item_hovered() {
        ui.tooltip_text(format!(
            "Hidden filters still apply: {}",
            hidden.join(" · ")
        ));
    }
    // The main row is the filter lens; its extra conditions, Reset, Clear and
    // Export live on the second row so English labels never run past the
    // window edge. The first item starts that row by itself -- a NewLine
    // here would add a blank line between the two rows.
    if win.filters_open {
        const PAYLOAD_HINT: &str = "payload 11 22";
        ui.set_next_item_width(crate::ui::hint_width(ui, PAYLOAD_HINT, 84.0));
        ui.input_text(format!("##tpayload{i}"), &mut win.payload)
            .hint(PAYLOAD_HINT)
            .build();
        if ui.is_item_hovered() {
            ui.tooltip_text(
                "Payload byte search: hex pairs, spaces optional; matches frames with that sequence. Unparsable input filters nothing",
            );
        }
        ui.same_line();
        ui.set_next_item_width(58.0);
        ui.combo_simple_string(
            format!("##tflags{i}"),
            &mut win.flags_kind,
            &["Any", "Data", "FD", "RTR", "Error"],
        );
        ui.same_line();
        ui.checkbox(format!("DBC##tdbc{i}"), &mut win.dbc_only);
        ui.same_line();
        // Expand FlexRay rows into their decoded signal children (needs the
        // arriving row's own cluster description).
        ui.checkbox(format!("FR signals##tfrx{i}"), &mut win.fr_expand);
        if ui.is_item_hovered() {
            ui.tooltip_text(
                "Expand decoded signal values under FlexRay frames (needs that bus's cluster description database)",
            );
        }
        // Time window: two small numeric boxes in seconds -- empty means
        // unbounded on that side. Same parse semantics as the filter's
        // time-range check (blank/invalid = no bound).
        ui.same_line();
        const FROM_HINT: &str = "from s";
        ui.set_next_item_width(crate::ui::hint_width(ui, FROM_HINT, 52.0));
        ui.input_text(format!("##tfrom{i}"), &mut win.time_from)
            .hint(FROM_HINT)
            .build();
        if ui.is_item_hovered() {
            ui.tooltip_text("Lower time bound (s): earlier frames stay hidden; empty = unbounded");
        }
        ui.same_line();
        const TO_HINT: &str = "to s";
        ui.set_next_item_width(crate::ui::hint_width(ui, TO_HINT, 52.0));
        ui.input_text(format!("##tto{i}"), &mut win.time_to)
            .hint(TO_HINT)
            .build();
        if ui.is_item_hovered() {
            ui.tooltip_text("Upper time bound (s): later frames stay hidden; empty = unbounded");
        }
        // Reset/Clear/Export continue the extras row.
        ui.same_line();
    }
    let filters_active = !win.filter.trim().is_empty()
        || win.scope != SigScope::All
        || win.dir != 0
        || win.dbc_only
        || !win.payload.trim().is_empty()
        || win.flags_kind != 0
        || !win.time_from.trim().is_empty()
        || !win.time_to.trim().is_empty();
    if filters_active {
        let _reset = ui.push_style_color(dear_imgui_rs::StyleColor::Text, [1.0, 0.75, 0.35, 1.0]);
        if ui.button(format!("Reset filters##treset{i}")) {
            clear_filter_win(win);
        }
        _reset.pop();
        if ui.is_item_hovered() {
            ui.tooltip_text(
                "Clear the text, scope, direction and every filter condition; captured rows are kept\n\
                 中文：清空文本、作用域、方向和全部过滤条件，已捕获的行保留",
            );
        }
        ui.same_line();
    }
    // Clear empties the display (ring + archive); the filter controls
    // keep their settings -- they are the viewer's lens, not its content.
    if ui.button(format!("Clear trace##tf{i}")) {
        ui.open_popup(format!("trace_clear_confirm{i}"));
    }
    if ui.is_item_hovered() {
        ui.tooltip_text(
            "Empty the captured trace (ring and archive); filter settings stay as they are",
        );
    }
    if let Some(_p) = ui.begin_popup(format!("trace_clear_confirm{i}")) {
        ui.text("Clear the captured trace?");
        ui.text_disabled("The ring and archive are emptied; filter settings stay.");
        if ui.button(format!("Clear##tclearok{i}")) {
            cmds.push(TraceCmd::ClearTrace);
            ui.close_current_popup();
        }
        ui.same_line();
        if ui.button(format!("Cancel##tclearcancel{i}")) {
            ui.close_current_popup();
        }
    }
    ui.same_line();
    if ui.button(format!("Export##tx{i}")) {
        cmds.push(TraceCmd::Export(i));
    }
    if ui.is_item_hovered() {
        ui.tooltip_text(
            "Export the filtered rows: ASC for CAN/FlexRay, PCAP for Ethernet (choose the extension in the save dialog)\n\
             中文：ASC 写 CAN/FlexRay，PCAP 写以太网，按保存扩展名选择",
        );
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
    let mut rows = std::mem::take(&mut win.rows);

    // Newest first (the default order); sorted when a column header is
    // clicked, back to default on the third click (tri-state). The row
    // cache is the app's; a sort re-orders it in place -- and a sorted cache
    // is no longer time-ordered, which is what the next refresh's incremental
    // walk depends on, so it says so and rebuilds instead. A FlexRay frame row
    // is one cache entry, so its decoded children travel with it rather than
    // scattering into the rows their name or value sorts next to.
    if let Some(mut specs) = ui.table_get_sort_specs()
        && let Some(s) = specs.iter().next()
    {
        let col = s.column_index.get();
        let asc = s.sort_direction == SortDirection::Ascending;
        rows.make_contiguous()
            .sort_by(|a, b| sort_frame(app, col, a, b, asc));
        specs.clear_dirty(ui);
        win.rows_sorted = true;
    }

    // The clipper counts *table* rows, and a FlexRay frame row occupies one plus
    // its decoded signal children, so the flattened total comes from the spans.
    // They are rebuilt from the list on every draw -- after the sort above, which
    // is why they are never stored between draws -- and the window's buffer only
    // saves the allocation.
    let mut ends = std::mem::take(&mut win.row_ends);
    let flat = crate::workspace::table_row_spans(&rows, &mut ends);

    // Virtual scrolling: the clipper submits only the visible slice of
    // the (possibly very long) filtered row list.
    // The picked row is read into a local so a row clicked *this* frame is
    // already tinted this frame; the keyboard walk and the write-back happen
    // after the loop, where the row list is still at hand.
    let mut picked = win.pick;
    let clip = ListClipper::new(flat).begin(ui);
    for n in clip.iter() {
        // Which cache entry this table row belongs to, and which part of it.
        let (r, offset) = crate::workspace::locate_row(&ends, n);
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
                // Last, so a marked error row is still recognisably both.
                if let Some(tint) = mark_tint(mark_us, f.t_us) {
                    ui.table_set_row_bg1_color(tint);
                }
                let here = TracePick::of(row);
                if picked == Some(here) {
                    ui.table_set_row_bg0_color(PICK_COLOR);
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
                if hovered && ui.is_mouse_clicked(dear_imgui_rs::MouseButton::Left) {
                    picked = Some(here);
                }
                can_ctx = if hovered && ui.is_mouse_released(dear_imgui_rs::MouseButton::Right) {
                    Some(*f)
                } else {
                    None
                };
            }
            TraceRow::Fr(fr, _) if offset == 0 => {
                // A distinct cool tint keeps the FR stream readable inside
                // the CAN rows; the reception channel rides the Bus cell.
                ui.table_set_row_bg1_color([0.15, 0.35, 0.55, 0.20]);
                if let Some(tint) = mark_tint(mark_us, fr.t_us) {
                    ui.table_set_row_bg1_color(tint);
                }
                let here = TracePick::of(row);
                if picked == Some(here) {
                    ui.table_set_row_bg0_color(PICK_COLOR);
                }
                if !ui.table_next_column() {
                    continue;
                }
                ui.text(format!("{:.6}", fr.t_us as f64 / 1e6));
                hovered |= ui.is_item_hovered();
                ui.table_next_column();
                ui.text_colored([0.55, 0.8, 1.0, 1.0], fr_bus_cell(app, fr.bus, fr.ab));
                hovered |= ui.is_item_hovered();
                ui.table_next_column();
                ui.text_colored([0.55, 0.8, 1.0, 1.0], format!("{}.{}", fr.slot, fr.cycle));
                hovered |= ui.is_item_hovered();
                ui.table_next_column();
                // The bus's own description wins; a name carried by the
                // log itself (CANoe ASC) is the fallback.
                match app.fr_row_name(fr) {
                    Some(name) => ui.text(name),
                    None => ui.text("-"),
                }
                hovered |= ui.is_item_hovered();
                ui.table_next_column();
                ui.text(format!("{}", fr.payload.len()));
                ui.table_next_column();
                ui.text("-");
                ui.table_next_column();
                let hex: String = fr.payload.iter().map(|b| format!("{b:02X} ")).collect();
                ui.text(hex.trim_end());
                ui.table_next_column();
                ui.text_colored([0.6, 0.65, 0.7, 1.0], "Rx");
                if hovered && ui.is_mouse_released(dear_imgui_rs::MouseButton::Right) {
                    *CTX_FR.lock().unwrap() = Some((i, (*fr).clone()));
                    ui.open_popup(format!("trace_fr_ctx{i}"));
                }
                if hovered && ui.is_mouse_clicked(dear_imgui_rs::MouseButton::Left) {
                    picked = Some(here);
                }
                can_ctx = None;
            }
            TraceRow::Fr(fr, _) => {
                // A decoded signal child under the frame row above it. The name
                // goes in the Name column, not the 68 px ID one: FlexRay signal
                // names are as long as any message name ("Drive_Attitude_
                // Alarm_Valid") and were cut to a fragment where a CAN row's
                // message name has room. The └ stays in ID, under the slot
                // address of the frame it belongs to.
                //
                // Decoded right here, for the rows on screen: the cache holds one
                // entry per frame and this is where its signals become text (see
                // [`TraceRow::Fr`]).
                // The group is one arrival: a child row carries its frame's key,
                // so picking a child picks the frame and the whole block lights up
                // together -- and one Up/Down step moves a whole frame, not one of
                // its signal lines.
                let here = TracePick::of(row);
                if picked == Some(here) {
                    ui.table_set_row_bg0_color(PICK_COLOR);
                }
                if !ui.table_next_column() {
                    continue;
                }
                let (signal, value) = app
                    .fr_child_cell(fr, (offset - 1) as u32)
                    .unwrap_or_default();
                ui.text("-");
                ui.table_next_column();
                ui.text("-");
                ui.table_next_column();
                ui.text("  └");
                ui.table_next_column();
                ui.text_colored([0.55, 0.8, 1.0, 1.0], signal);
                ui.table_next_column();
                ui.text("-");
                ui.table_next_column();
                ui.text("-");
                ui.table_next_column();
                ui.text_colored([0.75, 0.92, 1.0, 1.0], value);
                hovered |= ui.is_item_hovered();
                ui.table_next_column();
                ui.text("-");
                if hovered && ui.is_mouse_clicked(dear_imgui_rs::MouseButton::Left) {
                    picked = Some(here);
                }
                can_ctx = None;
            }
            TraceRow::Eth(f) => {
                // A warm tint keeps the Ethernet stream apart from the CAN and
                // FlexRay rows it is interleaved with.
                ui.table_set_row_bg1_color([0.65, 0.45, 0.10, 0.18]);
                if let Some(tint) = mark_tint(mark_us, f.t_us) {
                    ui.table_set_row_bg1_color(tint);
                }
                let here = TracePick::of(row);
                if picked == Some(here) {
                    ui.table_set_row_bg0_color(PICK_COLOR);
                }
                if !ui.table_next_column() {
                    continue;
                }
                ui.text(format!("{:.6}", f.t_us as f64 / 1e6));
                hovered |= ui.is_item_hovered();
                ui.table_next_column();
                ui.text_colored([0.95, 0.75, 0.35, 1.0], app.eth_bus_label(f.bus));
                hovered |= ui.is_item_hovered();
                ui.table_next_column();
                ui.text(format!("0x{:04X}", f.ethertype));
                hovered |= ui.is_item_hovered();
                ui.table_next_column();
                ui.text(app.eth_row_name(f));
                if ui.is_item_hovered() {
                    ui.tooltip_text(format!(
                        "{} > {}   VLAN {}",
                        crate::eth::db::mac_text(&f.src),
                        crate::eth::db::mac_text(&f.dst),
                        f.vlan
                            .map(|v| v.to_string())
                            .unwrap_or_else(|| "-".to_string())
                    ));
                }
                hovered |= ui.is_item_hovered();
                ui.table_next_column();
                ui.text(f.len().to_string());
                hovered |= ui.is_item_hovered();
                ui.table_next_column();
                if f.truncated {
                    ui.text_colored([1.0, 0.55, 0.35, 1.0], "Trunc");
                } else {
                    ui.text("-");
                }
                hovered |= ui.is_item_hovered();
                ui.table_next_column();
                // Jumbo frames are legal here; the cell previews the head of
                // the payload rather than formatting thousands of bytes per
                // row per frame. Copy takes the whole thing.
                let shown = f.len().min(64);
                let mut hex: String = f.payload[..shown]
                    .iter()
                    .map(|b| format!("{b:02X} "))
                    .collect();
                if shown < f.len() {
                    hex.push('…');
                }
                ui.text(hex.trim_end());
                hovered |= ui.is_item_hovered();
                ui.table_next_column();
                match f.dir {
                    Direction::Rx => ui.text_colored([0.6, 0.65, 0.7, 1.0], "Rx"),
                    Direction::Tx => ui.text_colored([1.0, 0.65, 0.2, 1.0], "Tx"),
                }
                if hovered && ui.is_mouse_released(dear_imgui_rs::MouseButton::Right) {
                    *CTX_ETH.lock().unwrap() = Some((i, f.clone()));
                    ui.open_popup(format!("trace_eth_ctx{i}"));
                }
                if hovered && ui.is_mouse_clicked(dear_imgui_rs::MouseButton::Left) {
                    picked = Some(here);
                }
                can_ctx = None;
            }
        }
        if let Some(f) = can_ctx {
            *CTX.lock().unwrap() = Some((i, f));
            ui.open_popup(format!("trace_row_ctx{i}"));
        }
    }
    // Keyboard: Up/Down walk the rows on screen from the picked one, and Ctrl+C
    // copies the picked row. Both need the pointer to be on this window, and
    // neither runs while something is typing -- an input field owns the arrows
    // and the clipboard keys while it is active, and taking them from the filter
    // box would be a trap.
    if ui.is_window_hovered() && !ui.is_any_item_active() {
        if ui.is_key_pressed_with_repeat(dear_imgui_rs::Key::DownArrow, true) {
            picked = crate::workspace::step_pick(&rows, picked, true);
        }
        if ui.is_key_pressed_with_repeat(dear_imgui_rs::Key::UpArrow, true) {
            picked = crate::workspace::step_pick(&rows, picked, false);
        }
        let copy = dear_imgui_rs::KeyChord::new(dear_imgui_rs::Key::C)
            .with_mods(dear_imgui_rs::KeyMods::CTRL);
        if ui.is_key_chord_pressed(copy)
            && let Some(p) = picked
            && let Some(row) = rows.iter().find(|r| TracePick::of(r) == p)
        {
            let text = match row {
                TraceRow::Can(f) => fmt_row(app, f),
                TraceRow::Fr(r, _) => fmt_fr_row(app, r),
                TraceRow::Eth(f) => fmt_eth_row(app, f),
            };
            crate::clipboard::Clipboard.set(&text);
        }
    }
    // The rows go back before the popup: its menu mutates the window's
    // filter state. The span buffer returns with them -- it describes that
    // list, and nothing reads it between draws.
    let w = &mut *win;
    w.row_ends = ends;
    w.rows = rows;
    w.pick = picked;

    // The FlexRay row menu: what the row is, the window's own view narrowed to
    // it, and the payload copied.
    if let Some(_p) = ui.begin_popup(format!("trace_fr_ctx{i}"))
        && let Some((pi, r)) = CTX_FR.lock().unwrap().clone()
        && pi == i
    {
        // Identified the way the CAN menu identifies its row: the cluster, the
        // address the table prints, and the name that table shows -- the
        // description's, else the one the log carried.
        let name = app.fr_row_name(&r).unwrap_or("-");
        let addr = format!("{}.{}", r.slot, r.cycle);
        ui.text(format!("{} slot {addr} · {name}", app.fr_bus_label(r.bus)));
        ui.separator();
        let label = format!("Watch FR{} slot {}", r.bus, r.slot);
        if ui.menu_item(label.clone()) {
            // The FlexRay twin of "Filter this ID", through the window's own
            // pick set rather than the text box: a bare "13" in there also
            // matches CAN id 13 and slot 113, while a `Pick::Fr` says which bus
            // it means. Additive, not a replacement -- the window's other picks
            // stay, so this can never quietly discard a curated selection.
            let w = &mut *win;
            w.manual.insert(crate::workspace::Pick::Fr {
                bus: r.bus,
                slot: r.slot,
            });
            w.scope = SigScope::Manual;
        }
        if ui.is_item_hovered() {
            ui.tooltip_text(format!(
                "Add this slot to the window's Manual picks and switch to that scope (other picks are kept); only selected rows are listed: {label}"
            ));
        }
        cursor_menu(win, ui, r.t_us);
        if ui.menu_item("Clear filter") {
            clear_filter_win(win);
        }
        let hex: String = r.payload.iter().map(|b| format!("{b:02X} ")).collect();
        if ui.menu_item("Copy payload") {
            crate::clipboard::Clipboard.set(hex.trim_end());
        }
        if ui.menu_item("Copy slot.cycle") {
            crate::clipboard::Clipboard.set(&format!("{}.{}", r.slot, r.cycle));
        }
        // The same last item the CAN menu ends with, for the same reason: the
        // row as the table shows it, so it can be pasted into a report.
        ui.separator();
        if ui.menu_item("Copy row") {
            crate::clipboard::Clipboard.set(&fmt_fr_row(app, &r));
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
            filter_to_id_win(win, &f);
        }
        cursor_menu(win, ui, f.t_us);
        if ui.menu_item("Clear filter") {
            clear_filter_win(win);
        }
        let addable = !f.is_error() && !f.is_remote();
        if ui.menu_item_enabled_selected(
            "Add to Interactive Generator",
            None::<&str>,
            false,
            addable,
        ) {
            // Add, then re-shape the fresh row via command: keep the flags
            // it was seen with, and for an id no database declares, keep
            // the observed payload. The command applies both steps after the
            // window is put back, so the draw stays read-only.
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
            cmds.push(TraceCmd::AddEntry {
                ch: f.channel,
                id: f.id,
                fd: f.flags.contains(crate::can::frame::FrameFlags::FD),
                cycle_us: app
                    .dbc_cycle_us(f.channel, f.id)
                    .unwrap_or(crate::app::DEFAULT_TX_CYCLE_US),
                flags: f.flags,
                data_text,
            });
        }
        ui.separator();
        if ui.menu_item("Copy row") {
            crate::clipboard::Clipboard.set(&fmt_row(app, &f));
        }
        if ui.menu_item("Copy ID") {
            crate::clipboard::Clipboard.set(&fmt_id(&f));
        }
    }

    // The Ethernet row menu: the flow's identity, the window narrowed to it,
    // and its payload/copy actions.
    if let Some(_p) = ui.begin_popup(format!("trace_eth_ctx{i}"))
        && let Some((pi, f)) = CTX_ETH.lock().unwrap().clone()
        && pi == i
    {
        let name = app.eth_row_name(&f);
        ui.text(format!(
            "{}  {} > {}  0x{:04X}",
            app.eth_bus_label(f.bus),
            crate::eth::db::mac_text(&f.src),
            crate::eth::db::mac_text(&f.dst),
            f.ethertype
        ));
        ui.text_disabled(&name);
        ui.separator();
        if ui.menu_item("Watch this flow") {
            // The Ethernet twin of the FlexRay row's pick: the L2 flow, not a
            // name, so the window's Manual scope admits exactly this stream.
            let w = &mut *win;
            w.manual.insert(crate::workspace::Pick::Eth(
                crate::aggregate::EthFlowKey::of_frame(&f),
            ));
            w.scope = SigScope::Manual;
        }
        cursor_menu(win, ui, f.t_us);
        if ui.menu_item("Clear filter") {
            clear_filter_win(win);
        }
        let hex: String = f.payload.iter().map(|b| format!("{b:02X} ")).collect();
        if ui.menu_item("Copy payload") {
            crate::clipboard::Clipboard.set(hex.trim_end());
        }
        if ui.menu_item("Copy MACs") {
            crate::clipboard::Clipboard.set(&format!(
                "{} > {}",
                crate::eth::db::mac_text(&f.src),
                crate::eth::db::mac_text(&f.dst)
            ));
        }
        ui.separator();
        if ui.menu_item("Copy row") {
            crate::clipboard::Clipboard.set(&fmt_eth_row(app, &f));
        }
    }
}

fn sort_frame(app: &App, col: usize, a: &TraceRow, b: &TraceRow, asc: bool) -> Ordering {
    // Bus sort keeps FR rows together after every CAN bus and Ethernet rows
    // after every FlexRay cluster; the address column is a CAN id, an FR slot
    // or an EtherType. A frame row sorts as the frame -- its decoded children
    // are not rows of the list, so they follow their parent instead of
    // scattering among the rows its value sorts next to.
    let bus = |r: &TraceRow| match r {
        TraceRow::Can(f) => f.channel as u32,
        TraceRow::Fr(r, _) => 0x1000 + u32::from(r.ab),
        TraceRow::Eth(f) => 0x2000 + u32::from(f.bus),
    };
    let addr = |r: &TraceRow| match r {
        TraceRow::Can(f) => f.id,
        TraceRow::Fr(r, _) => u32::from(r.slot),
        TraceRow::Eth(f) => u32::from(f.ethertype),
    };
    let name = |r: &TraceRow| match r {
        TraceRow::Can(f) => app
            .message_name(f.channel, f.id)
            .unwrap_or_default()
            .to_string(),
        TraceRow::Fr(f, _) => app.fr_row_name(f).unwrap_or_default().to_string(),
        TraceRow::Eth(f) => app.eth_row_name(f),
    };
    let len = |r: &TraceRow| match r {
        TraceRow::Can(f) => f.len as usize,
        TraceRow::Fr(r, _) => r.payload.len(),
        TraceRow::Eth(f) => f.len(),
    };
    let flags_rank = |r: &TraceRow| match r {
        TraceRow::Can(f) => (f.is_fd(), f.esi(), f.brs()),
        TraceRow::Fr(_, _) | TraceRow::Eth(_) => (false, false, false),
    };
    let payload = |r: &TraceRow| match r {
        TraceRow::Can(f) => f.payload().to_vec(),
        TraceRow::Fr(r, _) => r.payload.clone(),
        TraceRow::Eth(f) => f.payload.to_vec(),
    };
    let dir = |r: &TraceRow| match r {
        TraceRow::Can(f) => f.dir as u8,
        TraceRow::Fr(_, _) => 0,
        TraceRow::Eth(f) => f.dir as u8,
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

impl App {
    pub(crate) fn apply_trace_cmds(&mut self, cmds: Vec<TraceCmd>) {
        for cmd in cmds {
            match cmd {
                TraceCmd::ClearTrace => {
                    self.send(crate::bus::BusCommand::ClearTrace);
                }
                TraceCmd::Export(i) => self.export_trace_dialog(i),
                TraceCmd::OpenPicker(i) => {
                    self.id_filter.open_for(PopupTarget::Trace(i));
                }
                TraceCmd::ClearFocus => self.focus_title = None,
                TraceCmd::AddEntry {
                    ch,
                    id,
                    fd,
                    cycle_us,
                    flags,
                    data_text,
                } => {
                    let was_len = self.snap.tx.len();
                    self.add_tx(ch, id);
                    if self.snap.tx.len() > was_len {
                        self.send(crate::bus::BusCommand::SetEntryConfig {
                            ch,
                            id,
                            active: true,
                            cycle_us,
                            fd,
                            flags: Some(flags),
                            data_text,
                            srcs: Vec::new(),
                        });
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::mark_tint;
    use crate::can::frame::{CanFrame, Direction, FrameFlags, MAX_CAN_FD_LEN};
    use crate::trace::FrRow;
    use crate::workspace::{TracePick, TraceRow, step_pick};
    use std::collections::VecDeque;

    fn can(t_us: u64, id: u32) -> TraceRow {
        let mut data = [0u8; MAX_CAN_FD_LEN];
        data[0] = 1;
        data[1] = 2;
        TraceRow::Can(CanFrame {
            t_us,
            channel: 0,
            id,
            extended: false,
            len: 2,
            data,
            flags: FrameFlags::NONE,
            dir: Direction::Rx,
        })
    }

    fn fr(t_us: u64, slot: u16, kids: u32) -> TraceRow {
        TraceRow::Fr(
            FrRow {
                t_us,
                bus: 0,
                slot,
                cycle: 0,
                ab: 2,
                payload: vec![0; 8],
                header_crc: 0,
                flags: 0,
                name: None,
            },
            kids,
        )
    }

    fn eth(t_us: u64, seq: u64) -> TraceRow {
        let mut f = crate::eth::EthFrame::new(
            t_us,
            0,
            Direction::Rx,
            [0; 6],
            [1; 6],
            Some(1),
            0x0800,
            vec![0; 8],
        );
        f.seq = seq;
        TraceRow::Eth(f)
    }

    /// Up/Down walk the rows on screen, which is the cache list: newest-first,
    /// clamped at both ends, starting from the newest when nothing is picked, and
    /// re-anchoring at the newest when the picked row has left the list.
    #[test]
    fn the_keyboard_walks_the_row_list_and_clamps_at_both_ends() {
        let rows: VecDeque<TraceRow> = [can(40, 0x100), fr(30, 13, 2), can(20, 0x200)].into();
        assert_eq!(
            step_pick(&rows, None, true),
            Some(TracePick::of(&rows[0])),
            "the first press picks the newest row"
        );
        assert_eq!(
            step_pick(&rows, Some(TracePick::of(&rows[0])), true),
            Some(TracePick::of(&rows[1])),
            "down is toward the older end"
        );
        assert_eq!(
            step_pick(&rows, Some(TracePick::of(&rows[1])), false),
            Some(TracePick::of(&rows[0])),
            "and up comes back"
        );
        assert_eq!(
            step_pick(&rows, Some(TracePick::of(&rows[0])), false),
            Some(TracePick::of(&rows[0])),
            "up at the top stays put -- a step off the end is not a clear"
        );
        assert_eq!(
            step_pick(&rows, Some(TracePick::of(&rows[2])), true),
            Some(TracePick::of(&rows[2])),
            "and down at the bottom does too"
        );
        // The row a pick named is gone (filtered out, ring trimmed, trace
        // cleared): the walk lands on the newest row instead of dead-ending.
        let gone = TracePick {
            t_us: 999,
            bus: 0,
            addr: 0x7FF,
            fr: false,
            eth: false,
            seq: 0,
        };
        assert_eq!(
            step_pick(&rows, Some(gone), true),
            Some(TracePick::of(&rows[0]))
        );
        assert_eq!(
            step_pick(&rows, Some(gone), false),
            Some(TracePick::of(&rows[0]))
        );
        // Nothing to walk: no crash, no pick.
        let empty: VecDeque<TraceRow> = VecDeque::new();
        assert_eq!(step_pick(&empty, None, true), None);
    }

    /// A simulation stamps every frame of one step with the same microsecond, so
    /// a shared instant is the ordinary case, not the pathological one: the pick
    /// has to name the row -- bus and address included -- or one click lights up
    /// half a step and one arrow press jumps two rows. A CAN frame and a FlexRay
    /// frame can share the stamp too, and the two numbering spaces must not read
    /// each other's numbers. The decoded child rows of a frame are not rows of
    /// the list, so they share their parent's key: the group picks and steps as
    /// the one arrival it is. (The first version of this key was the timestamp
    /// plus the stream alone, and the GUI showed two rows lighting up from one
    /// click -- the case the address now separates.)
    #[test]
    fn a_pick_names_one_row_at_a_shared_instant() {
        let at_40 = [
            can(40, 0x100),
            can(40, 0x200),
            fr(40, 13, 3),
            eth(40, 0),
            eth(40, 1),
        ];
        let keys: Vec<TracePick> = at_40.iter().map(TracePick::of).collect();
        assert_ne!(keys[0], keys[1], "two CAN ids in the same microsecond");
        assert_ne!(
            keys[0], keys[2],
            "a CAN id and a FlexRay slot are two numbering spaces"
        );
        assert_ne!(keys[1], keys[2]);
        assert_ne!(
            keys[3], keys[4],
            "two Ethernet frames in one microsecond are separated by capture sequence"
        );
        assert_ne!(keys[2], keys[3], "Ethernet is its own stream");
        // Two children of the same frame: same cache entry, same key.
        assert_eq!(TracePick::of(&fr(40, 13, 3)), keys[2]);
        let rows: VecDeque<TraceRow> = at_40.into();
        assert_eq!(
            step_pick(&rows, Some(keys[0]), true),
            Some(keys[1]),
            "one step is one row, even when the next row shares the stamp"
        );
        assert_eq!(
            step_pick(&rows, Some(keys[1]), true),
            Some(keys[2]),
            "and the FlexRay frame is the next single step, children and all"
        );
        assert_eq!(
            step_pick(&rows, Some(keys[2]), true),
            Some(keys[3]),
            "Ethernet rows step one capture at a time"
        );
    }

    /// A cursor is an instant, so the tint is decided by the row's timestamp and
    /// nothing else -- not by its position in a table that reorders itself as
    /// frames arrive. Unset marks tint no row at all.
    #[test]
    fn a_cursor_tints_the_row_at_its_instant_on_either_bus() {
        let none = [None, None];
        assert_eq!(mark_tint(none, 4_000), None, "nothing marked");
        let a_at_4 = [Some(4_000), None];
        assert!(mark_tint(a_at_4, 4_000).is_some(), "the marked CAN row");
        assert_eq!(mark_tint(a_at_4, 5_000), None, "its neighbour is not");
        // A is the first colour, so a row carrying both reads as A rather than
        // picking one at random.
        assert_eq!(
            mark_tint([Some(4_000), Some(4_000)], 4_000),
            mark_tint([Some(4_000), None], 4_000)
        );
        assert_eq!(
            mark_tint([None, Some(5_000)], 5_000),
            Some(super::MARK_COLOR[1]),
            "B keeps its own colour"
        );
    }

    /// "Copy row" prints a FlexRay frame with the same eight columns, in the same
    /// order, as the table shows them -- and as a CAN row's copy does, so a pasted
    /// Trace of both streams lines up. The description's frame name wins over the
    /// one the log carried, and the flags column reads `-` because a FlexRay frame
    /// has nothing to report there.
    #[test]
    fn a_copied_flexray_row_carries_the_table_columns() {
        use crate::app::App;
        use crate::fr_db::{FrChannel, FrClusterParams, FrDb, FrFrameDb, FrPdu, FrTriggering};
        let mut app = App::headless();
        let db = FrDb::assemble(
            FrClusterParams::default(),
            vec![],
            vec![FrPdu {
                name: "P".into(),
                length: 2,
                dynamic: false,
                comment: String::new(),
                signals: vec![],
            }],
            vec![FrFrameDb {
                name: "ScheduledFrame".into(),
                length: 2,
                payload_preamble: false,
                triggering: FrTriggering {
                    channel: FrChannel::Both,
                    slot_id: 13,
                    base_cycle: 0,
                    cycle_repetition: 1,
                    startup: false,
                },
                pdus: vec![("P".into(), 0u32)],
                comment: String::new(),
            }],
        );
        app.fr_buses.insert(
            0,
            crate::app::FrBusCfg {
                path: "synthetic".into(),
                db: db.into(),
            },
        );
        let row = FrRow {
            t_us: 1_500_000,
            bus: 0,
            slot: 13,
            cycle: 7,
            ab: 0,
            payload: vec![0x11, 0x22],
            header_crc: 0,
            flags: 0,
            name: Some("FromLog".into()),
        };
        let text = super::fmt_fr_row(&app, &row);
        let cols: Vec<&str> = text.split("  ").collect();
        assert_eq!(cols.len(), 8, "the table's eight columns: {text}");
        assert_eq!(cols[0], "1.500000", "time in seconds, six places");
        assert_eq!(cols[1], "FR0 A", "the bus cell with its reception channel");
        assert_eq!(
            cols[2], "13.7",
            "slot.cycle, the way the ID column prints it"
        );
        assert_eq!(
            cols[3], "ScheduledFrame",
            "the description names the frame, not the log"
        );
        assert_eq!(cols[4], "2");
        assert_eq!(cols[5], "-", "no flags to report");
        assert_eq!(cols[6], "11 22");
        assert_eq!(cols[7], "Rx");
    }
}
