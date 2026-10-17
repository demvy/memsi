#![forbid(unsafe_code)]
use async_trait::async_trait;
use domain::*;
use serde::{Deserialize, Serialize};
use signal_api::{Signal, SignalLink};
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Notification {
    pub delivery_id: ContentHash,
    pub signal: Signal,
    pub links: Vec<SignalLink>,
    pub destination: String,
    pub part_index: u32,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DeliveryReceipt {
    pub remote_message_id: Option<String>,
    pub accepted_at: TimestampMs,
}

#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum NotificationError {
    #[error("rate limited: {retry_after_ms} ms")]
    RateLimited { retry_after_ms: u64 },
    #[error("unauthorized")]
    Unauthorized,
    #[error("forbidden destination")]
    ForbiddenDestination,
    #[error("invalid payload")]
    InvalidPayload,
    #[error("unknown remote outcome")]
    UnknownOutcome,
    #[error("transient transport error")]
    Transport,
    #[error("cancelled")]
    Cancelled,
}

#[async_trait]
pub trait NotificationSink: Send + Sync {
    fn id(&self) -> &InstanceId;
    /// Operational notices are distinct from strategy signals and their cooldowns.
    async fn send_notice(
        &self,
        _text: &str,
        _ctx: &provider_api::RequestContext,
    ) -> Result<DeliveryReceipt, NotificationError> {
        Err(NotificationError::InvalidPayload)
    }

    async fn send(
        &self,
        notification: &Notification,
        ctx: &provider_api::RequestContext,
    ) -> Result<DeliveryReceipt, NotificationError>;
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};

    struct SignalOnlySink(InstanceId);
    #[async_trait]
    impl NotificationSink for SignalOnlySink {
        fn id(&self) -> &InstanceId {
            &self.0
        }
        async fn send(
            &self,
            _: &Notification,
            _: &provider_api::RequestContext,
        ) -> Result<DeliveryReceipt, NotificationError> {
            Err(NotificationError::Transport)
        }
    }

    #[tokio::test]
    async fn default_notice_hook_rejects_unsupported_operational_messages() {
        let sink = SignalOnlySink(InstanceId::new("test").unwrap());
        let context = provider_api::RequestContext {
            priority: Default::default(),
            deadline: Instant::now() + Duration::from_secs(60),
            cancellation: Default::default(),
        };
        assert_eq!(
            sink.send_notice("notice", &context).await.unwrap_err(),
            NotificationError::InvalidPayload
        );
    }
}
