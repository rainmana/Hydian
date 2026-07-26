use std::fs;

use predicates::prelude::*;
use serde_json::Value;
use tempfile::TempDir;

#[test]
fn redirected_no_subcommand_and_plain_mode_print_help_without_tui_sequences() {
    assert_cmd::cargo::cargo_bin_cmd!("hydian")
        .arg("--plain")
        .assert()
        .success()
        .stdout(predicate::str::contains("Usage: hydian"))
        .stdout(predicate::str::contains("\u{1b}[").not());
}

#[test]
fn no_color_environment_never_adds_ansi() {
    assert_cmd::cargo::cargo_bin_cmd!("hydian")
        .env("NO_COLOR", "1")
        .args(["--plain", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("\u{1b}[").not());
}

#[test]
fn service_dry_run_uses_the_stable_json_envelope() {
    let directory = TempDir::new().unwrap();
    let output = assert_cmd::cargo::cargo_bin_cmd!("hydian")
        .args([
            "--home",
            directory.path().to_str().unwrap(),
            "--json",
            "service",
            "install",
            "--dry-run",
        ])
        .output()
        .unwrap();
    assert!(output.status.success());
    let value: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(value["schema_version"], 1);
    assert_eq!(value["ok"], true);
    assert_eq!(value["data"]["dry_run"], true);
}

#[test]
fn custom_provider_plan_is_transparent_and_expands_local_url() {
    let directory = TempDir::new().unwrap();
    assert_cmd::cargo::cargo_bin_cmd!("hydian")
        .args(["--home", directory.path().to_str().unwrap(), "init"])
        .assert()
        .success();
    let output = assert_cmd::cargo::cargo_bin_cmd!("hydian")
        .args([
            "--home",
            directory.path().to_str().unwrap(),
            "--json",
            "expose",
            "plan",
            "custom",
            "--",
            "my-tunnel",
            "--upstream",
            "{local_url}",
        ])
        .output()
        .unwrap();
    assert!(output.status.success());
    let value: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert!(
        value["data"]["plan"]["command_display"]
            .as_str()
            .unwrap()
            .contains("http://127.0.0.1:7337/mcp")
    );
    assert!(!directory.path().join("run/exposure.json").exists());
}

#[test]
fn import_preview_does_not_change_the_destination() {
    let directory = TempDir::new().unwrap();
    let source = directory.path().join("source.json");
    fs::write(
        &source,
        r#"{"mcpServers":{"fixture":{"command":"fixture"}}}"#,
    )
    .unwrap();
    assert_cmd::cargo::cargo_bin_cmd!("hydian")
        .args(["--home", directory.path().to_str().unwrap(), "init"])
        .assert()
        .success();
    let before = fs::read(directory.path().join("mcp.json")).unwrap();
    assert_cmd::cargo::cargo_bin_cmd!("hydian")
        .args([
            "--home",
            directory.path().to_str().unwrap(),
            "import",
            source.to_str().unwrap(),
            "--dry-run",
        ])
        .assert()
        .success();
    assert_eq!(before, fs::read(directory.path().join("mcp.json")).unwrap());
}
