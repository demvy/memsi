//! Simple five-minute pump signal over completed one-minute candles.
use domain::*;
use serde::{Deserialize, Serialize};
use strategy_api::{DecisionReason, StrategyError, Verdict};

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct PumpSignal {
    pub context_bars: usize,
    pub min_gain_pct: f64,
}
impl Default for PumpSignal {
    fn default() -> Self {
        Self {
            context_bars: 20,
            min_gain_pct: 10.0,
        }
    }
}
impl PumpSignal {
    pub fn validate(&self) -> Result<(), StrategyError> {
        if !(20..=120).contains(&self.context_bars)
            || !self.min_gain_pct.is_finite()
            || self.min_gain_pct <= 0.0
        {
            return Err(StrategyError::InvalidResult);
        }
        Ok(())
    }
    pub fn required_bars(&self) -> usize {
        6
    }
    pub fn evaluate(
        &self,
        series: Option<&CandleSeries>,
        pool: &PoolId,
        token: &TokenId,
        as_of: TimestampMs,
    ) -> (Verdict, Vec<DecisionReason>) {
        let Some(series) = series else {
            return result(Verdict::Unavailable, "PUMP_CANDLES_MISSING", None);
        };
        if self.validate().is_err()
            || series.validate(self.required_bars()).is_err()
            || &series.pool != pool
            || &series.priced_token != token
            || series.price_unit != PriceUnit::UsdPerToken
            || series.volume_unit != VolumeUnit::Usd
            || series.resolution.seconds() != 60
            || series.end_exclusive > as_of
            || as_of.get() - series.end_exclusive.get() > 120_000
        {
            return result(Verdict::Unavailable, "PUMP_CANDLES_INVALID", None);
        }
        self.evaluate_bars(&series.candles, series.end_exclusive)
    }
    fn evaluate_bars(
        &self,
        candles: &[Candle],
        end: TimestampMs,
    ) -> (Verdict, Vec<DecisionReason>) {
        let Some(start) = candles.len().checked_sub(self.required_bars()) else {
            return result(Verdict::Unavailable, "PUMP_CANDLES_MISSING", None);
        };
        let bars = &candles[start..];
        let last = &bars[5];
        let gain = (last.close.get() / bars[0].close.get() - 1.0) * 100.0;
        // A positive five-minute return alone can hide a pump that is already reversing.
        if last.close <= last.open || last.close <= bars[4].close {
            return result(
                Verdict::Watch,
                "PUMP_NOT_RISING",
                Some(format!("gain_pct={gain}")),
            );
        }
        if !gain.is_finite() || gain <= self.min_gain_pct {
            return result(
                Verdict::Watch,
                "PUMP_GAIN_TOO_SMALL",
                Some(format!("gain_pct={gain}")),
            );
        }
        let volume = bars[1..]
            .iter()
            .try_fold(0.0, |sum, bar| bar.volume.as_ref().map(|v| sum + v.get()));
        if volume.is_none_or(|v| !v.is_finite() || v <= 0.0) {
            return result(Verdict::Unavailable, "PUMP_VOLUME_MISSING", None);
        }
        result(
            Verdict::Match,
            "PUMP_HIGH",
            Some(format!("gain_pct={gain}; end={}", end.get())),
        )
    }
}
fn result(verdict: Verdict, code: &str, actual: Option<String>) -> (Verdict, Vec<DecisionReason>) {
    (verdict, vec![DecisionReason { code: code.into(), field: "signal.pump".into(), actual, expected: "five-minute gain above min_gain_pct, rising latest closed candle, known positive volume; token filters must also pass".into() }])
}

#[cfg(test)]
mod tests {
    use super::*;
    fn candles(closes: &[f64]) -> Vec<Candle> {
        closes
            .iter()
            .enumerate()
            .map(|(i, close)| {
                let open = if i == 0 { *close } else { closes[i - 1] };
                Candle::new(
                    TimestampMs::new((i as i64 + 1) * 60_000).unwrap(),
                    open,
                    open.max(*close),
                    open.min(*close),
                    *close,
                    Knowledge::known(NonNegative::new(1000.0).unwrap()),
                )
                .unwrap()
            })
            .collect()
    }
    #[test]
    fn detects_latest_five_minutes_in_full_twenty_bar_history() {
        let mut prices = vec![100.0; 15];
        prices.extend([102.0, 104.0, 107.0, 110.0, 115.0]);
        let decision = PumpSignal::default()
            .evaluate_bars(&candles(&prices), TimestampMs::new(1_260_000).unwrap());
        assert_eq!(decision.0, Verdict::Match);
        assert_eq!(decision.1[0].code, "PUMP_HIGH");
    }
    #[test]
    fn rejects_current_dump_even_with_positive_five_minute_return() {
        assert_eq!(
            PumpSignal::default()
                .evaluate_bars(
                    &candles(&[100.0, 110.0, 120.0, 140.0, 150.0, 125.0]),
                    TimestampMs::new(420_000).unwrap()
                )
                .1[0]
                .code,
            "PUMP_NOT_RISING"
        );
    }
    #[test]
    fn threshold_is_strict() {
        let config = PumpSignal {
            min_gain_pct: 25.0,
            ..Default::default()
        };
        assert_eq!(
            config
                .evaluate_bars(
                    &candles(&[100.0, 105.0, 110.0, 115.0, 120.0, 125.0]),
                    TimestampMs::new(420_000).unwrap()
                )
                .0,
            Verdict::Watch
        );
    }
    #[test]
    fn missing_volume_is_unavailable() {
        let mut bars = candles(&[100.0, 105.0, 110.0, 115.0, 120.0, 125.0]);
        bars[5].volume = Knowledge::unknown();
        assert_eq!(
            PumpSignal::default()
                .evaluate_bars(&bars, TimestampMs::new(420_000).unwrap())
                .0,
            Verdict::Unavailable
        );
    }
}
