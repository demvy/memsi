use super::*;
pub(super) use provider::market as fixtures;
use state_api::MonitorRepository;
use std::sync::atomic::{AtomicI64, Ordering};
pub(super) use test_support::provider;
use test_support::strategy_config;

struct TestClock(AtomicI64);
impl Clock for TestClock {
    fn now(&self) -> Result<TimestampMs, StateError> {
        Ok(fixtures::timestamp(self.0.load(Ordering::Relaxed)))
    }
}

struct Source {
    panic: bool,
    error: Option<provider_api::ProviderError>,
    calls: std::sync::atomic::AtomicUsize,
}
#[async_trait::async_trait]
impl CandidateSource for Source {
    async fn poll(
        &self,
        _: &DiscoveryQuery,
        _: Option<&DiscoveryCursor>,
        _: &RequestContext,
    ) -> Result<discovery_api::CandidateBatch, provider_api::ProviderError> {
        self.calls.fetch_add(1, Ordering::Relaxed);
        assert!(!self.panic, "simulated discovery panic");

        if let Some(error) = &self.error {
            return Err(error.clone());
        }

        Ok(discovery_api::CandidateBatch {
            items: vec![fixtures::candidate()],
            next_cursor: None,
            fetched_at: fixtures::timestamp(600_000),
            coverage: provider_api::Coverage {
                complete: true,
                truncated: false,
                warnings: vec![],
            },
        })
    }
}

struct Harness {
    config: AppConfig,
    services: Services,
    source: Source,
    strategies: BTreeMap<InstanceId, Box<dyn Strategy>>,
    analyzers: BTreeMap<InstanceId, Box<dyn Analyzer>>,
    state: state_memory::MemoryState,
    sinks: Sinks,
    clock: TestClock,
}

impl Harness {
    fn new() -> Self {
        let strategy = strategy_range::RangeStrategy::new(
            InstanceId::new("strategy").unwrap(),
            strategy_api::StrategyMode::Active,
            strategy_range::Profile::High,
            strategy_config::config(),
        )
        .unwrap();
        Self {
            config: AppConfig {
                schema_version: 1,
                mode: AppMode::Observe,
                chain: fixtures::chain(),
                monitor: Default::default(),
                plugins: vec![],
                strategies: vec![],
                notifications: vec![],
            },
            services: Services::new(Arc::new(provider::Provider::default())).unwrap(),
            source: Source {
                panic: false,
                error: None,
                calls: Default::default(),
            },
            strategies: [(
                strategy.instance().clone(),
                Box::new(strategy) as Box<dyn Strategy>,
            )]
            .into(),
            analyzers: [
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
            .into(),
            state: state_memory::MemoryState::new(Default::default()).unwrap(),
            sinks: Sinks::new(),
            clock: TestClock(AtomicI64::new(60_000_000)),
        }
    }

    fn dependencies(&self) -> Dependencies<'_> {
        Dependencies {
            config: &self.config,
            services: &self.services,
            source: &self.source,
            source_instance: InstanceId::new("source").unwrap(),
            strategies: &self.strategies,
            analyzers: &self.analyzers,
            state: &self.state,
            sinks: &self.sinks,
            targets: &[],
            clock: &self.clock,
        }
    }
}

#[tokio::test]
async fn monitor_discovers_evaluates_schedules_checkpoints_and_restores_watchlist() {
    let harness = Harness::new();
    let mut monitor = Monitor::new(harness.dependencies()).unwrap();
    let scope = monitor.checkpoint_scope().clone();
    let report = monitor.cycle(&Cancellation::default()).await.unwrap();
    assert_eq!(report.discovered, 1);
    assert_eq!(report.evaluated, 1);
    assert_eq!(report.watched, 1);
    assert_eq!(report.accepted, 0);
    let checkpoint = harness
        .state
        .load_checkpoint(&scope)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(checkpoint.watch.len(), 1);
    assert!(checkpoint.watch[0].next_due > harness.clock.now().unwrap());
    drop(monitor);
    let mut restored = Monitor::new(harness.dependencies()).unwrap();
    let report = restored.cycle(&Cancellation::default()).await.unwrap();
    assert_eq!(report.discovered, 0);
    assert_eq!(report.evaluated, 0);
    assert_eq!(report.watched, 1);
}

#[tokio::test]
async fn monitor_validates_constructor_bounds_and_cancelled_cycles_do_no_work() {
    let mut harness = Harness::new();
    harness.config.mode = AppMode::Signal;
    assert!(Monitor::new(harness.dependencies()).is_err());
    harness.config.mode = AppMode::Observe;
    let mut monitor = Monitor::new(harness.dependencies()).unwrap();
    let cancellation = Cancellation::default();
    cancellation.cancel();
    let report = monitor.cycle(&cancellation).await.unwrap();
    assert_eq!(report.discovered, 0);
    assert_eq!(report.evaluated, 0);
    assert!(monitor.run(&cancellation, |_| {}).await.is_ok());
}

