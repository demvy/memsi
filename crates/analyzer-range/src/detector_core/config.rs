use crate::detector_core::RangeError;
/// Tunable thresholds used by [`crate::detector_core::RangeDetector`].
#[derive(Clone, Debug)]
pub struct RangeDetectorConfig {
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

impl Default for RangeDetectorConfig {
    fn default() -> Self {
        Self {
            windows: vec![24, 36, 54, 80, 120, 180, 270],
            min_bars: 24,
            lower_quantile: 0.10,
            upper_quantile: 0.90,
            confirmed_threshold: 0.72,
            candidate_threshold: 0.58,
            near_best_tolerance: 0.07,
            drift_good: 0.20,
            drift_bad: 0.80,
            er_good: 0.10,
            er_bad: 0.45,
            traversal_zone: 0.45,
            traversal_target: 3,
            stability_good: 0.15,
            stability_bad: 0.60,
            tail_fraction: 0.20,
            acceptance_margin: 0.15,
            tail_acceptance_bad: 0.60,
            tail_acceptance_good: 0.90,
            max_outside_streak: 2,
            occupancy_bins: 8,
        }
    }
}

impl RangeDetectorConfig {
    pub(crate) fn validate(&self) -> Result<(), RangeError> {
        if self.windows.is_empty() {
            return Err(RangeError::InvalidConfig("windows must not be empty"));
        }

        if self.windows.iter().any(|&window| window <= 1) {
            return Err(RangeError::InvalidConfig(
                "all windows must be greater than 1",
            ));
        }

        if self.min_bars <= 1 {
            return Err(RangeError::InvalidConfig("min_bars must be greater than 1"));
        }

        check_unit_interval(self.lower_quantile, "lower_quantile must be in [0, 1]")?;
        check_unit_interval(self.upper_quantile, "upper_quantile must be in [0, 1]")?;
        if self.lower_quantile >= self.upper_quantile {
            return Err(RangeError::InvalidConfig(
                "lower_quantile must be less than upper_quantile",
            ));
        }

        check_unit_interval(
            self.candidate_threshold,
            "candidate_threshold must be in [0, 1]",
        )?;
        check_unit_interval(
            self.confirmed_threshold,
            "confirmed_threshold must be in [0, 1]",
        )?;
        if self.candidate_threshold > self.confirmed_threshold {
            return Err(RangeError::InvalidConfig(
                "candidate_threshold must not exceed confirmed_threshold",
            ));
        }

        check_nonnegative_finite(
            self.near_best_tolerance,
            "near_best_tolerance must be finite and nonnegative",
        )?;
        check_ordered_pair(
            self.drift_good,
            self.drift_bad,
            "drift_good must be less than drift_bad",
        )?;
        check_ordered_pair(
            self.er_good,
            self.er_bad,
            "er_good must be less than er_bad",
        )?;
        check_ordered_pair(
            self.stability_good,
            self.stability_bad,
            "stability_good must be less than stability_bad",
        )?;
        check_ordered_pair(
            self.tail_acceptance_bad,
            self.tail_acceptance_good,
            "tail_acceptance_bad must be less than tail_acceptance_good",
        )?;
        if !(self.tail_fraction.is_finite()
            && 0.0 < self.tail_fraction
            && self.tail_fraction <= 1.0)
        {
            return Err(RangeError::InvalidConfig("tail_fraction must be in (0, 1]"));
        }

        check_nonnegative_finite(
            self.acceptance_margin,
            "acceptance_margin must be finite and nonnegative",
        )?;
        if !(self.traversal_zone.is_finite()
            && 0.0 < self.traversal_zone
            && self.traversal_zone < 1.0)
        {
            return Err(RangeError::InvalidConfig(
                "traversal_zone must be in (0, 1)",
            ));
        }

        if self.traversal_target == 0 {
            return Err(RangeError::InvalidConfig(
                "traversal_target must be positive",
            ));
        }

        if self.occupancy_bins < 2 {
            return Err(RangeError::InvalidConfig(
                "occupancy_bins must be at least 2",
            ));
        }

        Ok(())
    }
}

fn check_unit_interval(value: f64, message: &'static str) -> Result<(), RangeError> {
    if value.is_finite() && (0.0..=1.0).contains(&value) {
        Ok(())
    } else {
        Err(RangeError::InvalidConfig(message))
    }
}

fn check_nonnegative_finite(value: f64, message: &'static str) -> Result<(), RangeError> {
    if value.is_finite() && value >= 0.0 {
        Ok(())
    } else {
        Err(RangeError::InvalidConfig(message))
    }
}

fn check_ordered_pair(good: f64, bad: f64, message: &'static str) -> Result<(), RangeError> {
    if good.is_finite() && bad.is_finite() && good < bad {
        Ok(())
    } else {
        Err(RangeError::InvalidConfig(message))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn defaults_are_valid() {
        assert!(RangeDetectorConfig::default().validate().is_ok());
    }

    #[test]
    fn rejects_invalid_configurations() {
        let cases = vec![
            RangeDetectorConfig {
                windows: Vec::new(),
                ..RangeDetectorConfig::default()
            },
            RangeDetectorConfig {
                windows: vec![1],
                ..RangeDetectorConfig::default()
            },
            RangeDetectorConfig {
                lower_quantile: 0.9,
                upper_quantile: 0.1,
                ..RangeDetectorConfig::default()
            },
            RangeDetectorConfig {
                candidate_threshold: 0.9,
                ..RangeDetectorConfig::default()
            },
            RangeDetectorConfig {
                tail_fraction: 0.0,
                ..RangeDetectorConfig::default()
            },
            RangeDetectorConfig {
                drift_good: 0.2,
                drift_bad: 0.2,
                ..RangeDetectorConfig::default()
            },
        ];
        assert!(cases.into_iter().all(|case| case.validate().is_err()));
    }
}
