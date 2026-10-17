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

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn successful_responses_require_positive_message_id_and_ok_status() {
        assert_eq!(
            message_id(200, br#"{"ok":true,"result":{"message_id":42}}"#),
            Ok("42".into())
        );
        for body in [
            json!({"ok":false}),
            json!({"ok":true}),
            json!({"ok":true,"result":{"message_id":0}}),
            json!({"ok":true,"result":{"message_id":-1}}),
            json!({"ok":true,"result":{"message_id":"42"}}),
        ] {
            assert_eq!(
                message_id(200, &serde_json::to_vec(&body).unwrap()),
                Err(NotificationError::UnknownOutcome)
            );
        }

        assert_eq!(
            message_id(500, b"{}"),
            Err(NotificationError::UnknownOutcome)
        );
        assert_eq!(
            message_id(200, b"invalid"),
            Err(NotificationError::UnknownOutcome)
        );
    }

    #[test]
    fn error_codes_and_rate_limits_are_mapped_and_bounded() {
        for (code, expected) in [
            (401, NotificationError::Unauthorized),
            (403, NotificationError::ForbiddenDestination),
            (400, NotificationError::InvalidPayload),
        ] {
            assert_eq!(message_id(code, b"{}"), Err(expected.clone()));
            assert_eq!(
                message_id(
                    200,
                    &serde_json::to_vec(&json!({"error_code":code})).unwrap()
                ),
                Err(expected)
            );
        }

        assert_eq!(
            message_id(401, b"not JSON"),
            Err(NotificationError::Unauthorized)
        );
        assert_eq!(
            message_id(403, b"not JSON"),
            Err(NotificationError::ForbiddenDestination)
        );
        assert_eq!(
            message_id(429, b"{}"),
            Err(NotificationError::RateLimited {
                retry_after_ms: 30_000
            })
        );
        for (seconds, expected) in [(0, 1_000), (2, 2_000), (9999, 3_600_000)] {
            let bytes =
                serde_json::to_vec(&json!({"error_code":429,"parameters":{"retry_after":seconds}}))
                    .unwrap();
            assert_eq!(
                message_id(200, &bytes),
                Err(NotificationError::RateLimited {
                    retry_after_ms: expected
                })
            );
        }
    }
}
