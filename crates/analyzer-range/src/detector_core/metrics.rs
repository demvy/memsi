use crate::detector_core::{quantile, regression};
pub(crate) const EPSILON: f64 = 1e-12;
#[inline]
pub(crate) fn lower_is_better(value: f64, good: f64, bad: f64) -> f64 {
    debug_assert!(good < bad);
    if value <= good {
        1.0
    } else if value >= bad {
        0.0
    } else {
        (1.0 - (value - good) / (bad - good)).clamp(0.0, 1.0)
    }
}

#[inline]
pub(crate) fn higher_is_better(value: f64, bad: f64, good: f64) -> f64 {
    debug_assert!(bad < good);
    if value <= bad {
        0.0
    } else if value >= good {
        1.0
    } else {
        ((value - bad) / (good - bad)).clamp(0.0, 1.0)
    }
}

pub(crate) fn normalized_drift(values: &[f64], width: f64) -> f64 {
    let slope = regression::ols_slope(values);
    slope.abs() * (values.len() - 1) as f64 / width.max(EPSILON)
}

pub(crate) fn efficiency_ratio(values: &[f64]) -> f64 {
    let path = values
        .windows(2)
        .map(|pair| (pair[1] - pair[0]).abs())
        .sum::<f64>();
    let displacement = (values[values.len() - 1] - values[0]).abs();
    displacement / path.max(EPSILON)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Side {
    Low,
    High,
}

pub(crate) fn traversal_count(values: &[f64], center: f64, width: f64, zone: f64) -> usize {
    let denominator = (width / 2.0).max(EPSILON);
    let mut last_side = None;
    let mut traversals = 0;
    for value in values {
        let position = (*value - center) / denominator;
        let side = if position <= -zone {
            Some(Side::Low)
        } else if position >= zone {
            Some(Side::High)
        } else {
            None
        };

        if let Some(side) = side {
            match last_side {
                None => last_side = Some(side),
                Some(previous) if previous != side => {
                    traversals += 1;
                    last_side = Some(side);
                }

                Some(_) => {}
            }
        }
    }

    traversals
}

pub(crate) fn boundary_instability(
    values: &[f64],
    full_width: f64,
    lower_q: f64,
    upper_q: f64,
    buffer: &mut Vec<f64>,
) -> f64 {
    let subwindow_len = (2 * values.len()).div_ceil(3);
    let first = quantile::band(&values[..subwindow_len], lower_q, upper_q, buffer);
    let last = quantile::band(
        &values[values.len() - subwindow_len..],
        lower_q,
        upper_q,
        buffer,
    );
    let denominator = full_width.max(EPSILON);
    let center_shift = (first.center - last.center).abs() / denominator;
    let width_shift = (first.width() - last.width()).abs() / denominator;
    center_shift + 0.5 * width_shift
}

pub(crate) fn tail_len(window: usize, fraction: f64) -> usize {
    ((window as f64 * fraction).round() as usize)
        .max(6)
        .min(window)
}

pub(crate) fn tail_acceptance(
    values: &[f64],
    accepted_lower: f64,
    accepted_upper: f64,
    fraction: f64,
) -> f64 {
    let length = tail_len(values.len(), fraction);
    let accepted = values[values.len() - length..]
        .iter()
        .filter(|&&value| (accepted_lower..=accepted_upper).contains(&value))
        .count();
    accepted as f64 / length as f64
}

pub(crate) fn outside_streak(values: &[f64], accepted_lower: f64, accepted_upper: f64) -> usize {
    values
        .iter()
        .rev()
        .take_while(|&&value| !(accepted_lower..=accepted_upper).contains(&value))
        .count()
}

pub(crate) fn occupancy_entropy(values: &[f64], lower: f64, upper: f64, bins: usize) -> f64 {
    let width = upper - lower;
    if width <= EPSILON {
        return 0.0;
    }

    let mut counts = vec![0_usize; bins];
    let mut included = 0_usize;
    for &value in values {
        if (lower..=upper).contains(&value) {
            let relative = ((value - lower) / width).clamp(0.0, 1.0);
            let index = ((relative * bins as f64) as usize).min(bins - 1);
            counts[index] += 1;
            included += 1;
        }
    }

    if included == 0 {
        return 0.0;
    }

    let entropy = counts
        .into_iter()
        .filter(|&count| count > 0)
        .map(|count| {
            let probability = count as f64 / included as f64;
            -probability * probability.ln()
        })
        .sum::<f64>();
    (entropy / (bins as f64).ln()).clamp(0.0, 1.0)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn score_helpers_interpolate_and_clamp() {
        assert_eq!(lower_is_better(0.1, 0.2, 0.8), 1.0);
        assert!((lower_is_better(0.5, 0.2, 0.8) - 0.5).abs() < 1e-12);
        assert_eq!(lower_is_better(0.9, 0.2, 0.8), 0.0);
        assert_eq!(higher_is_better(0.5, 0.6, 0.9), 0.0);
        assert!((higher_is_better(0.75, 0.6, 0.9) - 0.5).abs() < 1e-12);
        assert_eq!(higher_is_better(1.0, 0.6, 0.9), 1.0);
    }

    #[test]
    fn drift_is_normalized_by_width() {
        assert!((normalized_drift(&[0.0, 0.5, 1.0], 1.0) - 1.0).abs() < 1e-12);
        assert!(normalized_drift(&[2.0, 2.0, 2.0], 0.0).abs() < 1e-12);
    }

    #[test]
    fn efficiency_ratio_distinguishes_direction_from_return() {
        assert!((efficiency_ratio(&[0.0, 1.0, 2.0]) - 1.0).abs() < 1e-12);
        assert_eq!(efficiency_ratio(&[0.0, 1.0, 0.0]), 0.0);
        assert_eq!(efficiency_ratio(&[1.0, 1.0, 1.0]), 0.0);
    }

    #[test]
    fn traversals_use_hysteresis() {
        let values = [-1.0, 0.0, 0.1, 1.0, 0.0, -1.0, 1.0];
        assert_eq!(traversal_count(&values, 0.0, 2.0, 0.45), 3);
        assert_eq!(traversal_count(&[0.0, 0.1, -0.1], 0.0, 2.0, 0.45), 0);
    }

    #[test]
    fn instability_detects_shifted_subwindows() {
        let stable = [0.0, 1.0, 0.0, 1.0, 0.0, 1.0];
        let shifted = [0.0, 0.1, 0.0, 2.0, 2.1, 2.0];
        let mut buffer = Vec::new();
        assert!(boundary_instability(&stable, 1.0, 0.1, 0.9, &mut buffer) < 0.2);
        assert!(boundary_instability(&shifted, 2.0, 0.1, 0.9, &mut buffer) > 0.5);
    }

    #[test]
    fn tail_metrics_describe_current_acceptance() {
        let values = [0.0, 0.0, 0.0, 0.0, 2.0, 2.0];
        assert!((tail_acceptance(&values, -0.5, 0.5, 1.0) - 2.0 / 3.0).abs() < 1e-12);
        assert_eq!(outside_streak(&values, -0.5, 0.5), 2);
    }

    #[test]
    fn occupancy_entropy_is_normalized() {
        let uniform = [0.0, 0.25, 0.5, 0.75];
        assert!((occupancy_entropy(&uniform, 0.0, 1.0, 4) - 1.0).abs() < 1e-12);
        assert_eq!(occupancy_entropy(&[1.0; 4], 1.0, 1.0, 4), 0.0);
    }
}
