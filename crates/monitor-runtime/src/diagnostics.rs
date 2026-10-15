use domain::{Candidate, TimestampMs};
use std::time::Duration;

/// Optional local diagnostics. Implementations must not perform network I/O.
pub trait MonitorDiagnostics: Send + Sync {
    fn discovery(&self, finished_at: TimestampMs, elapsed: Duration, candidates: &[Candidate]);
    fn evaluation(&self, candidate: &Candidate, evaluation: &app_services::EvaluationReport);
    fn failure(&self, candidate: &Candidate, reason: &str);
}
