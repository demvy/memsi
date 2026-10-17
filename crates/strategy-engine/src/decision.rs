use crate::{analysis::PreparedAnalysis, inputs::Evaluation};
use domain::*;
use std::panic::{AssertUnwindSafe, catch_unwind};
use strategy_api::*;
pub(crate) fn decide(
    strategy: &dyn Strategy,
    pool: &PoolSnapshot,
    evaluation: &Evaluation<'_>,
    analysis: &PreparedAnalysis,
) -> Result<StrategyDecision, &'static str> {
    let mut snapshot_hash = ContentHash::of(&(
        pool,
        &analysis.input_hash,
        &analysis.range,
        &analysis.volume,
        evaluation.as_of,
    ))
    .map_err(|_| "invalid snapshot")?;
    if let Some(trend) = &analysis.trend {
        snapshot_hash =
            ContentHash::of(&(&snapshot_hash, trend)).map_err(|_| "invalid trend snapshot")?;
    }

    let mut input_snapshot_hash =
        ContentHash::of(&(1u8, pool, &analysis.input_hash, evaluation.as_of))
            .map_err(|_| "invalid input snapshot")?;
    if !evaluation.candidate.metrics.is_empty() {
        snapshot_hash = ContentHash::of(&(&snapshot_hash, &evaluation.candidate.metrics))
            .map_err(|_| "invalid metrics snapshot")?;
        input_snapshot_hash =
            ContentHash::of(&(&input_snapshot_hash, &evaluation.candidate.metrics))
                .map_err(|_| "invalid metrics input")?;
    }

    let context = StrategyContext {
        candles: Some(&analysis.candles),
        candidate: evaluation.candidate,
        pool,
        range: analysis.range.as_ref(),
        volume: analysis.volume.as_ref(),
        trend: analysis.trend.as_ref(),
        snapshot_hash,
        input_snapshot_hash,
        as_of: evaluation.as_of,
    };

    let result = catch_unwind(AssertUnwindSafe(|| strategy.evaluate(&context)));
    let mut decision = match result {
        Ok(Ok(decision)) if valid_decision(&decision, strategy, &context) => decision,
        _ => return Err("strategy returned an invalid result or panicked"),
    };

    decision.evidence.candle_input_hash = Some(analysis.candle_input_hash.clone());
    if !evaluation.pools.coverage.complete {
        decision.warnings.push("PartialDiscoveryCoverage".into());
    }

    Ok(decision)
}

fn valid_decision(
    decision: &StrategyDecision,
    strategy: &dyn Strategy,
    context: &StrategyContext<'_>,
) -> bool {
    if decision.evidence.market.as_ref().is_some_and(|market| {
        ContentHash::of(market.as_ref()).ok() != ContentHash::of(context.pool).ok()
    }) {
        return false;
    }

    if decision.evidence.token_metrics.as_ref()
        != (!context.candidate.metrics.is_empty()).then_some(&context.candidate.metrics)
    {
        return false;
    }

    let evidence_matches = ContentHash::of(&(
        &decision.evidence.range,
        &decision.evidence.volume,
        &decision.evidence.trend,
    ))
    .ok()
        == ContentHash::of(&(context.range, context.volume, context.trend)).ok();
    let primary_evidence = decision.evidence.range.is_some() && decision.evidence.volume.is_some();
    let eligible_match = decision.verdict != Verdict::Match
        || ((!strategy.requirements().pool.concentrated_only
            || context.pool.concentrated_at(context.as_of))
            && primary_evidence
            && (!strategy
                .requirements()
                .analyses
                .iter()
                .any(|binding| binding.name.as_str() == "trend" && binding.required)
                || decision.evidence.trend.is_some()));
    evidence_matches
        && eligible_match
        && decision.config_digest == *strategy.config_digest()
        && decision.instance == *strategy.instance()
        && decision.plugin == strategy.descriptor().id
        && decision.mode == strategy.mode()
        && decision.token == context.candidate.token
        && decision.pool.as_ref() == Some(&context.pool.pool)
        && decision.as_of == context.as_of
        && decision.evidence.snapshot_hash == context.snapshot_hash
        && decision.evidence.input_snapshot_hash.as_ref() == Some(&context.input_snapshot_hash)
        && decision.evidence.cl_verification == context.pool.verification
        && decision.evidence.cl_verified == context.pool.concentrated_at(context.as_of)
        && (decision.verdict != Verdict::Match || primary_evidence)
}
