use super::*;
use analysis_api::Analyzer;
use domain::*;
pub(super) use provider::market as fixtures;
use provider_api::*;
use std::{
    collections::BTreeMap,
    sync::Arc,
    time::{Duration, Instant},
};
use strategy_api::*;
pub(super) use test_support::provider;
use test_support::strategy_config;

fn strategies() -> BTreeMap<InstanceId, Box<dyn Strategy>> {
    [(
        InstanceId::new("strategy").unwrap(),
        Box::new(
            strategy_range::RangeStrategy::new(
                InstanceId::new("strategy").unwrap(),
                StrategyMode::Active,
                strategy_range::Profile::High,
                strategy_config::config(),
            )
            .unwrap(),
        ) as Box<dyn Strategy>,
    )]
    .into()
}

fn analyzers() -> BTreeMap<InstanceId, Box<dyn Analyzer>> {
    [
        (
            InstanceId::new("range").unwrap(),
            Box::new(
                analyzer_range::RangeAnalyzer::new(
                    InstanceId::new("range").unwrap(),
                    Default::default(),
                )
                .unwrap(),
            ) as Box<dyn Analyzer>,
        ),
        (
            InstanceId::new("volume").unwrap(),
            Box::new(
                analyzer_volume::VolumeAnalyzer::new(
                    InstanceId::new("volume").unwrap(),
                    Default::default(),
                )
                .unwrap(),
            ) as Box<dyn Analyzer>,
        ),
    ]
    .into()
}

fn context() -> RequestContext {
    RequestContext {
        priority: RequestPriority::default(),
        deadline: Instant::now() + Duration::from_secs(60),
        cancellation: Cancellation::default(),
    }
}

#[test]
fn planner_combines_requirements_and_rejects_missing_analyzers() {
    let strategies = strategies();
    let mut analyzers = analyzers();
    let strategy = strategies.values().next().unwrap().as_ref();
    let plan = planner::plan(strategy, &analyzers).unwrap();
    assert_eq!(plan.range.instance().as_str(), "range");
    assert_eq!(plan.volume.instance().as_str(), "volume");
    assert!(plan.trend.is_none());
    assert!(plan.candles.require_volume);
    assert!(plan.candles.desired_bars >= plan.candles.min_bars);
    let windows = planner::union_windows(&[plan.clone(), plan]);
    assert_eq!(windows.len(), 1);
    assert_eq!(windows[&Resolution::new(60).unwrap()], 300);
    analyzers.remove(&InstanceId::new("range").unwrap());
    assert!(matches!(
        planner::plan(strategy, &analyzers),
        Err(planner::PlanError::Analyzer(_))
    ));
}

#[tokio::test]
async fn engine_evaluates_real_analyzers_and_strategy_with_validated_evidence() {
    let provider = Arc::new(provider::Provider::default());
    let hub = data_hub::DataHub::new(
        provider.clone(),
        4,
        Duration::from_secs(60),
        Duration::from_secs(10),
    )
    .unwrap();
    let strategies = strategies();
    let analyzers = analyzers();
    let candidate = fixtures::candidate();
    let pools = provider
        .pools_for_token(&candidate.token, &context())
        .await
        .unwrap();
    let evaluation = Evaluation {
        candidate: &candidate,
        pools: &pools,
        as_of: fixtures::timestamp(60_000_000),
        close_grace_ms: 0,
    };
    let reports = evaluate(&evaluation, &strategies, &analyzers, &hub, &context()).await;
    assert_eq!(reports.len(), 1);
    let report = &reports[0];
    assert!(report.unavailable.is_none(), "{:?}", report.unavailable);
    let decision = report.decision.as_ref().unwrap();
    assert!(decision.evidence.candle_input_hash.is_some());
    assert!(decision.evidence.range.is_some());
    assert!(decision.evidence.volume.is_some());
    assert_eq!(decision.token, candidate.token);
    assert_eq!(decision.pool, Some(fixtures::pool_id(10)));
}

#[tokio::test]
async fn engine_reports_cancelled_context_and_no_usable_pools() {
    let hub = data_hub::DataHub::new(
        Arc::new(provider::Provider::default()),
        4,
        Duration::from_secs(60),
        Duration::from_secs(10),
    )
    .unwrap();
    let candidate = fixtures::candidate();
    let pools = PoolDiscoveryResult {
        pools: vec![],
        coverage: Coverage {
            complete: true,
            truncated: false,
            warnings: vec![],
        },
    };
    let evaluation = Evaluation {
        candidate: &candidate,
        pools: &pools,
        as_of: fixtures::timestamp(60_000_000),
        close_grace_ms: 0,
    };
    let reports = evaluate(&evaluation, &strategies(), &analyzers(), &hub, &context()).await;
    assert!(
        reports[0]
            .unavailable
            .as_ref()
            .unwrap()
            .contains("no usable pool")
    );
    let cancelled = context();
    cancelled.cancellation.cancel();
    let reports = evaluate(&evaluation, &strategies(), &analyzers(), &hub, &cancelled).await;
    assert_eq!(reports[0].unavailable.as_deref(), Some("cancelled"));
}

#[test]
fn token_metric_screening_distinguishes_missing_failed_and_passing_filters() {
    let mut config = strategy_config::config();
    config.filters.insert(
        TokenMetric::MarketCapUsd,
        MetricBounds {
            gt: Some(100.0),
            ..Default::default()
        },
    );
    let strategy = strategy_range::RangeStrategy::new(
        InstanceId::new("filtered").unwrap(),
        StrategyMode::Active,
        strategy_range::Profile::High,
        config,
    )
    .unwrap();
    let mut candidate = fixtures::candidate();
    let now = fixtures::timestamp(600_000);
    assert!(
        screen_token_metrics(&strategy, &candidate, now)
            .unwrap()
            .unavailable
            .is_some()
    );
    candidate
        .metrics
        .insert(TokenMetric::MarketCapUsd, fixtures::observation(50.0));
    assert!(
        screen_token_metrics(&strategy, &candidate, now)
            .unwrap()
            .prefilter_rejection
            .is_some()
    );
    candidate
        .metrics
        .insert(TokenMetric::MarketCapUsd, fixtures::observation(500.0));
    assert!(screen_token_metrics(&strategy, &candidate, now).is_none());
}

#[test]
fn unavailable_reports_retain_probe_reasons_without_a_decision() {
    let instance = InstanceId::new("strategy").unwrap();
    let mut report = StrategyReport::unavailable(&instance, "no usable pool");
    report.record(&fixtures::pool_id(10), "provider unavailable");
    assert_eq!(report.trace.len(), 1);
    assert_eq!(report.trace[0].pool, fixtures::pool_id(10));
    assert_eq!(report.trace[0].reason, "provider unavailable");
    assert!(report.decision.is_none());
}