#[tokio::test]
async fn repeated_discovery_panics_quarantine_the_source() {
    let mut harness = Harness::new();
    harness.source.panic = true;
    let mut monitor = Monitor::new(harness.dependencies()).unwrap();
    for attempt in 0..3 {
        harness
            .clock
            .0
            .store(60_000_000 + attempt * 60_000, Ordering::Relaxed);
        let report = monitor.cycle(&Cancellation::default()).await.unwrap();
        assert_eq!(report.worker_panics, 1);
        assert_eq!(report.discovery_quarantined, attempt == 2);
    }

    harness.clock.0.store(61_000_000, Ordering::Relaxed);
    let report = monitor.cycle(&Cancellation::default()).await.unwrap();
    assert_eq!(report.worker_panics, 0);
    assert!(report.discovery_quarantined);
}

#[tokio::test]
async fn continuous_run_stops_when_callback_cancels() {
    let harness = Harness::new();
    let mut monitor = Monitor::new(harness.dependencies()).unwrap();
    let cancellation = Cancellation::default();
    monitor
        .run(&cancellation, |_| cancellation.cancel())
        .await
        .unwrap();
    assert!(cancellation.is_cancelled());
}

#[tokio::test]
async fn candidate_failures_preserve_health_counts_when_diagnostic_logging_is_muted() {
    let mut harness = Harness::new();
    harness.config.strategies.push(app_config::PluginInstance {
        id: InstanceId::new("strategy").unwrap(),
        plugin: PluginId::new("strategy.range-high-risk").unwrap(),
        enabled: true,
        logging: false,
        mode: Default::default(),
        config: Default::default(),
    });
    let mut monitor = Monitor::new(harness.dependencies()).unwrap();
    let cancelled = Cancellation::default();
    cancelled.cancel();
    let mut report = monitor.cycle(&cancelled).await.unwrap();
    monitor
        .process_result(
            fixtures::candidate(),
            Ok(Err(provider_api::ProviderError::Transport)),
            &Cancellation::default(),
            &mut report,
        )
        .await
        .unwrap();
    assert_eq!(report.evaluated, 1);
    assert!(report.errors.is_empty());
    assert_eq!(report.muted_errors, 1);
    assert!(report.has_errors());
}

pub(super) fn report() -> CycleReport {
    CycleReport {
        discovered: 3,
        dropped: 0,
        watched: 2,
        evaluated: 1,
        accepted: 1,
        suppressed: 0,
        errors: vec!["error".into()],
        muted_errors: 2,
        worker_panics: 0,
        discovery_quarantined: false,
        quarantined_candidates: 0,
        state: state_api::StateStats {
            durability: "volatile",
            signals: 0,
            deliveries: 0,
            pending: 0,
            delivered: 0,
            failed: 0,
        },
    }
}

#[tokio::test]
async fn fatal_discovery_errors_stop_after_one_poll() {
    for error in [
        provider_api::ProviderError::Unauthorized,
        provider_api::ProviderError::QuotaExhausted,
    ] {
        let mut harness = Harness::new();
        harness.source.error = Some(error.clone());
        let mut monitor = Monitor::new(harness.dependencies()).unwrap();
        let cancellation = Cancellation::default();
        let result = tokio::time::timeout(
            Duration::from_secs(1),
            monitor.run(&cancellation, |_| panic!("fatal cycle must not succeed")),
        )
        .await
        .unwrap();

        assert!(matches!(result, Err(MonitorError::Provider(actual)) if actual == error));
        assert_eq!(harness.source.calls.load(Ordering::Relaxed), 1);
    }
}

#[tokio::test]
async fn transient_discovery_failure_remains_a_cycle_report_error() {
    let mut harness = Harness::new();
    harness.source.error = Some(provider_api::ProviderError::Transport);
    let mut monitor = Monitor::new(harness.dependencies()).unwrap();
    let report = monitor.cycle(&Cancellation::default()).await.unwrap();
    assert!(report.has_errors());
    assert_eq!(
        report.errors,
        vec![provider_api::ProviderError::Transport.to_string()]
    );
}

#[tokio::test]
async fn fatal_candidate_errors_are_not_reduced_to_report_errors() {
    let harness = Harness::new();
    let mut monitor = Monitor::new(harness.dependencies()).unwrap();
    let cancelled = Cancellation::default();
    cancelled.cancel();
    let mut report = monitor.cycle(&cancelled).await.unwrap();

    for error in [
        provider_api::ProviderError::Unauthorized,
        provider_api::ProviderError::QuotaExhausted,
    ] {
        let result = monitor
            .process_result(
                fixtures::candidate(),
                Ok(Err(error.clone())),
                &Cancellation::default(),
                &mut report,
            )
            .await;
        assert!(matches!(result, Err(MonitorError::Provider(actual)) if actual == error));
    }
}
