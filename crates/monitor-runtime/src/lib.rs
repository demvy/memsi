#![forbid(unsafe_code)]
mod candidate;
mod checkpoint;
mod cycle;
mod diagnostics;
pub use diagnostics::MonitorDiagnostics;
pub mod events;
mod lifecycle;
mod scheduling;
mod status;
pub use status::RuntimeStatus;
const PANIC_LIMIT: u8 = 3;
use analysis_api::Analyzer;
use app_config::{AppConfig, AppMode};
use app_services::Services;
use discovery_api::{CandidateSource, DiscoveryCursor, DiscoveryQuery};
use domain::*;
use event_api::{EventEnvelope, EventPublisher, LifecycleEvent};
use futures::{FutureExt, StreamExt, stream};
use provider_api::{Cancellation, RequestContext};
use signal_service::{
    Clock,
    delivery::{DeliveryPolicy, Sinks},
};

use state_api::{Acceptance, DeliveryTarget, StateError, StateStats, StateStore, WatchRecord};
use std::{
    collections::BTreeMap,
    panic::AssertUnwindSafe,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
    time::{Duration, Instant},
};

use strategy_api::Strategy;
pub struct Dependencies<'a> {
    pub config: &'a AppConfig,
    pub services: &'a Services,
    pub source: &'a dyn CandidateSource,
    pub source_instance: InstanceId,
    pub strategies: &'a BTreeMap<InstanceId, Box<dyn Strategy>>,
    pub analyzers: &'a BTreeMap<InstanceId, Box<dyn Analyzer>>,
    pub state: &'a dyn StateStore,
    pub sinks: &'a Sinks,
    pub targets: &'a [DeliveryTarget],
    pub clock: &'a dyn Clock,
}

pub struct Monitor<'a> {
    dependencies: Dependencies<'a>,
    query: DiscoveryQuery,
    legacy_query_digest: Option<ContentHash>,
    checkpoint_scope: ContentHash,
    restored: bool,
    cursor: Option<DiscoveryCursor>,
    watch: BTreeMap<TokenId, WatchRecord>,
    pending_screening: BTreeMap<TokenId, Candidate>,
    next_discovery: TimestampMs,
    sequence: u64,
    source_panics: u8,
    candidate_panics: BTreeMap<TokenId, u8>,
    events: Arc<events::EventRouter>,
    event_scope: ContentHash,
    event_sequence: u64,
    status: Arc<RuntimeStatus>,
    diagnostics: Option<Arc<dyn MonitorDiagnostics>>,
}

#[derive(serde::Serialize)]
pub struct CycleReport {
    pub discovered: usize,
    pub dropped: usize,
    pub watched: usize,
    pub evaluated: usize,
    pub accepted: usize,
    pub suppressed: usize,
    pub errors: Vec<String>,
    /// Count muted errors for health/status without exposing their diagnostic text.
    #[serde(skip)]
    muted_errors: usize,
    pub worker_panics: usize,
    pub discovery_quarantined: bool,
    pub quarantined_candidates: usize,
    pub event_enqueued: usize,
    pub event_dropped: usize,
    pub state: StateStats,
}

impl Drop for Monitor<'_> {
    fn drop(&mut self) {
        self.status.stop();
        let _ = self.events.close();
    }
}

impl<'a> Monitor<'a> {
    /// Identity of this monitor's persisted scheduler snapshot.
    pub fn checkpoint_scope(&self) -> &ContentHash {
        &self.checkpoint_scope
    }

