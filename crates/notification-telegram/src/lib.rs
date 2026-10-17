#![forbid(unsafe_code)]
mod config;
mod response;
pub use config::TelegramConfig;
use domain::*;
use notification_api::*;
use provider_api::RequestContext;
use serde_json::json;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use tokio::sync::Mutex;
pub struct TelegramSink {
    id: InstanceId,
    config: TelegramConfig,
    client: reqwest::Client,
    next_send: Mutex<Instant>,
}

pub fn descriptor() -> plugin_api::PluginDescriptor {
    plugin_api::PluginDescriptor {
        id: PluginId::new("notification.telegram").unwrap(),
        version: env!("CARGO_PKG_VERSION").into(),
        kind: plugin_api::PluginKind::NotificationSink,
        provides: vec![],
    }
}

impl TelegramSink {
    pub fn new(id: InstanceId, config: TelegramConfig) -> Result<Self, NotificationError> {
        config.validate()?;
        let client = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .timeout(Duration::from_secs(10))
            .build()
            .map_err(|_| NotificationError::Transport)?;
        Ok(Self {
            id,
            config,
            client,
            next_send: Mutex::new(Instant::now()),
        })
    }

    pub fn destination(&self) -> &str {
        &self.config.chat_id
    }

    async fn attempt(
        &self,
        text: String,
        html: bool,
    ) -> Result<DeliveryReceipt, NotificationError> {
        let mut next = self.next_send.lock().await;
        tokio::time::sleep_until((*next).into()).await;
        *next = Instant::now() + Duration::from_secs(1);
        let token = self
            .config
            .token
            .resolve()
            .map_err(|_| NotificationError::Unauthorized)?;
        if token.len() > 256
            || !token
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || b":_-".contains(&byte))
            || !token.contains(':')
        {
            return Err(NotificationError::Unauthorized);
        }

        let payload = if html {
            self.html_payload(text)
        } else {
            self.payload(text)
        };

        self.post(
            &format!("https://api.telegram.org/bot{token}/sendMessage"),
            &payload,
        )
        .await
    }

    async fn deliver_text(
        &self,
        text: String,
        ctx: &RequestContext,
        html: bool,
    ) -> Result<DeliveryReceipt, NotificationError> {
        ctx.check().map_err(|_| NotificationError::Cancelled)?;
        let work = self.attempt(text, html);
        tokio::pin!(work);
        loop {
            tokio::select! {
                result = &mut work => return result,
                _ = tokio::time::sleep(Duration::from_millis(25)) => {
                    if ctx.check().is_err() {
                        return Err(NotificationError::UnknownOutcome);
                    }
                }
            }
        }
    }

    fn payload(&self, text: String) -> serde_json::Value {
        let mut payload = json!({
            "chat_id": self.config.chat_id,
            "text": text,
            "link_preview_options": {
                "is_disabled": true
            }
        });
        if let Some(thread) = self.config.message_thread_id {
            payload["message_thread_id"] = json!(thread);
        }

        payload
    }

    fn html_payload(&self, text: String) -> serde_json::Value {
        let mut payload = self.payload(text);
        payload["parse_mode"] = json!("HTML");
        payload
    }

    async fn post(
        &self,
        endpoint: &str,
        payload: &serde_json::Value,
    ) -> Result<DeliveryReceipt, NotificationError> {
        let mut response = self
            .client
            .post(endpoint)
            .json(payload)
            .send()
            .await
            .map_err(|error| {
                if error.is_connect() {
                    NotificationError::Transport
                } else {
                    NotificationError::UnknownOutcome
                }
            })?;
        let status = response.status().as_u16();
        let mut bytes = Vec::new();
        while let Some(chunk) = response
            .chunk()
            .await
            .map_err(|_| NotificationError::UnknownOutcome)?
        {
            if bytes.len() + chunk.len() > 65_536 {
                return Err(NotificationError::UnknownOutcome);
            }

            bytes.extend_from_slice(&chunk);
        }

        let message_id = response::message_id(status, &bytes)?;
        let millis = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|_| NotificationError::UnknownOutcome)?
            .as_millis();
        let accepted_at =
            TimestampMs::new(i64::try_from(millis).map_err(|_| NotificationError::UnknownOutcome)?)
                .map_err(|_| NotificationError::UnknownOutcome)?;
        Ok(DeliveryReceipt {
            remote_message_id: Some(message_id),
            accepted_at,
        })
    }
}

#[async_trait::async_trait]
impl NotificationSink for TelegramSink {
    fn id(&self) -> &InstanceId {
        &self.id
    }

    async fn send_notice(
        &self,
        text: &str,
        ctx: &RequestContext,
    ) -> Result<DeliveryReceipt, NotificationError> {
        if text.chars().count() > 4096 {
            return Err(NotificationError::InvalidPayload);
        }

        self.deliver_text(text.to_owned(), ctx, false).await
    }

    async fn send(
        &self,
        notification: &Notification,
        ctx: &RequestContext,
    ) -> Result<DeliveryReceipt, NotificationError> {
        ctx.check().map_err(|_| NotificationError::Cancelled)?;
        if notification.destination != self.config.chat_id {
            return Err(NotificationError::ForbiddenDestination);
        }

        if notification.part_index != 0 {
            return Err(NotificationError::InvalidPayload);
        }

        self.deliver_text(notification_render::telegram_html(notification), ctx, true)
            .await
    }
}

#[cfg(test)]
mod tests;
