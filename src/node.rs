//! Simulation nodes: a compiled script bound to one bus channel,
//! CANoe-style. A node reacts to events -- measurement start, frames
//! carrying ids it watches, its own periodic timers -- and produces
//! behaviour by queueing frames onto the bus and printing into its log.
//! All node interaction is text; there are no panels.
//!
//! The runtime lives in the bus core: handlers run inside `step` with an
//! instruction budget per callback, and a `send` from the script lands in
//! an outbox that the core drains onto the bus as real frames.

use crate::script::{Handler, HandlerKind, HostInput, Value, Vm, compile};
use std::collections::{HashMap, VecDeque};

/// The CAN model a node's buffer builtins encode and decode through.
pub type CanCodec = (
    std::sync::Arc<crate::model::Comm>,
    std::sync::Arc<crate::model::PlanCache>,
    crate::model::ClusterId,
);

/// Log lines kept per node, oldest first.
const LOG_CAP: usize = 200;
/// Instruction allowance per handler run: plenty for stimulus logic,
/// finite enough that a runaway loop cannot stall the core thread.
const NODE_HANDLER_BUDGET: u64 = 100_000;

pub struct ScriptNode {
    /// Stable identity for commands and persistence -- never an index.
    pub id: u64,
    pub name: String,
    pub channel: u8,
    pub source: String,
    pub enabled: bool,
    /// Optional file path for standalone `.rxcan` source files. Set when
    /// the user saves or loads a node script from disk. Persisted with
    /// the project so the user can find their files again.
    pub file_path: Option<String>,
    /// Last content saved to / loaded from `file_path`; separate from the
    /// applied runtime source so Save and Apply remain independent.
    pub file_source: String,
    /// Source that passed the last explicit check, plus its diagnostics.
    pub checked_source: Option<String>,
    pub check_issue: Option<ScriptIssue>,
    pub check_summary: Option<ScriptCheckSummary>,
    /// `on panel_action("…")` names declared by the last successful compile.
    /// Kept after stop so the editor can still offer them; cleared when the
    /// source changes or fails to compile.
    pub panel_actions: Vec<String>,
    /// 绑定的 DBC 节点（总线, 节点名）。绑定脚本的发帧受该节点角色
    /// 角色开关控制——节点离线或仅监听时脚本同样不发送。None = 独立脚本。
    pub attached: Option<(u8, String)>,
    /// Defined system variable keys ("ns::name"), refreshed by the bus
    /// before every (re)start: the start check reports script references
    /// outside this set.
    pub sysvar_keys: Vec<String>,
    /// Present only while measuring: recompiled from `source` at every
    /// start, so edits apply without a separate compile action.
    runtime: Option<NodeRuntime>,
    /// `print` output and runtime errors, oldest first, capped.
    log: VecDeque<String>,
    log_dirty: bool,
    /// Set when a handler failed: the node stops executing until the next
    /// start or source edit, so one bad loop cannot spam the log.
    errored: bool,
    /// The last compile failure, kept structured for the editor's inline
    /// diagnostic. Cleared on source edits, restarts and stops.
    issue: Option<ScriptIssue>,
    /// `test(...)` results from the last run, filled by `on stop` when
    /// the measurement ends. Cleared by the next start.
    pub tests: Vec<crate::script::TestResult>,
    /// Lines pushed to `log` and not yet mirrored into the bus's Write
    /// ring. Drained by the bus on the same step.
    pending_lines: Vec<String>,
}

/// A positioned compile or runtime failure, mirrored to the web editor.
#[derive(Clone, Debug)]
pub struct ScriptIssue {
    pub line: u32,
    pub col: Option<u32>,
    pub message: String,
}

/// Result of a read-only compile/check. It never changes the running source.
#[derive(Clone, Debug, Default)]
pub struct ScriptCheckSummary {
    pub handlers: usize,
    pub sends: Vec<(u32, bool)>,
    pub receives: Vec<(u32, bool)>,
    pub lines: Vec<String>,
}

