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
    assert_eq!(plan["schema_version"], 2);
    assert_eq!(plan["profile"], "windows");
    assert_eq!(plan["profile_version"], 1);
    assert_eq!(plan["status"], "ready");
    assert_eq!(plan["root"], root.to_str().unwrap());
    assert_eq!(plan["proposals"], serde_json::json!([]));
    assert_eq!(plan["collisions"], serde_json::json!([]));
    assert!(plan["identities"]["root"].is_object());
    assert_eq!(plan["identities"]["sources"], serde_json::json!([]));

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

fn verify(path: &Path) -> Output {
    Command::new(env!("CARGO_BIN_EXE_fntidy"))
        .arg("verify-plan")
        .arg(path)
        .output()
        .unwrap()
}

#[test]
fn verifies_an_unchanged_export_and_rejects_an_unknown_version() {
    let workspace = tempfile::tempdir().unwrap();
    let root = workspace.path().join("source");
    fs::create_dir(&root).unwrap();
    let path = workspace.path().join("plan.json");

    assert_eq!(export(&root, &path).status.code(), Some(0));
    assert_eq!(verify(&path).status.code(), Some(0));

    let mut report: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    report["schema_version"] = 999.into();
    fs::write(&path, serde_json::to_vec(&report).unwrap()).unwrap();
    let result = verify(&path);
    assert_eq!(result.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&result.stderr).contains("Unsupported plan format"));
    assert_eq!(fs::read_dir(&root).unwrap().count(), 0);
}

#[cfg(unix)]
#[test]
fn rejects_changed_proposals_and_newly_occupied_targets_without_renaming() {
    let workspace = tempfile::tempdir().unwrap();
    let root = workspace.path().join("source");
    fs::create_dir(&root).unwrap();
    let source = root.join("März?.txt");
    let target = root.join("März_.txt");
    fs::write(&source, "original").unwrap();
    let path = workspace.path().join("plan.json");
    assert_eq!(export(&root, &path).status.code(), Some(0));
    assert_eq!(verify(&path).status.code(), Some(0));
    assert!(!target.exists());

    let original_plan = fs::read(&path).unwrap();
    let mut report: Value = serde_json::from_slice(&original_plan).unwrap();
    report["proposals"][0]["target"] = "../outside.txt".into();
    fs::write(&path, serde_json::to_vec(&report).unwrap()).unwrap();
    assert_eq!(verify(&path).status.code(), Some(2));
    assert!(!workspace.path().join("outside.txt").exists());

    fs::write(&path, original_plan).unwrap();
    fs::write(&target, "occupied").unwrap();
    let result = verify(&path);
    assert_eq!(result.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&result.stderr).contains("naming conflict"));
    assert_eq!(fs::read_to_string(&source).unwrap(), "original");
    assert_eq!(fs::read_to_string(&target).unwrap(), "occupied");

    fs::remove_file(&source).unwrap();
    let result = verify(&path);
    assert_eq!(result.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&result.stderr).contains("differ"));
}

#[test]
fn rejects_a_replaced_root_with_the_same_preview() {
    let workspace = tempfile::tempdir().unwrap();
    let root = workspace.path().join("source");
    fs::create_dir(&root).unwrap();
    let path = workspace.path().join("plan.json");

    assert_eq!(export(&root, &path).status.code(), Some(0));
    fs::rename(&root, workspace.path().join("previous")).unwrap();
    fs::create_dir(&root).unwrap();

    let result = verify(&path);
    assert_eq!(result.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&result.stderr).contains("directory was replaced"));
}

#[cfg(unix)]
#[test]
fn rejects_a_replacement_at_the_same_source_name() {
    let workspace = tempfile::tempdir().unwrap();
    let root = workspace.path().join("source");
    fs::create_dir(&root).unwrap();
    let source = root.join("CON.txt");
    let plan = workspace.path().join("plan.json");
    fs::write(&source, "same content").unwrap();

    assert_eq!(export(&root, &plan).status.code(), Some(0));
    assert_eq!(verify(&plan).status.code(), Some(0));
    fs::rename(&source, workspace.path().join("previous.txt")).unwrap();
    fs::write(&source, "same content").unwrap();

    let result = verify(&plan);
    assert_eq!(result.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&result.stderr).contains("source \"CON.txt\" was replaced"));
    assert_eq!(fs::read(&source).unwrap(), b"same content");
    assert!(!root.join("_CON.txt").exists());
}
