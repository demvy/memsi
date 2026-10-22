#![forbid(unsafe_code)]
//! A small, deterministic detector for current horizontal price ranges.

mod candidate;
mod config;
mod detector;
mod error;
mod metrics;
mod quantile;
mod regression;
mod scratch;
mod types;

pub use config::RangeDetectorConfig;
pub use detector::RangeDetector;
pub use error::RangeError;
pub use scratch::RangeDetectorScratch;
pub use types::{DetectedRange, RangeDetection, RangeDiagnostics, RangeState};