/// Runtime failures are formatted by the VM as `line N: message`; the
/// editor can mark that line even though there is no token column.
fn runtime_issue(message: &str) -> Option<ScriptIssue> {
    let rest = message.strip_prefix("line ")?;
    let (line, text) = rest.split_once(':')?;
    let line = line.trim().parse().ok()?;
    Some(ScriptIssue {
        line,
        col: None,
        message: text.trim().to_string(),
    })
}

/// Declared `on panel_action` names, in declaration order, deduped.
fn panel_action_names(handlers: &[Handler]) -> Vec<String> {
    let mut names: Vec<String> = Vec::new();
    for handler in handlers {
        if let HandlerKind::PanelAction { name, .. } = &handler.kind
            && !names.contains(name)
        {
            names.push(name.clone());
        }
    }
    names
}

struct NodeRuntime {
    vm: Vm,
    handlers: Vec<Handler>,
    /// DBC-named handlers resolved to `(id, extended)`. Kept separate from the
    /// handler kind so a database reload can re-point a name without rebuilding
    /// the compiled script.
    named_handler_ids: HashMap<String, (u32, bool)>,
    /// Derived-signal samples queued by `emit_value` and not yet taken by
    /// the bus. Drained from the VM after every handler run.
    emitted: Vec<(String, f64)>,
    /// System variable writes queued by `sys_set` and not yet taken by
    /// the bus. Same lifecycle as `emitted`.
    pending_sys: Vec<(String, f64)>,
    /// FlexRay slots queued by `fr_send`, same lifecycle as `emitted`.
    fr_out: Vec<(u8, u16, Vec<u8>)>,
    /// Signal writes queued by `set_fr_sig`, same lifecycle as `emitted`.
    fr_sig: Vec<(u8, u16, String, f64)>,
    /// Frame instances queued by `send_frame`, same lifecycle as `emitted`.
    frames: Vec<crate::script::FrameInstance>,
    /// SOME/IP datagrams queued by `someip_send`.
    someip: Vec<crate::script::SomeIpSend>,
    /// SOME/IP TCP datagrams queued by `someip_tcp_send`.
    someip_tcp: Vec<crate::script::SomeIpTcpSend>,
    /// SD offers queued by `someip_offer`.
    someip_offer: Vec<crate::script::SomeIpOffer>,
    /// Auto-reply payloads queued by `someip_reply`.
    someip_reply: Vec<crate::script::SomeIpReply>,
    nm_timing: Vec<crate::script::NmTiming>,
    /// Last observed values for on signal / on sysvar change detection.
    last_values: HashMap<String, f64>,
    /// One slot per Timer handler, in handler order; named one-shot slots
    /// join on `set_timer`. `next_due_us == 0` means "not armed yet": the
    /// first step after start arms periodic slots one period out.
    timers: Vec<TimerSlot>,
}

struct TimerSlot {
    handler_index: usize,
    period_ms: u64,
    next_due_us: u64,
    /// Some for a named one-shot: spent after firing, re-armed by
    /// `set_timer` from any handler.
    name: Option<String>,
    /// Set by `stop_timer()` inside a periodic handler or by a spent /
    /// cancelled one-shot: the slot never fires again until re-armed.
    stopped: bool,
}

/// The node's log/outbox side of one handler run, bundled so the three
/// delivery paths share one call signature.
struct RunSink<'a> {
    log: &'a mut VecDeque<String>,
    log_dirty: &'a mut bool,
    pending_lines: &'a mut Vec<String>,
    out: &'a mut Vec<crate::script::ScriptSend>,
}

mod core_dispatch;
mod core_lifecycle;
mod core_outbox;
mod core_signals;

#[cfg(test)]
mod tests;

