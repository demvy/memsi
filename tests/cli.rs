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
    let invalid = cli(&["--error-json", "unknown-command"]);
    assert!(!invalid.status.success());
    let stderr = String::from_utf8(invalid.stderr).unwrap();
    assert!(stderr.contains("\"code\":\"invalid_request\""));
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
    assert!(
        String::from_utf8(failure.stderr)
            .unwrap()
            .contains("\"code\":\"command_failed\"")
    );
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
        .args(["monitor", "--once", "--config", config.to_str().unwrap()])
        .current_dir(&directory)
        .env_clear()
        .env("MEMSI_TEST_EMPTY_API_KEY", "")
        .envs(std::env::vars_os().filter(|(key, _)| key == "LLVM_PROFILE_FILE"))
        .output()
        .unwrap();
    std::fs::remove_dir_all(&directory).unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let diagnostic = String::from_utf8(output.stderr).unwrap();
    assert!(diagnostic.contains("\"discovered\":0"));
    assert!(diagnostic.contains("unauthorized"));
    assert!(diagnostic.contains("\"http_attempts_total\":0"));
}
