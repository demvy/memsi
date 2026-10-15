//! Best-effort bounded lifecycle fan-out. Never a durable delivery queue.
use domain::InstanceId;
use event_api::{EventEnvelope, EventPublisher, PublishReport};
use serde::Serialize;
use std::{collections::BTreeMap, sync::Mutex};
use tokio::sync::mpsc;
pub const DEFAULT_CAPACITY: usize = 128;
const MAX_SUBSCRIBERS: usize = 64;
const MAX_QUEUE_SLOTS: usize = 8192;
#[derive(Clone, Copy, Debug, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SubscriberKind {
    Telemetry,
    UiProjection,
}
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("invalid subscriber capacity")]
    Invalid,
    #[error("subscriber already exists")]
    Conflict,
    #[error("event subscriber capacity exceeded")]
    Capacity,
    #[error("event router closed or unavailable")]
    Closed,
}
#[derive(Clone, Debug, Serialize)]
pub struct SubscriberStats {
    pub kind: SubscriberKind,
    pub capacity: usize,
    pub enqueued: u64,
    pub dropped: u64,
    pub dirty: bool,
    pub closed: bool,
}
struct Subscriber {
    sender: Option<mpsc::Sender<EventEnvelope>>,
    stats: SubscriberStats,
}
#[derive(Default)]
struct Registry {
    subscribers: BTreeMap<InstanceId, Subscriber>,
    queue_slots: usize,
    closed: bool,
    invalid_events: u64,
}
#[derive(Default)]
pub struct EventRouter {
    registry: Mutex<Registry>,
}
pub struct Subscription {
    receiver: mpsc::Receiver<EventEnvelope>,
}
impl Subscription {
    pub async fn recv(&mut self) -> Option<EventEnvelope> {
        self.receiver.recv().await
    }
    pub fn try_recv(&mut self) -> Result<EventEnvelope, mpsc::error::TryRecvError> {
        self.receiver.try_recv()
    }
}
impl EventRouter {
    pub fn subscribe(
        &self,
        id: InstanceId,
        kind: SubscriberKind,
        capacity: usize,
    ) -> Result<Subscription, Error> {
        if !(1..=1024).contains(&capacity) {
            return Err(Error::Invalid);
        }
        let mut registry = self.registry.lock().map_err(|_| Error::Closed)?;
        if registry.closed {
            return Err(Error::Closed);
        }
        if registry.subscribers.contains_key(&id) {
            return Err(Error::Conflict);
        }
        if registry.subscribers.len() >= MAX_SUBSCRIBERS
            || registry.queue_slots + capacity > MAX_QUEUE_SLOTS
        {
            return Err(Error::Capacity);
        }
        let (sender, receiver) = mpsc::channel(capacity);
        registry.queue_slots += capacity;
        registry.subscribers.insert(
            id,
            Subscriber {
                sender: Some(sender),
                stats: SubscriberStats {
                    kind,
                    capacity,
                    enqueued: 0,
                    dropped: 0,
                    dirty: false,
                    closed: false,
                },
            },
        );
        Ok(Subscription { receiver })
    }
    pub fn unsubscribe(&self, id: &InstanceId) -> Result<bool, Error> {
        let mut registry = self.registry.lock().map_err(|_| Error::Closed)?;
        if let Some(subscriber) = registry.subscribers.remove(id) {
            registry.queue_slots -= subscriber.stats.capacity;
            Ok(true)
        } else {
            Ok(false)
        }
    }
    pub fn stats(&self) -> Result<BTreeMap<InstanceId, SubscriberStats>, Error> {
        let registry = self.registry.lock().map_err(|_| Error::Closed)?;
        Ok(registry
            .subscribers
            .iter()
            .map(|(id, subscriber)| {
                let mut stats = subscriber.stats.clone();
                stats.closed = subscriber
                    .sender
                    .as_ref()
                    .is_none_or(|sender| sender.is_closed());
                stats.dirty |= stats.closed;
                (id.clone(), stats)
            })
            .collect())
    }
    pub fn invalid_events(&self) -> Result<u64, Error> {
        Ok(self
            .registry
            .lock()
            .map_err(|_| Error::Closed)?
            .invalid_events)
    }
    /// A snapshot resync may clear dirty only if no newer drop raced with it.
    pub fn mark_resynced(&self, id: &InstanceId, expected_dropped: u64) -> Result<(), Error> {
        let mut registry = self.registry.lock().map_err(|_| Error::Closed)?;
        let subscriber = registry.subscribers.get_mut(id).ok_or(Error::Closed)?;
        if subscriber
            .sender
            .as_ref()
            .is_none_or(|sender| sender.is_closed())
        {
            return Err(Error::Closed);
        }
        if subscriber.stats.dropped != expected_dropped {
            return Err(Error::Conflict);
        }
        subscriber.stats.dirty = false;
        Ok(())
    }
    /// Stop producers first, then close senders. Receivers can drain buffered events.
    /// The host owns subscriber futures and their bounded shutdown/join deadline.
    pub fn close(&self) -> Result<(), Error> {
        let mut registry = self.registry.lock().map_err(|_| Error::Closed)?;
        registry.closed = true;
        for subscriber in registry.subscribers.values_mut() {
            subscriber.sender.take();
        }
        Ok(())
    }
}
impl EventPublisher for EventRouter {
    fn publish(&self, event: EventEnvelope) -> PublishReport {
        let Ok(mut registry) = self.registry.lock() else {
            return PublishReport {
                accepted_subscribers: 0,
                dropped_subscribers: 0,
            };
        };
        let valid = event.validate().is_ok();
        if !valid {
            registry.invalid_events = registry.invalid_events.saturating_add(1);
        }
        let mut report = PublishReport {
            accepted_subscribers: 0,
            dropped_subscribers: 0,
        };
        for subscriber in registry.subscribers.values_mut() {
            let accepted = valid
                && subscriber
                    .sender
                    .as_ref()
                    .is_some_and(|sender| sender.try_send(event.clone()).is_ok());
            if accepted {
                subscriber.stats.enqueued = subscriber.stats.enqueued.saturating_add(1);
                report.accepted_subscribers += 1;
            } else {
                subscriber.stats.dropped = subscriber.stats.dropped.saturating_add(1);
                subscriber.stats.dirty = true;
                report.dropped_subscribers += 1;
            }
        }
        report
    }
}
