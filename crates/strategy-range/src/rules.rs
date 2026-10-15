use crate::{StrategyConfig, Thresholds};
use analysis_model::{RangeAnalysis, RangeState, VolumeAnalysis};
use domain::*;
use strategy_api::*;
fn reason(code: &str, field: &str, actual: Option<f64>, expected: impl ToString) -> DecisionReason {
    DecisionReason {
        code: code.into(),
        field: field.into(),
        actual: actual.map(|value| value.to_string()),
        expected: expected.to_string(),
    }
}
#[path = "rule_checks.rs"]
mod checks;
#[path = "rule_inputs.rs"]
mod inputs;
use checks::{hard_failures, volume_failures, watch_reasons};
use inputs::{Inputs, required_inputs};
pub(super) fn evaluate(
    context: &StrategyContext<'_>,
    config: &StrategyConfig,
    thresholds: Thresholds,
) -> (Verdict, Vec<DecisionReason>) {
    if let Some(result) = crate::filters::evaluate(context, config) {
        return result;
    }
    if let Some(signal) = &config.signal {
        if context
            .candles
            .is_some_and(|series| series.resolution.seconds() != thresholds.resolution_secs)
        {
            return (
                Verdict::Unavailable,
                vec![reason(
                    "PATTERN_RESOLUTION_MISMATCH",
                    "signal",
                    None,
                    thresholds.resolution_secs,
                )],
            );
        }
        return signal.evaluate(
            context.candles,
            &context.pool.pool,
            &context.candidate.token,
            context.as_of,
        );
    }
    let inputs = match required_inputs(context, config) {
        Ok(inputs) => inputs,
        Err(reasons) => return (Verdict::Unavailable, reasons),
    };
    let rejected = hard_failures(context, config, thresholds, &inputs);
    if !rejected.is_empty() {
        return (Verdict::Reject, rejected);
    }
    if config
        .trend
        .as_ref()
        .is_some_and(|binding| binding.reject_downtrend)
        && context
            .trend
            .is_some_and(|trend| trend.direction == analysis_model::TrendDirection::Down)
    {
        return (
            Verdict::Reject,
            vec![reason(
                "DOWNTREND",
                "trend.direction",
                None,
                "neutral or up",
            )],
        );
    }
    let watch = watch_reasons(inputs.range, thresholds);
    if !watch.is_empty() {
        return (Verdict::Watch, watch);
    }
    let inactive = volume_failures(inputs.volume, thresholds, config.research.is_some());
    if !inactive.is_empty() {
        return (Verdict::Reject, inactive);
    }
    if let Some(policy) = &config.research {
        let position = inputs
            .range
            .range
            .as_ref()
            .and_then(|range| range.position.as_ref());
        let Some(position) = position else {
            return (
                Verdict::Unavailable,
                vec![reason(
                    "POSITION_UNAVAILABLE",
                    "range.position",
                    None,
                    "price-linear position",
                )],
            );
        };
        let bounds = inputs.range.range.as_ref().unwrap();
        let expected = analysis_model::RangePosition::new(
            bounds.lower.get(),
            bounds.upper.get(),
            position.current_price.get(),
        );
        if expected.is_none_or(|expected| {
            expected.raw_pct != position.raw_pct
                || expected.clamped_pct != position.clamped_pct
                || expected.zone != position.zone
        }) {
            return (
                Verdict::Unavailable,
                vec![reason(
                    "POSITION_INVALID",
                    "range.position",
                    None,
                    "consistent price-linear metrics",
                )],
            );
        }
        if position.raw_pct < policy.min_position_pct || position.raw_pct > policy.max_position_pct
        {
            return (
                Verdict::Reject,
                vec![reason(
                    "POSITION_OUTSIDE_POLICY",
                    "range.position.raw_pct",
                    Some(position.raw_pct),
                    format!("{}..={}", policy.min_position_pct, policy.max_position_pct),
                )],
            );
        }
        return (
            Verdict::Match,
            vec![
                reason(
                    "CONFIRMED_RANGE",
                    "range.confidence",
                    Some(inputs.range.confidence.get()),
                    thresholds.confidence,
                ),
                reason(
                    "POSITION_ACCEPTED",
                    "range.position.raw_pct",
                    Some(position.raw_pct),
                    format!("{}..={}", policy.min_position_pct, policy.max_position_pct),
                ),
                reason(
                    "HEALTHY_VOLUME",
                    "volume.robust.relative_volume",
                    inputs
                        .volume
                        .robust
                        .as_ref()
                        .and_then(|v| v.relative_volume)
                        .map(|v| v.get()),
                    thresholds.relative_volume,
                ),
                reason(
                    "LIQUIDITY_ACCEPTED",
                    "liquidity_usd",
                    Some(inputs.liquidity),
                    thresholds.liquidity_usd,
                ),
                reason(
                    "POOL_AGE_ACCEPTED",
                    "pool_age_secs",
                    Some(inputs.age),
                    thresholds.age_secs,
                ),
                reason(
                    "VOLUME_24H_ACCEPTED",
                    "volume_24h_usd",
                    Some(inputs.volume_24h),
                    thresholds.volume_24h_usd,
                ),
            ],
        );
    }
    (Verdict::Match, vec![])
}
