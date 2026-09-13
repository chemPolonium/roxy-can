//! Profile 档案：一套工程配多个环境档案。Profile 只存**覆盖**——
//! 角色声明与硬件映射叠加在工程之上（硬件映射即 P0 模型的第三层
//! overlay，随本档案走），同一份工程在 simulation / bench / CI 之间
//! 零修改切换。
//!
//! 错配是显式失败：档案写了工程里没有的总线、DBC 里没有的节点、
//! 拼错的角色词，整份档案被拒绝而非部分生效——半套覆盖比没有覆盖
//! 更难排查。角色词汇与工程 JSON 相同（`Simulated` / `Monitor` /
//! `Absent`）；缺省节点不写即离线，Profile 无需抄写工程。硬件映射是
//! **全量声明**：`[[hw]]` 列出每条要挂适配器的总线，未列出的总线应用
//! 档案时解挂——档案即完整硬件快照，可复现。
//!
//! 波特率（仲裁与 FD 数据段）不进档案：挂接取总线自己的设置，与 GUI
//! 挂接同一语义，档案只回答"哪条总线接哪个通道"。

use std::path::Path;

use crate::app::{App, NodeRole};

/// One `[[node]]` entry: which node on which bus takes which role.
struct RoleOverride {
    bus: String,
    node: String,
    role: NodeRole,
}

/// One `[[hw]]` entry: which Kvaser channel a bus attaches to.
struct HwOverride {
    bus: String,
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
                format!("[[node]] #{i}: unknown role `{role_word}` (want Simulated / Monitor / Absent)")
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
            hw.push(HwOverride { bus, channel });
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
        let known = app
            .channel_dbc(ch as u8)
            .is_some_and(|db| db.nodes.iter().any(|n| n == &ov.node));
        if !known {
            return Err(format!(
                "profile `{name}`: bus `{}` has no DBC node `{}`",
                ov.bus, ov.node
            ));
        }
        plan.push((ch as u8, ov.node.clone(), ov.role));
    }
    let mut hw_plan: Vec<(u8, i32)> = Vec::new();
    for ov in &hw_overrides {
        let Some(ch) = app.snap.channels.iter().position(|c| c.name == ov.bus) else {
            return Err(format!(
                "profile `{name}`: [[hw]] names bus `{}`, but no such bus exists in the project",
                ov.bus
            ));
        };
        hw_plan.push((ch as u8, ov.channel));
    }
    for (ch, node, role) in plan {
        app.set_node_role(ch, &node, role);
    }
    for (ch, channel) in &hw_plan {
        let view = &app.snap.channels[*ch as usize];
        app.set_hardware_channel(
            *ch,
            crate::hw::HwDriver::Kvaser,
            *channel,
            view.bitrate_kbps,
            Some(view.fd_data_kbps),
        );
    }
    // Unlisted buses lose their attachment: the profile is the whole map.
    let attached: Vec<u8> = app.snap.hw.iter().map(|v| v.bus).collect();
    let mapped: Vec<u8> = hw_plan.iter().map(|(ch, _)| *ch).collect();
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

/// Lists the profile names available in the project's `profiles/`
/// directory (`*.toml`, sorted, extension stripped). An unreadable or
/// missing directory yields an empty list.
pub fn list_profiles(project_dir: &Path) -> Vec<String> {
    let dir = project_dir.join("profiles");
    let Ok(rd) = std::fs::read_dir(&dir) else {
        return Vec::new();
    };
    let mut names: Vec<String> = rd
        .flatten()
        .filter(|e| e.path().extension().is_some_and(|x| x == "toml"))
        .filter_map(|e| {
            e.file_name()
                .to_string_lossy()
                .strip_suffix(".toml")
                .map(|s| s.to_string())
        })
        .collect();
    names.sort();
    names
}

#[cfg(test)]
#[path = "profile_tests.rs"]
mod tests;
