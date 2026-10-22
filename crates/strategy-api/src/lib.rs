#![forbid(unsafe_code)]
use analysis_api::{AnalysisBinding, CandleRequirement};
use analysis_model::{RangeAnalysis, TrendAnalysis, VolumeAnalysis};
use domain::*;
use serde::{Deserialize, Serialize};
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Verdict {
    Reject,
    Watch,
    Match,
    Unavailable,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StrategyMode {
    Active,
    Shadow,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DecisionReason {
    pub code: String,
    pub field: String,
    pub actual: Option<String>,
    pub expected: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DecisionEvidence {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub token_metrics: Option<TokenMetrics>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub market: Option<Box<PoolSnapshot>>,
    /// Exact validated candle input used by the engine; absent on legacy/direct evaluations.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub candle_input_hash: Option<ContentHash>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cl_verification: Option<PoolVerificationEvidence>,
    pub snapshot_hash: ContentHash,
    /// Market inputs only, excluding analyzer outputs. Missing on older persisted decisions.
    #[serde(default)]
    pub input_snapshot_hash: Option<ContentHash>,
    pub range: Option<RangeAnalysis>,
    pub volume: Option<VolumeAnalysis>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub trend: Option<TrendAnalysis>,
    pub cl_verified: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct WatchDirective {
    pub after_secs: u64,
    pub priority: u8,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct StrategyDecision {
    pub instance: InstanceId,
    pub plugin: PluginId,
    pub version: String,
    pub config_digest: ContentHash,
    pub mode: StrategyMode,
    pub token: TokenId,
    pub pool: Option<PoolId>,
    pub as_of: TimestampMs,
    pub verdict: Verdict,
    pub reasons: Vec<DecisionReason>,
    pub warnings: Vec<String>,
    pub evidence: DecisionEvidence,
    pub score: Option<UnitInterval>,
    pub watch: WatchDirective,
}

#[derive(Clone, Debug)]
pub enum PrefilterDecision {
    Reject(Vec<DecisionReason>),
    Continue,
}

pub enum MetricScreen {
    Continue,
    Reject(Vec<DecisionReason>),
    Unavailable(Vec<DecisionReason>),
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PoolSelectionPolicy {
    pub concentrated_only: bool,
    pub min_liquidity_usd: NonNegative,
    pub min_pool_age_secs: u64,
    pub quote_tokens: Vec<TokenId>,
    pub max_probes: usize,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct StrategyRequirements {
    pub pool: PoolSelectionPolicy,
    pub candles: CandleRequirement,
    pub analyses: Vec<AnalysisBinding>,
}

pub struct StrategyContext<'a> {
    pub candles: Option<&'a CandleSeries>,
    pub candidate: &'a Candidate,
    pub pool: &'a PoolSnapshot,
    pub range: Option<&'a RangeAnalysis>,
    pub volume: Option<&'a VolumeAnalysis>,
    pub trend: Option<&'a TrendAnalysis>,
    pub snapshot_hash: ContentHash,
    pub input_snapshot_hash: ContentHash,
    pub as_of: TimestampMs,
}

#[derive(Debug, thiserror::Error)]
pub enum StrategyError {
    #[error("missing mandatory input")]
    MissingInput,
    #[error("invalid strategy result")]
    InvalidResult,
}

pub trait Strategy: Send + Sync {
    fn descriptor(&self) -> plugin_api::PluginDescriptor;
    fn instance(&self) -> &InstanceId;
    fn mode(&self) -> StrategyMode;
    fn config_digest(&self) -> &ContentHash;
    fn prefilter(&self, candidate: &Candidate, now: TimestampMs) -> PrefilterDecision;
    fn requirements(&self) -> StrategyRequirements;
    /// Token-scoped filters available for broad discovery pushdown.
    fn discovery_token_filters(&self) -> std::collections::BTreeMap<TokenMetric, MetricBounds> {
        std::collections::BTreeMap::new()
    }

    fn uses_token_metrics(&self) -> bool {
        false
    }

    /// None preserves legacy providers that expose an unscoped metrics snapshot.
    fn required_token_metrics(&self) -> Option<Vec<TokenMetric>> {
        None
    }

    /// Cheap token-only gate, called after metrics acquisition and before pool/candle I/O.
    fn screen_token_metrics(&self, _candidate: &Candidate, _now: TimestampMs) -> MetricScreen {
        MetricScreen::Continue
    }

    /// Rejected token filters need a slower retry than an admitted watchlist entry.
    fn token_filter_recheck_secs(&self) -> u64 {
        60
    }

    /// Closed-candle strategies may wait until this cutoff before doing another live read.
    fn next_market_check(&self, _now: TimestampMs, _close_grace_ms: u64) -> Option<TimestampMs> {
        None
    }

    fn evaluate(&self, context: &StrategyContext<'_>) -> Result<StrategyDecision, StrategyError>;
}

#[cfg(test)]
mod tests {
    use super::*;

    struct MinimalStrategy {
        instance: InstanceId,
        digest: ContentHash,
    }
    impl Strategy for MinimalStrategy {
        fn descriptor(&self) -> plugin_api::PluginDescriptor {
            plugin_api::PluginDescriptor {
                id: PluginId::new("test").unwrap(),
                version: "1".into(),
                kind: plugin_api::PluginKind::Strategy,
                provides: vec![],
            }
        }
        fn instance(&self) -> &InstanceId {
            &self.instance
        }
        fn mode(&self) -> StrategyMode {
            StrategyMode::Active
        }
        fn config_digest(&self) -> &ContentHash {
            &self.digest
        }
        fn prefilter(&self, _: &Candidate, _: TimestampMs) -> PrefilterDecision {
            PrefilterDecision::Continue
        }
        fn requirements(&self) -> StrategyRequirements {
            panic!("defaults should not query requirements")
        }
        fn evaluate(&self, _: &StrategyContext<'_>) -> Result<StrategyDecision, StrategyError> {
            Err(StrategyError::MissingInput)
        }
    }

    #[test]
    fn default_strategy_hooks_preserve_legacy_unscoped_metrics_and_immediate_scheduling() {
        let strategy = MinimalStrategy {
            instance: InstanceId::new("test").unwrap(),
            digest: ContentHash::of_bytes(b"test"),
        };
        assert!(strategy.discovery_token_filters().is_empty());
        assert!(!strategy.uses_token_metrics());
        assert_eq!(strategy.required_token_metrics(), None);
        assert_eq!(strategy.token_filter_recheck_secs(), 60);
        let now = TimestampMs::new(0).unwrap();
        assert_eq!(strategy.next_market_check(now, 5000), None);
        let candidate = Candidate {
            metrics: Default::default(),
            token: TokenId::new(
                ChainId::Evm { chain_id: 1 },
                "0x1111111111111111111111111111111111111111",
            )
            .unwrap(),
            pool_hints: vec![],
            symbol: None,
            discovered_at: now,
            source: ProviderId::new("test").unwrap(),
            liquidity_usd: Knowledge::unknown(),
        };
        assert!(matches!(
            strategy.screen_token_metrics(&candidate, now),
            MetricScreen::Continue
        ));
    }
}
