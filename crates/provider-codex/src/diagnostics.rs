use std::{error::Error, time::Duration};

/// Classify internally; never emit reqwest errors, which can contain URLs/secrets.
pub(super) fn category(error: &reqwest::Error) -> &'static str {
    if error.is_timeout() {
        return "timeout";
    }

    let mut source = error.source();
    while let Some(cause) = source {
        let text = cause.to_string().to_ascii_lowercase();
        if text.contains("certificate") || text.contains("tls") {
            return "tls";
        }

        if text.contains("dns") || text.contains("name resolution") {
            return "dns";
        }

        if let Some(io) = cause.downcast_ref::<std::io::Error>() {
            match io.kind() {
                std::io::ErrorKind::ConnectionRefused => return "connection_refused",
                std::io::ErrorKind::ConnectionReset => return "connection_reset",
                _ => {}
            }
        }

        source = cause.source();
    }

    if error.is_connect() {
        "connect"
    } else {
        "transport"
    }
}

pub(super) fn failure(
    category: &'static str,
    status: Option<u16>,
    attempt: u32,
    elapsed: Duration,
    retrying: bool,
) {
    eprintln!("{}", record(category, status, attempt, elapsed, retrying));
}

fn record(
    category: &'static str,
    status: Option<u16>,
    attempt: u32,
    elapsed: Duration,
    retrying: bool,
) -> serde_json::Value {
    serde_json::json!({
        "event": "codex_request_failed",
        "category": category,
        "http_status": status,
        "attempt": attempt + 1,
        "elapsed_ms": elapsed.as_millis().min(u64::MAX as u128) as u64,
        "retrying": retrying,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn diagnostic_records_are_bounded_structured_and_use_one_based_attempts() {
        let value = record("http_status", Some(503), 1, Duration::from_millis(42), true);
        assert_eq!(
            value,
            serde_json::json!({"event":"codex_request_failed","category":"http_status","http_status":503,"attempt":2,"elapsed_ms":42,"retrying":true})
        );
        let value = record("transport", None, 0, Duration::MAX, false);
        assert_eq!(value["elapsed_ms"], u64::MAX);
        assert!(value["http_status"].is_null());
    }
}

#[cfg(test)]
mod classification_tests {
    use super::*;

    #[test]
    fn builder_errors_are_classified_without_exposing_the_original_error_text() {
        let error = reqwest::Client::new().get(":invalid").build().unwrap_err();
        assert_eq!(category(&error), "transport");
        failure("transport", None, 0, Duration::ZERO, false);
    }
}
