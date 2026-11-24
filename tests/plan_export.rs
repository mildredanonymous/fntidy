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
fn tidy_is_opt_in_and_saved_plans_use_its_rules() {
    let workspace = tempfile::tempdir().unwrap();
    let root = workspace.path().join("sample");
    fs::create_dir(&root).unwrap();
    let source = root.join("März  🇨🇭.txt");
    let target = root.join("März.txt");
    fs::write(&source, "original").unwrap();

    let command = |action: &str| {
        let mut command = Command::new(env!("CARGO_BIN_EXE_fntidy"));
        command.arg(action).arg(&root);
        command
    };
    assert_eq!(command("check").output().unwrap().status.code(), Some(0));
    let checked = command("check")
        .args(["--profile", "tidy"])
        .output()
        .unwrap();
    assert_eq!(checked.status.code(), Some(1));
    let findings = String::from_utf8(checked.stdout).unwrap();
    assert!(findings.contains("TIDY_EMOJI"));
    assert!(findings.contains("TIDY_WHITESPACE"));

    let plan = workspace.path().join("tidy.json");
    let preview = || {
        command("plan")
            .arg("--out")
            .arg(&plan)
            .args(["--profile", "tidy"])
            .output()
            .unwrap()
    };
    let result = preview();
    assert_eq!(result.status.code(), Some(0), "{result:?}");
    let exported: Value = serde_json::from_slice(&fs::read(&plan).unwrap()).unwrap();
    assert_eq!(exported["profile"], "tidy");
    assert_eq!(exported["profile_version"], 1);
    assert_eq!(exported["proposals"][0]["target"], "März.txt");
    assert_eq!(
        exported["proposals"][0]["rules"],
        serde_json::json!(["TIDY_EMOJI", "TIDY_WHITESPACE"])
    );
    assert_eq!(verify(&plan).status.code(), Some(0));
    assert!(!target.exists());

    fs::write(&target, "occupied").unwrap();
    assert_eq!(verify(&plan).status.code(), Some(2));
    fs::remove_file(&plan).unwrap();
    assert_eq!(preview().status.code(), Some(1));
    let blocked: Value = serde_json::from_slice(&fs::read(&plan).unwrap()).unwrap();
    assert_eq!(blocked["proposals"][0]["status"], "proposed_collision");
    assert_eq!(fs::read_to_string(&source).unwrap(), "original");
    assert_eq!(fs::read_to_string(&target).unwrap(), "occupied");

    #[cfg(windows)]
    {
        fs::remove_file(&target).unwrap();
        fs::remove_file(&plan).unwrap();
        assert_eq!(preview().status.code(), Some(0));
        let result = Command::new(env!("CARGO_BIN_EXE_fntidy"))
            .arg("apply")
            .arg(&plan)
            .env("LOCALAPPDATA", workspace.path())
            .output()
            .unwrap();
        assert_eq!(result.status.code(), Some(0), "{result:?}");
        assert!(!source.exists());
        assert_eq!(fs::read_to_string(&target).unwrap(), "original");
    }
}

