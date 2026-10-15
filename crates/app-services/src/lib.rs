#![forbid(unsafe_code)]
mod analysis;
mod metrics;
mod strategies;
pub use analysis::{AnalysisReport, AnalyzeRequest, AnalyzerReport, ProbeFailure};
use analysis_api::*;
use data_hub::{
    DataHub,
    selection::{self, Exclusion, Purpose, SelectionPolicy},
};
use domain::*;
use futures::FutureExt;
pub use provider_api::MarketProvider;
use provider_api::*;
use serde::Serialize;
use std::{
    collections::BTreeMap,
    panic::AssertUnwindSafe,
    sync::Arc,
    time::{Duration, SystemTime, UNIX_EPOCH},
};
pub use strategies::EvaluationReport;
pub struct Services {
    provider: Arc<dyn MarketProvider>,
    hub: Arc<DataHub>,
}
impl Services {
    /// Existing validated cache observations only; this handle cannot initiate provider requests.
    pub fn candle_observations(&self) -> Arc<dyn provider_api::CandleObservationReader> {
        self.hub.clone()
    }
    pub fn new(provider: Arc<dyn MarketProvider>) -> Result<Self, ProviderError> {
        let hub = Arc::new(DataHub::new(
            provider.clone(),
            128,
            Duration::from_secs(3600),
            Duration::from_secs(30),
        )?);
        Ok(Self { provider, hub })
    }
    fn evaluation_time(&self) -> Result<TimestampMs, ProviderError> {
        let millis = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|_| ProviderError::InvalidRequest)?
            .as_millis();
        TimestampMs::new(i64::try_from(millis).map_err(|_| ProviderError::InvalidRequest)?)
            .map_err(Into::into)
    }
    pub fn stats(&self) -> data_hub::CacheStats {
        self.hub.stats()
    }
    pub async fn list_pools(
        &self,
        token: &TokenId,
        ctx: &RequestContext,
    ) -> Result<PoolDiscoveryResult, ProviderError> {
        let result = read_bounded(self.provider.pools_for_token(token, ctx), ctx).await?;
        if result.pools.len() > 1000 {
            return Err(ProviderError::InvalidData("too many pools".into()));
        }
        for pool in &result.pools {
            pool.validate()?;
            if !pool.contains(token) {
                return Err(ProviderError::InvalidData("pool token mismatch".into()));
            }
        }
        Ok(result)
    }
}
/// Service boundary for provider futures, including those that ignore RequestContext.
async fn read_bounded<T>(
    operation: impl std::future::Future<Output = Result<T, ProviderError>>,
    ctx: &RequestContext,
) -> Result<T, ProviderError> {
    ctx.check()?;
    let cancelled = async {
        while !ctx.cancellation.is_cancelled() {
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
    };
    let read = AssertUnwindSafe(operation).catch_unwind();
    let result = tokio::select! {
        _ = cancelled => return Err(ProviderError::Cancelled), result =
        tokio::time::timeout(ctx.deadline
        .saturating_duration_since(std::time::Instant::now()), read) => result.map_err(|
        _ | ProviderError::Timeout) ?.map_err(| _ | ProviderError::WorkerPanicked) ??,
    };
    ctx.check()?;
    Ok(result)
}
