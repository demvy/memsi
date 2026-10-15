use crate::analysis::{analyze_pool, candle_request};
use crate::compute::ComputeCache;
use crate::decision::decide;
use crate::planner::{self, StrategyPlan};
use crate::{inputs::*, report::StrategyReport, select_strategy_pools};
use analysis_api::Analyzer;
use data_hub::DataHub;
use domain::*;
use futures::{StreamExt, stream};
use provider_api::*;
use std::{
    collections::BTreeMap,
    panic::{AssertUnwindSafe, catch_unwind},
};
use strategy_api::*;
/// At most four strategy branches perform I/O concurrently. Output order is stable.
pub async fn evaluate(
    evaluation: &Evaluation<'_>,
    strategies: &BTreeMap<InstanceId, Box<dyn Strategy>>,
    analyzers: &BTreeMap<InstanceId, Box<dyn Analyzer>>,
    hub: &DataHub,
    ctx: &RequestContext,
) -> Vec<StrategyReport> {
    evaluate_internal(evaluation, strategies, analyzers, hub, ctx, None).await
}
pub async fn evaluate_mode(
    evaluation: &Evaluation<'_>,
    strategies: &BTreeMap<InstanceId, Box<dyn Strategy>>,
    analyzers: &BTreeMap<InstanceId, Box<dyn Analyzer>>,
    hub: &DataHub,
    ctx: &RequestContext,
    mode: StrategyMode,
) -> Vec<StrategyReport> {
    evaluate_internal(evaluation, strategies, analyzers, hub, ctx, Some(mode)).await
}
async fn evaluate_internal(
    evaluation: &Evaluation<'_>,
    strategies: &BTreeMap<InstanceId, Box<dyn Strategy>>,
    analyzers: &BTreeMap<InstanceId, Box<dyn Analyzer>>,
    hub: &DataHub,
    ctx: &RequestContext,
    selected_mode: Option<StrategyMode>,
) -> Vec<StrategyReport> {
    let mut reports = Vec::new();
    let mut plans = Vec::new();
    for (instance, strategy) in strategies {
        if selected_mode.is_some_and(|mode| strategy.mode() != mode) {
            continue;
        }
        match catch_unwind(AssertUnwindSafe(|| {
            strategy.prefilter(evaluation.candidate, evaluation.as_of)
        })) {
            Ok(PrefilterDecision::Continue) => {}
            Ok(PrefilterDecision::Reject(reasons)) => {
                reports.push(StrategyReport {
                    prefilter_rejection: Some(reasons),
                    instance: instance.clone(),
                    decision: None,
                    unavailable: None,
                    trace: vec![],
                });
                continue;
            }
            Err(_) => {
                reports.push(StrategyReport::unavailable(instance, "prefilter panic"));
                continue;
            }
        }
        if let Some(report) =
            crate::screen_token_metrics(strategy.as_ref(), evaluation.candidate, evaluation.as_of)
        {
            reports.push(report);
            continue;
        }
        let plan = planner::plan(strategy.as_ref(), analyzers);
        match plan {
            Ok(plan) => plans.push(plan),
            Err(error) => reports.push(StrategyReport::unavailable(instance, error)),
        }
    }
    let cache = ComputeCache::default();
    for mode in [StrategyMode::Active, StrategyMode::Shadow] {
        let phase: Vec<_> = plans
            .iter()
            .filter(|plan| plan.strategy.mode() == mode)
            .cloned()
            .collect();
        let windows = planner::union_windows(&phase);
        let mut phase_context = ctx.clone();
        if mode == StrategyMode::Shadow {
            phase_context.priority = RequestPriority::shadow();
            let now = std::time::Instant::now();
            let remaining = ctx.deadline.saturating_duration_since(now);
            let budget = (remaining / 2).min(std::time::Duration::from_secs(5));
            phase_context.deadline = now + budget;
        }
        let jobs = phase.iter().map(|plan| async {
            let work = run_strategy(
                plan,
                evaluation,
                windows[&plan.candles.resolution],
                &cache,
                hub,
                &phase_context,
            );
            if mode == StrategyMode::Shadow {
                tokio::time::timeout_at(phase_context.deadline.into(), work)
                    .await
                    .unwrap_or_else(|_| {
                        StrategyReport::unavailable(
                            plan.strategy.instance(),
                            "shadow budget exhausted",
                        )
                    })
            } else {
                work.await
            }
        });
        reports.extend(
            stream::iter(jobs)
                .buffer_unordered(4)
                .collect::<Vec<_>>()
                .await,
        );
    }
    reports.sort_by(|left, right| left.instance.cmp(&right.instance));
    reports
}
async fn run_strategy(
    plan: &StrategyPlan<'_>,
    evaluation: &Evaluation<'_>,
    bars: usize,
    cache: &ComputeCache,
    hub: &DataHub,
    ctx: &RequestContext,
) -> StrategyReport {
    let strategy = plan.strategy;
    let instance = strategy.instance();
    if let Err(error) = ctx.check() {
        return StrategyReport::unavailable(instance, error);
    }
    if evaluation.close_grace_ms > 60_000 {
        return StrategyReport::unavailable(instance, "invalid close grace");
    }
    let max_probes = strategy.requirements().pool.max_probes;
    let selection = match select_strategy_pools(
        strategy,
        &evaluation.candidate.token,
        evaluation.pools,
        evaluation.as_of,
    ) {
        Ok(selection) => selection,
        Err(error) => return StrategyReport::unavailable(instance, error),
    };
    let mut report = StrategyReport::unavailable(instance, "no usable pool within probe budget");
    for excluded in selection.excluded {
        report.record(&excluded.pool, excluded.reason);
    }
    for pool in selection.ranked.iter().take(max_probes) {
        let request = match candle_request(plan, pool, evaluation, bars) {
            Ok(request) => request,
            Err(error) => return StrategyReport::unavailable(instance, error),
        };
        let series = match hub.candles(&request, ctx).await {
            Ok(series) => series,
            Err(error) => {
                report.record(&pool.pool, error);
                continue;
            }
        };
        if let Err(error) = series.validate(plan.candles.min_bars) {
            report.record(&pool.pool, error);
            continue;
        }
        let analysis = match analyze_pool(plan, &series, cache, evaluation.as_of) {
            Ok(analysis) => analysis,
            Err(error) => {
                report.record(&pool.pool, error);
                continue;
            }
        };
        if let Err(error) = ctx.check() {
            return StrategyReport::unavailable(instance, error);
        }
        let decision = match decide(strategy, pool, evaluation, &analysis) {
            Ok(decision) => decision,
            Err(error) => return StrategyReport::unavailable(instance, error),
        };
        let retry_pool = matches!(decision.verdict, Verdict::Reject | Verdict::Unavailable);
        report.decision = Some(decision);
        report.unavailable = None;
        if !retry_pool {
            return report;
        }
    }
    report
}
