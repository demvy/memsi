#![forbid(unsafe_code)]
mod checkpoint;
use async_trait::async_trait;
pub use checkpoint::*;
use domain::*;
use notification_api::{DeliveryReceipt, Notification};
use serde::{Deserialize, Serialize};
use signal_api::{Signal, SignalDraft};
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Durability {
    Volatile,
    Durable,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DeliveryTarget {
    pub sink: InstanceId,
    pub destination: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum Acceptance {
    Accepted(ContentHash),
    Suppressed,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DeliveryLease {
    pub delivery_id: ContentHash,
    pub lease_token: ContentHash,
    pub lease_until: TimestampMs,
    pub sink: InstanceId,
    pub notification: Notification,
    pub attempt: u32,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum DeliveryStatus {
    Pending,
    Sending,
    Delivered,
    RetryScheduled,
    UnknownOutcome,
    FailedPermanent,
    Expired,
}
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum StateError {
    #[error("unsupported or inconsistent state schema")]
    SchemaMismatch,
    #[error("state conflict")]
    Conflict,
    #[error("capacity exceeded")]
    Capacity,
    #[error("state unavailable")]
    Unavailable,
    #[error("invalid state request")]
    InvalidRequest,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SignalCursor {
    pub created_at: TimestampMs,
    pub id: ContentHash,
}
#[async_trait]
pub trait SignalRepository: Send + Sync {
    fn durability(&self) -> Durability;
    async fn accept_signal_and_deliveries(
        &self,
        draft: SignalDraft,
        targets: &[DeliveryTarget],
        now: TimestampMs,
    ) -> Result<Acceptance, StateError>;
    async fn list_signals(&self, limit: usize) -> Result<Vec<Signal>, StateError>;
    /// Newest first, id ascending at equal time; exclusive cursor, limit1..200.
    async fn signal_history(
        &self,
        before: Option<&SignalCursor>,
        limit: usize,
    ) -> Result<Vec<Signal>, StateError>;
}
#[async_trait]
pub trait DeliveryRepository: Send + Sync {
    async fn claim_due(
        &self,
        worker: &InstanceId,
        now: TimestampMs,
        lease_secs: u64,
        limit: usize,
    ) -> Result<Vec<DeliveryLease>, StateError>;
    async fn complete(
        &self,
        lease: &DeliveryLease,
        receipt: DeliveryReceipt,
        now: TimestampMs,
    ) -> Result<(), StateError>;
    async fn retry(
        &self,
        lease: &DeliveryLease,
        due: TimestampMs,
        unknown_outcome: bool,
        now: TimestampMs,
    ) -> Result<(), StateError>;
    async fn fail(&self, lease: &DeliveryLease, now: TimestampMs) -> Result<(), StateError>;
}
#[derive(Debug, serde::Serialize)]
pub struct StateStats {
    pub durability: &'static str,
    pub signals: usize,
    pub deliveries: usize,
    pub pending: usize,
    pub delivered: usize,
    pub failed: usize,
}
/// Common runtime port; selecting a backend does not change monitor behavior.
#[async_trait]
pub trait StateStore: SignalRepository + DeliveryRepository + MonitorRepository {
    async fn stats(&self) -> Result<StateStats, StateError>;
}
pub fn validate_acceptance(
    draft: &SignalDraft,
    targets: &[DeliveryTarget],
    now: TimestampMs,
) -> Result<(), StateError> {
    let signal = &draft.signal;
    if signal.created_at > now
        || signal.expires_at <= now
        || signal.pool.chain() != signal.token.chain()
        || draft.cooldown_secs > 604_800
        || targets.is_empty()
        || targets.len() > 16
        || signal.facts.len() > 32
        || signal.warnings.len() > 32
        || signal.reasons.len() > 32
    {
        return Err(StateError::InvalidRequest);
    }
    let payload = serde_json::to_vec(signal).map_err(|_| StateError::InvalidRequest)?;
    if payload.len() > 16_384 {
        return Err(StateError::InvalidRequest);
    }
    let mut unique = std::collections::BTreeSet::new();
    for target in targets {
        if target.destination.is_empty()
            || target.destination.len() > 256
            || !unique.insert((&target.sink, &target.destination))
        {
            return Err(StateError::InvalidRequest);
        }
    }
    Ok(())
}
