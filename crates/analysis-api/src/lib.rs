#![forbid(unsafe_code)]
pub mod identifiers {
    pub const RANGE_BINDING: &str = "range";
    pub const VOLUME_BINDING: &str = "volume";
    pub const TREND_BINDING: &str = "trend";
    pub const RANGE_SCHEMA: &str = "analysis.range";
    pub const VOLUME_SCHEMA: &str = "analysis.volume";
    pub const TREND_SCHEMA: &str = "analysis.trend";
    pub const SCHEMA_MAJOR: u16 = 1;
}

use domain::*;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CandleRequirement {
    pub resolution: Resolution,
    pub min_bars: usize,
    pub desired_bars: usize,
    pub price_unit: PriceUnit,
    pub require_volume: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AnalysisBinding {
    pub name: BindingId,
    pub instance: InstanceId,
    pub schema: SchemaId,
    pub major: u16,
    pub required: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AnalysisRequirements {
    pub candles: CandleRequirement,
}

pub struct AnalysisContext<'a> {
    pub candles: &'a CandleSeries,
    pub as_of: TimestampMs,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum AnalysisValue {
    Range(analysis_model::RangeAnalysis),
    Volume(analysis_model::VolumeAnalysis),
    Trend(analysis_model::TrendAnalysis),
}

#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum AnalysisError {
    #[error("invalid analysis input: {0}")]
    InvalidInput(String),
    #[error("insufficient history: {have}/{need}")]
    InsufficientHistory { have: usize, need: usize },
    #[error("unsupported or missing volume units")]
    UnsupportedUnits,
    #[error("missing dependency")]
    MissingDependency,
    #[error("numeric failure")]
    NumericFailure,
}

pub trait Analyzer: Send + Sync {
    /// Stateful analyzers must not share computed outputs across different instances.
    fn stateful(&self) -> bool {
        false
    }

    fn descriptor(&self) -> plugin_api::PluginDescriptor;
    fn instance(&self) -> &InstanceId;
    fn config_digest(&self) -> &ContentHash;
    fn requirements(&self, resolution: Resolution, price_unit: PriceUnit) -> AnalysisRequirements;
    fn analyze(&self, ctx: &AnalysisContext<'_>) -> Result<AnalysisValue, AnalysisError>;
}