    pub fn new(dependencies: Dependencies<'a>) -> Result<Self, StateError> {
        dependencies
            .config
            .validate()
            .map_err(|_| StateError::InvalidRequest)?;
        if dependencies.strategies.is_empty()
            || dependencies.strategies.len() > 64
            || !matches!(dependencies.config.mode, AppMode::Observe | AppMode::Signal)
        {
            return Err(StateError::InvalidRequest);
        }

        if dependencies.config.mode == AppMode::Signal && dependencies.targets.is_empty() {
            return Err(StateError::InvalidRequest);
        }

        let thresholds: Vec<_> = dependencies
            .strategies
            .values()
            .map(|strategy| Some(strategy.requirements().pool.min_liquidity_usd))
            .collect();
        let mut query = DiscoveryQuery::union(
            dependencies.config.chain.clone(),
            &thresholds,
            dependencies.config.monitor.max_candidates,
        )
        .map_err(|_| StateError::InvalidRequest)?;
        let filters: Vec<_> = dependencies
            .strategies
            .values()
            .map(|strategy| strategy.discovery_token_filters())
            .collect();
        query.liquidity_filter = discovery_api::union_lower_bound(
            &filters
                .iter()
                .map(|f| f.get(&TokenMetric::TotalLiquidityUsd).cloned())
                .collect::<Vec<_>>(),
        );
        query.market_cap_filter = discovery_api::union_lower_bound(
            &filters
                .iter()
                .map(|f| f.get(&TokenMetric::MarketCapUsd).cloned())
                .collect::<Vec<_>>(),
        );
        if let Some(upper) = discovery_api::union_upper_bound(
            &filters
                .iter()
                .map(|f| f.get(&TokenMetric::FdvUsd).cloned())
                .collect::<Vec<_>>(),
        ) {
            let bounds = query
                .market_cap_filter
                .get_or_insert_with(MetricBounds::default);
            bounds.lt = upper.lt;
            bounds.lte = upper.lte;
        }

        let legacy_query_digest = if query.max_items > 100 {
            let mut legacy = query.clone();
            legacy.max_items = 100;
            Some(legacy.digest())
        } else {
            None
        };

        let checkpoint_scope = ContentHash::of(&(
            1u8,
            dependencies.config.digest(),
            &dependencies.source_instance,
        ))
        .map_err(|_| StateError::InvalidRequest)?;
        static NEXT_RUN: AtomicU64 = AtomicU64::new(0);
        let event_scope = ContentHash::of(&(
            "monitor.events.v1",
            &checkpoint_scope,
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos(),
            NEXT_RUN.fetch_add(1, Ordering::Relaxed),
        ))
        .map_err(|_| StateError::InvalidRequest)?;
        Ok(Self {
            checkpoint_scope,
            restored: false,
            dependencies,
            query,
            legacy_query_digest,
            cursor: None,
            watch: BTreeMap::new(),
            pending_screening: BTreeMap::new(),
            next_discovery: TimestampMs::new(0).unwrap(),
            sequence: 0,
            source_panics: 0,
            candidate_panics: BTreeMap::new(),
            events: Arc::new(events::EventRouter::default()),
            event_scope,
            event_sequence: 0,
            status: Arc::new(RuntimeStatus::default()),
            diagnostics: None,
        })
    }

    pub fn runtime_status(&self) -> Arc<RuntimeStatus> {
        self.status.clone()
    }

    pub fn with_diagnostics(mut self, diagnostics: Arc<dyn MonitorDiagnostics>) -> Self {
        self.diagnostics = Some(diagnostics);
        self
    }

    /// Subscribers own independent bounded receivers; they cannot veto core work.
    pub fn events(&self) -> Arc<events::EventRouter> {
        self.events.clone()
    }

    /// Bind the host's per-run registry before starting producers.
    pub fn with_events(mut self, events: Arc<events::EventRouter>) -> Self {
        self.events = events;
        self
    }

    /// One bounded discovery page and one fair batch of due candidates.
    fn panic_stats(&self, report: &mut CycleReport) {
        report.discovery_quarantined = self.source_panics >= PANIC_LIMIT;
        report.quarantined_candidates = self
            .candidate_panics
            .values()
            .filter(|count| **count >= PANIC_LIMIT)
            .count();
    }

    fn emit(&mut self, payload: LifecycleEvent, at: TimestampMs, report: &mut CycleReport) {
        let Some(sequence) = self.event_sequence.checked_add(1) else {
            return;
        };

        self.event_sequence = sequence;
        if let Ok(event) =
            EventEnvelope::new(at, self.event_scope.as_str().to_owned(), sequence, payload)
        {
            let published = self.events.publish(event);
            report.event_enqueued += published.accepted_subscribers;
            report.event_dropped += published.dropped_subscribers;
        }
    }
}

fn request_context(config: &AppConfig, cancellation: &Cancellation) -> RequestContext {
    RequestContext {
        priority: provider_api::RequestPriority::default(),
        deadline: Instant::now() + Duration::from_secs(config.monitor.candidate_deadline_secs),
        cancellation: cancellation.clone(),
    }
}

#[cfg(test)]
mod tests;
