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
