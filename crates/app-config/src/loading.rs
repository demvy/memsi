use crate::{AppConfig, ConfigError, environment, simple};
use std::{collections::BTreeMap, path::Path};

impl AppConfig {
    pub fn load(
        path: &Path,
        environment: &BTreeMap<String, String>,
        cli: &BTreeMap<String, String>,
    ) -> Result<Self, ConfigError> {
        let input = std::fs::read_to_string(path).map_err(|_| ConfigError::Read)?;
        let mut value: toml::Value = toml::from_str(&input).map_err(|_| ConfigError::Parse)?;
        let mut local = environment::dotenv();
        local.retain(|key, _| key.starts_with("MEME__"));
        let compact = value.get("strategies_file").is_some();
        if compact {
            for overrides in [&local, environment, cli] {
                for (key, text) in overrides {
                    match key.as_str() {
                        "MEME__STRATEGIES_FILE" => {
                            value["strategies_file"] = toml::Value::String(text.clone())
                        }
                        "MEME__TELEGRAM" => {
                            let enabled = text.parse().map_err(|_| {
                                ConfigError::Validation("invalid telegram override".into())
                            })?;
                            value
                                .as_table_mut()
                                .ok_or(ConfigError::Parse)?
                                .insert("telegram".into(), toml::Value::Boolean(enabled));
                        }
                        key if key.starts_with("MEME__API_USAGE__") => {
                            let field = key
                                .trim_start_matches("MEME__API_USAGE__")
                                .to_ascii_lowercase();
                            let entry = match field.as_str() {
                                "monthly_limit" | "warn_at" | "reset_day" => {
                                    toml::Value::Integer(text.parse().map_err(|_| {
                                        ConfigError::Validation("invalid API usage override".into())
                                    })?)
                                }
                                "database" | "initial_period" => toml::Value::String(text.clone()),
                                _ => return Err(ConfigError::UnknownOverride(key.into())),
                            };
                            let table = value
                                .as_table_mut()
                                .ok_or(ConfigError::Parse)?
                                .entry("api_usage")
                                .or_insert_with(|| toml::Value::Table(toml::Table::new()));
                            table
                                .as_table_mut()
                                .ok_or(ConfigError::Parse)?
                                .insert(field, entry);
                        }
                        _ => (),
                    }
                }
            }
            value = simple::expand(value, path)?;
        }
        for overrides in [&local, environment, cli] {
            for (key, text) in overrides {
                if compact
                    && (matches!(key.as_str(), "MEME__STRATEGIES_FILE" | "MEME__TELEGRAM")
                        || key.starts_with("MEME__API_USAGE__"))
                {
                    continue;
                }
                let path = key
                    .strip_prefix("MEME__")
                    .ok_or_else(|| ConfigError::UnknownOverride(key.clone()))?
                    .to_ascii_lowercase();
                let allowed = [
                    "mode",
                    "monitor__poll_interval_secs",
                    "monitor__watchlist_retention_minutes",
                    "monitor__min_recheck_secs",
                    "monitor__candidate_workers",
                    "monitor__queue_capacity",
                    "monitor__candidate_deadline_secs",
                    "monitor__max_candidates",
                    "monitor__close_grace_ms",
                ];
                if !allowed.contains(&path.as_str()) {
                    return Err(ConfigError::UnknownOverride(key.clone()));
                }
                if path == "mode" {
                    value
                        .as_table_mut()
                        .ok_or(ConfigError::Parse)?
                        .insert("mode".into(), toml::Value::String(text.clone()));
                } else {
                    let field = path.strip_prefix("monitor__").expect("validated key");
                    let number = text
                        .parse::<i64>()
                        .map_err(|_| ConfigError::Validation(format!("invalid override {key}")))?;
                    let root = value.as_table_mut().ok_or(ConfigError::Parse)?;
                    let table = root
                        .entry("monitor")
                        .or_insert_with(|| toml::Value::Table(toml::Table::new()));
                    table
                        .as_table_mut()
                        .ok_or(ConfigError::Parse)?
                        .insert(field.into(), toml::Value::Integer(number));
                }
            }
        }
        let config: Self = value.try_into().map_err(|_| ConfigError::Parse)?;
        config.validate()?;
        Ok(config)
    }
}