/// The unique wire slot that declares `sig` in a FlexRay cluster
/// description. Several frames can hold the same slot in different cycle
/// phases; they all name the same wire address, so only *distinct* slots
/// make a dotted read ambiguous. Every message already says which bus it
/// is about, so the caller can log it verbatim.
fn fr_slot_of(db: &crate::fr_db::FrDb, bus: u8, sig: &str) -> Result<u16, String> {
    let mut slots: Vec<u16> = db
        .slot_signals()
        .iter()
        .filter(|(_, _, names)| names.iter().any(|n| n == sig))
        .map(|(slot, _, _)| *slot)
        .collect();
    slots.sort_unstable();
    slots.dedup();
    match slots.as_slice() {
        [slot] => Ok(*slot),
        [] => Err(format!(
            "FlexRay signal \"{sig}\" is not declared in the FR{bus} description"
        )),
        many => Err(format!(
            "FlexRay signal \"{sig}\" is declared in several FR{bus} slots ({many:?}); \
             use fr_sig({bus}, slot, \"{sig}\")"
        )),
    }
}

/// The FlexRay bus a description name belongs to. Names come from the
/// cluster file, so re-loading another file can move them; an unknown or
/// duplicated name falls back to the numeric `FRn` form.
fn fr_bus_by_name(
    dbs: &std::collections::BTreeMap<u8, std::sync::Arc<crate::fr_db::FrDb>>,
    name: &str,
) -> Result<u8, String> {
    let mut hits: Vec<u8> = dbs
        .iter()
        .filter(|(_, db)| db.params.name.trim() == name)
        .map(|(bus, _)| *bus)
        .collect();
    hits.sort_unstable();
    match hits.as_slice() {
        [bus] => Ok(*bus),
        [] => Err(format!(
            "no FlexRay cluster named {name:?}; use the FR index form"
        )),
        many => Err(format!(
            "FlexRay cluster name {name:?} is ambiguous ({many:?}); use the FR index form"
        )),
    }
}

/// The Ethernet bus a cluster name belongs to (the ARXML cluster or a
/// frame's declared cluster).
fn eth_bus_by_name(
    dbs: &std::collections::BTreeMap<u8, std::sync::Arc<crate::eth::db::EthDb>>,
    name: &str,
) -> Result<u8, String> {
    let mut hits: Vec<u8> = dbs
        .iter()
        .filter(|(_, db)| {
            db.clusters.iter().any(|c| c.name == name)
                || db.frames.iter().any(|f| f.cluster == name)
        })
        .map(|(bus, _)| *bus)
        .collect();
    hits.sort_unstable();
    match hits.as_slice() {
        [bus] => Ok(*bus),
        [] => Err(format!(
            "no Ethernet cluster named {name:?}; use the ETH index form"
        )),
        many => Err(format!(
            "Ethernet cluster name {name:?} is ambiguous ({many:?}); use the ETH index form"
        )),
    }
}

/// Does the ARXML frame declare this signal? The error names the bus so the
/// same text serves the numeric and named read forms.
fn eth_signal_check(
    db: &crate::eth::db::EthDb,
    bus: u8,
    frame: &str,
    sig: &str,
) -> Result<(), String> {
    let Some(def) = db.frames.iter().find(|def| def.name == frame) else {
        return Err(format!(
            "Ethernet frame \"{frame}\" is not in the ETH{bus} description"
        ));
    };
    if def
        .pdus
        .iter()
        .any(|pdu| pdu.signals.iter().any(|s| s.name == sig))
    {
        Ok(())
    } else {
        Err(format!(
            "Ethernet signal \"{sig}\" is not in frame \"{frame}\" (ETH{bus})"
        ))
    }
}

/// Turns statically known `frame(...)` sends into numeric send-set facts when
/// the catalog path resolves on this node's CAN database. Unknown paths are
/// left to `frame_checks`; FR/ETH paths have no numeric send set here.
pub fn resolve_frame_send_refs(script: &mut crate::script::Script, db: &crate::dbc::SymbolTable) {
    for path in script.frame_sends.clone() {
        let mut parts = path.split('.');
        let first = parts.next().unwrap_or_default();
        let second = parts.next();
        if second.is_some() && (first.starts_with("FR") || first.starts_with("ETH")) {
            continue;
        }
        let name = second.unwrap_or(first);
        let Some(key) = db.message_key_by_name(name) else {
            continue;
        };
        if !script
            .send_refs
            .iter()
            .any(|(_, id, ext)| (*id, *ext) == key)
        {
            script
                .send_refs
                .push((format!("<frame {path}>"), key.0, key.1));
        }
    }
}

