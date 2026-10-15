#![forbid(unsafe_code)]
mod analyzer;
mod config;
pub mod detector_core;
pub use analyzer::RangeAnalyzer;
pub use config::RangeConfig;
