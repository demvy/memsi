use super::*;
pub(super) use provider::market as fixtures;
use std::{sync::atomic::Ordering, time::Instant};
use strategy_api::{Strategy, StrategyMode};
pub(super) use test_support::provider;
use test_support::strategy_config;

fn context() -> RequestContext {
    RequestContext {
        priority: RequestPriority::default(),
        deadline: Instant::now() + Duration::from_secs(60),
        cancellation: Cancellation::default(),
    }
}

fn strategies(metrics: bool) -> BTreeMap<InstanceId, Box<dyn Strategy>> {
    let mut config = strategy_config::config();
    if metrics {
        config.filters.insert(
            TokenMetric::MarketCapUsd,
            MetricBounds {
                gt: Some(100.0),
                ..Default::default()
            },
        );
    }
    let strategy = strategy_range::RangeStrategy::new(
        InstanceId::new("strategy").unwrap(),
        StrategyMode::Active,
        strategy_range::Profile::High,
        config,
    )
    .unwrap();
    [(
        strategy.instance().clone(),
        Box::new(strategy) as Box<dyn Strategy>,
    )]
    .into()
}

#[tokio::test]
async fn services_validate_pool_membership_and_bound_provider_panics() {
    let provider = Arc::new(provider::Provider::default());
    let services = Services::new(provider.clone()).unwrap();
    assert_eq!(
        services
            .list_pools(&fixtures::token(1), &context())
            .await
            .unwrap()
            .pools
            .len(),
        1
    );
    assert_eq!(provider.pool_calls.load(Ordering::Relaxed), 1);
    assert!(
        services
            .list_pools(&fixtures::token(99), &context())
            .await
            .is_err()
    );
    assert_eq!(services.stats().upstream_attempts, 0);
    let services = Services::new(Arc::new(provider::Provider {
        panic: true,
        ..Default::default()
    }))
    .unwrap();
    assert_eq!(
        services
            .list_pools(&fixtures::token(1), &context())
            .await
            .unwrap_err(),
        ProviderError::WorkerPanicked
    );
}

#[tokio::test]
async fn bounded_reads_preserve_results_and_handle_panics_timeout_and_cancellation() {
    assert_eq!(
        read_bounded(async { Ok(42) }, &context()).await.unwrap(),
        42
    );
    assert_eq!(
        read_bounded(async { Err::<(), _>(ProviderError::NotFound) }, &context()).await,
        Err(ProviderError::NotFound)
    );
    assert_eq!(
        read_bounded(
            async {
                panic!("simulated read panic");
                #[allow(unreachable_code)]
                Ok::<(), ProviderError>(())
            },
            &context()
        )
        .await,
        Err(ProviderError::WorkerPanicked)
    );
    let mut expired = context();
    expired.deadline = Instant::now();
    assert_eq!(
        read_bounded(async { Ok(()) }, &expired).await,
        Err(ProviderError::Timeout)
    );
    let cancelled = context();
    cancelled.cancellation.cancel();
    assert_eq!(
        read_bounded(async { Ok(()) }, &cancelled).await,
        Err(ProviderError::Cancelled)
    );
}

#[tokio::test]
async fn metrics_prefetch_skips_unused_fields_deduplicates_tokens_and_validates_bounds() {
    let provider = Arc::new(provider::Provider::default());
    let services = Services::new(provider.clone()).unwrap();
    let candidates = [fixtures::candidate(), fixtures::candidate()];
    assert!(
        services
            .prefetch_token_metrics(&candidates, &strategies(false), &context())
            .await
            .unwrap()
            .is_empty()
    );
    assert_eq!(provider.metrics_calls.load(Ordering::Relaxed), 0);
    let result = services
        .prefetch_token_metrics(&candidates, &strategies(true), &context())
        .await
        .unwrap();
    assert_eq!(result.len(), 1);
    assert_eq!(provider.metrics_calls.load(Ordering::Relaxed), 1);
    assert!(
        services
            .prefetch_token_metrics(&candidates, &BTreeMap::new(), &context())
            .await
            .is_err()
    );
    assert!(
        services
            .prefetch_token_metrics(
                &vec![fixtures::candidate(); 101],
                &strategies(true),
                &context()
            )
            .await
            .is_err()
    );
    assert_eq!(metrics::selected_fields(&strategies(false)), Some(vec![]));
    assert_eq!(
        metrics::selected_fields(&strategies(true)),
        Some(vec![
            TokenMetric::MarketCapUsd,
            TokenMetric::ProviderSeenAgeSecs
        ])
    );
}

