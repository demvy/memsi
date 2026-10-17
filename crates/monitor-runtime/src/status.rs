use super::CycleReport;
use domain::TimestampMs;
use event_api::{CyclePhase, CycleSummary, RuntimeSnapshot, RuntimeStatusReader};
use std::sync::{Arc, Mutex};
pub struct RuntimeStatus(Mutex<RuntimeSnapshot>);
impl Default for RuntimeStatus {
    fn default() -> Self {
        Self(Mutex::new(RuntimeSnapshot {
            schema_version: 1,
            phase: CyclePhase::Ready,
            completed_cycles: 0,
            failed_cycles: 0,
            started_at: None,
            finished_at: None,
            last_cycle: None,
        }))
    }
}

impl RuntimeStatusReader for RuntimeStatus {
    fn runtime_snapshot(&self) -> Option<RuntimeSnapshot> {
        self.0.lock().ok().map(|s| s.clone())
    }
}

impl RuntimeStatus {
    pub(super) fn begin(self: &Arc<Self>, at: Option<TimestampMs>) -> CycleGuard {
        if let Ok(mut s) = self.0.lock() {
            s.phase = CyclePhase::Running;
            s.started_at = at;
            s.finished_at = None;
        }

        CycleGuard {
            status: self.clone(),
            settled: false,
        }
    }

    pub(super) fn finish_run(&self, failed: bool) {
        if failed {
            if let Ok(mut s) = self.0.lock() {
                s.phase = CyclePhase::Failed;
            }
        } else {
            self.stop();
        }
    }

    pub(super) fn stop(&self) {
        if let Ok(mut s) = self.0.lock()
            && !matches!(s.phase, CyclePhase::Failed | CyclePhase::Interrupted)
        {
            s.phase = CyclePhase::Stopped;
        }
    }
}

pub(super) struct CycleGuard {
    status: Arc<RuntimeStatus>,
    settled: bool,
}

impl CycleGuard {
    pub(super) fn finish(
        mut self,
        result: &Result<CycleReport, state_api::StateError>,
        at: Option<TimestampMs>,
    ) {
        if let Ok(mut s) = self.status.0.lock() {
            s.finished_at = at;
            match result {
                Ok(r) => {
                    s.phase = CyclePhase::Idle;
                    s.completed_cycles = s.completed_cycles.saturating_add(1);
                    s.last_cycle = Some(CycleSummary {
                        discovered: r.discovered,
                        watched: r.watched,
                        evaluated: r.evaluated,
                        accepted: r.accepted,
                        suppressed: r.suppressed,
                        errors: r.errors.len() + r.muted_errors,
                        worker_panics: r.worker_panics,
                        discovery_quarantined: r.discovery_quarantined,
                        quarantined_candidates: r.quarantined_candidates,
                    });
                }

                Err(_) => {
                    s.phase = CyclePhase::Failed;
                    s.failed_cycles = s.failed_cycles.saturating_add(1);
                }
            }
        }

        self.settled = true;
    }
}

impl Drop for CycleGuard {
    fn drop(&mut self) {
        if !self.settled
            && let Ok(mut s) = self.status.0.lock()
        {
            s.phase = CyclePhase::Interrupted;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::tests::report;

    #[test]
    fn cycle_status_tracks_success_failure_and_interruption() {
        let status = Arc::new(RuntimeStatus::default());
        assert!(matches!(
            status.runtime_snapshot().unwrap().phase,
            CyclePhase::Ready
        ));
        let guard = status.begin(Some(TimestampMs::new(100).unwrap()));
        assert!(matches!(
            status.runtime_snapshot().unwrap().phase,
            CyclePhase::Running
        ));
        guard.finish(&Ok(report()), Some(TimestampMs::new(200).unwrap()));
        let snapshot = status.runtime_snapshot().unwrap();
        assert!(matches!(snapshot.phase, CyclePhase::Idle));
        assert_eq!(snapshot.completed_cycles, 1);
        assert_eq!(snapshot.finished_at.unwrap().get(), 200);
        assert_eq!(snapshot.last_cycle.unwrap().errors, 3);
        status
            .begin(None)
            .finish(&Err(state_api::StateError::Unavailable), None);
        let snapshot = status.runtime_snapshot().unwrap();
        assert_eq!(snapshot.failed_cycles, 1);
        assert!(matches!(snapshot.phase, CyclePhase::Failed));
        status.stop();
        assert!(matches!(
            status.runtime_snapshot().unwrap().phase,
            CyclePhase::Failed
        ));
        drop(status.begin(None));
        assert!(matches!(
            status.runtime_snapshot().unwrap().phase,
            CyclePhase::Interrupted
        ));
        status.stop();
        assert!(matches!(
            status.runtime_snapshot().unwrap().phase,
            CyclePhase::Interrupted
        ));
    }

    #[test]
    fn finishing_run_records_stopped_or_failed() {
        let status = RuntimeStatus::default();
        status.finish_run(false);
        assert!(matches!(
            status.runtime_snapshot().unwrap().phase,
            CyclePhase::Stopped
        ));
        status.finish_run(true);
        assert!(matches!(
            status.runtime_snapshot().unwrap().phase,
            CyclePhase::Failed
        ));
    }
}
