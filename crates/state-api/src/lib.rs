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

    if signal.created_at > now {
        tracing::warn!("invalid signal acceptance: signal creation time is in the future");
        return Err(StateError::InvalidRequest);
    }

    if signal.expires_at <= now {
        tracing::warn!("invalid signal acceptance: signal has expired");
        return Err(StateError::InvalidRequest);
    }

    if signal.pool.chain() != signal.token.chain() {
        tracing::warn!("invalid signal acceptance: pool chain differs from token chain");
        return Err(StateError::InvalidRequest);
    }

    if draft.cooldown_secs > 604_800 {
        tracing::warn!("invalid signal acceptance: cooldown exceeds 604800 seconds");
        return Err(StateError::InvalidRequest);
    }

    if targets.is_empty() {
        tracing::warn!("invalid signal acceptance: delivery targets are empty");
        return Err(StateError::InvalidRequest);
    }

    if targets.len() > 16 {
        tracing::warn!("invalid signal acceptance: delivery targets exceed 16 entries");
        return Err(StateError::InvalidRequest);
    }

    if signal.facts.len() > 32 {
        tracing::warn!("invalid signal acceptance: signal facts exceed 32 entries");
        return Err(StateError::InvalidRequest);
    }

    if signal.warnings.len() > 32 {
        tracing::warn!("invalid signal acceptance: signal warnings exceed 32 entries");
        return Err(StateError::InvalidRequest);
    }

    if signal.reasons.len() > 32 {
        tracing::warn!("invalid signal acceptance: signal reasons exceed 32 entries");
        return Err(StateError::InvalidRequest);
    }

    let payload = serde_json::to_vec(signal).map_err(|_| {
        tracing::warn!("invalid signal acceptance: serialization failed");
        StateError::InvalidRequest
    })?;

    if payload.len() > 16_384 {
        tracing::warn!("invalid signal acceptance: serialized signal exceeds 16384 bytes");
        return Err(StateError::InvalidRequest);
    }

    let mut unique = std::collections::BTreeSet::new();

    for (index, target) in targets.iter().enumerate() {
        if target.destination.is_empty() {
            tracing::warn!("invalid signal acceptance: target[{index}] destination is empty");
            return Err(StateError::InvalidRequest);
        }

        if target.destination.len() > 256 {
            tracing::warn!(
                "invalid signal acceptance: target[{index}] destination exceeds 256 bytes"
            );
            return Err(StateError::InvalidRequest);
        }

        if !unique.insert((&target.sink, &target.destination)) {
            tracing::warn!(
                "invalid signal acceptance: target[{index}] duplicate sink and destination"
            );
            return Err(StateError::InvalidRequest);
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests;
