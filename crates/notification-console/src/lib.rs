#![forbid(unsafe_code)]
use domain::*;
use notification_api::*;
use std::{
    io::Write,
    time::{SystemTime, UNIX_EPOCH},
};
#[derive(Default, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ConsoleConfig {}
pub struct ConsoleSink {
    id: InstanceId,
}
impl ConsoleSink {
    pub fn new(id: InstanceId, _: ConsoleConfig) -> Self {
        Self { id }
    }
}
pub fn descriptor() -> plugin_api::PluginDescriptor {
    plugin_api::PluginDescriptor {
        id: PluginId::new("notification.console").unwrap(),
        version: env!("CARGO_PKG_VERSION").into(),
        kind: plugin_api::PluginKind::NotificationSink,
        provides: vec![],
    }
}
#[async_trait::async_trait]
impl NotificationSink for ConsoleSink {
    fn id(&self) -> &InstanceId {
        &self.id
    }
    async fn send_notice(
        &self,
        text: &str,
        ctx: &provider_api::RequestContext,
    ) -> Result<DeliveryReceipt, NotificationError> {
        ctx.check().map_err(|_| NotificationError::Cancelled)?;
        write_notice(text)
    }
    async fn send(
        &self,
        notification: &Notification,
        ctx: &provider_api::RequestContext,
    ) -> Result<DeliveryReceipt, NotificationError> {
        ctx.check().map_err(|_| NotificationError::Cancelled)?;
        if notification.destination != "stdout" {
            return Err(NotificationError::ForbiddenDestination);
        }
        let text = notification_render::summary_text(notification);
        write_notice(&text)
    }
}
fn write_notice(text: &str) -> Result<DeliveryReceipt, NotificationError> {
    std::io::stdout()
        .lock()
        .write_all(text.as_bytes())
        .map_err(|_| NotificationError::UnknownOutcome)?;
    let millis = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| NotificationError::UnknownOutcome)?
        .as_millis();
    let accepted_at =
        TimestampMs::new(i64::try_from(millis).map_err(|_| NotificationError::UnknownOutcome)?)
            .map_err(|_| NotificationError::UnknownOutcome)?;
    Ok(DeliveryReceipt {
        remote_message_id: None,
        accepted_at,
    })
}
