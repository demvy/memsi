#![forbid(unsafe_code)]
use domain::*;
use serde::{Deserialize, Serialize};
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RangeState {
    InsufficientData,
    NoRange,
    Candidate,
    Confirmed,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RangeBounds {
    pub lower: PositivePrice,
    pub upper: PositivePrice,
    pub center: PositivePrice,
    pub width_pct: NonNegative,
    pub age_bars: usize,
    pub latest_position: f64,
    /// Price-linear display metrics; absent in historical artifacts.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub position: Option<Box<RangePosition>>,
    pub latest_inside: bool,
    pub latest_inside_bounds: bool,
    pub outside_streak: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RangeZone {
    Below,
    LowerThird,
    MiddleThird,
    UpperThird,
    Above,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RangePosition {
    pub current_price: PositivePrice,
    pub raw_pct: f64,
    pub clamped_pct: f64,
    pub zone: RangeZone,
}

impl RangePosition {
    pub fn new(lower: f64, upper: f64, price: f64) -> Option<Self> {
        if !lower.is_finite() || !upper.is_finite() || lower <= 0.0 || upper <= lower {
            return None;
        }

        let current_price = PositivePrice::new(price).ok()?;
        let raw_pct = ((price - lower) / (upper - lower)) * 100.0;
        if !raw_pct.is_finite() {
            return None;
        }

        let zone = if raw_pct < 0.0 {
            RangeZone::Below
        } else if raw_pct <= 100.0 / 3.0 {
            RangeZone::LowerThird
        } else if raw_pct <= 200.0 / 3.0 {
            RangeZone::MiddleThird
        } else if raw_pct <= 100.0 {
            RangeZone::UpperThird
        } else {
            RangeZone::Above
        };

        Some(Self {
            current_price,
            raw_pct,
            clamped_pct: raw_pct.clamp(0.0, 100.0),
            zone,
        })
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RangeAnalysis {
    pub state: RangeState,
    pub range: Option<RangeBounds>,
    pub confidence: UnitInterval,
    pub price_unit: PriceUnit,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ActivityState {
    Inactive,
    Normal,
    Elevated,
    Unknown,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct VolumeAnalysis {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub robust: Option<Box<RobustVolumeAnalysis>>,
    pub recent_mean: NonNegative,
    pub baseline_mean: NonNegative,
    pub relative_volume: Option<NonNegative>,
    pub activity_fraction: UnitInterval,
    pub last_volume_percentile: UnitInterval,
    pub unit: VolumeUnit,
    pub recent_bars: usize,
    pub baseline_bars: usize,
    pub state: ActivityState,
}

/// Opt-in median metrics; legacy mean fields retain their original semantics.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RobustVolumeAnalysis {
    pub baseline_median: NonNegative,
    pub recent_median: NonNegative,
    pub relative_volume: Option<NonNegative>,
    pub short_slope: f64,
    pub long_slope: f64,
    pub state: ActivityState,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ArtifactProvenance {
    pub schema: SchemaId,
    pub schema_major: u16,
    pub producer: PluginId,
    pub producer_version: String,
    pub analyzer_instance: InstanceId,
    pub config_digest: ContentHash,
    pub input_hash: ContentHash,
    pub as_of: TimestampMs,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Artifact<T> {
    pub provenance: ArtifactProvenance,
    pub value: T,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TrendDirection {
    Up,
    Down,
    Sideways,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TrendAnalysis {
    pub slope_log_per_bar: f64,
    pub r_squared: UnitInterval,
    pub direction: TrendDirection,
    pub window: usize,
    pub resolution: Resolution,
    pub price_unit: PriceUnit,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn range_positions_classify_boundaries_and_clamp_only_display_percentages() {
        for (price, percentage, zone) in [
            (5.0, -50.0, RangeZone::Below),
            (10.0, 0.0, RangeZone::LowerThird),
            (12.0, 20.0, RangeZone::LowerThird),
            (15.0, 50.0, RangeZone::MiddleThird),
            (18.0, 80.0, RangeZone::UpperThird),
            (20.0, 100.0, RangeZone::UpperThird),
            (25.0, 150.0, RangeZone::Above),
        ] {
            let position = RangePosition::new(10.0, 20.0, price).unwrap();
            assert_eq!(position.current_price.get(), price);
            assert_eq!(position.raw_pct, percentage);
            assert_eq!(position.clamped_pct, percentage.clamp(0.0, 100.0));
            assert_eq!(position.zone, zone);
        }

        assert_eq!(
            RangePosition::new(1.0, 4.0, 2.0).unwrap().zone,
            RangeZone::LowerThird
        );
        assert_eq!(
            RangePosition::new(1.0, 4.0, 3.0).unwrap().zone,
            RangeZone::MiddleThird
        );
    }

    #[test]
    fn range_positions_reject_invalid_bounds_prices_and_numeric_overflow() {
        for (lower, upper, price) in [
            (0.0, 2.0, 1.0),
            (-1.0, 2.0, 1.0),
            (2.0, 2.0, 2.0),
            (3.0, 2.0, 2.0),
            (f64::NAN, 2.0, 1.0),
            (1.0, f64::INFINITY, 1.0),
            (1.0, 2.0, 0.0),
            (1.0, 2.0, f64::NAN),
            (1.0, 2.0, f64::INFINITY),
            (1.0, 2.0, f64::MAX),
        ] {
            assert!(RangePosition::new(lower, upper, price).is_none());
        }
    }
}
