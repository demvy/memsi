/// Hyndman-Fan type 7 quantile: linear interpolation at `q * (n - 1)`.
pub(crate) fn quantile(values: &[f64], q: f64, buffer: &mut Vec<f64>) -> f64 {
    debug_assert!(!values.is_empty());
    debug_assert!((0.0..=1.0).contains(&q));
    buffer.clear();
    buffer.extend_from_slice(values);
    buffer.sort_by(f64::total_cmp);
    let rank = q * (buffer.len() - 1) as f64;
    let lower = rank.floor() as usize;
    let upper = rank.ceil() as usize;
    let fraction = rank - lower as f64;
    buffer[lower] + fraction * (buffer[upper] - buffer[lower])
}
#[derive(Clone, Copy, Debug)]
pub(crate) struct Band {
    pub lower: f64,
    pub center: f64,
    pub upper: f64,
}
impl Band {
    pub fn width(self) -> f64 {
        self.upper - self.lower
    }
}
pub(crate) fn band(values: &[f64], lower_q: f64, upper_q: f64, buffer: &mut Vec<f64>) -> Band {
    Band {
        lower: quantile(values, lower_q, buffer),
        center: quantile(values, 0.5, buffer),
        upper: quantile(values, upper_q, buffer),
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn type_seven_quantiles_are_interpolated_deterministically() {
        let mut buffer = Vec::new();
        let values = [0.0, 10.0, 20.0, 30.0, 40.0];
        assert_eq!(quantile(&values, 0.0, &mut buffer), 0.0);
        assert_eq!(quantile(&values, 0.1, &mut buffer), 4.0);
        assert_eq!(quantile(&values, 0.5, &mut buffer), 20.0);
        assert_eq!(quantile(&values, 0.9, &mut buffer), 36.0);
        assert_eq!(quantile(&values, 1.0, &mut buffer), 40.0);
    }
    #[test]
    fn quantile_does_not_depend_on_input_order() {
        let mut buffer = Vec::new();
        assert_eq!(quantile(&[4.0, 1.0, 3.0, 2.0], 0.5, &mut buffer), 2.5);
    }
}
