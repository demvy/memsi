use crate::RobustVolumeConfig;
use domain::NonNegative;
use serde::{Deserialize, Serialize};
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct VolumeConfig {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub robust: Option<RobustVolumeConfig>,
    pub recent_bars: usize,
    pub baseline_bars: usize,
    pub elevated_ratio: NonNegative,
}

impl Default for VolumeConfig {
    fn default() -> Self {
        Self {
            robust: None,
            recent_bars: 12,
            baseline_bars: 48,
            elevated_ratio: NonNegative::new(1.5).unwrap(),
        }
    }
}
