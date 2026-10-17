//! Complete decision fixture for signal and delivery tests.
#![allow(dead_code)]
use analysis_model::*;
use domain::*;
use strategy_api::*;

#[path = "market.rs"]
pub mod market;

pub fn range() -> RangeAnalysis {
    RangeAnalysis {
        state: RangeState::Confirmed,
        range: Some(RangeBounds {
            lower: PositivePrice::new(1.0).unwrap(),
            upper: PositivePrice::new(2.0).unwrap(),
            center: PositivePrice::new(1.5).unwrap(),
            width_pct: NonNegative::new(20.0).unwrap(),
            age_bars: 300,
            latest_position: 0.5,
            position: Some(Box::new(RangePosition::new(1.0, 2.0, 1.5).unwrap())),
            latest_inside: true,
            latest_inside_bounds: true,
            outside_streak: 0,
        }),
        confidence: UnitInterval::new(0.95).unwrap(),
        price_unit: PriceUnit::UsdPerToken,
    }
}

pub fn volume() -> VolumeAnalysis {
    VolumeAnalysis {
        robust: None,
        recent_mean: NonNegative::new(2.0).unwrap(),
        baseline_mean: NonNegative::new(1.0).unwrap(),
        relative_volume: Some(NonNegative::new(2.0).unwrap()),
        activity_fraction: UnitInterval::new(1.0).unwrap(),
        last_volume_percentile: UnitInterval::new(0.8).unwrap(),
        unit: VolumeUnit::Usd,
        recent_bars: 12,
        baseline_bars: 48,
        state: ActivityState::Elevated,
    }
}

pub fn decision() -> StrategyDecision {
    StrategyDecision {
        instance: InstanceId::new("strategy").unwrap(),
        plugin: PluginId::new("strategy.range-high-risk").unwrap(),
        version: "1".into(),
        config_digest: ContentHash::of_bytes(b"config"),
        mode: StrategyMode::Active,
        token: market::token(1),
        pool: Some(market::pool_id(10)),
        as_of: market::timestamp(600_000),
        verdict: Verdict::Match,
        reasons: vec![],
        warnings: vec!["ResearchOnly".into()],
        evidence: DecisionEvidence {
            token_metrics: None,
            market: Some(Box::new(market::pool(10))),
            candle_input_hash: Some(ContentHash::of_bytes(b"candles")),
            cl_verification: None,
            snapshot_hash: ContentHash::of_bytes(b"snapshot"),
            input_snapshot_hash: Some(ContentHash::of_bytes(b"input")),
            range: Some(range()),
            volume: Some(volume()),
            trend: None,
            cl_verified: false,
        },
        score: Some(UnitInterval::new(0.95).unwrap()),
        watch: WatchDirective {
            after_secs: 60,
            priority: 1,
        },
    }
}
