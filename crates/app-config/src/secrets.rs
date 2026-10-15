use crate::{ConfigError, local_env};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct SecretRef(String);
impl SecretRef {
    pub fn new(value: String) -> Result<Self, ConfigError> {
        let key = value
            .strip_prefix("env:")
            .ok_or_else(|| ConfigError::Validation("secret must be env:NAME reference".into()))?;
        if key.is_empty()
            || key.len() > 128
            || !key
                .bytes()
                .all(|b| b.is_ascii_uppercase() || b.is_ascii_digit() || b == b'_')
        {
            return Err(ConfigError::Validation("invalid secret reference".into()));
        }
        Ok(Self(value))
    }
    pub fn resolve(&self) -> Result<String, ConfigError> {
        local_env(&self.0[4..])
            .filter(|v| !v.trim().is_empty())
            .ok_or_else(|| ConfigError::Validation(format!("missing secret reference {}", self.0)))
    }
}
impl TryFrom<String> for SecretRef {
    type Error = ConfigError;
    fn try_from(v: String) -> Result<Self, Self::Error> {
        Self::new(v)
    }
}
impl From<SecretRef> for String {
    fn from(v: SecretRef) -> String {
        v.0
    }
}
