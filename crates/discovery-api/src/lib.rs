#![forbid(unsafe_code)]
mod filters;
use async_trait::async_trait;
use domain::*;
pub use filters::{union_lower_bound, union_upper_bound};
use provider_api::{Coverage, ProviderError, RequestContext};
use serde::{Deserialize, Serialize};
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DiscoveryQuery {
    pub chain: ChainId,
    pub min_liquidity_usd: Option<NonNegative>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub liquidity_filter: Option<MetricBounds>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub market_cap_filter: Option<MetricBounds>,
    pub max_items: usize,
}

impl DiscoveryQuery {
    pub fn union(
        chain: ChainId,
        thresholds: &[Option<NonNegative>],
        max_items: usize,
    ) -> Result<Self, ProviderError> {
        if thresholds.is_empty() || max_items == 0 || max_items > 1000 {
            return Err(ProviderError::InvalidRequest);
        }

        let min_liquidity_usd = if thresholds.iter().any(Option::is_none) {
            None
        } else {
            thresholds
                .iter()
                .flatten()
                .copied()
                .min_by(|a, b| a.get().total_cmp(&b.get()))
        };

        Ok(Self {
            chain,
            min_liquidity_usd,
            liquidity_filter: None,
            market_cap_filter: None,
            max_items,
        })
    }

    pub fn digest(&self) -> ContentHash {
        ContentHash::of(self).expect("validated finite discovery values")
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DiscoveryCursor {
    pub query_digest: ContentHash,
    pub value: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CandidateBatch {
    pub items: Vec<Candidate>,
    pub next_cursor: Option<DiscoveryCursor>,
    pub fetched_at: TimestampMs,
    pub coverage: Coverage,
}

#[async_trait]
pub trait CandidateSource: Send + Sync {
    async fn poll(
        &self,
        query: &DiscoveryQuery,
        cursor: Option<&DiscoveryCursor>,
        ctx: &RequestContext,
    ) -> Result<CandidateBatch, ProviderError>;
}

#[cfg(test)]
mod tests;
