use domain::*;
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AppMode {
    Observe,
    Signal,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[derive(Default)]
pub enum InstanceMode {
    #[default]
    Active,
    Shadow,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PluginInstance {
    /// Diagnostic output only; does not disable evaluation or notifications.
    #[serde(default = "enabled", skip_serializing_if = "is_true")]
    pub logging: bool,
    pub id: InstanceId,
    pub plugin: PluginId,
    #[serde(default = "enabled")]
    pub enabled: bool,
    #[serde(default)]
    pub mode: InstanceMode,
    #[serde(default)]
    pub config: toml::Table,
}

fn is_true(value: &bool) -> bool {
    *value
}

fn is_zero(value: &u64) -> bool {
    *value == 0
}

fn enabled() -> bool {
    true
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct MonitorConfig {
    pub watchlist_retention_minutes: u64,
    pub poll_interval_secs: u64,
    /// Operational floor for all candidate outcomes, including unavailable data.
    #[serde(skip_serializing_if = "is_zero")]
    pub min_recheck_secs: u64,
    pub candidate_workers: usize,
    pub queue_capacity: usize,
    pub candidate_deadline_secs: u64,
    pub max_candidates: usize,
    pub close_grace_ms: u64,
}

impl Default for MonitorConfig {
    fn default() -> Self {
        Self {
            watchlist_retention_minutes: 30,
            poll_interval_secs: 30,
            min_recheck_secs: 0,
            candidate_workers: 8,
            queue_capacity: 256,
            candidate_deadline_secs: 30,
            max_candidates: 200,
            close_grace_ms: 5000,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AppConfig {
    pub schema_version: u16,
    pub mode: AppMode,
    pub chain: ChainId,
    #[serde(default)]
    pub monitor: MonitorConfig,
    #[serde(default)]
    pub plugins: Vec<PluginInstance>,
    #[serde(default)]
    pub strategies: Vec<PluginInstance>,
    #[serde(default)]
    pub notifications: Vec<PluginInstance>,
}

#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    #[error("cannot read configuration file")]
    Read,
    #[error("invalid TOML configuration")]
    Parse,
    #[error("invalid configuration: {0}")]
    Validation(String),
    #[error("unknown environment override: {0}")]
    UnknownOverride(String),
}

impl AppConfig {
    pub fn parse(input: &str) -> Result<Self, ConfigError> {
        let config: Self = toml::from_str(input).map_err(|_| ConfigError::Parse)?;
        config.validate()?;
        Ok(config)
    }

    pub fn validate(&self) -> Result<(), ConfigError> {
        if self.schema_version != 1 {
            return Err(ConfigError::Validation("unsupported schema_version".into()));
        }

        self.chain
            .validate()
            .map_err(|_| ConfigError::Validation("chain identity".into()))?;
        let m = &self.monitor;
        if !(1..=10080).contains(&m.watchlist_retention_minutes)
            || m.poll_interval_secs == 0
            || m.poll_interval_secs > 86400
            || m.min_recheck_secs > 86400
            || m.candidate_workers == 0
            || m.candidate_workers > 64
            || m.queue_capacity == 0
            || m.queue_capacity > 10000
            || m.candidate_deadline_secs == 0
            || m.candidate_deadline_secs > 300
            || m.max_candidates == 0
            || m.max_candidates > 1000
            || m.close_grace_ms > 60000
        {
            return Err(ConfigError::Validation("monitor resource bounds".into()));
        }

        let mut ids = BTreeSet::new();
        for p in self.instances() {
            if !ids.insert(&p.id) {
                return Err(ConfigError::Validation(format!(
                    "duplicate instance {}",
                    p.id
                )));
            }
        }

        Ok(())
    }

    pub fn instances(&self) -> impl Iterator<Item = &PluginInstance> {
        self.plugins
            .iter()
            .chain(&self.strategies)
            .chain(&self.notifications)
    }

    pub fn digest(&self) -> ContentHash {
        ContentHash::of(self).expect("validated config serializable")
    }
}
