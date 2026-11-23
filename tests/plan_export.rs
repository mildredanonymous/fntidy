use std::{
    fs,
    path::Path,
    process::{Command, Output},
};

use serde_json::Value;

fn export(root: &Path, output: &Path) -> Output {
    Command::new(env!("CARGO_BIN_EXE_fntidy"))
        .arg("plan")
        .arg(root)
        .arg("--out")
        .arg(output)
        .output()
        .unwrap()
}

#[test]
fn exports_a_versioned_plan_and_preserves_existing_output() {
    let workspace = tempfile::tempdir().unwrap();
    let root = workspace.path().join("März");
    fs::create_dir(&root).unwrap();
    fs::write(root.join("existing.txt"), "contents").unwrap();
    let path = workspace.path().join("plan.json");

    let result = export(&root, &path);
    assert_eq!(result.status.code(), Some(0), "{result:?}");
    let original = fs::read(&path).unwrap();
    let plan: Value = serde_json::from_slice(&original).unwrap();
    assert_eq!(plan["schema_version"], 1);
    assert_eq!(plan["profile"], "windows");
    assert_eq!(plan["profile_version"], 1);
    assert_eq!(plan["status"], "ready");
    assert_eq!(plan["root"], root.to_str().unwrap());
    assert_eq!(plan["proposals"], serde_json::json!([]));
    assert_eq!(plan["collisions"], serde_json::json!([]));

    assert_eq!(export(&root, &path).status.code(), Some(2));
    assert_eq!(fs::read(&path).unwrap(), original);
    assert_eq!(fs::read(root.join("existing.txt")).unwrap(), b"contents");
    assert_eq!(fs::read_dir(&root).unwrap().count(), 1);
    assert_eq!(fs::read_dir(workspace.path()).unwrap().count(), 2);
}

#[cfg(unix)]
#[test]
fn exports_blocked_proposals_with_exact_names_and_rule_ids() {
    let workspace = tempfile::tempdir().unwrap();
    let root = workspace.path().join("source");
    fs::create_dir(&root).unwrap();
    fs::write(root.join("März?.txt"), "original").unwrap();
    fs::write(root.join("März_.txt"), "occupied").unwrap();
    let path = workspace.path().join("blocked.json");

    assert_eq!(export(&root, &path).status.code(), Some(1));
    let plan: Value = serde_json::from_slice(&fs::read(path).unwrap()).unwrap();
    assert_eq!(plan["status"], "blocked");
    assert_eq!(
        plan["proposals"][0],
        serde_json::json!({
            "source": "März?.txt",
            "target": "März_.txt",
            "status": "proposed_collision",
            "rules": ["WIN_INVALID_CHARACTER"],
        })
    );
    assert_eq!(
        plan["collisions"],
        serde_json::json!([["März?.txt", "März_.txt"]])
    );
    assert_eq!(fs::read(root.join("März?.txt")).unwrap(), b"original");
    assert_eq!(fs::read(root.join("März_.txt")).unwrap(), b"occupied");
}
