#![forbid(unsafe_code)]
use domain::InstanceId;
use strategy_api::{StrategyError, StrategyMode};
pub use strategy_range::{RangeStrategy, StrategyConfig, TrendBinding};

pub fn descriptor() -> plugin_api::PluginDescriptor {
    strategy_range::descriptor(strategy_range::Profile::High)
}

pub fn build(
    instance: InstanceId,
    mode: StrategyMode,
    config: StrategyConfig,
) -> Result<RangeStrategy, StrategyError> {
    RangeStrategy::new(instance, mode, strategy_range::Profile::High, config)
}

#[cfg(test)]
mod tests;