#[test]
fn recursive_scan_uses_relative_paths_and_checks_collisions_per_directory() {
    let workspace = tempfile::tempdir().unwrap();
    let root = workspace.path().join("root");
    let good = root.join("A").join("deep");
    let second = root.join("B");
    fs::create_dir_all(&good).unwrap();
    fs::create_dir_all(&second).unwrap();
    let source = "Foto 😀.txt";
    fs::write(good.join(source), "first").unwrap();
    fs::write(second.join(source), "second").unwrap();
    let relative_source = Path::new("A").join("deep").join(source);
    let relative_target = relative_source.with_file_name("Foto.txt");
    let command = |action| {
        let mut command = Command::new(env!("CARGO_BIN_EXE_fntidy"));
        command.arg(action).arg(&root).args(["--profile", "tidy"]);
        command
    };

    assert_eq!(command("check").output().unwrap().status.code(), Some(0));
    let check = command("check").arg("--recursive").output().unwrap();
    assert_eq!(check.status.code(), Some(1));
    assert!(
        String::from_utf8(check.stdout)
            .unwrap()
            .contains("TIDY_EMOJI")
    );

    let plan = workspace.path().join("recursive.json");
    let preview = || {
        command("plan")
            .arg("--out")
            .arg(&plan)
            .arg("--recursive")
            .output()
            .unwrap()
    };
    let result = preview();
    assert_eq!(result.status.code(), Some(0), "{result:?}");
    let report: Value = serde_json::from_slice(&fs::read(&plan).unwrap()).unwrap();
    assert_eq!(report["schema_version"], 3);
    assert_eq!(report["recursive"], true);
    assert_eq!(report["proposals"].as_array().unwrap().len(), 2);
    assert_eq!(
        report["proposals"][0]["source"],
        relative_source.to_str().unwrap()
    );
    assert_eq!(
        report["proposals"][0]["target"],
        relative_target.to_str().unwrap()
    );
    assert_eq!(report["collisions"], serde_json::json!([]));
    assert_eq!(verify(&plan).status.code(), Some(0));

    fs::write(good.join("Foto.txt"), "occupied").unwrap();
    assert_eq!(verify(&plan).status.code(), Some(2));
    fs::remove_file(&plan).unwrap();
    assert_eq!(preview().status.code(), Some(1));
    let report: Value = serde_json::from_slice(&fs::read(&plan).unwrap()).unwrap();
    let collision = report["collisions"][0].as_array().unwrap();
    assert!(collision.contains(&serde_json::json!(relative_source.to_str().unwrap())));
    assert!(collision.contains(&serde_json::json!(relative_target.to_str().unwrap())));
    assert_eq!(fs::read_to_string(good.join(source)).unwrap(), "first");
    assert_eq!(fs::read_to_string(second.join(source)).unwrap(), "second");
}

#[test]
fn recursive_plan_rejects_a_replaced_parent_even_with_the_same_file() {
    let workspace = tempfile::tempdir().unwrap();
    let root = workspace.path().join("root");
    let original_parent = root.join("A");
    fs::create_dir_all(&original_parent).unwrap();
    let source = "Foto 😀.txt";
    fs::write(original_parent.join(source), "first").unwrap();
    let plan = workspace.path().join("recursive.json");
    let result = Command::new(env!("CARGO_BIN_EXE_fntidy"))
        .arg("plan")
        .arg(&root)
        .args(["--recursive", "--profile", "tidy", "--out"])
        .arg(&plan)
        .output()
        .unwrap();
    assert_eq!(result.status.code(), Some(0), "{result:?}");

    #[cfg(windows)]
    {
        let result = Command::new(env!("CARGO_BIN_EXE_fntidy"))
            .arg("apply")
            .arg(&plan)
            .env("LOCALAPPDATA", workspace.path())
            .output()
            .unwrap();
        assert_eq!(result.status.code(), Some(0), "{result:?}");
        assert!(!original_parent.join(source).exists());
        let target = original_parent.join("Foto.txt");
        assert_eq!(fs::read_to_string(&target).unwrap(), "first");
        fs::rename(target, original_parent.join(source)).unwrap();
    }

    let previous = workspace.path().join("previous");
    fs::rename(&original_parent, &previous).unwrap();
    fs::create_dir(&original_parent).unwrap();
    fs::rename(previous.join(source), original_parent.join(source)).unwrap();
    let result = verify(&plan);
    assert_eq!(result.status.code(), Some(2), "{result:?}");
    assert!(String::from_utf8_lossy(&result.stderr).contains("parent directory was replaced"));
}