/// Checks the catalog paths and physical fields of statically known
/// `frame(...)` instances against the loaded descriptions, plus the declared
/// range of constant field writes. A typo or an out-of-range constant here is
/// an assembly warning, not a dropped frame on the first click.
pub fn frame_checks(
    script: &crate::script::Script,
    dbc: Option<&crate::dbc::SymbolTable>,
    fr_dbs: Option<&std::collections::BTreeMap<u8, std::sync::Arc<crate::fr_db::FrDb>>>,
    eth_dbs: Option<&std::collections::BTreeMap<u8, std::sync::Arc<crate::eth::db::EthDb>>>,
) -> Vec<String> {
    let mut lines = Vec::new();
    let check = |path: &str, field: Option<&str>| -> Result<(), String> {
        let mut parts = path.split('.');
        let first = parts.next().unwrap_or_default();
        let second = parts.next();
        if parts.next().is_some() || first.is_empty() {
            return Err(format!("[check] frame path is malformed: \"{path}\""));
        }
        if let Some(bus) = first.strip_prefix("FR").filter(|_| second.is_some()) {
            let bus: u8 = bus
                .parse()
                .map_err(|_| format!("[check] frame path has a bad FlexRay bus: \"{path}\""))?;
            let name = second.expect("checked");
            let Some(db) = fr_dbs.and_then(|dbs| dbs.get(&bus)) else {
                return Err(format!("[check] FR{bus} has no description for \"{path}\""));
            };
            let Some(ix) = db.frames.iter().position(|frame| frame.name == name) else {
                return Err(format!("[check] frame not found: \"{path}\""));
            };
            if let Some(field) = field {
                let slot = db.frame_index(ix).expect("index").triggering.slot_id as u16;
                if db.slot_signal(slot, field).is_none() {
                    return Err(format!("[check] signal not found: \"{path}\" \"{field}\""));
                }
            }
            return Ok(());
        }
        if let Some(bus) = first.strip_prefix("ETH").filter(|_| second.is_some()) {
            let bus: u8 = bus
                .parse()
                .map_err(|_| format!("[check] frame path has a bad Ethernet bus: \"{path}\""))?;
            let name = second.expect("checked");
            let Some(db) = eth_dbs.and_then(|dbs| dbs.get(&bus)) else {
                return Err(format!(
                    "[check] ETH{bus} has no description for \"{path}\""
                ));
            };
            let Some(frame) = db.frames.iter().find(|frame| frame.name == name) else {
                return Err(format!("[check] frame not found: \"{path}\""));
            };
            if let Some(field) = field
                && !frame
                    .pdus
                    .iter()
                    .flat_map(|pdu| &pdu.signals)
                    .any(|signal| signal.name == field)
            {
                return Err(format!("[check] signal not found: \"{path}\" \"{field}\""));
            }
            return Ok(());
        }
        let name = second.unwrap_or(first);
        let Some(db) = dbc else {
            return Err(format!("[check] CAN database not loaded for \"{path}\""));
        };
        let Some(id) = db.message_id_by_name(name) else {
            return Err(format!("[check] message not found: \"{path}\""));
        };
        if let Some(field) = field
            && !db
                .message_of(id)
                .is_some_and(|message| message.signals.iter().any(|s| s.name == field))
        {
            return Err(format!("[check] signal not found: \"{path}\" \"{field}\""));
        }
        Ok(())
    };
    for path in &script.frame_sends {
        if let Err(line) = check(path, None) {
            lines.push(line);
        }
    }
    for (path, field) in &script.frame_fields {
        if let Err(line) = check(path, Some(field)) {
            lines.push(line);
        }
    }
    for (path, field, value) in &script.frame_consts {
        let Some((min, max)) = frame_signal_bounds(path, field, dbc, fr_dbs, eth_dbs) else {
            continue;
        };
        if *value < min || *value > max {
            let name = path.rsplit('.').next().unwrap_or(path);
            lines.push(format!(
                "[check] frame \"{name}\" signal \"{field}\" value {value} is outside [{min}, {max}]"
            ));
        }
    }
    lines.sort();
    lines.dedup();
    lines
}

