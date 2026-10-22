use super::*;
use provider_api::{Cancellation, RequestContext, RequestPriority};
use std::time::{Duration, Instant};
use test_support::signal as fixtures;

#[tokio::test]
async fn console_sink_preserves_identity_and_validates_destination_and_cancellation() {
    let id = InstanceId::new("console").unwrap();
    let sink = ConsoleSink::new(id.clone(), ConsoleConfig::default());
    assert_eq!(sink.id(), &id);
    assert_eq!(descriptor().id.as_str(), "notification.console");
    let context = RequestContext {
        priority: RequestPriority::default(),
        deadline: Instant::now() + Duration::from_secs(60),
        cancellation: Cancellation::default(),
    };
    let notification = Notification {
        delivery_id: ContentHash::of_bytes(b"delivery"),
        signal: fixtures::signal(),
        links: vec![],
        destination: "other".into(),
        part_index: 0,
    };
    assert_eq!(
        sink.send(&notification, &context).await.unwrap_err(),
        NotificationError::ForbiddenDestination
    );
    let receipt = sink.send_notice("", &context).await.unwrap();
    assert!(receipt.remote_message_id.is_none());
    assert!(receipt.accepted_at.get() > 0);
    context.cancellation.cancel();
    assert_eq!(
        sink.send(&notification, &context).await.unwrap_err(),
        NotificationError::Cancelled
    );
    assert_eq!(
        sink.send_notice("", &context).await.unwrap_err(),
        NotificationError::Cancelled
    );
}
