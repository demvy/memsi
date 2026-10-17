use crate::{compute::ComputeCache, inputs::Evaluation, planner::StrategyPlan};
use analysis_api::AnalysisValue;
use analysis_model::{RangeAnalysis, TrendAnalysis, VolumeAnalysis};
use domain::*;
use provider_api::*;
pub(crate) struct PreparedAnalysis {
    pub(crate) candles: CandleSeries,
    pub(crate) candle_input_hash: ContentHash,
    pub(crate) range: Option<RangeAnalysis>,
    pub(crate) volume: Option<VolumeAnalysis>,
    pub(crate) trend: Option<TrendAnalysis>,
    pub(crate) input_hash: ContentHash,
}

pub(crate) fn analyze_pool(
    plan: &StrategyPlan<'_>,
    series: &CandleSeries,
    cache: &ComputeCache,
    as_of: TimestampMs,
) -> Result<PreparedAnalysis, &'static str> {
    let range = cache
        .analyze(plan.range.ok_or("missing range binding")?, series, as_of)
        .map_err(|_| "range analysis unavailable")?;
    let volume = cache
        .analyze(plan.volume.ok_or("missing volume binding")?, series, as_of)
        .map_err(|_| "volume analysis unavailable")?;
    let mut input_hash = ContentHash::of(&(1u8, &range.input_hash, &volume.input_hash))
        .map_err(|_| "invalid input hash")?;
    let trend = if let Some(analyzer) = plan.trend {
        let computed = cache.analyze(analyzer, series, as_of).ok();
        input_hash = ContentHash::of(&(
            &input_hash,
            analyzer.instance(),
            analyzer.config_digest(),
            computed.as_ref().map(|result| &result.input_hash),
        ))
        .map_err(|_| "invalid trend input hash")?;
        computed.and_then(|result| match result.value {
            AnalysisValue::Trend(trend)
                if trend.slope_log_per_bar.is_finite()
                    && trend.window >= 2
                    && trend.window <= series.candles.len()
                    && trend.resolution == series.resolution
                    && trend.price_unit == series.price_unit =>
            {
                Some(trend)
            }

            _ => None,
        })
    } else {
        None
    };

    match (range.value, volume.value) {
        (AnalysisValue::Range(range), AnalysisValue::Volume(volume)) => Ok(PreparedAnalysis {
            candles: series.clone(),
            candle_input_hash: series.provenance.input_hash.clone(),
            range: Some(range),
            volume: Some(volume),
            trend,
            input_hash,
        }),
        _ => Err("incompatible analyzer output"),
    }
}

pub(crate) fn candle_request(
    plan: &StrategyPlan<'_>,
    pool: &PoolSnapshot,
    evaluation: &Evaluation<'_>,
    bars: usize,
) -> Result<CandleRequest, ProviderError> {
    let resolution = plan.candles.resolution;
    let cutoff = evaluation
        .as_of
        .closed_cutoff(resolution, evaluation.close_grace_ms)?;
    let from = TimestampMs::new(
        cutoff
            .get()
            .checked_sub(bars as i64 * resolution.millis())
            .ok_or(ProviderError::InvalidRequest)?,
    )?;
    Ok(CandleRequest {
        pool: pool.pool.clone(),
        priced_token: evaluation.candidate.token.clone(),
        price_unit: plan.candles.price_unit.clone(),
        resolution,
        from_inclusive: from,
        end_exclusive: cutoff,
        min_bars: 1,
        max_bars: bars,
    })
}