/// The declared physical range of one frame field, when the description
/// states a non-empty one. Ethernet signals carry no range here, and an
/// unknown path is a `frame_checks` error of its own.
fn frame_signal_bounds(
    path: &str,
    field: &str,
    dbc: Option<&crate::dbc::SymbolTable>,
    fr_dbs: Option<&std::collections::BTreeMap<u8, std::sync::Arc<crate::fr_db::FrDb>>>,
    _eth_dbs: Option<&std::collections::BTreeMap<u8, std::sync::Arc<crate::eth::db::EthDb>>>,
) -> Option<(f64, f64)> {
    let mut parts = path.split('.');
    let first = parts.next()?;
    let second = parts.next();
    if parts.next().is_some() {
        return None;
    }
    if let Some(bus) = first.strip_prefix("FR").filter(|_| second.is_some()) {
        let bus: u8 = bus.parse().ok()?;
        let db = fr_dbs?.get(&bus)?;
        let name = second?;
        let ix = db.frames.iter().position(|frame| frame.name == name)?;
        let slot = db.frame_index(ix)?.triggering.slot_id as u16;
        let signal = db.slot_signal(slot, field)?;
        return (signal.max > signal.min).then_some((signal.min, signal.max));
    }
    if first.starts_with("ETH") && second.is_some() {
        return None;
    }
    let key = dbc?.message_key_by_name(second.unwrap_or(first))?;
    let signal = dbc?
        .messages
        .get(&key)?
        .signals
        .iter()
        .find(|signal| signal.name == field)?;
    (signal.max > signal.min).then_some((signal.min, signal.max))
}

