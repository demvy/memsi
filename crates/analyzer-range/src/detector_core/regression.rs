/// OLS slope for observations indexed from zero.
pub(crate) fn ols_slope(values: &[f64]) -> f64 {
    debug_assert!(values.len() > 1);
    let mean_t = (values.len() - 1) as f64 / 2.0;
    let mean_y = values.iter().sum::<f64>() / values.len() as f64;
    let (numerator, denominator) =
        values
            .iter()
            .enumerate()
            .fold((0.0, 0.0), |(numerator, denominator), (index, value)| {
                let centered_t = index as f64 - mean_t;
                (
                    numerator + centered_t * (value - mean_y),
                    denominator + centered_t * centered_t,
                )
            });
    numerator / denominator
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slope_matches_linear_series() {
        assert!((ols_slope(&[1.0, 3.0, 5.0, 7.0]) - 2.0).abs() < 1e-12);
        assert!(ols_slope(&[4.0, 4.0, 4.0]).abs() < 1e-12);
    }
}