#[tokio::test]
async fn strategy_evaluation_rejects_invalid_bounds_and_screens_before_pool_io() {
    let provider = Arc::new(provider::Provider::default());
    let services = Services::new(provider.clone()).unwrap();
    let token = fixtures::token(1);
    assert!(
        services
            .evaluate_strategies(&token, &BTreeMap::new(), &BTreeMap::new(), 0, &context())
            .await
            .is_err()
    );
    assert!(
        services
            .evaluate_strategies(
                &token,
                &strategies(false),
                &BTreeMap::new(),
                60001,
                &context()
            )
            .await
            .is_err()
    );
    let result = services
        .evaluate_with_metrics(
            &token,
            &strategies(true),
            &BTreeMap::new(),
            0,
            Some(&TokenMetrics::default()),
            &context(),
        )
        .await
        .unwrap();
    assert_eq!(result.strategies.len(), 1);
    assert!(result.strategies[0].unavailable.is_some());
    assert_eq!(provider.pool_calls.load(Ordering::Relaxed), 0);
    assert_eq!(provider.metrics_calls.load(Ordering::Relaxed), 0);
    let result = services
        .evaluate_strategies(&token, &strategies(false), &BTreeMap::new(), 0, &context())
        .await
        .unwrap();
    assert_eq!(result.strategies.len(), 1);
    assert!(result.strategies[0].unavailable.is_some());
    assert_eq!(provider.pool_calls.load(Ordering::Relaxed), 1);
}

#[tokio::test]
async fn manual_analysis_runs_real_analyzer_for_discovered_or_explicit_pool() {
    let services = Services::new(Arc::new(provider::Provider::default())).unwrap();
    let analyzer = analyzer_volume::VolumeAnalyzer::new(
        InstanceId::new("volume").unwrap(),
        Default::default(),
    )
    .unwrap();
    let analyzers: BTreeMap<_, Box<dyn Analyzer>> = [(
        analyzer.instance().clone(),
        Box::new(analyzer) as Box<dyn Analyzer>,
    )]
    .into();
    let mut request = AnalyzeRequest {
        token: fixtures::token(1),
        pool: None,
        resolution: Resolution::new(60).unwrap(),
        analyzer_instances: vec![],
        policy: SelectionPolicy::default(),
        close_grace_ms: 0,
    };
    let report = services
        .analyze(&request, &analyzers, &context())
        .await
        .unwrap();
    assert_eq!(report.analyzers.len(), 1);
    assert!(report.analyzers[0].result.is_some());
    assert!(report.analyzers[0].input_hash.is_some());
    request.pool = Some(fixtures::pool_id(10));
    let report = services
        .analyze(&request, &analyzers, &context())
        .await
        .unwrap();
    assert!(!report.coverage.complete);
    assert!(report.analyzers[0].result.is_some());
    request.analyzer_instances = vec![InstanceId::new("unknown").unwrap()];
    assert!(
        services
            .analyze(&request, &analyzers, &context())
            .await
            .is_err()
    );
    request.analyzer_instances.clear();
    request.close_grace_ms = 60001;
    assert!(
        services
            .analyze(&request, &analyzers, &context())
            .await
            .is_err()
    );
    request.close_grace_ms = 0;
    assert!(
        services
            .analyze(&request, &BTreeMap::new(), &context())
            .await
            .is_err()
    );
}
