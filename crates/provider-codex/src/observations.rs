use provider_api::ProviderError;
use provider_api::TransportObservations;
use std::{
    sync::{Arc, Mutex},
    time::Instant,
};
#[derive(Default)]
pub(super) struct Observations(Mutex<TransportObservations>);
impl Observations {
    pub(super) fn snapshot(&self) -> Option<TransportObservations> {
        self.0.lock().ok().map(|s| s.clone())
    }
    pub(super) fn attempt(&self, retry: bool) {
        if let Ok(mut stats) = self.0.lock() {
            stats.http_attempts = stats.http_attempts.saturating_add(1);
            if retry {
                stats.http_retries = stats.http_retries.saturating_add(1);
            }
        }
    }
    pub(super) fn rate_limited(&self) {
        if let Ok(mut stats) = self.0.lock() {
            stats.rate_limited_responses = stats.rate_limited_responses.saturating_add(1);
        }
    }
    pub(super) fn begin(self: &Arc<Self>) -> Guard {
        if let Ok(mut s) = self.0.lock() {
            s.started = s.started.saturating_add(1);
            s.in_flight += 1;
        }
        Guard {
            stats: self.clone(),
            start: Instant::now(),
            settled: false,
        }
    }
}
pub(super) struct Guard {
    stats: Arc<Observations>,
    start: Instant,
    settled: bool,
}
impl Guard {
    pub(super) fn finish<T>(mut self, result: &Result<T, ProviderError>) {
        if let Ok(mut s) = self.stats.0.lock() {
            s.in_flight = s.in_flight.saturating_sub(1);
            s.last_elapsed_micros =
                Some(self.start.elapsed().as_micros().min(u64::MAX as u128) as u64);
            match result {
                Ok(_) => s.succeeded = s.succeeded.saturating_add(1),
                Err(ProviderError::Cancelled) => {
                    s.cancelled = s.cancelled.saturating_add(1);
                }
                Err(ProviderError::Timeout) => {
                    s.timed_out = s.timed_out.saturating_add(1);
                }
                Err(_) => s.failed = s.failed.saturating_add(1),
            }
        }
        self.settled = true;
    }
}
impl Drop for Guard {
    fn drop(&mut self) {
        if !self.settled
            && let Ok(mut s) = self.stats.0.lock()
        {
            s.in_flight = s.in_flight.saturating_sub(1);
            s.interrupted = s.interrupted.saturating_add(1);
        }
    }
}
