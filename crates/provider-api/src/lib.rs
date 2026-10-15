#![forbid(unsafe_code)]
use async_trait::async_trait;
use domain::*;
use serde::{Deserialize, Serialize};
use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Instant,
};
#[derive(Clone, Default)]
pub struct Cancellation(Arc<AtomicBool>);
impl Cancellation {
    pub fn cancel(&self) {
        self.0.store(true, Ordering::Release)
    }
    pub fn is_cancelled(&self) -> bool {
        self.0.load(Ordering::Acquire)
    }
}
/// Shared monotonic admission priority. Promotion never cancels in-flight work.
#[derive(Clone)]
pub struct RequestPriority(Arc<AtomicBool>);
impl Default for RequestPriority {
    fn default() -> Self {
        Self(Arc::new(AtomicBool::new(true)))
    }
}
impl RequestPriority {
    pub fn shadow() -> Self {
        Self(Arc::new(AtomicBool::new(false)))
    }
    pub fn is_active(&self) -> bool {
        self.0.load(Ordering::Acquire)
    }
    pub fn promote_active(&self) {
        self.0.store(true, Ordering::Release);
    }
    /// A new request's promotion must not change other requests in its parent phase.
    pub fn independent(&self) -> Self {
        Self(Arc::new(AtomicBool::new(self.is_active())))
    }
}
#[derive(Clone)]
pub struct RequestContext {
    pub priority: RequestPriority,
    pub deadline: Instant,
    pub cancellation: Cancellation,
}
impl RequestContext {
    pub fn check(&self) -> Result<(), ProviderError> {
        if self.cancellation.is_cancelled() {
            Err(ProviderError::Cancelled)
        } else if Instant::now() >= self.deadline {
            Err(ProviderError::Timeout)
        } else {
            Ok(())
        }
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CandleRequest {
    pub pool: PoolId,
    pub priced_token: TokenId,
    pub price_unit: PriceUnit,
    pub resolution: Resolution,
    pub from_inclusive: TimestampMs,
    pub end_exclusive: TimestampMs,
    pub min_bars: usize,
    pub max_bars: usize,
}
impl CandleRequest {
    pub fn validate(&self) -> Result<(), ProviderError> {
        if self.min_bars == 0
            || self.min_bars > self.max_bars
            || self.max_bars > 10000
            || self.from_inclusive >= self.end_exclusive
            || self.pool.chain() != self.priced_token.chain()
            || self.from_inclusive.get() % self.resolution.millis() != 0
            || self.end_exclusive.get() % self.resolution.millis() != 0
        {
            return Err(ProviderError::InvalidRequest);
        }
        Ok(())
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Coverage {
    pub complete: bool,
    pub truncated: bool,
    pub warnings: Vec<String>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PoolDiscoveryResult {
    pub pools: Vec<PoolSnapshot>,
    pub coverage: Coverage,
}
#[derive(Debug, Clone, PartialEq, thiserror::Error, Serialize, Deserialize)]
pub enum ProviderError {
    #[error("recorded data unavailable at replay time")]
    UnavailableReplayData,
    #[error("all configured API keys have exhausted their local quota")]
    QuotaExhausted,
    #[error("API usage accounting unavailable")]
    UsageAccountingUnavailable,
    #[error("unsupported capability: {0}")]
    Unsupported(String),
    #[error("invalid request")]
    InvalidRequest,
    #[error("not found")]
    NotFound,
    #[error("unauthorized")]
    Unauthorized,
    #[error("rate limited; retry after {retry_after_ms} ms")]
    RateLimited { retry_after_ms: u64 },
    #[error("deadline exceeded")]
    Timeout,
    #[error("transport failure")]
    Transport,
    #[error("provider worker panicked")]
    WorkerPanicked,
    #[error("invalid provider schema: {0}")]
    InvalidSchema(String),
    #[error("invalid market data: {0}")]
    InvalidData(String),
    #[error("insufficient history: {have}/{need}")]
    InsufficientHistory { have: usize, need: usize },
    #[error("cancelled")]
    Cancelled,
}
impl From<DomainError> for ProviderError {
    fn from(e: DomainError) -> Self {
        Self::InvalidData(e.to_string())
    }
}
#[async_trait]
pub trait PoolDiscoveryProvider: Send + Sync {
    /// Fresh token-scoped facts. Unsupported metrics remain absent; no pool proxies.
    async fn token_metrics(
        &self,
        _token: &TokenId,
        _ctx: &RequestContext,
    ) -> Result<TokenMetrics, ProviderError> {
        Ok(TokenMetrics::default())
    }
    /// Requested token fields only; default keeps existing record/replay providers compatible.
    async fn token_metrics_selected(
        &self,
        token: &TokenId,
        _metrics: &[TokenMetric],
        ctx: &RequestContext,
    ) -> Result<TokenMetrics, ProviderError> {
        self.token_metrics(token, ctx).await
    }
    /// At most 100 unique tokens. None requests the legacy complete metrics snapshot.
    async fn token_metrics_batch(
        &self,
        tokens: &[TokenId],
        metrics: Option<&[TokenMetric]>,
        ctx: &RequestContext,
    ) -> Result<std::collections::BTreeMap<TokenId, TokenMetrics>, ProviderError> {
        if tokens.is_empty() || tokens.len() > 100 {
            return Err(ProviderError::InvalidRequest);
        }
        let mut output = std::collections::BTreeMap::new();
        for token in tokens {
            ctx.check()?;
            if output.contains_key(token) {
                return Err(ProviderError::InvalidRequest);
            }
            let facts = match metrics {
                Some(fields) => self.token_metrics_selected(token, fields, ctx).await?,
                None => self.token_metrics(token, ctx).await?,
            };
            output.insert(token.clone(), facts);
        }
        Ok(output)
    }
    fn descriptor(&self) -> plugin_api::PluginDescriptor;
    async fn pools_for_token(
        &self,
        token: &TokenId,
        ctx: &RequestContext,
    ) -> Result<PoolDiscoveryResult, ProviderError>;
}
#[async_trait]
pub trait PoolMetadataProvider: Send + Sync {
    async fn pool(
        &self,
        pool: &PoolId,
        ctx: &RequestContext,
    ) -> Result<PoolSnapshot, ProviderError>;
}
#[async_trait]
pub trait CandleProvider: Send + Sync {
    /// Explicit opt-in: closed history supports incremental reads with a small revision overlap.
    /// Exact-query record/replay adapters keep the default full-window behavior.
    fn supports_closed_candle_deltas(&self) -> bool {
        false
    }
    async fn candles(
        &self,
        request: &CandleRequest,
        ctx: &RequestContext,
    ) -> Result<CandleSeries, ProviderError>;
}
/// Base market facts remain independent of optional on-chain verification.
pub trait MarketProvider: CandleProvider + PoolDiscoveryProvider + PoolMetadataProvider {}
impl<T: CandleProvider + PoolDiscoveryProvider + PoolMetadataProvider + ?Sized> MarketProvider
    for T
{
}
#[async_trait]
pub trait PoolVerifier: Send + Sync {
    async fn verify_pool(
        &self,
        pool: &PoolId,
        ctx: &RequestContext,
    ) -> Result<PoolSnapshot, ProviderError>;
}
#[derive(Clone, Debug, Default, Serialize)]
pub struct TransportObservations {
    /// HTTP attempts, including bounded retries; logical calls remain in started.
    pub http_attempts: u64,
    pub http_retries: u64,
    pub rate_limited_responses: u64,
    pub started: u64,
    pub in_flight: usize,
    pub succeeded: u64,
    pub failed: u64,
    pub cancelled: u64,
    pub timed_out: u64,
    pub interrupted: u64,
    pub last_elapsed_micros: Option<u64>,
}
/// Local per-key accounting, including reserved HTTP attempts.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ApiKeyUsage {
    pub key_id: String,
    pub period: String,
    pub used: u64,
    pub limit: u64,
    pub warn_at: u64,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ApiUsageStatus {
    pub keys: Vec<ApiKeyUsage>,
    pub exhausted: bool,
}
/// Existing transport facts only. Implementations must not issue network probes.
#[async_trait]
pub trait TransportObservationReader: Send + Sync {
    fn transport_observations(&self) -> Option<TransportObservations>;
    async fn api_usage(&self) -> Result<Option<ApiUsageStatus>, ProviderError> {
        Ok(None)
    }
    /// Claims a single warning/exhaustion delivery attempt durably before sending.
    async fn claim_usage_notice(
        &self,
        _usage: &ApiKeyUsage,
        _exhausted: bool,
    ) -> Result<bool, ProviderError> {
        Ok(false)
    }
}
/// Successful series currently retained by a session cache, not discovery completeness/current health.
#[derive(Clone, Debug)]
pub struct CandleObservations {
    pub series: Vec<Arc<CandleSeries>>,
    pub observed_series: usize,
    pub truncated: bool,
}
/// Passive reads only: implementations must not fetch/probe providers or wait for pending flights.
#[async_trait]
pub trait CandleObservationReader: Send + Sync {
    async fn candle_observations(
        &self,
        token: &TokenId,
        limit: usize,
    ) -> Result<CandleObservations, ProviderError>;
}
