use super::*;
pub(super) fn hard_failures(
    context: &StrategyContext<'_>,
    config: &StrategyConfig,
    thresholds: Thresholds,
    inputs: &Inputs<'_>,
) -> Vec<DecisionReason> {
    let pool = context.pool;
    let range = inputs.range;
    let mut rejected = Vec::new();
    for (field, value, minimum, code) in [
        (
            "liquidity_usd",
            inputs.liquidity,
            thresholds.liquidity_usd,
            "LIQUIDITY_BELOW_MIN",
        ),
        (
            "volume_24h_usd",
            inputs.volume_24h,
            thresholds.volume_24h_usd,
            "VOLUME_24H_BELOW_MIN",
        ),
        (
            "pool_age_secs",
            inputs.age,
            thresholds.age_secs as f64,
            "AGE_BELOW_MIN",
        ),
    ] {
        if value < minimum {
            rejected.push(reason(code, field, Some(value), minimum));
        }
    }

    let quote = if pool.token0 == context.candidate.token {
        &pool.token1
    } else {
        &pool.token0
    };

    if !config.quote_tokens.is_empty() && !config.quote_tokens.contains(quote) {
        rejected.push(reason(
            "QUOTE_NOT_ALLOWED",
            "pool.quote",
            None,
            "configured quote token",
        ));
    }

    if !config.research_only && !pool.concentrated_at(context.as_of) {
        rejected.push(reason(
            "POOL_KIND_NOT_ALLOWED",
            "pool.kind",
            None,
            "concentrated pool",
        ));
    }

    if !config.protocols.is_empty()
        && !pool
            .protocol
            .as_ref()
            .is_some_and(|protocol| config.protocols.contains(protocol))
    {
        rejected.push(reason(
            "PROTOCOL_NOT_ALLOWED",
            "pool.protocol",
            None,
            "configured protocol",
        ));
    }

    if range.state == RangeState::NoRange {
        rejected.push(reason("NO_RANGE", "range.state", None, "confirmed"));
    }

    if let Some(bounds) = &range.range
        && (bounds.width_pct.get() < 2. || bounds.width_pct.get() > thresholds.max_width_pct)
    {
        rejected.push(reason(
            "RANGE_WIDTH_OUTSIDE",
            "width_pct",
            Some(bounds.width_pct.get()),
            format!("2..={}", thresholds.max_width_pct),
        ));
    }

    rejected
}

pub(super) fn watch_reasons(range: &RangeAnalysis, thresholds: Thresholds) -> Vec<DecisionReason> {
    let mut watch = Vec::new();
    if range.state != RangeState::Confirmed {
        watch.push(reason(
            "RANGE_UNCONFIRMED",
            "range.state",
            None,
            "confirmed",
        ));
    }

    if range.confidence.get() < thresholds.confidence {
        watch.push(reason(
            "RANGE_CONFIDENCE_LOW",
            "confidence",
            Some(range.confidence.get()),
            thresholds.confidence,
        ));
    }

    if range
        .range
        .as_ref()
        .is_some_and(|bounds| !bounds.latest_inside_bounds)
    {
        watch.push(reason(
            "PRICE_OUTSIDE_RANGE",
            "latest_inside_bounds",
            None,
            "true",
        ));
    }

    watch
}

pub(super) fn volume_failures(
    volume: &VolumeAnalysis,
    thresholds: Thresholds,
    robust: bool,
) -> Vec<DecisionReason> {
    let mut reasons = Vec::new();
    if volume.activity_fraction.get() < thresholds.activity {
        reasons.push(reason(
            "VOLUME_INACTIVE",
            "activity_fraction",
            Some(volume.activity_fraction.get()),
            thresholds.activity,
        ));
    }

    let relative = (if robust {
        volume.robust.as_ref().and_then(|v| v.relative_volume)
    } else {
        volume.relative_volume
    })
    .expect("required_inputs checked relative volume");
    if relative.get() < thresholds.relative_volume {
        reasons.push(reason(
            "RELATIVE_VOLUME_LOW",
            "relative_volume",
            Some(relative.get()),
            thresholds.relative_volume,
        ));
    }

    reasons
}