#[cfg(unix)]
#[test]
fn recursive_scan_skips_directory_links() {
    use std::os::unix::fs::symlink;

    let workspace = tempfile::tempdir().unwrap();
    let root = workspace.path().join("root");
    let outside = workspace.path().join("outside");
    fs::create_dir(&root).unwrap();
    fs::create_dir(&outside).unwrap();
    fs::write(outside.join("Unseen 😀.txt"), "original").unwrap();
    symlink(&outside, root.join("external")).unwrap();
    symlink(&root, root.join("loop")).unwrap();
    let result = Command::new(env!("CARGO_BIN_EXE_fntidy"))
        .arg("plan")
        .arg(&root)
        .args(["--profile", "tidy", "--recursive"])
        .output()
        .unwrap();
    assert_eq!(result.status.code(), Some(0), "{result:?}");
    assert!(String::from_utf8_lossy(&result.stdout).contains("0 proposed rename(s)"));
    assert_eq!(
        String::from_utf8_lossy(&result.stderr)
            .matches("Skipped redirected entry")
            .count(),
        2
    );
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

#[cfg(windows)]
#[test]
fn checks_exact_windows_names_and_rejects_replacements_and_occupied_targets() {
    let workspace = tempfile::tempdir().unwrap();
    let root = workspace.path().join("source");
    fs::create_dir(&root).unwrap();
    // The verbatim path preserves the trailing period in the test name.
    let exact_root = root.canonicalize().unwrap();
    let source = exact_root.join("März.txt.");
    let target = exact_root.join("März.txt");
    fs::write(&source, "same content").unwrap();
    let plan = workspace.path().join("plan.json");

    let result = export(&root, &plan);
    assert_eq!(result.status.code(), Some(0), "{result:?}");
    let report: Value = serde_json::from_slice(&fs::read(&plan).unwrap()).unwrap();
    assert_eq!(report["proposals"][0]["source"], "März.txt.");
    assert_eq!(report["proposals"][0]["target"], "März.txt");
    assert_eq!(report["identities"]["sources"][0]["platform"], "windows");
    let result = verify(&plan);
    assert_eq!(result.status.code(), Some(0), "{result:?}");

    fs::write(&target, "occupied").unwrap();
    let result = verify(&plan);
    assert_eq!(result.status.code(), Some(2), "{result:?}");
    assert!(String::from_utf8_lossy(&result.stderr).contains("naming conflict"));
    assert_eq!(fs::read(&source).unwrap(), b"same content");
    assert_eq!(fs::read(&target).unwrap(), b"occupied");
    fs::remove_file(&target).unwrap();

    fs::rename(&source, exact_root.join("previous.txt")).unwrap();
    fs::write(&source, "same content").unwrap();
    let result = verify(&plan);
    assert_eq!(result.status.code(), Some(2), "{result:?}");
    assert!(String::from_utf8_lossy(&result.stderr).contains("was replaced"));
    assert!(!target.exists());
}

#[cfg(windows)]
#[test]
fn applies_one_verified_rename_and_keeps_its_recovery_journal() {
    let workspace = tempfile::tempdir().unwrap();
    let root = workspace.path().join("source");
    let local = workspace.path().join("local");
    fs::create_dir(&root).unwrap();
    let base = root.canonicalize().unwrap();
    let source = base.join("Bericht.txt.");
    let target = base.join("Bericht.txt");
    let plan = workspace.path().join("plan.json");
    fs::write(&source, b"test content").unwrap();
    assert_eq!(export(&root, &plan).status.code(), Some(0));

    let apply = || {
        Command::new(env!("CARGO_BIN_EXE_fntidy"))
            .arg("apply")
            .arg(&plan)
            .env("LOCALAPPDATA", &local)
            .output()
            .unwrap()
    };
    fs::write(&target, b"occupied").unwrap();
    assert_eq!(apply().status.code(), Some(2));
    assert_eq!(fs::read(&source).unwrap(), b"test content");
    assert_eq!(fs::read(&target).unwrap(), b"occupied");
    assert!(!local.exists());
    fs::remove_file(&target).unwrap();

    let result = apply();
    assert_eq!(result.status.code(), Some(0), "{result:?}");
    assert_eq!(fs::read(&target).unwrap(), b"test content");
    assert!(!source.exists());
    let runs = local.join("fntidy").join("runs");
    let run = fs::read_dir(runs).unwrap().next().unwrap().unwrap();
    let log = fs::read_to_string(run.path().join("run.jsonl")).unwrap();
    let events: Vec<Value> = log
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    assert_eq!(events[1]["source"], "Bericht.txt.");
    assert_eq!(events[1]["target"], "Bericht.txt");
    assert_eq!(events[2]["event"], "renamed");
    assert_eq!(events[3]["event"], "finished");
    assert!(String::from_utf8_lossy(&result.stderr).contains("run.jsonl"));
    assert_eq!(apply().status.code(), Some(2));
}
