use data_hub::selection::{self, Purpose, SelectionPolicy};
use domain::*;
use provider_api::*;
use strategy_api::Strategy;
pub fn select_strategy_pools(
    strategy: &dyn Strategy,
    token: &TokenId,
    pools: &PoolDiscoveryResult,
    as_of: TimestampMs,
) -> Result<selection::SelectionPlan, ProviderError> {
    let requirements = strategy.requirements();
    selection::rank(
        token,
        pools.pools.clone(),
        &SelectionPolicy {
            purpose: Purpose::Research,
            min_liquidity: None,
            min_age_secs: 0,
            quote_tokens: requirements.pool.quote_tokens,
            max_probes: requirements.pool.max_probes,
            ..Default::default()
        },
        as_of,
    )
}
