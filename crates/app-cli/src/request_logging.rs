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
                serde_json::json!({ "event" : "codex_request_counts",
                "http_attempts_since_report" : stats.http_attempts.saturating_sub(self
                .previous), "http_attempts_total" : stats.http_attempts,
                "http_retries_total" : stats.http_retries, })
            );
            self.previous = stats.http_attempts;
        }
    }
}
