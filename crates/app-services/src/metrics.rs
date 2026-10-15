use crate::{Services, read_bounded};
use domain::*;
use provider_api::*;
use std::{
    collections::{BTreeMap, BTreeSet},
    panic::{AssertUnwindSafe, catch_unwind},
};
use strategy_api::{PrefilterDecision, Strategy};

pub(super) fn selected_fields(
    strategies: &BTreeMap<InstanceId, Box<dyn Strategy>>,
) -> Option<Vec<TokenMetric>> {
    let fields: Option<Vec<Vec<_>>> = strategies
        .values()
        .filter(|s| s.uses_token_metrics())
        .map(|s| s.required_token_metrics())
        .collect();
    fields.map(|fields| {
        fields
            .into_iter()
            .flatten()
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect()
    })
}
impl Services {
    /// One metrics read for the due watchlist only; discovered but not due tokens incur no read.
    pub async fn prefetch_token_metrics(
        &self,
        candidates: &[Candidate],
        strategies: &BTreeMap<InstanceId, Box<dyn Strategy>>,
        ctx: &RequestContext,
    ) -> Result<BTreeMap<TokenId, TokenMetrics>, ProviderError> {
        ctx.check()?;
        if candidates.len() > 100 || strategies.is_empty() || strategies.len() > 64 {
            return Err(ProviderError::InvalidRequest);
        }
        if !strategies.values().any(|s| s.uses_token_metrics()) {
            return Ok(BTreeMap::new());
        }
        let now = self.evaluation_time()?;
        let tokens: Vec<_> = candidates
            .iter()
            .filter(|candidate| {
                strategies.values().any(|s| {
                    !matches!(
                        catch_unwind(AssertUnwindSafe(|| s.prefilter(candidate, now))),
                        Ok(PrefilterDecision::Reject(_))
                    )
                })
            })
            .map(|c| c.token.clone())
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect();
        if tokens.is_empty() {
            return Ok(BTreeMap::new());
        }
        let fields = selected_fields(strategies);
        let metrics = read_bounded(
            self.provider
                .token_metrics_batch(&tokens, fields.as_deref(), ctx),
            ctx,
        )
        .await?;
        Ok(metrics)
    }
}
