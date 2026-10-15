use domain::*;
use provider_api::PoolDiscoveryResult;
pub struct Evaluation<'a> {
    pub candidate: &'a Candidate,
    pub pools: &'a PoolDiscoveryResult,
    pub as_of: TimestampMs,
    pub close_grace_ms: u64,
}
