use provider_api::TransportObservationReader;
pub(super) struct Counter<'a> {
    reader: Option<&'a dyn TransportObservationReader>,
    previous: u64,
}

impl<'a> Counter<'a> {
    pub(super) fn new(reader: Option<&'a dyn TransportObservationReader>) -> Self {
        let previous = reader
            .and_then(|r| r.transport_observations())
            .map_or(0, |s| s.http_attempts);
        Self { reader, previous }
    }

    pub(super) fn log(&mut self) {
        if let Some(stats) = self.reader.and_then(|r| r.transport_observations()) {
            eprintln!(
                "{}",
                serde_json::json!({
                    "event": "codex_request_counts",
                    "http_attempts_since_report": stats.http_attempts.saturating_sub(self.previous),
                    "http_attempts_total": stats.http_attempts,
                    "http_retries_total": stats.http_retries,
                })
            );
            self.previous = stats.http_attempts;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    struct Reader(AtomicU64);
    impl TransportObservationReader for Reader {
        fn transport_observations(&self) -> Option<provider_api::TransportObservations> {
            Some(provider_api::TransportObservations {
                http_attempts: self.0.load(Ordering::Relaxed),
                ..Default::default()
            })
        }
    }

    #[test]
    fn request_counter_reads_initial_snapshot_and_tracks_latest_observation() {
        let reader = Reader(AtomicU64::new(4));
        let mut counter = Counter::new(Some(&reader));
        assert_eq!(counter.previous, 4);
        reader.0.store(7, Ordering::Relaxed);
        counter.log();
        assert_eq!(counter.previous, 7);
        reader.0.store(1, Ordering::Relaxed);
        counter.log();
        assert_eq!(counter.previous, 1);
        let mut absent = Counter::new(None);
        absent.log();
        assert_eq!(absent.previous, 0);
    }
}
