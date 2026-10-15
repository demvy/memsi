use app_config::SecretRef;
use provider_api::ProviderError;
use serde::{Deserialize, Serialize};
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CodexConfig {
    pub api_key: SecretRef,
    #[serde(default)]
    pub fallback_api_keys: Vec<SecretRef>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub usage: Option<app_config::ApiUsageConfig>,
    /// Explicit provider-specific network identifier. Not an EVM chain identifier.
    pub network_id: i32,
    #[serde(default = "default_concurrency")]
    pub concurrency: usize,
    #[serde(default = "default_interval")]
    pub min_interval_ms: u64,
}
fn default_concurrency() -> usize {
    2
}
fn default_interval() -> u64 {
    500
}
impl CodexConfig {
    pub fn validate(&self) -> Result<(), ProviderError> {
        if let Some(usage) = &self.usage {
            usage
                .validate()
                .map_err(|_| ProviderError::InvalidRequest)?;
        }
        if self.fallback_api_keys.len() > 8
            || self.network_id <= 0
            || self.concurrency == 0
            || self.concurrency > 8
            || self.min_interval_ms < 100
            || self.min_interval_ms > 60000
        {
            return Err(ProviderError::InvalidRequest);
        }
        Ok(())
    }
}
