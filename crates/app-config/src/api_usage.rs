use crate::ConfigError;
use serde::{Deserialize, Serialize};
/// Local accounting; calendar periods are explicitly configured, not inferred from 429.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ApiUsageConfig {
    pub monthly_limit: u64,
    pub warn_at: u64,
    pub reset_day: u8,
    pub database: String,
    pub initial_period: Option<String>,
    pub initial_used: std::collections::BTreeMap<String, u64>,
}
impl Default for ApiUsageConfig {
    fn default() -> Self {
        Self {
            monthly_limit: 10_000,
            warn_at: 9_000,
            reset_day: 1,
            database: "api-usage.sqlite".into(),
            initial_period: None,
            initial_used: Default::default(),
        }
    }
}
impl ApiUsageConfig {
    pub fn validate(&self) -> Result<(), ConfigError> {
        if self.monthly_limit == 0
            || self.monthly_limit > i64::MAX as u64
            || self.warn_at == 0
            || self.warn_at >= self.monthly_limit
            || !(1..=31).contains(&self.reset_day)
            || self
                .initial_period
                .as_ref()
                .is_some_and(|period| !valid_period(period))
            || self.database.is_empty()
            || (!self.initial_used.is_empty() && self.initial_period.is_none())
            || self.initial_used.iter().any(|(name, used)| {
                name.is_empty()
                    || !name.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'_')
                    || *used > self.monthly_limit
            })
        {
            return Err(ConfigError::Validation(
                "API usage limits/reset/database".into(),
            ));
        }
        Ok(())
    }
}
fn valid_period(period: &str) -> bool {
    let parts: Vec<_> = period.split('-').collect();
    if parts.len() != 3 || parts[0].len() != 4 || parts[1].len() != 2 || parts[2].len() != 2 {
        return false;
    }
    let (Ok(year), Ok(month), Ok(day)) = (
        parts[0].parse::<u16>(),
        parts[1].parse::<u8>(),
        parts[2].parse::<u8>(),
    ) else {
        return false;
    };
    let days = match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if year.is_multiple_of(4) && (!year.is_multiple_of(100) || year.is_multiple_of(400)) => {
            29
        }
        2 => 28,
        _ => return false,
    };
    year > 0 && (1..=days).contains(&day)
}
