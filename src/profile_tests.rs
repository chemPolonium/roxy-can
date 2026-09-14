//! Profile 档案的行为测试：覆盖生效、错配整份拒绝。

use std::path::Path;

use super::apply_profile;
use super::parse;
use crate::app::{App, NodeRole};
use crate::config::{Config, ProjectFile};

/// Fixture: a directory with `net.rxproj` (saved from the default
/// headless workspace: CAN1 + sample DBC, CAN2 + motbus) and the given
/// profile text at `profiles/<name>.toml`. Returns a fresh app with the
/// project already open and the snapshot settled.
fn fixture(dir: &Path, name: &str, profile: &str) -> App {
    let fresh = App::headless();
    let proj = ProjectFile {
        version: 1,
        layout: String::new(),
        project: None,
        config: Config::from_app(&fresh, None),
    };
    std::fs::create_dir_all(dir.join("profiles")).unwrap();
    std::fs::write(
        dir.join("net.rxproj"),
        serde_json::to_string_pretty(&proj).expect("project serializes"),
    )
    .unwrap();
    std::fs::write(dir.join("profiles").join(format!("{name}.toml")), profile).unwrap();

    let mut app = App::headless();
    app.open_project_path(&dir.join("net.rxproj"));
    app.settle();
    assert!(
        app.project_path.is_some(),
        "the fixture project must open: {}",
        app.status
    );
    app
}

/// A valid profile lands its overrides on top of the project: roles flip
/// and a simulated node actually takes the wire.
#[test]
fn a_profile_overrides_roles_on_top_of_a_project() {
    let dir = std::env::temp_dir().join("roxy_can_profile_ok");
    let mut app = fixture(
        &dir,
        "bench",
        r#"# bench: the ECU under test is real hardware, the rest is simulated
description = "bench rig"

[[node]]
bus = "CAN1"
node = "EngineECU"
role = "Simulated"

[[node]]
bus = "CAN2"
node = "ABS"
role = "Absent"
"#,
    );
    let summary = apply_profile(&mut app, &dir, "bench").expect("profile applies");
    app.settle();

    assert_eq!(app.node_role(0, "EngineECU"), NodeRole::Simulated);
    // 默认关：角色应用不改写条目开关——Simulated 只开放闸门，用户在
    // 生成器里逐条启用后才会发车。
    assert!(
        app.tx_list
            .iter()
            .filter(|t| t.channel == 0 && t.node == "EngineECU")
            .all(|t| !t.active),
        "the muted project stays muted until the user enables entries"
    );
    assert_eq!(app.node_role(1, "ABS"), NodeRole::Absent);
    assert!(summary.contains("2 role override(s)"), "{summary}");
    std::fs::remove_dir_all(&dir).ok();
}

/// One bad entry refuses the WHOLE profile: the good entry must not have
/// landed -- a half-applied overlay is the hardest state to debug.
#[test]
fn a_profile_with_one_unknown_node_is_refused_wholesale() {
    let dir = std::env::temp_dir().join("roxy_can_profile_bad_node");
    let mut app = fixture(
        &dir,
        "typo",
        r#"[[node]]
bus = "CAN1"
node = "EngineECU"
role = "Simulated"

[[node]]
bus = "CAN1"
node = "EngineEcu"
role = "Absent"
"#,
    );
    let err = apply_profile(&mut app, &dir, "typo").expect_err("case mismatch is refused");
    assert!(err.contains("EngineEcu"), "{err}");
    app.settle();
    assert_eq!(
        app.node_role(0, "EngineECU"),
        NodeRole::Absent,
        "the good entry did not land either"
    );
    std::fs::remove_dir_all(&dir).ok();
}

/// A bus the project does not have is refused by name: profiles travel
/// between machines and projects, and a renamed bus must not silently
/// skip its entries.
#[test]
fn a_profile_with_an_unknown_bus_is_refused() {
    let dir = std::env::temp_dir().join("roxy_can_profile_bad_bus");
    let mut app = fixture(
        &dir,
        "wrong_bus",
        r#"[[node]]
bus = "Chassis"
node = "ABS"
role = "Absent"
"#,
    );
    let err = apply_profile(&mut app, &dir, "wrong_bus").expect_err("no such bus");
    assert!(err.contains("Chassis"), "{err}");
    std::fs::remove_dir_all(&dir).ok();
}

/// A missing profile file names what it looked for, so a typo in
/// `--profile` explains itself instead of silently running bare.
#[test]
fn a_missing_profile_file_names_the_path() {
    let mut app = App::headless();
    let err = apply_profile(&mut app, Path::new("nowhere"), "ghost")
        .expect_err("nothing exists under nowhere/");
    assert!(err.contains("profiles"), "{err}");
    assert!(err.contains("ghost.toml"), "{err}");
}

/// The parse layer: shape and vocabulary errors point at the entry.
#[test]
fn parse_errors_name_the_offending_entry() {
    let (roles, hw) = parse("").unwrap();
    assert!(roles.is_empty() && hw.is_empty(), "no tables is a no-op");
    assert!(parse("node = 3\n").is_err(), "not an array of tables");
    assert!(
        parse("[[node]]\nbus = \"A\"\nnode = \"B\"\n").is_err(),
        "missing role"
    );
    assert!(
        parse("[[node]]\nbus = \"A\"\nnode = \"B\"\nrole = \"Sim\"\n").is_err(),
        "an unknown role word is a parse error, not a default"
    );
    let doc = "[[node]]\nbus = \"A\"\nnode = \"B\"\nrole = \"Absent\"\n";
    let (roles, hw) = parse(doc).unwrap();
    assert_eq!(roles.len(), 1);
    assert!(hw.is_empty());
    // The retired three-state vocabulary stays rejected: Monitor folded
    // into Absent, and old profiles saying so are refused wholesale (the
    // user re-saves them with the current words).
    assert!(
        parse("[[node]]\nbus = \"A\"\nnode = \"B\"\nrole = \"Monitor\"\n").is_err(),
        "Monitor is retired vocabulary now"
    );
}

/// The `[[hw]]` layer: integer channel required, negatives refused, and
/// a hw-only profile (no [[node]]) parses to an empty role list.
#[test]
fn hw_entries_parse_their_channel_field() {
    let doc = "[[hw]]\nbus = \"A\"\nchannel = 2\n";
    let (roles, hw) = parse(doc).unwrap();
    assert!(roles.is_empty(), "a hw-only profile carries no roles");
    assert_eq!(hw.len(), 1);
    assert_eq!(hw[0].bus, "A");
    assert_eq!(hw[0].channel, 2);

    assert!(
        parse("[[hw]]\nbus = \"A\"\n").is_err(),
        "missing channel"
    );
    assert!(
        parse("[[hw]]\nbus = \"A\"\nchannel = -1\n").is_err(),
        "negative channel"
    );
    assert!(
        parse("[[hw]]\nbus = \"A\"\nchannel = \"0\"\n").is_err(),
        "channel must be an integer, not a string"
    );
    assert!(
        parse("hw = 3\n").is_err(),
        "hw must be an array of tables"
    );
}
