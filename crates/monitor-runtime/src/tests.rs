use super::*;
use event_api::{CyclePhase, RuntimeStatusReader};
use state_api::MonitorRepository;
use std::sync::atomic::AtomicI64;
#[path = "../../../test-support/provider.rs"]
pub(super) mod provider;
pub(super) use provider::market as fixtures;
#[path = "../../../test-support/strategy_config.rs"]
mod strategy_config;

struct TestClock(AtomicI64);
impl Clock for TestClock {
    fn now(&self) -> Result<TimestampMs, StateError> {
        Ok(fixtures::timestamp(self.0.load(Ordering::Relaxed)))
    }
}

struct Source {
    panic: bool,
}
#[async_trait::async_trait]
impl CandidateSource for Source {
    fn descriptor(&self) -> plugin_api::PluginDescriptor {
        plugin_api::PluginDescriptor {
            id: PluginId::new("source.fixture").unwrap(),
            version: "1".into(),
            kind: plugin_api::PluginKind::CandidateSource,
            provides: vec![],
        }
    }
    async fn poll(
        &self,
        _: &DiscoveryQuery,
        _: Option<&DiscoveryCursor>,
        _: &RequestContext,
    ) -> Result<discovery_api::CandidateBatch, provider_api::ProviderError> {
        assert!(!self.panic, "simulated discovery panic");
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
            source: Source { panic: false },
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
    let router = Arc::new(events::EventRouter::default());
    let mut subscription = router
        .subscribe(
            InstanceId::new("test").unwrap(),
            events::SubscriberKind::Telemetry,
            32,
        )
        .unwrap();
    let mut monitor = Monitor::new(harness.dependencies())
        .unwrap()
        .with_events(router);
    let scope = monitor.checkpoint_scope().clone();
    let report = monitor.cycle(&Cancellation::default()).await.unwrap();
    assert_eq!(report.discovered, 1);
    assert_eq!(report.evaluated, 1);
    assert_eq!(report.watched, 1);
    assert_eq!(report.accepted, 0);
    assert!(report.event_enqueued >= 1);
    assert!(subscription.try_recv().is_ok());
    assert_eq!(monitor.events().stats().unwrap().len(), 1);
    assert_eq!(
        monitor
            .runtime_status()
            .runtime_snapshot()
            .unwrap()
            .completed_cycles,
        1
    );
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
    harness.config.mode = AppMode::Live;
    assert!(Monitor::new(harness.dependencies()).is_err());
    harness.config.mode = AppMode::Signal;
    assert!(Monitor::new(harness.dependencies()).is_err());
    harness.config.mode = AppMode::Observe;
    let mut monitor = Monitor::new(harness.dependencies()).unwrap();
    let cancellation = Cancellation::default();
    cancellation.cancel();
    let report = monitor.cycle(&cancellation).await.unwrap();
    assert_eq!(report.discovered, 0);
    assert_eq!(report.evaluated, 0);
    assert!(monitor.run_cycles(&cancellation, 0, |_| {}).await.is_err());
    assert!(
        monitor
            .run_cycles(&cancellation, 101, |_| {})
            .await
            .is_err()
    );
    assert_eq!(
        monitor.run_cycles(&cancellation, 1, |_| {}).await.unwrap(),
        0
    );
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
async fn bounded_and_continuous_runs_stop_when_callback_cancels() {
    let harness = Harness::new();
    let mut monitor = Monitor::new(harness.dependencies()).unwrap();
    let cancellation = Cancellation::default();
    let count = monitor
        .run_cycles(&cancellation, 2, |_| cancellation.cancel())
        .await
        .unwrap();
    assert_eq!(count, 1);
    assert!(matches!(
        monitor.runtime_status().runtime_snapshot().unwrap().phase,
        CyclePhase::Stopped
    ));
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
        event_enqueued: 0,
        event_dropped: 0,
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
