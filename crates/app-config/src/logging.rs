//! Resolved per-instance diagnostic policy, independent of strategy behaviour.
use crate::AppConfig;
use domain::InstanceId;

impl AppConfig {
    pub fn strategy_logging(&self, instance: &InstanceId) -> bool {
        self.strategies
            .iter()
            .find(|s| &s.id == instance)
            .is_none_or(|s| s.logging)
    }
}
