#![forbid(unsafe_code)]
mod admission;
mod config;
mod diagnostics;
mod market;
mod observations;
mod parse;
mod pool_tokens;
mod provider;
mod retry_after;
mod token_metrics;
mod usage;
pub use config::CodexConfig;
pub use provider::{CodexProvider, descriptor};
pub use provider_api::TransportObservations;

#[async_trait::async_trait]
impl provider_api::TransportObservationReader for CodexProvider {
    fn transport_observations(&self) -> Option<TransportObservations> {
        self.observations.snapshot()
    }
    async fn api_usage(
        &self,
    ) -> Result<Option<provider_api::ApiUsageStatus>, provider_api::ProviderError> {
        self.usage_status().await
    }
    async fn claim_usage_notice(
        &self,
        usage: &provider_api::ApiKeyUsage,
        exhausted: bool,
    ) -> Result<bool, provider_api::ProviderError> {
        match &self.usage {
            Some(ledger) => ledger.claim_notice(usage, exhausted).await,
            None => Ok(false),
        }
    }
}