/// The DBC-backed assembly checks a compiled script must pass at start:
/// literal signal references exist, the send set matches the DBC's
/// transmitter declarations (both directions), and literal `set_sig`
/// values sit inside their declared ranges. `attached` is the bound DBC
/// node's name when the script has one; the send-set and reverse checks
/// only apply to bound scripts. Returns formatted log lines -- shared
/// verbatim by the GUI's node start and the headless `--check-script
/// --dbc` gate.
pub fn dbc_checks(
    script: &crate::script::Script,
    db: &crate::dbc::SymbolTable,
    attached: Option<&str>,
) -> Vec<String> {
    let mut lines = Vec::new();
    // Literal signal references in `sig` / `set_sig` must exist, so a
    // typo'd name is reported at assembly instead of silently missing.
    for (id, sig) in &script.signal_refs {
        let known = db
            .message_of(*id)
            .is_some_and(|m| m.signals.iter().any(|s| s.name == *sig));
        if !known {
            lines.push(format!(
                "[check] signal not found in DBC: 0x{id:X} \"{sig}\""
            ));
        }
    }
    // 命名读取的名字对同样要能落到库上：报文名查不到、或查
    // 到了但信号名不在该报文里，都在装配时报出——拼错名字是显式失败。
    for (msg, sig) in &script.named_signal_refs {
        let Some(id) = db.message_id_by_name(msg) else {
            lines.push(format!("[check] message name not found in DBC: \"{msg}\""));
            continue;
        };
        let known = db
            .message_of(id)
            .is_some_and(|m| m.signals.iter().any(|s| s.name == *sig));
        if !known {
            lines.push(format!(
                "[check] signal not found in DBC: \"{msg}\" \"{sig}\""
            ));
        }
    }
    // `on message Name` handlers resolve through the same name index; an
    // unknown name would otherwise just never fire.
    for handler in &script.handlers {
        if let HandlerKind::NamedMessage { name } = &handler.kind
            && db.message_key_by_name(name).is_none()
        {
            lines.push(format!(
                "[check] message name not found in DBC: \"{name}\" (handler)"
            ));
        }
    }
    // 发送集对照 DBC 发送者声明：发送了不属于绑定节点的报文 →
    // warning；不在 DBC 中的自定义报文 → info。
    if let Some(attached_node) = attached {
        for (_, id, ext) in &script.send_refs {
            let key = (*id, *ext);
            match db.messages.get(&key) {
                Some(m) if m.transmitter == attached_node => {} // own message
                Some(m) => lines.push(format!(
                    "[check] 0x{id:X} belongs to {}, not {attached_node}",
                    m.transmitter
                )),
                None => lines.push(format!(
                    "[check] 0x{id:X} is not in the DBC (custom message)"
                )),
            }
        }
    }
    // 反向漏实现：DBC 声明本节点发送、脚本却从不发送的报文。只是
    // info——生成器代发、脚本只做响应/监听都是常态；通配脚本（转发者/
    // 记录者）不检查。列出条目封顶，全量看编辑器收发 tab。
    if let Some(attached_node) = attached
        && !script.recv_wildcard
    {
        let mut missing: Vec<(u32, bool, &str)> = db
            .messages
            .iter()
            .filter(|(k, m)| {
                m.transmitter == attached_node
                    && !script
                        .send_refs
                        .iter()
                        .any(|(_, id, ext)| (*id, *ext) == **k)
            })
            .map(|(k, m)| (k.0, k.1, m.name.as_str()))
            .collect();
        missing.sort();
        if !missing.is_empty() {
            const SHOW: usize = 8;
            let list = missing
                .iter()
                .take(SHOW)
                .map(|(id, ext, name)| format!("0x{id:X}{} {name}", if *ext { "x" } else { "" }))
                .collect::<Vec<_>>()
                .join(", ");
            let more = if missing.len() > SHOW {
                format!(" and {} more", missing.len())
            } else {
                String::new()
            };
            lines.push(format!(
                "[info] the DBC declares frames this node sends but the script never sends them (the generator may send them): {list}{more}"
            ));
        }
    }
    // 通配转发归类（R2）：`on message *` 把脚本声明成「转发者/记录者」
    // ——发送集 = 接收集 ∩ 库声明，逐 id 的收发核对都不适用（上面反向
    // 检查已跳过）。转发器把发出的帧再收回是自激成环的唯一来源，检测
    // 到通配 handler 内有发送就如实提示（防护是否在，静态不可判定，
    // 所以是 info 不是 check——sniffer 示例的 frame_id 自排除即答案）。
    if let Some(attached_node) = attached
        && script.recv_wildcard
    {
        lines.push(format!(
            "[info] on message *: classified as forwarder/recorder; send set = received set ∩ DBC declaration ({attached_node})"
        ));
        if script.wildcard_sends() {
            lines.push(
                "[info] a wildcard handler sends frames: they re-enter this handler (feedback-loop risk) -- exclude your own frames with frame_id() before forwarding (see examples/sniffer.rxcan)"
                    .to_string(),
            );
        }
    }
    // 静态写入集：set_sig 的字面量值对照 DBC 信号物理界限。字面量才能
    // 静态判定；表达式的越界交给运行时（编码函数自然截断）。min==max
    // 的区间在 DBC 里表示"未声明界限"，不检查。
    for (id, sig, v) in &script.set_sig_values {
        let Some(m) = db.message_of(*id) else {
            continue; // 未知名已由信号引用检查报告
        };
        let Some(s) = m.signals.iter().find(|s| s.name == *sig) else {
            continue;
        };
        if s.max > s.min && (*v < s.min || *v > s.max) {
            lines.push(format!(
                "[check] set_sig 0x{id:X} \"{sig}\" value {v} is outside the DBC range {}..{}",
                s.min, s.max
            ));
        }
    }
    lines
}

