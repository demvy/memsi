//! Compose independent compact strategy definitions over shared screening/data.
use crate::{AppConfig, ConfigError};
use domain::InstanceId;
use serde::Deserialize;
use std::collections::{BTreeMap, BTreeSet};
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct StrategySet {
    #[serde(default = "enabled")]
    logging: bool,
    filter_interval_minutes: u64,
    timeframe_secs: u64,
    #[serde(default = "freshness")]
    metrics_max_age_secs: u64,
    #[serde(default)]
    filters: toml::Table,
    #[serde(default)]
    profiles: toml::Table,
    strategies: Vec<Definition>,
}
fn freshness() -> u64 {
    60
}
fn enabled() -> bool {
    true
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Definition {
    #[serde(default)]
    logging: Option<bool>,
    id: InstanceId,
    kind: String,
    #[serde(default)]
    timeframe_secs: Option<u64>,
    #[serde(default = "enabled")]
    enabled: bool,
    signal: toml::Table,
    #[serde(default)]
    profiles: Option<toml::Table>,
}
pub(crate) fn apply(input: toml::Value, config: &mut AppConfig) -> Result<(), ConfigError> {
    let set: StrategySet = input.try_into().map_err(|_| ConfigError::Parse)?;
    let mut names = BTreeSet::new();
    if set.strategies.is_empty()
        || set.strategies.len() > 32
        || set.strategies.iter().any(|s| !names.insert(s.id.clone()))
    {
        return Err(ConfigError::Validation(
            "unique strategy definitions required (1..32)".into(),
        ));
    }
    let prototype = config.clone();
    config.strategies.clear();
    config
        .plugins
        .retain(|p| !p.plugin.as_str().starts_with("analyzer."));
    for definition in set.strategies.into_iter().filter(|s| s.enabled) {
        let signal = definition.signal;
        let profiles = match definition.profiles {
            None => set.profiles.clone(),
            Some(local) => local
                .into_iter()
                .map(|(name, value)| {
                    let mut profile = set
                        .profiles
                        .get(&name)
                        .and_then(|v| v.as_table())
                        .cloned()
                        .unwrap_or_default();
                    let overrides = value.as_table().ok_or(ConfigError::Parse)?;
                    profile.extend(overrides.clone());
                    Ok((name, toml::Value::Table(profile)))
                })
                .collect::<Result<toml::Table, ConfigError>>()?,
        };
        let fragment = toml::Table::from_iter([
            (
                "logging".into(),
                toml::Value::Boolean(definition.logging.unwrap_or(set.logging)),
            ),
            ("kind".into(), toml::Value::String(definition.kind)),
            (
                "timeframe_secs".into(),
                toml::Value::Integer(definition.timeframe_secs.unwrap_or(set.timeframe_secs) as i64),
            ),
            (
                "filter_interval_minutes".into(),
                toml::Value::Integer(set.filter_interval_minutes as i64),
            ),
            (
                "metrics_max_age_secs".into(),
                toml::Value::Integer(set.metrics_max_age_secs as i64),
            ),
            ("filters".into(), toml::Value::Table(set.filters.clone())),
            ("profiles".into(), toml::Value::Table(profiles)),
            ("signal".into(), toml::Value::Table(signal)),
        ]);
        let mut expanded = prototype.clone();
        crate::screener::apply(toml::Value::Table(fragment), &mut expanded)?;
        let binding_keys = ["range_analyzer", "volume_analyzer", "trend_analyzer"];
        let mut bindings = BTreeMap::new();
        for strategy in &expanded.strategies {
            for key in binding_keys {
                if let Some(id) = strategy.config.get(key).and_then(|v| v.as_str()) {
                    let namespaced =
                        InstanceId::new(format!("{}-{id}", definition.id)).map_err(|_| {
                            ConfigError::Validation("strategy namespace too long".into())
                        })?;
                    bindings.insert(id.to_owned(), namespaced);
                }
            }
        }
        for mut plugin in expanded.plugins {
            if let Some(id) = bindings.get(plugin.id.as_str()) {
                plugin.id = id.clone();
                config.plugins.push(plugin);
            }
        }
        for mut strategy in expanded.strategies {
            let profile = strategy.id.as_str();
            strategy.id =
                InstanceId::new(format!("{}-{profile}", definition.id)).map_err(|_| {
                    ConfigError::Validation("strategy/profile namespace too long".into())
                })?;
            for key in binding_keys {
                if let Some(id) = strategy.config.get(key).and_then(|v| v.as_str()) {
                    let renamed = bindings
                        .get(id)
                        .ok_or(ConfigError::Parse)?
                        .as_str()
                        .to_owned();
                    strategy
                        .config
                        .insert(key.into(), toml::Value::String(renamed));
                }
            }
            config.strategies.push(strategy);
        }
        config.monitor.poll_interval_secs = expanded.monitor.poll_interval_secs;
    }
    if config.strategies.is_empty() {
        return Err(ConfigError::Validation(
            "enable at least one strategy".into(),
        ));
    }
    Ok(())
}
