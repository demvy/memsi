use app_config::SecretRef;
use notification_api::NotificationError;
use serde::Deserialize;
#[derive(Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TelegramConfig {
    pub token: SecretRef,
    pub chat_id: String,
    pub message_thread_id: Option<i64>,
}
impl TelegramConfig {
    pub fn validate(&self) -> Result<(), NotificationError> {
        if !self.chat_id.parse::<i64>().is_ok_and(|id| id != 0)
            || self.message_thread_id.is_some_and(|id| id <= 0)
        {
            return Err(NotificationError::InvalidPayload);
        }
        Ok(())
    }
}
