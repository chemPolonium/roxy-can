//! Profile 档案：一套工程配多个环境档案。Profile 只存**覆盖**——
//! 角色声明与硬件映射叠加在工程之上（硬件映射即 P0 模型的第三层
//! overlay，随本档案走），同一份工程在 simulation / bench / CI 之间
//! 零修改切换。
//!
//! 错配是显式失败：档案写了工程里没有的总线、DBC 里没有的节点、
//! 拼错的角色词，整份档案被拒绝而非部分生效——只应用一半的覆盖比完全没覆盖
//! 更难排查。角色词汇与工程 JSON 相同（`Simulated` / `Absent`，旧的
//! `Monitor` 词已随两态化退役）；缺省节点不写即离线，Profile 无需抄
//! 写工程。硬件映射是**全量声明**：`[[hw]]` 列出每条要连接适配器的总
//! 线，未列出的总线应用档案时断开——档案即完整硬件快照，可复现。
//!
//! 波特率（仲裁与 FD 数据段）不进档案：连接取总线自己的设置，与 GUI
//! 连接同一语义，档案只回答"哪条总线接哪个通道"。

use std::path::Path;

use crate::app::{App, NodeRole};
use crate::hw::HwDriver;

/// One `[[node]]` entry: which node on which bus takes which role.
struct RoleOverride {
    bus: String,
    node: String,
    role: NodeRole,
}

/// One `[[hw]]` entry: which channel (of which driver) a bus attaches
/// to. `driver` is optional in the file for pre-Vector profiles and
/// defaults to Kvaser, the only driver that existed when the format
/// was born.
struct HwOverride {
    bus: String,
    driver: HwDriver,
    channel: i32,
}

/// Parses the profile text into role + hardware overrides. `toml_edit`
/// is used directly (it is already in the dependency tree, and the
/// environment cannot fetch the `toml` wrapper): the schema is a few
/// strings and one word per entry, so hand extraction with pointed
/// error messages is the smaller machine.
fn parse(text: &str) -> Result<(Vec<RoleOverride>, Vec<HwOverride>), String> {
    let doc: toml_edit::DocumentMut = text
        .parse()
        .map_err(|e| format!("profile TOML error: {e}"))?;
    let mut roles = Vec::new();
    if let Some(item) = doc.get("node") {
        let Some(entries) = item.as_array_of_tables() else {
            return Err("`node` must be an array of tables: [[node]]".to_string());
        };
        for (i, t) in entries.iter().enumerate() {
            let field = |name: &str| -> Result<String, String> {
                t.get(name)
                    .and_then(|v| v.as_str())
                    .map(str::to_string)
                    .ok_or_else(|| format!("[[node]] #{i}: missing string field `{name}`"))
            };
            let bus = field("bus")?;
            let node = field("node")?;
            let role_word = field("role")?;
            let role = NodeRole::parse(&role_word).ok_or_else(|| {
                format!("[[node]] #{i}: unknown role `{role_word}` (want Simulated / Absent)")
            })?;
            roles.push(RoleOverride { bus, node, role });
        }
    }
    let mut hw = Vec::new();
    if let Some(item) = doc.get("hw") {
        let Some(entries) = item.as_array_of_tables() else {
            return Err("`hw` must be an array of tables: [[hw]]".to_string());
        };
        for (i, t) in entries.iter().enumerate() {
            let bus = t
                .get("bus")
                .and_then(|v| v.as_str())
                .map(str::to_string)
                .ok_or_else(|| format!("[[hw]] #{i}: missing string field `bus`"))?;
            let channel = t
                .get("channel")
                .and_then(|v| v.as_integer())
                .ok_or_else(|| format!("[[hw]] #{i}: missing integer field `channel`"))?;
            let Ok(channel) = i32::try_from(channel) else {
                return Err(format!("[[hw]] #{i}: channel must fit i32"));
            };
            if channel < 0 {
                return Err(format!("[[hw]] #{i}: channel must be >= 0"));
            }
            // The driver word is optional: profiles from before the
            // Vector binding only ever meant Kvaser.
            let driver = match t.get("driver").and_then(|v| v.as_str()) {
                None => HwDriver::Kvaser,
                Some(w) => match HwDriver::parse(w) {
                    Some(d) => d,
                    None => {
                        return Err(format!(
                            "[[hw]] #{i}: unknown driver `{w}` (want Kvaser / Vector)"
                        ));
                    }
                },
            };
            hw.push(HwOverride {
                bus,
                driver,
                channel,
            });
        }
    }
    Ok((roles, hw))
}

/// Loads `profiles/<name>.toml` from the project directory and overlays
/// its role declarations and hardware mapping. Every entry is resolved
/// and validated against the live snapshot **before** any command goes
/// out: a refused profile changes nothing. The hardware mapping is a
/// full snapshot: buses not listed in `[[hw]]` are detached on apply.
/// A hardware *open* failing at runtime (adapter gone, driver missing)
/// reports through the status line but does not abort — the role
/// overrides stand on their own.
pub fn apply_profile(app: &mut App, project_dir: &Path, name: &str) -> Result<String, String> {
    let path = project_dir.join("profiles").join(format!("{name}.toml"));
    let text = std::fs::read_to_string(&path)
        .map_err(|_| format!("profile `{name}` not found: {}", path.display()))?;
    let (overrides, hw_overrides) = parse(&text)?;

    let mut plan: Vec<(u8, String, NodeRole)> = Vec::new();
    for ov in &overrides {
        let Some(ch) = app.snap.channels.iter().position(|c| c.name == ov.bus) else {
            return Err(format!(
                "profile `{name}`: no bus named `{}` in the project",
                ov.bus
            ));
        };
        eprintln!(
            "channels {:?}",
            app.snap
                .channels
                .iter()
                .map(|c| (&c.name, &c.dbc_paths))
                .collect::<Vec<_>>()
        );
        let known = app
            .can_comm(ch)
            .is_some_and(|comm| comm.ecus.values().any(|ecu| ecu.name == ov.node));
        if !known {
            return Err(format!(
                "profile `{name}`: bus `{}` has no DBC node `{}`",
                ov.bus, ov.node
            ));
        }
        plan.push((ch as u8, ov.node.clone(), ov.role));
    }
    let mut hw_plan: Vec<(u8, HwDriver, i32)> = Vec::new();
    for ov in &hw_overrides {
        let Some(ch) = app.snap.channels.iter().position(|c| c.name == ov.bus) else {
            return Err(format!(
                "profile `{name}`: [[hw]] names bus `{}`, but no such bus exists in the project",
                ov.bus
            ));
        };
        hw_plan.push((ch as u8, ov.driver, ov.channel));
    }
    for (ch, node, role) in plan {
        app.set_node_role(ch, &node, role);
    }
    for (ch, driver, channel) in &hw_plan {
        let view = &app.snap.channels[*ch as usize];
        app.set_hardware_channel(
            *ch,
            *driver,
            *channel,
            view.bitrate_kbps,
            Some(view.fd_data_kbps),
        );
    }
    // Unlisted buses lose their attachment: the profile is the whole map.
    let attached: Vec<u8> = app.snap.hw.iter().map(|v| v.bus).collect();
    let mapped: Vec<u8> = hw_plan.iter().map(|(ch, ..)| *ch).collect();
    for bus in attached {
        if !mapped.contains(&bus) {
            app.detach_hardware(bus);
        }
    }
    Ok(format!(
        "profile `{name}` applied: {} role override(s), {} hardware mapping(s)",
        overrides.len(),
        hw_overrides.len()
    ))
}

#[cfg(test)]
#[path = "profile_tests.rs"]
mod tests;
