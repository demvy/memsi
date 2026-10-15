use super::*;
fn fresh_value(value: &Knowledge<Observation<NonNegative>>, now: TimestampMs) -> Option<f64> {
    value
        .as_ref()
        .filter(|observation| observation.is_fresh(now, 60_000, false))
        .map(|observation| observation.value.get())
}
pub(super) struct Inputs<'a> {
    pub(super) liquidity: f64,
    pub(super) volume_24h: f64,
    pub(super) age: f64,
    pub(super) range: &'a RangeAnalysis,
    pub(super) volume: &'a VolumeAnalysis,
}
pub(super) fn required_inputs<'a>(
    context: &StrategyContext<'a>,
    config: &StrategyConfig,
) -> Result<Inputs<'a>, Vec<DecisionReason>> {
    let pool = context.pool;
    let liquidity = fresh_value(&pool.liquidity_usd, context.as_of);
    let volume_24h = fresh_value(&pool.volume_24h_usd, context.as_of);
    let age = pool
        .created_at
        .as_ref()
        .filter(|created| **created <= context.as_of)
        .map(|created| (context.as_of.get() - created.get()) as f64 / 1000.);
    let mut missing = Vec::new();
    for (field, value) in [
        ("liquidity_usd", liquidity),
        ("volume_24h_usd", volume_24h),
        ("pool_age_secs", age),
    ] {
        if value.is_none() {
            missing.push(reason(
                "INPUT_MISSING_OR_STALE",
                field,
                None,
                "known, fresh value",
            ));
        }
    }
    if config
        .trend
        .as_ref()
        .is_some_and(|binding| binding.required)
        && context.trend.is_none()
    {
        missing.push(reason("INPUT_MISSING", "trend", None, "trend analysis"));
    }
    if context.range.is_none() {
        missing.push(reason("INPUT_MISSING", "range", None, "range analysis"));
    }
    if context.volume.is_none() {
        missing.push(reason("INPUT_MISSING", "volume", None, "volume analysis"));
    }
    if !config.research_only
        && (pool.kind == PoolKind::Unknown
            || !pool.kind_verified
            || pool
                .verification
                .as_ref()
                .is_some_and(|proof| !proof.is_fresh(context.as_of, u64::MAX)))
    {
        missing.push(reason(
            "POOL_KIND_UNVERIFIED",
            "pool.kind",
            None,
            "verified concentrated pool",
        ));
    }
    if let Some(volume) = context.volume
        && ((if config.research.is_some() {
            volume.robust.as_ref().and_then(|v| v.relative_volume)
        } else {
            volume.relative_volume
        })
        .is_none()
            || volume.unit != VolumeUnit::Usd)
    {
        missing.push(reason(
            "VOLUME_UNAVAILABLE",
            "relative_volume",
            None,
            "known USD volume ratio",
        ));
    }
    if let Some(range) = context.range {
        if range.price_unit != PriceUnit::UsdPerToken || range.state == RangeState::InsufficientData
        {
            missing.push(reason(
                "RANGE_UNAVAILABLE",
                "range",
                None,
                "closed USD history",
            ));
        }
        if matches!(range.state, RangeState::Candidate | RangeState::Confirmed)
            && range.range.is_none()
        {
            missing.push(reason("RANGE_UNAVAILABLE", "range.bounds", None, "bounds"));
        }
    }
    if !missing.is_empty() {
        return Err(missing);
    }
    Ok(Inputs {
        liquidity: liquidity.unwrap(),
        volume_24h: volume_24h.unwrap(),
        age: age.unwrap(),
        range: context.range.unwrap(),
        volume: context.volume.unwrap(),
    })
}
