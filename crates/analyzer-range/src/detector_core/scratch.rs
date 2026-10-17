/// Reusable working memory for repeated detector calls.
#[derive(Clone, Debug, Default)]
pub struct RangeDetectorScratch {
    pub(crate) log_prices: Vec<f64>,
    pub(crate) quantile_buf: Vec<f64>,
}

impl RangeDetectorScratch {
    pub fn new() -> Self {
        Self::default()
    }
}
