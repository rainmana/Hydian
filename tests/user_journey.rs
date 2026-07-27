use std::fs;

use predicates::prelude::*;
use serde_json::Value;
use tempfile::TempDir;

fn run_json(home: &TempDir, arguments: &[&str]) -> Value {
    let output = assert_cmd::cargo::cargo_bin_cmd!("hydian")
        .arg("--home")
        .arg(home.path())
        .arg("--json")
        .args(arguments)
        .output()
        .expect("Hydian should run");
    assert!(
        output.status.success(),
        "command failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).expect("output should be JSON")
}

#[test]
fn first_run_preview_then_initialize_is_safe_and_repeatable() {
    let home = TempDir::new().unwrap();

    let preview = run_json(&home, &["init", "--dry-run"]);
    assert_eq!(preview["ok"], true);
    assert_eq!(preview["data"]["dry_run"], true);
    assert!(!home.path().join("config.toml").exists());

    let initialized = run_json(&home, &["init"]);
    assert_eq!(initialized["data"]["dry_run"], false);
    assert!(home.path().join("config.toml").is_file());
    assert!(home.path().join("mcp.json").is_file());

    assert_cmd::cargo::cargo_bin_cmd!("hydian")
        .arg("--home")
        .arg(home.path())
        .arg("init")
        .assert()
        .failure()
        .stderr(predicate::str::contains("already exists"));

    let endpoint = run_json(&home, &["endpoint", "--format", "json"]);
    assert_eq!(endpoint["data"]["url"], "http://127.0.0.1:7337/mcp");
}

#[test]
fn import_preview_and_apply_form_a_complete_configuration_journey() {
    let home = TempDir::new().unwrap();
    run_json(&home, &["init"]);
    let source = home.path().join("client.json");
    fs::write(
        &source,
        r#"{"mcpServers":{"notes":{"command":"notes-server","args":["--stdio"]}}}"#,
    )
    .unwrap();

    let preview = run_json(&home, &["import", source.to_str().unwrap(), "--dry-run"]);
    assert_eq!(preview["data"]["applied"], false);
    let before = fs::read(home.path().join("mcp.json")).unwrap();

    let applied = run_json(&home, &["import", source.to_str().unwrap(), "--apply"]);
    assert_eq!(applied["data"]["applied"], true);
    let after: Value = serde_json::from_slice(&fs::read(home.path().join("mcp.json")).unwrap())
        .expect("written MCP configuration should be JSON");
    assert_ne!(before, serde_json::to_vec_pretty(&after).unwrap());
    assert_eq!(after["mcpServers"]["notes"]["command"], "notes-server");

    let servers = run_json(&home, &["servers", "list"]);
    assert_eq!(servers["data"][0]["name"], "notes");
}
