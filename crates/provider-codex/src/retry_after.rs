//! HTTP Retry-After values; malformed headers use a conservative default.
use std::time::{Duration, SystemTime};

pub(super) fn delay(header: Option<&str>, now: SystemTime) -> Duration {
    let Some(value) = header.map(str::trim) else {
        return Duration::from_secs(30);
    };
    if let Ok(seconds) = value.parse::<u64>() {
        return Duration::from_millis(seconds.saturating_mul(1000).max(1000));
    }
    httpdate::parse_http_date(value)
        .ok()
        .map(|date| date.duration_since(now).unwrap_or_default())
        .unwrap_or(Duration::from_secs(30))
        .max(Duration::from_secs(1))
}
