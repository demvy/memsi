//! Compact profiles for token screening and closed-candle pump signals.
use crate::{AppConfig, ConfigError};
use domain::{MetricBounds, TokenMetric};
use serde::Deserialize;
use std::collections::BTreeMap;
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Screener {
    #[serde(default = "logging_default")]
    logging: bool,
    kind: String,
    #[serde(default = "filter_interval")]
    filter_interval_minutes: u64,
    #[serde(default = "timeframe")]
    timeframe_secs: u64,
    #[serde(default = "freshness")]
    metrics_max_age_secs: u64,
    signal: toml::Table,
    /// Common mandatory AND filters, inherited by every profile.
    #[serde(default)]
    filters: BTreeMap<TokenMetric, MetricBounds>,
    profiles: BTreeMap<String, Profile>,
}

fn logging_default() -> bool {
    true
}

fn filter_interval() -> u64 {
    3
}

fn timeframe() -> u64 {
    300
}

fn freshness() -> u64 {
    60
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Profile {
    #[serde(default)]
    logging: Option<bool>,
    filters: BTreeMap<TokenMetric, MetricBounds>,
    #[serde(default)]
    signal: Option<toml::Table>,
}

pub(crate) fn apply(input: toml::Value, config: &mut AppConfig) -> Result<(), ConfigError> {
    let input: Screener = input.try_into().map_err(|_| ConfigError::Parse)?;
    if input.kind != "pump_high"
        || !(1..=1440).contains(&input.filter_interval_minutes)
        || input.profiles.is_empty()
        || input.profiles.len() > 64
        || input.timeframe_secs != 60
        || !(1..=86400).contains(&input.metrics_max_age_secs)
    {
        return Err(ConfigError::Validation(
            "screener kind/profiles/timeframe/freshness".into(),
        ));
    }

    config.monitor.poll_interval_secs = input.filter_interval_minutes * 60;
    let template = config
        .strategies
        .iter()
        .find(|s| s.id.as_str() == "high")
        .ok_or(ConfigError::Parse)?
        .clone();
    config.strategies.clear();
    for (name, profile) in input.profiles {
        let mut filters = input.filters.clone();
        for (metric, bounds) in profile.filters {
            if filters.insert(metric, bounds).is_some() {
                return Err(ConfigError::Validation(
                    "duplicate common/profile token filter".into(),
                ));
            }
        }

        for (metric, bounds) in &filters {
            bounds
                .validate(*metric)
                .map_err(|_| ConfigError::Validation("invalid token filter bounds".into()))?;
        }

        let mut strategy = template.clone();
        strategy.logging = profile.logging.unwrap_or(input.logging);
        strategy.id = domain::InstanceId::new(name)
            .map_err(|_| ConfigError::Validation("invalid profile name".into()))?;
        strategy.config.remove("research");
        strategy.config.insert(
            "filters".into(),
            toml::Value::try_from(filters).map_err(|_| ConfigError::Parse)?,
        );
        let mut signal = input.signal.clone();
        if let Some(overrides) = profile.signal {
            signal.extend(overrides);
        }

        strategy
            .config
            .insert("signal".into(), toml::Value::Table(signal));
        strategy.config.insert(
            "metrics_max_age_secs".into(),
            toml::Value::Integer(input.metrics_max_age_secs as i64),
        );
        strategy.config.insert(
            "thresholds".into(),
            toml::Value::Table(toml::Table::from_iter([
                ("liquidity_usd".into(), toml::Value::Float(0.0)),
                ("age_secs".into(), toml::Value::Integer(0)),
                ("volume_24h_usd".into(), toml::Value::Float(0.0)),
                (
                    "resolution_secs".into(),
                    toml::Value::Integer(input.timeframe_secs as i64),
                ),
            ])),
        );
        config.strategies.push(strategy);
    }

    let range = config
        .plugins
        .iter_mut()
        .find(|p| p.id.as_str() == "range-low")
        .ok_or(ConfigError::Parse)?;
    range.config.insert(
        "windows".into(),
        toml::Value::Array(vec![toml::Value::Integer(20)]),
    );
    range
        .config
        .insert("history_bars".into(), toml::Value::Integer(20));
    range
        .config
        .insert("min_bars".into(), toml::Value::Integer(20));
    let volume = config
        .plugins
        .iter_mut()
        .find(|p| p.id.as_str() == "volume")
        .ok_or(ConfigError::Parse)?;
    volume.config.clear();
    volume
        .config
        .insert("baseline_bars".into(), toml::Value::Integer(4));
    volume
        .config
        .insert("recent_bars".into(), toml::Value::Integer(3));
    Ok(())
}
