//! Profile 档案：一套工程配多个环境档案。Profile 只存**覆盖**——
//! 角色声明叠加在工程之上（阶段 5 的硬件映射也走这里），同一份工程
//! 在 simulation / bench / CI 之间零修改切换。
//!
//! 错配是显式失败：档案写了工程里没有的总线、DBC 里没有的节点、
//! 拼错的角色词，整份档案被拒绝而非部分生效——半套覆盖比没有覆盖
//! 更难排查。角色词汇与工程 JSON 相同（`Simulated` / `Monitor` /
//! `Absent`）；缺省节点不写即离线，Profile 无需抄写工程。

use std::path::Path;

use crate::app::{App, NodeRole};

/// One `[[node]]` entry: which node on which bus takes which role.
struct RoleOverride {
    bus: String,
    node: String,
    role: NodeRole,
}

/// Parses the profile text into role overrides. `toml_edit` is used
/// directly (it is already in the dependency tree, and the environment
/// cannot fetch the `toml` wrapper): the schema is two strings and one
/// word per entry, so hand extraction with pointed error messages is the
/// smaller machine.
fn parse(text: &str) -> Result<Vec<RoleOverride>, String> {
    let doc: toml_edit::DocumentMut = text
        .parse()
        .map_err(|e| format!("profile TOML error: {e}"))?;
    let mut out = Vec::new();
    let Some(item) = doc.get("node") else {
        return Ok(out);
    };
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
        out.push(RoleOverride { bus, node, role });
    }
    Ok(out)
}

/// Loads `profiles/<name>.toml` from the project directory and overlays
/// its role declarations. Every entry is resolved and validated against
/// the live snapshot **before** any command goes out: a refused profile
/// changes nothing.
pub fn apply_profile(app: &mut App, project_dir: &Path, name: &str) -> Result<String, String> {
    let path = project_dir.join("profiles").join(format!("{name}.toml"));
    let text = std::fs::read_to_string(&path)
        .map_err(|_| format!("profile `{name}` not found: {}", path.display()))?;
    let overrides = parse(&text)?;

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
    for (ch, node, role) in plan {
        app.set_node_role(ch, &node, role);
    }
    Ok(format!(
        "profile `{name}` applied: {} role override(s)",
        overrides.len()
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
