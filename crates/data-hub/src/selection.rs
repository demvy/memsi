use domain::*;
use provider_api::ProviderError;
use serde::Serialize;
use std::collections::BTreeSet;
#[derive(Clone, Copy, Debug, Serialize, PartialEq, Eq)]
pub enum Purpose {
    Research,
    ConcentratedLp,
}
#[derive(Clone, Debug)]
pub struct SelectionPolicy {
    pub purpose: Purpose,
    pub min_liquidity: Option<NonNegative>,
    pub min_age_secs: u64,
    pub max_metadata_age_ms: u64,
    pub quote_tokens: Vec<TokenId>,
    pub max_probes: usize,
}
impl Default for SelectionPolicy {
    fn default() -> Self {
        Self {
            purpose: Purpose::Research,
            min_liquidity: None,
            min_age_secs: 0,
            max_metadata_age_ms: 60000,
            quote_tokens: vec![],
            max_probes: 5,
        }
    }
}
#[derive(Debug, Serialize)]
pub struct Exclusion {
    pub pool: PoolId,
    pub reason: String,
}
#[derive(Debug, Serialize)]
pub struct SelectionPlan {
    pub ranked: Vec<PoolSnapshot>,
    pub excluded: Vec<Exclusion>,
}
pub fn rank(
    token: &TokenId,
    pools: Vec<PoolSnapshot>,
    policy: &SelectionPolicy,
    as_of: TimestampMs,
) -> Result<SelectionPlan, ProviderError> {
    if policy.max_probes == 0
        || policy.max_probes > 20
        || policy.min_age_secs > i64::MAX as u64 / 1000
    {
        return Err(ProviderError::InvalidRequest);
    }
    let mut ranked = vec![];
    let mut excluded = vec![];
    let mut seen = BTreeSet::new();
    for p in pools {
        p.validate()?;
        if !seen.insert(p.pool.clone()) {
            return Err(ProviderError::InvalidData("duplicate pool identity".into()));
        }
        let quote = if p.token0 == *token {
            &p.token1
        } else {
            &p.token0
        };
        let reason = if !p.contains(token) {
            Some("token_not_in_pool")
        } else if policy.purpose == Purpose::ConcentratedLp && !p.concentrated_at(as_of) {
            Some("concentrated_pool_not_verified")
        } else if !policy.quote_tokens.is_empty() && !policy.quote_tokens.contains(quote) {
            Some("quote_not_allowed")
        } else if policy.min_liquidity.is_some_and(|min| {
            !p.liquidity_usd.as_ref().is_some_and(|v| {
                v.is_fresh(as_of, policy.max_metadata_age_ms, false) && v.value.get() >= min.get()
            })
        }) {
            Some("liquidity_missing_stale_or_below_minimum")
        } else if policy.min_age_secs > 0
            && !p
                .created_at
                .as_ref()
                .is_some_and(|t| as_of.get() - t.get() >= policy.min_age_secs as i64 * 1000)
        {
            Some("pool_age_missing_or_too_young")
        } else {
            None
        };
        if let Some(reason) = reason {
            excluded.push(Exclusion {
                pool: p.pool,
                reason: reason.into(),
            });
        } else {
            ranked.push(p);
        }
    }
    let quote_rank = |p: &PoolSnapshot| {
        let quote = if p.token0 == *token {
            &p.token1
        } else {
            &p.token0
        };
        policy
            .quote_tokens
            .iter()
            .position(|q| q == quote)
            .unwrap_or(usize::MAX)
    };
    let metric = |v: &Knowledge<Observation<NonNegative>>| {
        v.as_ref()
            .filter(|v| v.is_fresh(as_of, policy.max_metadata_age_ms, false))
            .map(|v| v.value.get())
    };
    let descending = |a: Option<f64>, b: Option<f64>| match (a, b) {
        (Some(a), Some(b)) => b.total_cmp(&a),
        (Some(_), None) => std::cmp::Ordering::Less,
        (None, Some(_)) => std::cmp::Ordering::Greater,
        _ => std::cmp::Ordering::Equal,
    };
    ranked.sort_by(|a, b| {
        quote_rank(a)
            .cmp(&quote_rank(b))
            .then_with(|| descending(metric(&a.liquidity_usd), metric(&b.liquidity_usd)))
            .then_with(|| descending(metric(&a.volume_24h_usd), metric(&b.volume_24h_usd)))
            .then_with(|| a.pool.cmp(&b.pool))
    });
    excluded.sort_by(|a, b| a.pool.cmp(&b.pool));
    Ok(SelectionPlan { ranked, excluded })
}
