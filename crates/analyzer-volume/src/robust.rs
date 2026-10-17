use analysis_api::AnalysisError;
use analysis_model::{ActivityState, RobustVolumeAnalysis};
use domain::NonNegative;
use serde::{Deserialize, Serialize};
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct RobustVolumeConfig {
    pub short_bars: usize,
    pub long_bars: usize,
}

impl Default for RobustVolumeConfig {
    fn default() -> Self {
        Self {
            short_bars: 5,
            long_bars: 20,
        }
    }
}

impl RobustVolumeConfig {
    pub fn validate(&self, available: usize) -> Result<(), AnalysisError> {
        if self.short_bars < 2 || self.long_bars < self.short_bars || self.long_bars > available {
            return Err(AnalysisError::InvalidInput("volume slope windows".into()));
        }

        Ok(())
    }
}

fn median(values: &[f64]) -> f64 {
    let mut sorted = values.to_vec();
    let mid = sorted.len() / 2;
    let (lower, upper, _) = sorted.select_nth_unstable_by(mid, f64::total_cmp);
    if values.len().is_multiple_of(2) {
        let lower_middle = lower.iter().copied().max_by(f64::total_cmp).unwrap();
        lower_middle + (*upper - lower_middle) / 2.0
    } else {
        *upper
    }
}

fn slope(values: &[f64]) -> f64 {
    let center = (values.len() - 1) as f64 / 2.0;
    let mut numerator = 0.0;
    let mut denominator = 0.0;
    for (i, volume) in values.iter().enumerate() {
        let x = i as f64 - center;
        numerator += x * volume.ln_1p();
        denominator += x * x;
    }

    numerator / denominator
}

pub fn analyze(
    config: &RobustVolumeConfig,
    baseline: &[f64],
    recent: &[f64],
    elevated: NonNegative,
) -> Result<RobustVolumeAnalysis, AnalysisError> {
    let baseline_median = median(baseline);
    let recent_median = median(recent);
    let relative_volume = if baseline_median > 0.0 {
        Some(
            NonNegative::new(recent_median / baseline_median)
                .map_err(|_| AnalysisError::NumericFailure)?,
        )
    } else {
        None
    };

    let values: Vec<_> = baseline
        .iter()
        .chain(recent)
        .skip(baseline.len() + recent.len() - config.long_bars)
        .copied()
        .collect();
    let short_slope = slope(&values[values.len() - config.short_bars..]);
    let long_slope = slope(&values[values.len() - config.long_bars..]);
    if !short_slope.is_finite() || !long_slope.is_finite() {
        return Err(AnalysisError::NumericFailure);
    }

    let state = if recent_median == 0.0 {
        ActivityState::Inactive
    } else {
        match relative_volume {
            None => ActivityState::Unknown,
            Some(ratio) if ratio.get() >= elevated.get() => ActivityState::Elevated,
            Some(_) => ActivityState::Normal,
        }
    };

    Ok(RobustVolumeAnalysis {
        baseline_median: NonNegative::new(baseline_median)
            .map_err(|_| AnalysisError::NumericFailure)?,
        recent_median: NonNegative::new(recent_median)
            .map_err(|_| AnalysisError::NumericFailure)?,
        relative_volume,
        short_slope,
        long_slope,
        state,
    })
}
