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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn destination_validation_accepts_nonzero_chat_and_positive_thread_ids() {
        for chat in ["1", "-100123", "9223372036854775807"] {
            for thread in [None, Some(1)] {
                let config = TelegramConfig {
                    token: SecretRef::new("env:TEST_TOKEN".into()).unwrap(),
                    chat_id: chat.into(),
                    message_thread_id: thread,
                };
                assert!(config.validate().is_ok());
            }
        }

        for (chat, thread) in [
            ("0", None),
            ("", None),
            ("@name", None),
            ("9223372036854775808", None),
            ("1", Some(0)),
            ("1", Some(-1)),
        ] {
            let config = TelegramConfig {
                token: SecretRef::new("env:TEST_TOKEN".into()).unwrap(),
                chat_id: chat.into(),
                message_thread_id: thread,
            };
            assert_eq!(config.validate(), Err(NotificationError::InvalidPayload));
        }
    }
}
