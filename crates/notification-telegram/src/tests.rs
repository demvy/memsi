use super::*;
use provider_api::{Cancellation, RequestPriority};
#[path = "../../../test-support/signal.rs"]
mod fixtures;

fn build_sink(thread: Option<i64>) -> TelegramSink {
    TelegramSink::new(
        InstanceId::new("telegram").unwrap(),
        TelegramConfig {
            token: app_config::SecretRef::new(format!(
                "env:MEMSI_TEST_MISSING_{}_BOT",
                std::process::id()
            ))
            .unwrap(),
            chat_id: "-100123".into(),
            message_thread_id: thread,
        },
    )
    .unwrap()
}

fn context() -> RequestContext {
    RequestContext {
        priority: RequestPriority::default(),
        deadline: Instant::now() + Duration::from_secs(60),
        cancellation: Cancellation::default(),
    }
}

#[test]
fn payloads_preserve_destination_disable_previews_and_opt_into_html() {
    let sink = build_sink(Some(42));
    assert_eq!(sink.id().as_str(), "telegram");
    assert_eq!(sink.destination(), "-100123");
    assert_eq!(descriptor().id.as_str(), "notification.telegram");
    let plain = sink.payload("text".into());
    assert_eq!(plain["chat_id"], "-100123");
    assert_eq!(plain["message_thread_id"], 42);
    assert_eq!(plain["link_preview_options"]["is_disabled"], true);
    assert!(plain.get("parse_mode").is_none());
    assert_eq!(
        sink.html_payload("<b>text</b>".into())["parse_mode"],
        "HTML"
    );
    assert!(
        build_sink(None)
            .payload("text".into())
            .get("message_thread_id")
            .is_none()
    );
}

#[tokio::test]
async fn invalid_destination_parts_and_cancelled_contexts_fail_before_network_io() {
    let sink = build_sink(None);
    let mut notification = Notification {
        delivery_id: ContentHash::of_bytes(b"delivery"),
        signal: fixtures::signal(),
        links: vec![],
        destination: "other".into(),
        part_index: 0,
    };
    assert_eq!(
        sink.send(&notification, &context()).await.unwrap_err(),
        NotificationError::ForbiddenDestination
    );
    notification.destination = sink.destination().into();
    notification.part_index = 1;
    assert_eq!(
        sink.send(&notification, &context()).await.unwrap_err(),
        NotificationError::InvalidPayload
    );
    assert_eq!(
        sink.send_notice(&"x".repeat(4097), &context())
            .await
            .unwrap_err(),
        NotificationError::InvalidPayload
    );
    let cancelled = context();
    cancelled.cancellation.cancel();
    assert_eq!(
        sink.send_notice("text", &cancelled).await.unwrap_err(),
        NotificationError::Cancelled
    );
    assert_eq!(
        sink.send(&notification, &cancelled).await.unwrap_err(),
        NotificationError::Cancelled
    );
    assert_eq!(
        sink.attempt("text".into(), false).await.unwrap_err(),
        NotificationError::Unauthorized
    );
    assert_eq!(
        sink.post(":invalid", &serde_json::json!({}))
            .await
            .unwrap_err(),
        NotificationError::UnknownOutcome
    );
}
