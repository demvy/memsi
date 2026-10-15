#![forbid(unsafe_code)]
mod deliveries;
mod signals;
use domain::*;
use notification_api::{DeliveryReceipt, Notification};
use signal_api::Signal;
pub use state_api::StateStats;
use state_api::*;
use std::{
    collections::BTreeMap,
    sync::{Mutex, MutexGuard},
};
#[derive(Clone, Copy)]
pub struct Limits {
    pub signals: usize,
    pub deliveries: usize,
    pub dedupe_keys: usize,
    pub retry_abandoned_sends: bool,
}
impl Default for Limits {
    fn default() -> Self {
        Self {
            signals: 10_000,
            deliveries: 2_000,
            dedupe_keys: 10_000,
            retry_abandoned_sends: false,
        }
    }
}
pub struct MemoryState {
    limits: Limits,
    inner: Mutex<Inner>,
}
#[derive(Default)]
struct Inner {
    checkpoint: Option<MonitorCheckpoint>,
    signals: BTreeMap<ContentHash, Signal>,
    deliveries: BTreeMap<ContentHash, DeliveryRow>,
    dedupe: BTreeMap<ContentHash, DedupeEntry>,
}
struct DedupeEntry {
    signal: ContentHash,
    until: TimestampMs,
}
struct DeliveryRow {
    sink: InstanceId,
    notification: Notification,
    status: DeliveryStatus,
    due: Option<TimestampMs>,
    attempt: u32,
    lease: Option<DeliveryLease>,
    receipt: Option<DeliveryReceipt>,
}
impl DeliveryRow {
    fn terminal(&self) -> bool {
        matches!(
            self.status,
            DeliveryStatus::Delivered | DeliveryStatus::FailedPermanent | DeliveryStatus::Expired
        )
    }
}
impl MemoryState {
    pub fn new(limits: Limits) -> Result<Self, StateError> {
        if [limits.signals, limits.deliveries, limits.dedupe_keys]
            .iter()
            .any(|limit| *limit == 0 || *limit > 100_000)
        {
            return Err(StateError::InvalidRequest);
        }
        Ok(Self {
            limits,
            inner: Mutex::new(Inner::default()),
        })
    }
    fn lock(&self) -> Result<MutexGuard<'_, Inner>, StateError> {
        self.inner.lock().map_err(|_| StateError::Unavailable)
    }
    pub fn stats(&self) -> Result<StateStats, StateError> {
        let inner = self.lock()?;
        Ok(StateStats {
            durability: "volatile",
            signals: inner.signals.len(),
            deliveries: inner.deliveries.len(),
            pending: inner
                .deliveries
                .values()
                .filter(|row| !row.terminal())
                .count(),
            delivered: inner
                .deliveries
                .values()
                .filter(|row| matches!(row.status, DeliveryStatus::Delivered))
                .count(),
            failed: inner
                .deliveries
                .values()
                .filter(|row| {
                    matches!(
                        row.status,
                        DeliveryStatus::FailedPermanent | DeliveryStatus::Expired
                    )
                })
                .count(),
        })
    }
    pub fn delivery_status(&self, id: &ContentHash) -> Result<DeliveryStatus, StateError> {
        self.lock()?
            .deliveries
            .get(id)
            .map(|row| row.status.clone())
            .ok_or(StateError::InvalidRequest)
    }
    /// Only abandoned leases can expire; a current owner may still acknowledge a send.
    fn maintain(&self, inner: &mut Inner, now: TimestampMs) {
        for row in inner.deliveries.values_mut() {
            if row.terminal() {
                continue;
            }
            let sending = matches!(row.status, DeliveryStatus::Sending);
            let lease_active = row
                .lease
                .as_ref()
                .is_some_and(|lease| lease.lease_until > now);
            if sending && lease_active {
                continue;
            }
            if row.notification.signal.expires_at <= now {
                row.status = DeliveryStatus::Expired;
                row.due = None;
                row.lease = None;
            } else if sending {
                row.status = DeliveryStatus::UnknownOutcome;
                row.due = self.limits.retry_abandoned_sends.then_some(now);
                row.lease = None;
            }
        }
        let active_signals: std::collections::BTreeSet<_> = inner
            .deliveries
            .values()
            .filter(|row| !row.terminal())
            .map(|row| row.notification.signal.id.clone())
            .collect();
        inner
            .dedupe
            .retain(|_, entry| entry.until > now || active_signals.contains(&entry.signal));
    }
}
#[async_trait::async_trait]
impl StateStore for MemoryState {
    async fn stats(&self) -> Result<StateStats, StateError> {
        MemoryState::stats(self)
    }
}
#[async_trait::async_trait]
impl MonitorRepository for MemoryState {
    async fn load_checkpoint(
        &self,
        scope: &ContentHash,
    ) -> Result<Option<MonitorCheckpoint>, StateError> {
        Ok(self
            .lock()?
            .checkpoint
            .as_ref()
            .filter(|checkpoint| &checkpoint.scope == scope)
            .cloned())
    }
    async fn save_checkpoint(&self, checkpoint: MonitorCheckpoint) -> Result<(), StateError> {
        checkpoint.validate()?;
        self.lock()?.checkpoint = Some(checkpoint);
        Ok(())
    }
}
