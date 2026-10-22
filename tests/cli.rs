use std::{
    path::PathBuf,
    process::{Command, Output},
};

fn cli(arguments: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_memes_finder"))
        .args(arguments)
        .env("MEME__TELEGRAM", "false")
        .output()
        .unwrap()
}

fn shipped_config() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("memes-finder.toml")
}

#[test]
fn executable_help_version_and_argument_errors_have_expected_exit_codes() {
    assert!(cli(&["--help"]).status.success());
    assert!(cli(&["--version"]).status.success());
    for arguments in [
        vec!["--error-json", "unknown-command"],
        vec!["--error-json", "--network", "unknown", "config-check"],
        vec!["config-check", "--error-json", "--unknown-option"],
    ] {
        let invalid = cli(&arguments);
        assert!(!invalid.status.success());
        let error: serde_json::Value = serde_json::from_slice(&invalid.stderr).unwrap();
        assert_eq!(error["code"], "invalid_request");
        assert_eq!(error["message"], "invalid command arguments");
    }

    let invalid = cli(&["unknown-command"]);
    assert!(
        String::from_utf8(invalid.stderr)
            .unwrap()
            .contains("unrecognized subcommand")
    );
    assert!(
        !cli(&["--network", "unknown", "config-check"])
            .status
            .success()
    );
}

#[test]
fn offline_commands_validate_configuration_and_emit_public_envelopes() {
    let preset = shipped_config();
    let path = preset.to_str().unwrap();
    for arguments in [
        vec!["plugins", "--envelope"],
        vec!["status", "--config", path],
        vec!["strategies", "--config", path],
        vec!["config-check", "--config", path],
        vec!["--network", "base", "config-check", "--config", path],
    ] {
        let output = cli(&arguments);
        assert!(
            output.status.success(),
            "{arguments:?}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(!output.stdout.is_empty());
    }

    let failure = cli(&[
        "--error-json",
        "config-check",
        "--config",
        "/nonexistent/memsi-config.toml",
    ]);
    assert!(!failure.status.success());
    let error: serde_json::Value = serde_json::from_slice(&failure.stderr).unwrap();
    assert_eq!(error["code"], "command_failed");
    assert_eq!(error["level"], "ERROR");
    assert_eq!(error["message"], "command failed");
    assert!(error["request_id"].as_str().unwrap().starts_with("cli-"));
}

#[test]
fn market_commands_fail_with_redacted_errors_before_network_when_configuration_is_missing() {
    for command in ["list-pools", "evaluate-token", "analyze-token"] {
        let output = cli(&[
            "--error-json",
            command,
            "--token",
            "0x1111111111111111111111111111111111111111",
            "--config",
            "/nonexistent/memsi-config.toml",
        ]);
        assert!(!output.status.success());
        let error = String::from_utf8(output.stderr).unwrap();
        assert!(error.contains("\"code\":\"command_failed\""));
        assert!(!error.contains("/nonexistent/"));
    }

    let output = cli(&[
        "--error-json",
        "monitor",
        "--once",
        "--config",
        "/nonexistent/memsi-config.toml",
    ]);
    assert!(!output.status.success());
    assert!(
        String::from_utf8(output.stderr)
            .unwrap()
            .contains("\"code\":\"command_failed\"")
    );
}

#[test]
fn one_monitor_cycle_reports_missing_credentials_without_network_access() {
    let directory = std::env::temp_dir().join(format!("memsi-offline-cli-{}", std::process::id()));
    std::fs::create_dir(&directory).unwrap();
    let config =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/offline-monitor.toml");
    let output = Command::new(env!("CARGO_BIN_EXE_memes_finder"))
        .args([
            "--error-json",
            "monitor",
            "--once",
            "--config",
            config.to_str().unwrap(),
        ])
        .current_dir(&directory)
        .env_clear()
        .env("MEMSI_TEST_EMPTY_API_KEY", "")
        .envs(std::env::vars_os().filter(|(key, _)| key == "LLVM_PROFILE_FILE"))
        .output()
        .unwrap();
    std::fs::remove_dir_all(&directory).unwrap();
    assert!(
        !output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let diagnostic = String::from_utf8(output.stderr).unwrap();
    let records: Vec<serde_json::Value> = diagnostic
        .lines()
        .map(|line| serde_json::from_str(line).expect("each log line must be valid JSON"))
        .collect();
    let error = records
        .iter()
        .find(|record| record["code"] == "command_failed")
        .unwrap();
    assert_eq!(error["level"], "ERROR");
    assert_eq!(error["message"], "command failed");
}
