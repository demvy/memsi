#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RangeState {
    InsufficientData,
    NoRange,
    RangeCandidate,
    RangeConfirmed,
}
#[derive(Clone, Debug)]
pub struct RangeDetection {
    pub state: RangeState,
    pub confidence: f64,
    pub range: Option<DetectedRange>,
    pub diagnostics: RangeDiagnostics,
}
#[derive(Clone, Debug)]
pub struct DetectedRange {
    pub lower: f64,
    pub upper: f64,
    pub center: f64,
    pub width_pct: f64,
    pub age_bars: usize,
    pub latest_position: f64,
    pub latest_inside: bool,
    pub outside_streak: usize,
    pub confidence: f64,
}
#[derive(Clone, Debug, Default)]
pub struct RangeDiagnostics {
    pub selected_window: Option<usize>,
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
}
#[derive(Clone, Copy, Debug)]
pub struct Candle {
    pub timestamp_ms: i64,
    pub open: f64,
    pub high: f64,
    pub low: f64,
    pub close: f64,
    pub volume: f64,
}
