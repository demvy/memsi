use domain::*;
use serde::{Deserialize, Serialize};
/// Risk names describe filter strictness, not a safety guarantee.
#[derive(Clone, Copy, Debug, Serialize)]
pub enum Profile {
    Low,
    Medium,
    High,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct StrategyConfig {
    #[serde(default, skip_serializing_if = "std::collections::BTreeMap::is_empty")]
    pub filters: std::collections::BTreeMap<TokenMetric, MetricBounds>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub signal: Option<crate::pump::PumpSignal>,
    #[serde(default = "metrics_age", skip_serializing_if = "default_metrics_age")]
    pub metrics_max_age_secs: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thresholds: Option<ThresholdOverrides>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub research: Option<ResearchPolicy>,
    pub range_analyzer: InstanceId,
    pub volume_analyzer: InstanceId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub trend: Option<TrendBinding>,
    #[serde(default)]
    pub research_only: bool,
    #[serde(default)]
    pub quote_tokens: Vec<TokenId>,
    #[serde(default)]
    pub protocols: Vec<ProtocolId>,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct ThresholdOverrides {
    pub liquidity_usd: Option<f64>,
    pub volume_24h_usd: Option<f64>,
    pub age_secs: Option<u64>,
    pub resolution_secs: Option<u64>,
    pub confidence: Option<f64>,
    pub max_width_pct: Option<f64>,
    pub activity: Option<f64>,
    pub relative_volume: Option<f64>,
}

impl ThresholdOverrides {
    pub fn apply(&self, mut base: Thresholds) -> Thresholds {
        macro_rules! apply {
            ($($field:ident),*) => {
                $(if let Some(value) = self.$field { base.$field = value; })*
            };
        }

        apply!(
            liquidity_usd,
            volume_24h_usd,
            age_secs,
            resolution_secs,
            confidence,
            max_width_pct,
            activity,
            relative_volume
        );
        base
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct ResearchPolicy {
    pub min_position_pct: f64,
    pub max_position_pct: f64,
    pub allowed_chains: Vec<ChainId>,
}

impl Default for ResearchPolicy {
    fn default() -> Self {
        Self {
            min_position_pct: 0.0,
            max_position_pct: 55.0,
            allowed_chains: vec![],
        }
    }
}

impl Thresholds {
    pub fn validate(&self) -> Result<(), strategy_api::StrategyError> {
        if [
            self.liquidity_usd,
            self.volume_24h_usd,
            self.max_width_pct,
            self.relative_volume,
        ]
        .iter()
        .any(|x| !x.is_finite() || *x < 0.0)
            || self.max_width_pct < 2.0
            || !self.confidence.is_finite()
            || !(0.0..=1.0).contains(&self.confidence)
            || !self.activity.is_finite()
            || !(0.0..=1.0).contains(&self.activity)
            || Resolution::new(self.resolution_secs).is_err()
        {
            return Err(strategy_api::StrategyError::InvalidResult);
        }

        Ok(())
    }
}

impl ResearchPolicy {
    pub fn validate(&self) -> Result<(), strategy_api::StrategyError> {
        if !self.min_position_pct.is_finite()
            || !self.max_position_pct.is_finite()
            || self.min_position_pct > self.max_position_pct
            || self.allowed_chains.len() > 8
            || self
                .allowed_chains
                .iter()
                .any(|chain| chain.validate().is_err())
        {
            return Err(strategy_api::StrategyError::InvalidResult);
        }

        Ok(())
    }
}

#[derive(Clone, Copy, Debug, Serialize)]
pub struct Thresholds {
    pub liquidity_usd: f64,
    pub volume_24h_usd: f64,
    pub age_secs: u64,
    pub resolution_secs: u64,
    pub confidence: f64,
    pub max_width_pct: f64,
    pub activity: f64,
    pub relative_volume: f64,
}

impl Profile {
    pub fn plugin_id(self) -> &'static str {
        match self {
            Self::Low => "strategy.range-low-risk",
            Self::Medium => "strategy.range-medium-risk",
            Self::High => "strategy.range-high-risk",
        }
    }

    pub fn thresholds(self) -> Thresholds {
        match self {
            Self::Low => Thresholds {
                liquidity_usd: 300_000.,
                volume_24h_usd: 500_000.,
                age_secs: 43_200,
                resolution_secs: 300,
                confidence: 0.85,
                max_width_pct: 20.,
                activity: 0.8,
                relative_volume: 0.8,
            },
            Self::Medium => Thresholds {
                liquidity_usd: 150_000.,
                volume_24h_usd: 250_000.,
                age_secs: 21_600,
                resolution_secs: 300,
                confidence: 0.80,
                max_width_pct: 30.,
                activity: 0.65,
                relative_volume: 0.65,
            },
            Self::High => Thresholds {
                liquidity_usd: 50_000.,
                volume_24h_usd: 100_000.,
                age_secs: 1_800,
                resolution_secs: 60,
                confidence: 0.72,
                max_width_pct: 40.,
                activity: 0.5,
                relative_volume: 0.5,
            },
        }
    }
}

/// Explicit versioned opt-in; observational by default, with no hidden new filter.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct TrendBinding {
    pub analyzer: InstanceId,
    #[serde(default)]
    pub required: bool,
    #[serde(default)]
    pub reject_downtrend: bool,
}

fn metrics_age() -> u64 {
    60
}

fn default_metrics_age(value: &u64) -> bool {
    *value == 60
}
