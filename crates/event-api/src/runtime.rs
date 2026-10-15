use domain::TimestampMs;
use serde::Serialize;
#[derive(Clone, Copy, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CyclePhase {
    Ready,
    Running,
    Idle,
    Failed,
    Interrupted,
    Stopped,
}
#[derive(Clone, Debug, Serialize)]
pub struct CycleSummary {
    pub discovered: usize,
    pub watched: usize,
    pub evaluated: usize,
    pub accepted: usize,
    pub suppressed: usize,
    pub errors: usize,
    pub worker_panics: usize,
    pub discovery_quarantined: bool,
    pub quarantined_candidates: usize,
}
#[derive(Clone, Debug, Serialize)]
pub struct RuntimeSnapshot {
    pub schema_version: u16,
    pub phase: CyclePhase,
    pub completed_cycles: u64,
    pub failed_cycles: u64,
    pub started_at: Option<TimestampMs>,
    pub finished_at: Option<TimestampMs>,
    pub last_cycle: Option<CycleSummary>,
}
/// Redacted bounded scheduler facts; no provider health inference.
pub trait RuntimeStatusReader: Send + Sync {
    fn runtime_snapshot(&self) -> Option<RuntimeSnapshot>;
}
