//! Extend closed history, refreshing two previous bars for late provider revisions.
use domain::*;
use provider_api::{CandleRequest, ProviderError};
pub(super) fn can_extend(series: &CandleSeries, request: &CandleRequest) -> bool {
    series.pool == request.pool
        && series.priced_token == request.priced_token
        && series.price_unit == request.price_unit
        && series.resolution == request.resolution
        && series.end_exclusive < request.end_exclusive
        && series.end_exclusive > request.from_inclusive
        && series
            .candles
            .first()
            .is_some_and(|c| c.open_time <= request.from_inclusive)
        && series.end_exclusive.get() - request.from_inclusive.get()
            > 2 * request.resolution.millis()
}
pub(super) fn delta_request(
    series: &CandleSeries,
    request: &CandleRequest,
) -> Result<CandleRequest, ProviderError> {
    let from = series
        .end_exclusive
        .checked_add(-2 * request.resolution.millis())?
        .max(request.from_inclusive);
    let count =
        usize::try_from((request.end_exclusive.get() - from.get()) / request.resolution.millis())
            .map_err(|_| ProviderError::InvalidRequest)?;
    let delta = CandleRequest {
        from_inclusive: from,
        min_bars: count,
        max_bars: count,
        ..request.clone()
    };
    delta.validate()?;
    Ok(delta)
}
pub(super) fn merge(
    prefix: &CandleSeries,
    mut delta: CandleSeries,
    upstream: &CandleRequest,
    full: &CandleRequest,
) -> Result<CandleSeries, ProviderError> {
    if prefix.volume_unit != delta.volume_unit
        || prefix.provenance.provider != delta.provenance.provider
        || prefix.provenance.contract_version != delta.provenance.contract_version
    {
        return Err(ProviderError::InvalidData(
            "incremental candle provenance/units changed".into(),
        ));
    }
    let mut candles: Vec<_> = prefix
        .candles
        .iter()
        .filter(|c| c.open_time >= full.from_inclusive && c.open_time < upstream.from_inclusive)
        .cloned()
        .collect();
    candles.extend(delta.candles.iter().cloned());
    delta.candles = candles.into();
    Ok(delta)
}
