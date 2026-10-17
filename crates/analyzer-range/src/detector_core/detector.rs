use crate::detector_core::{
    Candle, DetectedRange, RangeDetection, RangeDetectorConfig, RangeDetectorScratch,
    RangeDiagnostics, RangeError, RangeState,
    candidate::{CandidateRange, evaluate_candidate},
    metrics::EPSILON,
};

pub trait DetectRange {
    fn detect_closes(&self, closes: &[f64]) -> Result<RangeDetection, RangeError>;
}

/// Immutable, shareable detector configured for a particular bar cadence.
#[derive(Clone, Debug)]
pub struct RangeDetector {
    config: RangeDetectorConfig,
}

impl RangeDetector {
    pub fn new(mut config: RangeDetectorConfig) -> Result<Self, RangeError> {
        config.validate()?;
        config.windows.sort_unstable();
        config.windows.dedup();
        Ok(Self { config })
    }

    pub fn detect_closes(&self, closes: &[f64]) -> Result<RangeDetection, RangeError> {
        self.detect_closes_with_scratch(closes, &mut RangeDetectorScratch::default())
    }

    pub fn detect_closes_with_scratch(
        &self,
        closes: &[f64],
        scratch: &mut RangeDetectorScratch,
    ) -> Result<RangeDetection, RangeError> {
        validate_closes(closes)?;
        if closes.len() < self.config.min_bars {
            return Ok(empty_detection(RangeState::InsufficientData, 0.0));
        }

        let max_window = *self.config.windows.last().expect("validated windows");
        let retained = &closes[closes.len().saturating_sub(max_window)..];
        preprocess_closes(retained, &mut scratch.log_prices);
        let mut candidates = Vec::with_capacity(self.config.windows.len());
        for &window in &self.config.windows {
            if window > scratch.log_prices.len() {
                continue;
            }

            let start = scratch.log_prices.len() - window;
            let candidate = evaluate_candidate(
                &scratch.log_prices[start..],
                &self.config,
                &mut scratch.quantile_buf,
            );
            if !candidate.hard_rejected {
                candidates.push(candidate);
            }
        }

        let Some(max_quality) = candidates
            .iter()
            .map(|candidate| candidate.quality)
            .reduce(f64::max)
        else {
            return Ok(empty_detection(RangeState::NoRange, 0.0));
        };

        let near_best_floor = max_quality - self.config.near_best_tolerance;
        if let Some(selected) = longest_qualifying(
            &candidates,
            self.config.confirmed_threshold,
            near_best_floor,
        ) {
            return Ok(public_detection(selected, RangeState::RangeConfirmed));
        }

        if let Some(selected) = longest_qualifying(
            &candidates,
            self.config.candidate_threshold,
            near_best_floor,
        ) {
            return Ok(public_detection(selected, RangeState::RangeCandidate));
        }

        Ok(empty_detection(RangeState::NoRange, max_quality))
    }

    pub fn detect_candles(&self, candles: &[Candle]) -> Result<RangeDetection, RangeError> {
        let closes: Vec<_> = candles.iter().map(|candle| candle.close).collect();
        self.detect_closes(&closes)
    }
}

impl DetectRange for RangeDetector {
    fn detect_closes(&self, closes: &[f64]) -> Result<RangeDetection, RangeError> {
        RangeDetector::detect_closes(self, closes)
    }
}

fn validate_closes(closes: &[f64]) -> Result<(), RangeError> {
    if closes.is_empty() {
        return Err(RangeError::EmptyInput);
    }

    for (index, &value) in closes.iter().enumerate() {
        if !value.is_finite() {
            return Err(RangeError::NonFinitePrice { index, value });
        }

        if value <= 0.0 {
            return Err(RangeError::NonPositivePrice { index, value });
        }
    }

    Ok(())
}

fn preprocess_closes(closes: &[f64], output: &mut Vec<f64>) {
    output.clear();
    output.reserve(closes.len());
    output.extend(closes.iter().map(|price| price.ln()));
}

fn longest_qualifying(
    candidates: &[CandidateRange],
    threshold: f64,
    near_best_floor: f64,
) -> Option<&CandidateRange> {
    candidates
        .iter()
        .filter(|candidate| candidate.quality >= threshold && candidate.quality >= near_best_floor)
        .max_by_key(|candidate| candidate.window)
}

fn public_detection(candidate: &CandidateRange, state: RangeState) -> RangeDetection {
    let latest_position =
        (candidate.latest_log - candidate.lower_log) / candidate.width_log.max(EPSILON);
    let confidence = candidate.quality.clamp(0.0, 1.0);
    RangeDetection {
        state,
        confidence,
        range: Some(DetectedRange {
            lower: candidate.lower_log.exp(),
            upper: candidate.upper_log.exp(),
            center: candidate.center_log.exp(),
            width_pct: candidate.width_log.exp() - 1.0,
            age_bars: candidate.window,
            latest_position,
            latest_inside: (candidate.accepted_lower..=candidate.accepted_upper)
                .contains(&candidate.latest_log),
            outside_streak: candidate.outside_streak,
            confidence,
        }),
        diagnostics: RangeDiagnostics {
            selected_window: Some(candidate.window),
            normalized_drift: candidate.normalized_drift,
            horizontality_score: candidate.horizontality_score,
            efficiency_ratio: candidate.efficiency_ratio,
            mean_reversion_score: candidate.mean_reversion_score,
            traversals: candidate.traversals,
            traversal_score: candidate.traversal_score,
            boundary_instability: candidate.boundary_instability,
            boundary_stability_score: candidate.boundary_stability_score,
            tail_acceptance: candidate.tail_acceptance,
            tail_acceptance_score: candidate.tail_acceptance_score,
            outside_streak: candidate.outside_streak,
            occupancy_entropy: candidate.occupancy_entropy,
        },
    }
}

fn empty_detection(state: RangeState, confidence: f64) -> RangeDetection {
    RangeDetection {
        state,
        confidence: confidence.clamp(0.0, 1.0),
        range: None,
        diagnostics: RangeDiagnostics::default(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn validates_input_before_insufficient_data() {
        let detector = RangeDetector::new(RangeDetectorConfig::default()).unwrap();
        assert!(matches!(
            detector.detect_closes(&[]),
            Err(RangeError::EmptyInput)
        ));
        assert!(matches!(
            detector.detect_closes(&[1.0, f64::NAN]),
            Err(RangeError::NonFinitePrice { index: 1, .. })
        ));
        assert!(matches!(
            detector.detect_closes(&[1.0, 0.0]),
            Err(RangeError::NonPositivePrice { index: 1, .. })
        ));
        assert_eq!(
            detector.detect_closes(&[1.0; 10]).unwrap().state,
            RangeState::InsufficientData
        );
    }

    #[test]
    fn windows_are_sorted_and_deduplicated() {
        let config = RangeDetectorConfig {
            windows: vec![54, 24, 54],
            ..RangeDetectorConfig::default()
        };

        let detector = RangeDetector::new(config).unwrap();
        assert_eq!(detector.config.windows, [24, 54]);
    }

    #[test]
    fn preprocessing_converts_prices_to_natural_logs() {
        let mut output = vec![99.0];
        preprocess_closes(&[1.0, std::f64::consts::E], &mut output);
        assert_eq!(output.len(), 2);
        assert_eq!(output[0], 0.0);
        assert!((output[1] - 1.0).abs() < 1e-12);
    }
}
