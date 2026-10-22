use crate::detector_core::{
    RangeDetectorConfig,
    metrics::{
        boundary_instability, efficiency_ratio, higher_is_better, lower_is_better,
        normalized_drift, occupancy_entropy, outside_streak, tail_acceptance, traversal_count,
    },
    quantile::{self, Band},
};

#[derive(Clone, Debug)]
pub(crate) struct CandidateRange {
    pub window: usize,
    pub lower_log: f64,
    pub center_log: f64,
    pub upper_log: f64,
    pub width_log: f64,
    pub latest_log: f64,
    pub normalized_drift: f64,
    pub horizontality_score: f64,
    pub efficiency_ratio: f64,
    pub mean_reversion_score: f64,
    pub traversals: usize,
    pub traversal_score: f64,
    pub boundary_instability: f64,
    pub boundary_stability_score: f64,
    pub tail_acceptance: f64,
    pub tail_acceptance_score: f64,
    pub outside_streak: usize,
    pub occupancy_entropy: f64,
    pub accepted_lower: f64,
    pub accepted_upper: f64,
    pub quality: f64,
    pub hard_rejected: bool,
}

pub(crate) fn evaluate_candidate(
    values: &[f64],
    config: &RangeDetectorConfig,
    quantile_buffer: &mut Vec<f64>,
) -> CandidateRange {
    let Band {
        lower,
        center,
        upper,
    } = quantile::band(
        values,
        config.lower_quantile,
        config.upper_quantile,
        quantile_buffer,
    );
    let width = upper - lower;
    let margin = config.acceptance_margin * width;
    let accepted_lower = lower - margin;
    let accepted_upper = upper + margin;
    let drift = normalized_drift(values, width);
    let horizontality_score = lower_is_better(drift, config.drift_good, config.drift_bad);
    let efficiency_ratio = efficiency_ratio(values);
    let mean_reversion_score = lower_is_better(efficiency_ratio, config.er_good, config.er_bad);
    let traversals = traversal_count(values, center, width, config.traversal_zone);
    let traversal_score = (traversals as f64 / config.traversal_target as f64).clamp(0.0, 1.0);
    let instability = boundary_instability(
        values,
        width,
        config.lower_quantile,
        config.upper_quantile,
        quantile_buffer,
    );
    let boundary_stability_score =
        lower_is_better(instability, config.stability_good, config.stability_bad);
    let tail_acceptance =
        tail_acceptance(values, accepted_lower, accepted_upper, config.tail_fraction);
    let tail_acceptance_score = higher_is_better(
        tail_acceptance,
        config.tail_acceptance_bad,
        config.tail_acceptance_good,
    );
    let outside_streak = outside_streak(values, accepted_lower, accepted_upper);
    let occupancy_entropy = occupancy_entropy(values, lower, upper, config.occupancy_bins);
    let scores = [
        horizontality_score,
        mean_reversion_score,
        traversal_score,
        boundary_stability_score,
        tail_acceptance_score,
    ];
    let required_values = [
        lower,
        center,
        upper,
        width,
        drift,
        efficiency_ratio,
        instability,
        tail_acceptance,
        occupancy_entropy,
    ];
    let quality = (0.28 * scores[0].clamp(0.0, 1.0)
        + 0.22 * scores[1].clamp(0.0, 1.0)
        + 0.20 * scores[2].clamp(0.0, 1.0)
        + 0.18 * scores[3].clamp(0.0, 1.0)
        + 0.12 * scores[4].clamp(0.0, 1.0))
    .clamp(0.0, 1.0);
    let valid_band = lower <= center && center <= upper && width >= 0.0;
    let finite = required_values.into_iter().all(f64::is_finite)
        && scores.into_iter().all(f64::is_finite)
        && quality.is_finite();
    CandidateRange {
        window: values.len(),
        lower_log: lower,
        center_log: center,
        upper_log: upper,
        width_log: width,
        latest_log: values[values.len() - 1],
        normalized_drift: drift,
        horizontality_score,
        efficiency_ratio,
        mean_reversion_score,
        traversals,
        traversal_score,
        boundary_instability: instability,
        boundary_stability_score,
        tail_acceptance,
        tail_acceptance_score,
        outside_streak,
        occupancy_entropy,
        accepted_lower,
        accepted_upper,
        quality,
        hard_rejected: !valid_band || !finite || outside_streak > config.max_outside_streak,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn oscillation(length: usize) -> Vec<f64> {
        let pattern: [f64; 8] = [1.00, 1.04, 1.09, 1.05, 1.01, 1.03, 1.08, 1.04];
        (0..length)
            .map(|index| pattern[index % pattern.len()].ln())
            .collect()
    }

    #[test]
    fn evaluates_range_more_highly_than_trend() {
        let config = RangeDetectorConfig::default();
        let mut buffer = Vec::new();
        let range = evaluate_candidate(&oscillation(80), &config, &mut buffer);
        let trend: Vec<_> = (0..80)
            .map(|index| (1.0 + index as f64 * 0.01).ln())
            .collect();
        let trend = evaluate_candidate(&trend, &config, &mut buffer);
        assert!(range.quality >= config.confirmed_threshold);
        assert!(range.quality > trend.quality);
        assert!(trend.quality < config.candidate_threshold);
    }

    #[test]
    fn sustained_outside_closes_are_hard_rejected() {
        let config = RangeDetectorConfig::default();
        let mut values = oscillation(51);
        values.extend([1.20_f64.ln(), 1.25_f64.ln(), 1.30_f64.ln()]);
        let candidate = evaluate_candidate(&values, &config, &mut Vec::new());
        assert_eq!(candidate.outside_streak, 3);
        assert!(candidate.hard_rejected);
    }
}
