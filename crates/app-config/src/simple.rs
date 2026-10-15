//! Compact Codex-only research configuration, expanded to the existing contracts.
use crate::{ApiUsageConfig, AppConfig, ConfigError, PluginInstance, local_env};
use serde::Deserialize;
use std::path::Path;
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Program {
    #[serde(default)]
    watchlist_retention_minutes: Option<u64>,
    strategies_file: String,
    #[serde(default)]
    telegram: bool,
    #[serde(default)]
    monitor: toml::Table,
    #[serde(default)]
    api_usage: ApiUsageConfig,
    #[serde(default)]
    fallback_api_keys: Vec<crate::SecretRef>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Profiles {
    low: Profile,
    high: Profile,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Profile {
    liquidity_usd: f64,
    volume_24h_usd: f64,
    age_secs: u64,
    confidence: f64,
    max_width_pct: f64,
    relative_volume: f64,
    max_position_pct: f64,
}
impl Profile {
    fn apply(self, strategy: &mut PluginInstance) -> Result<(), ConfigError> {
        let thresholds = strategy
            .config
            .get_mut("thresholds")
            .and_then(toml::Value::as_table_mut)
            .ok_or(ConfigError::Parse)?;
        for (name, value) in [
            ("liquidity_usd", self.liquidity_usd),
            ("volume_24h_usd", self.volume_24h_usd),
            ("confidence", self.confidence),
            ("max_width_pct", self.max_width_pct),
            ("relative_volume", self.relative_volume),
        ] {
            thresholds.insert(name.into(), toml::Value::Float(value));
        }
        thresholds.insert(
            "age_secs".into(),
            toml::Value::Integer(i64::try_from(self.age_secs).map_err(|_| ConfigError::Parse)?),
        );
        let policy = strategy
            .config
            .get_mut("research")
            .and_then(toml::Value::as_table_mut)
            .ok_or(ConfigError::Parse)?;
        policy.insert(
            "max_position_pct".into(),
            toml::Value::Float(self.max_position_pct),
        );
        Ok(())
    }
}
pub(crate) fn expand(input: toml::Value, path: &Path) -> Result<toml::Value, ConfigError> {
    let program: Program = input.try_into().map_err(|_| ConfigError::Parse)?;
    let strategies = path
        .parent()
        .unwrap_or(Path::new("."))
        .join(program.strategies_file);
    let strategy_input: toml::Value =
        toml::from_str(&std::fs::read_to_string(strategies).map_err(|_| ConfigError::Read)?)
            .map_err(|_| ConfigError::Parse)?;
    program.api_usage.validate()?;
    let mut config = AppConfig::parse(include_str!("research-preset.toml"))?;
    config.monitor.poll_interval_secs = 60;
    config.monitor.min_recheck_secs = 0;
    config.monitor.queue_capacity = 512;
    config.monitor.max_candidates = 100;
    let provider = config
        .plugins
        .iter_mut()
        .find(|plugin| plugin.plugin.as_str() == "provider.codex")
        .ok_or(ConfigError::Parse)?;
    provider.config.insert(
        "usage".into(),
        toml::Value::try_from(program.api_usage).map_err(|_| ConfigError::Parse)?,
    );
    provider.config.insert(
        "fallback_api_keys".into(),
        toml::Value::try_from(program.fallback_api_keys).map_err(|_| ConfigError::Parse)?,
    );
    if strategy_input.get("strategies").is_some() {
        crate::strategy_set::apply(strategy_input, &mut config)?;
    } else if strategy_input.get("profiles").is_some() {
        crate::screener::apply(strategy_input, &mut config)?;
    } else {
        let profiles: Profiles = strategy_input.try_into().map_err(|_| ConfigError::Parse)?;
        for (id, profile) in [("low", profiles.low), ("high", profiles.high)] {
            let strategy = config
                .strategies
                .iter_mut()
                .find(|strategy| strategy.id.as_str() == id)
                .ok_or(ConfigError::Parse)?;
            profile.apply(strategy)?;
        }
    }
    let mut monitor = toml::Value::try_from(&config.monitor).map_err(|_| ConfigError::Parse)?;
    monitor
        .as_table_mut()
        .ok_or(ConfigError::Parse)?
        .extend(program.monitor);
    config.monitor = monitor.try_into().map_err(|_| ConfigError::Parse)?;
    if let Some(minutes) = program.watchlist_retention_minutes {
        config.monitor.watchlist_retention_minutes = minutes;
    }
    if program.telegram {
        let chat = local_env("TELEGRAM_CHAT_ID")
            .ok_or_else(|| ConfigError::Validation("missing TELEGRAM_CHAT_ID".into()))?;
        let mut notification = config.notifications[0].clone();
        notification.id = domain::InstanceId::new("telegram").map_err(|_| ConfigError::Parse)?;
        notification.plugin =
            domain::PluginId::new("notification.telegram").map_err(|_| ConfigError::Parse)?;
        notification.config = toml::Table::from_iter([
            (
                "token".into(),
                toml::Value::String("env:TELEGRAM_BOT_TOKEN".into()),
            ),
            ("chat_id".into(), toml::Value::String(chat)),
        ]);
        config.notifications.push(notification);
    }
    toml::Value::try_from(config).map_err(|_| ConfigError::Parse)
}
