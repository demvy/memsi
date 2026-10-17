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

#[cfg(test)]
mod tests {
    use super::*;
    use domain::{ContentHash, TimestampMs};
    use event_api::LifecycleEvent;

    fn event() -> EventEnvelope {
        EventEnvelope::new(
            TimestampMs::new(0).unwrap(),
            "test".into(),
            1,
            LifecycleEvent::AnalysisCompleted {
                snapshot: ContentHash::of_bytes(b"snapshot"),
            },
        )
        .unwrap()
    }

    #[test]
    fn bounded_subscribers_drop_independently_and_resync_checks_drop_counter() {
        let router = EventRouter::default();
        let id = InstanceId::new("ui").unwrap();
        let mut small = router
            .subscribe(id.clone(), SubscriberKind::UiProjection, 1)
            .unwrap();
        let mut large = router
            .subscribe(
                InstanceId::new("telemetry").unwrap(),
                SubscriberKind::Telemetry,
                2,
            )
            .unwrap();
        assert_eq!(router.publish(event()).accepted_subscribers, 2);
        let report = router.publish(event());
        assert_eq!(report.accepted_subscribers, 1);
        assert_eq!(report.dropped_subscribers, 1);
        let stats = router.stats().unwrap();
        assert_eq!(stats[&id].enqueued, 1);
        assert_eq!(stats[&id].dropped, 1);
        assert!(stats[&id].dirty);
        assert!(matches!(router.mark_resynced(&id, 0), Err(Error::Conflict)));
        router.mark_resynced(&id, 1).unwrap();
        assert!(!router.stats().unwrap()[&id].dirty);
        assert_eq!(small.try_recv().unwrap().id, event().id);
        assert_eq!(
            futures::executor::block_on(large.recv()).unwrap().id,
            event().id
        );
        assert_eq!(large.try_recv().unwrap().id, event().id);
        assert!(small.try_recv().is_err());
        assert!(router.unsubscribe(&id).unwrap());
        assert!(!router.unsubscribe(&id).unwrap());
    }

    #[test]
    fn invalid_events_and_closed_receivers_are_counted_and_cannot_be_resynced() {
        let router = EventRouter::default();
        let id = InstanceId::new("ui").unwrap();
        let subscription = router
            .subscribe(id.clone(), SubscriberKind::UiProjection, 1)
            .unwrap();
        let mut invalid = event();
        invalid.sequence = 0;
        assert_eq!(router.publish(invalid).dropped_subscribers, 1);
        assert_eq!(router.invalid_events().unwrap(), 1);
        drop(subscription);
        assert!(router.stats().unwrap()[&id].closed);
        assert!(matches!(router.mark_resynced(&id, 1), Err(Error::Closed)));
        assert_eq!(router.publish(event()).dropped_subscribers, 1);
    }

    #[test]
    fn router_checks_capacities_duplicates_and_closes_after_buffer_drain() {
        let router = EventRouter::default();
        let id = InstanceId::new("ui").unwrap();
        for capacity in [0, 1025] {
            assert!(matches!(
                router.subscribe(id.clone(), SubscriberKind::Telemetry, capacity),
                Err(Error::Invalid)
            ));
        }
        let mut subscription = router
            .subscribe(id.clone(), SubscriberKind::Telemetry, 1)
            .unwrap();
        assert!(matches!(
            router.subscribe(id.clone(), SubscriberKind::Telemetry, 1),
            Err(Error::Conflict)
        ));
        router.publish(event());
        router.close().unwrap();
        assert!(router.stats().unwrap()[&id].closed);
        assert!(subscription.try_recv().is_ok());
        assert!(futures::executor::block_on(subscription.recv()).is_none());
        assert!(matches!(
            router.subscribe(
                InstanceId::new("other").unwrap(),
                SubscriberKind::Telemetry,
                1
            ),
            Err(Error::Closed)
        ));
        let router = EventRouter::default();
        let mut subscriptions = vec![];
        for index in 0..8 {
            subscriptions.push(
                router
                    .subscribe(
                        InstanceId::new(format!("s{index}")).unwrap(),
                        SubscriberKind::Telemetry,
                        1024,
                    )
                    .unwrap(),
            );
        }
        assert!(matches!(
            router.subscribe(id, SubscriberKind::Telemetry, 1),
            Err(Error::Capacity)
        ));
    }
}
