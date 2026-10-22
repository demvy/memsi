#![forbid(unsafe_code)]
mod candidate;
mod checkpoint;
mod cycle;
mod diagnostics;
pub use diagnostics::MonitorDiagnostics;
mod lifecycle;
mod scheduling;
const PANIC_LIMIT: u8 = 3;
use analysis_api::Analyzer;
use app_config::{AppConfig, AppMode};
use app_services::Services;
use discovery_api::{CandidateSource, DiscoveryCursor, DiscoveryQuery};
use domain::*;
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
    sync::Arc,
    time::{Duration, Instant},
};

use strategy_api::Strategy;
#[derive(Debug, thiserror::Error)]
pub enum MonitorError {
    #[error(transparent)]
    State(#[from] StateError),
    #[error(transparent)]
    Provider(#[from] provider_api::ProviderError),
}

fn fatal_provider_error(error: &provider_api::ProviderError) -> bool {
    matches!(
        error,
        provider_api::ProviderError::Unauthorized | provider_api::ProviderError::QuotaExhausted
    )
}

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
    pub state: StateStats,
}

impl CycleReport {
    pub fn has_errors(&self) -> bool {
        !self.errors.is_empty() || self.muted_errors > 0
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
        if dependencies.strategies.is_empty() || dependencies.strategies.len() > 64 {
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
            diagnostics: None,
        })
    }

    pub fn with_diagnostics(mut self, diagnostics: Arc<dyn MonitorDiagnostics>) -> Self {
        self.diagnostics = Some(diagnostics);
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
