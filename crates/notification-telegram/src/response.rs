use notification_api::NotificationError;
use serde_json::Value;
pub(super) fn message_id(status: u16, bytes: &[u8]) -> Result<String, NotificationError> {
    if status == 401 {
        return Err(NotificationError::Unauthorized);
    }
    if status == 403 {
        return Err(NotificationError::ForbiddenDestination);
    }
    let body: Value =
        serde_json::from_slice(bytes).map_err(|_| NotificationError::UnknownOutcome)?;
    let code = body
        .get("error_code")
        .and_then(Value::as_u64)
        .unwrap_or(status as u64);
    if code == 429 {
        let seconds = body
            .pointer("/parameters/retry_after")
            .and_then(Value::as_u64)
            .unwrap_or(30)
            .clamp(1, 3600);
        return Err(NotificationError::RateLimited {
            retry_after_ms: seconds * 1000,
        });
    }
    match code {
        401 => return Err(NotificationError::Unauthorized),
        403 => return Err(NotificationError::ForbiddenDestination),
        400 => return Err(NotificationError::InvalidPayload),
        _ => {}
    }
    if status != 200 || body.get("ok").and_then(Value::as_bool) != Some(true) {
        return Err(NotificationError::UnknownOutcome);
    }
    body.pointer("/result/message_id")
        .and_then(Value::as_i64)
        .filter(|id| *id > 0)
        .map(|id| id.to_string())
        .ok_or(NotificationError::UnknownOutcome)
}
