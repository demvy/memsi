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

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::UNIX_EPOCH;

    #[test]
    fn delays_handle_seconds_dates_minimums_defaults_and_overflow() {
        let now = UNIX_EPOCH + Duration::from_secs(1_000_000);
        for (header, seconds) in [
            (None, 30),
            (Some("bad"), 30),
            (Some(""), 30),
            (Some("-1"), 30),
            (Some("0"), 1),
            (Some(" 12 "), 12),
        ] {
            assert_eq!(delay(header, now), Duration::from_secs(seconds));
        }

        let future = httpdate::fmt_http_date(now + Duration::from_secs(60));
        let past = httpdate::fmt_http_date(now - Duration::from_secs(60));
        assert_eq!(delay(Some(&future), now), Duration::from_secs(60));
        assert_eq!(delay(Some(&past), now), Duration::from_secs(1));
        assert_eq!(
            delay(Some(&u64::MAX.to_string()), now),
            Duration::from_millis(u64::MAX)
        );
    }
}
