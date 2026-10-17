use serde::{Deserialize, Serialize};
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct RangeConfig {
    /// Optional explicit acquisition window; None preserves legacy 300-bar requests/digests.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub history_bars: Option<usize>,
    pub windows: Vec<usize>,
    pub min_bars: usize,
    pub lower_quantile: f64,
    pub upper_quantile: f64,
    pub confirmed_threshold: f64,
    pub candidate_threshold: f64,
    pub near_best_tolerance: f64,
    pub drift_good: f64,
    pub drift_bad: f64,
    pub er_good: f64,
    pub er_bad: f64,
    pub traversal_zone: f64,
    pub traversal_target: usize,
    pub stability_good: f64,
    pub stability_bad: f64,
    pub tail_fraction: f64,
    pub acceptance_margin: f64,
    pub tail_acceptance_bad: f64,
    pub tail_acceptance_good: f64,
    pub max_outside_streak: usize,
    pub occupancy_bins: usize,
}

macro_rules! fields {
    ($mapping:ident, $config:ident) => {
        $mapping ! ($config; windows, min_bars, lower_quantile, upper_quantile,
        confirmed_threshold, candidate_threshold, near_best_tolerance, drift_good,
        drift_bad, er_good, er_bad, traversal_zone, traversal_target, stability_good,
        stability_bad, tail_fraction, acceptance_margin, tail_acceptance_bad,
        tail_acceptance_good, max_outside_streak, occupancy_bins,)
    };
}

impl Default for RangeConfig {
    fn default() -> Self {
        let c = crate::detector_core::RangeDetectorConfig::default();
        macro_rules! build {
            ($c:ident; $($f:ident),* $(,)?) => {
                Self { history_bars : None, $($f : $c .$f),* }
            };
        }

        fields!(build, c)
    }
}

impl From<RangeConfig> for crate::detector_core::RangeDetectorConfig {
    fn from(c: RangeConfig) -> Self {
        macro_rules! build {
            ($c:ident; $($f:ident),* $(,)?) => {
                Self { $($f : $c .$f),* }
            };
        }

        fields!(build, c)
    }
}
