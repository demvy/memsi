//! Minimal research-only strategy config; no providers, runtime, or secrets.
use domain::InstanceId;
use strategy_range::StrategyConfig;

pub fn config() -> StrategyConfig {
    StrategyConfig {
        filters: Default::default(),
        signal: None,
        metrics_max_age_secs: 60,
        thresholds: None,
        research: None,
        range_analyzer: InstanceId::new("range").unwrap(),
        volume_analyzer: InstanceId::new("volume").unwrap(),
        trend: None,
        research_only: true,
        quote_tokens: vec![],
        protocols: vec![],
    }
}
