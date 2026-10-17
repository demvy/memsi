use std::path::Path;

pub(super) fn log(config: &Path) {
    let cwd = std::env::current_dir().ok();
    let config = cwd
        .as_ref()
        .map(|cwd| cwd.join(config))
        .unwrap_or_else(|| config.into());
    let dotenv = cwd.as_ref().map(|cwd| cwd.join(".env"));
    let process_key = std::env::var_os("CODEX_API_KEY");
    let key_source = if process_key.is_some() {
        "environment"
    } else if app_config::local_env("CODEX_API_KEY").is_some() {
        "dotenv"
    } else {
        "missing"
    };

    eprintln!(
        "{}",
        serde_json::json!({
            "event": "monitor_startup",
            "cwd": cwd.map(|p| p.display().to_string()),
            "config": config.display().to_string(),
            "dotenv": dotenv.as_ref().map(|p| p.display().to_string()),
            "dotenv_present": dotenv.is_some_and(|p| p.is_file()),
            "codex_api_key_source": key_source,
            "codex_api_key_empty": app_config::local_env("CODEX_API_KEY").is_some_and(|key| key.is_empty()),
        })
    );
}
