use crate::Services;
use analysis_api::Analyzer;
use domain::*;
use provider_api::{Coverage, ProviderError, RequestContext};
use serde::Serialize;
use std::collections::BTreeMap;
use strategy_api::Strategy;
use strategy_engine::{Evaluation, StrategyReport};
#[derive(Debug, Serialize)]
pub struct EvaluationReport {
    pub schema_version: u16,
    pub token: TokenId,
    /// Latest phase cutoff; each strategy decision retains its exact own cutoff.
    pub as_of: TimestampMs,
    pub coverage: Coverage,
    pub strategies: Vec<StrategyReport>,
}

impl Services {
    pub async fn evaluate_strategies(
        &self,
        token: &TokenId,
        strategies: &BTreeMap<InstanceId, Box<dyn Strategy>>,
        analyzers: &BTreeMap<InstanceId, Box<dyn Analyzer>>,
        close_grace_ms: u64,
        ctx: &RequestContext,
    ) -> Result<EvaluationReport, ProviderError> {
        self.evaluate_with_metrics(token, strategies, analyzers, close_grace_ms, None, ctx)
            .await
    }

    /// Prefetched facts belong to this evaluation only; final strategy freshness checks still apply.
    pub async fn evaluate_with_metrics(
        &self,
        token: &TokenId,
        strategies: &BTreeMap<InstanceId, Box<dyn Strategy>>,
        analyzers: &BTreeMap<InstanceId, Box<dyn Analyzer>>,
        close_grace_ms: u64,
        prefetched: Option<&TokenMetrics>,
        ctx: &RequestContext,
    ) -> Result<EvaluationReport, ProviderError> {
        if strategies.is_empty() || strategies.len() > 64 || close_grace_ms > 60_000 {
            return Err(ProviderError::InvalidRequest);
        }

        ctx.check()?;
        let prefilter_time = self.evaluation_time()?;
        let mut candidate = Candidate {
            metrics: Default::default(),
            token: token.clone(),
            pool_hints: vec![],
            symbol: None,
            discovered_at: prefilter_time,
            source: ProviderId::new("manual").unwrap(),
            liquidity_usd: Knowledge::unknown(),
        };

        let rejected: Option<Vec<_>> = strategies
            .iter()
            .map(|(instance, strategy)| {
                match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    strategy.prefilter(&candidate, prefilter_time)
                })) {
                    Ok(strategy_api::PrefilterDecision::Reject(reasons)) => Some(StrategyReport {
                        prefilter_rejection: Some(reasons),
                        instance: instance.clone(),
                        decision: None,
                        unavailable: None,
                        trace: vec![],
                    }),
                    _ => None,
                }
            })
            .collect();
        if let Some(strategies) = rejected {
            return Ok(EvaluationReport {
                schema_version: 1,
                token: token.clone(),
                as_of: prefilter_time,
                coverage: Coverage {
                    complete: false,
                    truncated: false,
                    warnings: vec!["PrefilteredBeforePoolDiscovery".into()],
                },
                strategies,
            });
        }

        let metrics = if let Some(metrics) = prefetched {
            metrics.clone()
        } else if strategies
            .values()
            .any(|strategy| strategy.uses_token_metrics())
        {
            match crate::metrics::selected_fields(strategies) {
                Some(fields) => {
                    crate::read_bounded(
                        self.provider.token_metrics_selected(token, &fields, ctx),
                        ctx,
                    )
                    .await?
                }

                None => crate::read_bounded(self.provider.token_metrics(token, ctx), ctx).await?,
            }
        } else {
            TokenMetrics::default()
        };

        let screened_at = self.evaluation_time()?;
        candidate.metrics = metrics;
        let screened: Option<Vec<_>> = strategies
            .values()
            .map(|strategy| {
                strategy_engine::screen_token_metrics(strategy.as_ref(), &candidate, screened_at)
            })
            .collect();
        if let Some(strategies) = screened {
            return Ok(EvaluationReport {
                schema_version: 1,
                token: token.clone(),
                as_of: screened_at,
                coverage: Coverage {
                    complete: false,
                    truncated: false,
                    warnings: vec!["TokenFiltersBeforePoolDiscovery".into()],
                },
                strategies,
            });
        }

        let pools = self.list_pools(token, ctx).await?;
        let as_of = self.evaluation_time()?;
        candidate.discovered_at = as_of;

        let evaluation = Evaluation {
            candidate: &candidate,
            pools: &pools,
            as_of,
            close_grace_ms,
        };

        let reports =
            strategy_engine::evaluate(&evaluation, strategies, analyzers, &self.hub, ctx).await;
        Ok(EvaluationReport {
            schema_version: 1,
            token: token.clone(),
            as_of,
            coverage: pools.coverage,
            strategies: reports,
        })
    }
}