/// The node's host-registered builtins: anything beyond the language's
/// builtin table lands here.
///
/// - `set_sig(buf, id, "Name", value)`: encodes one physical value into
///   the buffer through the channel's CAN model (buffer pads to 8 bytes).
/// - `get_sig(buf, id, "Name")`: decodes one physical value out of the
///   buffer, erroring when the model lacks the signal or no value is
///   representable.
fn node_extern(
    can_codec: Option<&CanCodec>,
    name: &str,
    args: &[Value],
) -> Result<Option<Value>, String> {
    if can_codec.is_none() && matches!(name, "set_sig" | "get_sig") {
        return Err("no CAN model on this channel: signal builtins unavailable".to_string());
    }
    match name {
        "set_sig" => {
            if args.len() != 4 {
                return Err("set_sig(buf, id, \"Name\", value) takes 4 arguments".to_string());
            }
            let Value::Bytes(buf) = &args[0] else {
                return Err("set_sig: first argument must be a byte buffer".into());
            };
            let Value::Int(id) = args[1] else {
                return Err("set_sig: message id must be an int".into());
            };
            let Value::Str(sig) = &args[2] else {
                return Err("set_sig: signal name must be a string".into());
            };
            let value = match &args[3] {
                Value::Int(n) => *n as f64,
                Value::Float(f) => *f,
                _ => return Err("set_sig: value must be a number".into()),
            };
            let Some((comm, plans, cluster)) = can_codec else {
                return Err("no CAN model on this channel: signal builtins unavailable".to_string());
            };
            let Some(frame) = comm.frames_in_cluster(*cluster).find(|frame| {
                matches!(
                    frame.address,
                    crate::model::FrameAddress::Can {
                        id: frame_id,
                        extended: false,
                        ..
                    } if frame_id == id as u32
                )
            }) else {
                return Err(format!("set_sig: unknown message {id:#x}"));
            };
            let Some(plan) = plans.get(frame.id) else {
                return Err(format!("set_sig: unknown message {id:#x}"));
            };
            let Some(signal) = comm
                .signals_in_frame(frame)
                .into_iter()
                .find(|signal| signal.name == *sig)
            else {
                return Err(format!("set_sig: unknown message/signal {id:#x} {sig}"));
            };
            let mut data = buf.lock().expect("buffer poisoned");
            if data.len() < 8 {
                data.resize(8, 0);
            }
            let written =
                crate::model::encode_payload(comm, plan, &[(signal.id, value)], &mut data[..]);
            if written == 1 {
                Ok(Some(Value::Nil))
            } else {
                Err(format!(
                    "set_sig: signal \"{sig}\" does not fit the payload"
                ))
            }
        }
        "get_sig" => {
            if args.len() != 3 {
                return Err("get_sig(buf, id, \"Name\") takes 3 arguments".into());
            }
            let Value::Bytes(buf) = &args[0] else {
                return Err("get_sig: first argument must be a byte buffer".into());
            };
            let Value::Int(id) = args[1] else {
                return Err("get_sig: message id must be an int".into());
            };
            let Value::Str(sig) = &args[2] else {
                return Err("get_sig: signal name must be a string".into());
            };
            let data = buf.lock().expect("buffer poisoned");
            let Some((comm, plans, cluster)) = can_codec else {
                return Err("no CAN model on this channel: signal builtins unavailable".to_string());
            };
            let Some(frame) = comm.frames_in_cluster(*cluster).find(|frame| {
                matches!(
                    frame.address,
                    crate::model::FrameAddress::Can {
                        id: frame_id,
                        extended: false,
                        ..
                    } if frame_id == id as u32
                )
            }) else {
                return Err(format!("get_sig: unknown message {id:#x}"));
            };
            let Some(plan) = plans.get(frame.id) else {
                return Err(format!("get_sig: unknown message {id:#x}"));
            };
            for value in crate::model::decode_payload(comm, plan, &data) {
                let Some(signal) = comm.signals.get(&value.signal) else {
                    continue;
                };
                if signal.name != *sig {
                    continue;
                }
                return match value.physical {
                    Some(phys) => Ok(Some(Value::Float(phys))),
                    None => Err(format!("get_sig: signal \"{sig}\" is outside the buffer")),
                };
            }
            Err(format!("get_sig: unknown message/signal {id:#x} {sig}"))
        }
        _ => Ok(None),
    }
}
