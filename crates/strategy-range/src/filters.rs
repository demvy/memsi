use crate::StrategyConfig;
use domain::*;
use strategy_api::{DecisionReason, StrategyContext, Verdict};
pub(super) fn evaluate(
    context: &StrategyContext<'_>,
    config: &StrategyConfig,
) -> Option<(Verdict, Vec<DecisionReason>)> {
    check(
        config,
        |metric| match metric {
            TokenMetric::PoolLiquidityUsd => context
                .pool
                .liquidity_usd
                .as_ref()
                .filter(|o| {
                    o.fetched_at <= context.as_of
                        && o.is_fresh(context.as_of, config.metrics_max_age_secs * 1000, false)
                })
                .map(|o| o.value.get()),
            _ => token_value(context.candidate, metric, context.as_of, config),
        },
        false,
        |metric| {
            unavailable_code(
                if *metric == TokenMetric::PoolLiquidityUsd {
                    context.pool.liquidity_usd.as_ref()
                } else {
                    context.candidate.metrics.get(metric)
                },
                context.as_of,
                config.metrics_max_age_secs * 1000,
            )
        },
    )
}
pub(super) fn screen(
    candidate: &Candidate,
    now: TimestampMs,
    config: &StrategyConfig,
) -> strategy_api::MetricScreen {
    match check(
        config,
        |metric| token_value(candidate, metric, now, config),
        true,
        |metric| {
            unavailable_code(
                candidate.metrics.get(metric),
                now,
                config.metrics_max_age_secs * 1000,
            )
        },
    ) {
        Some((Verdict::Reject, reasons)) => strategy_api::MetricScreen::Reject(reasons),
        Some((_, reasons)) => strategy_api::MetricScreen::Unavailable(reasons),
        None => strategy_api::MetricScreen::Continue,
    }
}
fn token_value(
    candidate: &Candidate,
    metric: &TokenMetric,
    now: TimestampMs,
    config: &StrategyConfig,
) -> Option<f64> {
    candidate
        .metrics
        .get(metric)
        .filter(|o| {
            o.fetched_at <= now && o.is_fresh(now, config.metrics_max_age_secs * 1000, false)
        })
        .map(|o| {
            let age = matches!(
                metric,
                TokenMetric::TokenAgeSecs
                    | TokenMetric::TradingAgeSecs
                    | TokenMetric::ProviderSeenAgeSecs
                    | TokenMetric::TopPoolAgeSecs
            );
            o.value.get()
                + if age {
                    (now.get() - o.fetched_at.get()) as f64 / 1000.0
                } else {
                    0.0
                }
        })
}
fn check(
    config: &StrategyConfig,
    value: impl Fn(&TokenMetric) -> Option<f64>,
    token_only: bool,
    unknown_code: impl Fn(&TokenMetric) -> &'static str,
) -> Option<(Verdict, Vec<DecisionReason>)> {
    let mut unknown = Vec::new();
    let mut failed = Vec::new();
    for (metric, bounds) in &config.filters {
        if token_only && *metric == TokenMetric::PoolLiquidityUsd {
            continue;
        }
        let value = value(metric);
        let reason = DecisionReason {
            code: if value.is_none() {
                unknown_code(metric)
            } else {
                "METRIC_OUTSIDE_FILTER"
            }
            .into(),
            field: format!("token.{metric:?}"),
            actual: value.map(|v| v.to_string()),
            expected: format!("{bounds:?}"),
        };
        match value {
            None => unknown.push(reason),
            Some(value)
                if (metric.is_percentage() && value > 100.0)
                    || (metric.is_count() && value.fract() != 0.0) =>
            {
                unknown.push(reason)
            }
            Some(value) if !bounds.accepts(value) => failed.push(reason),
            Some(_) => {}
        }
    }
    if !failed.is_empty() {
        Some((Verdict::Reject, failed))
    } else if !unknown.is_empty() {
        Some((Verdict::Unavailable, unknown))
    } else {
        None
    }
}
fn unavailable_code(
    observation: Option<&Observation<NonNegative>>,
    now: TimestampMs,
    max_age_ms: u64,
) -> &'static str {
    match observation {
        None => "METRIC_MISSING",
        Some(o) if o.fetched_at > now => "METRIC_FUTURE_TIMESTAMP",
        Some(o) if !o.is_fresh(now, max_age_ms, false) => "METRIC_STALE",
        Some(_) => "METRIC_UNKNOWN_OR_STALE",
    }
}
