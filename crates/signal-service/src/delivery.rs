use crate::{Clock, links};
use domain::*;
use event_api::{EventEnvelope, EventPublisher, LifecycleEvent};
use futures::FutureExt;
use notification_api::*;
use provider_api::{Cancellation, RequestContext};
use state_api::*;
use std::{
    collections::BTreeMap,
    panic::{AssertUnwindSafe, catch_unwind},
    sync::Arc,
    time::{Duration, Instant},
};
pub type Sinks = BTreeMap<InstanceId, Arc<dyn NotificationSink>>;
#[derive(Default)]
pub struct DeliveryPolicy {
    pub retry_unknown: bool,
}
/// Claims one row at a time: the lease never spends its lifetime waiting in a worker queue.
pub async fn deliver_one(
    repository: &dyn DeliveryRepository,
    sinks: &Sinks,
    clock: &dyn Clock,
    cancellation: &Cancellation,
    policy: &DeliveryPolicy,
) -> Result<bool, StateError> {
    deliver(repository, sinks, clock, cancellation, policy, None).await
}
pub async fn deliver_one_with_events(
    repository: &dyn DeliveryRepository,
    sinks: &Sinks,
    clock: &dyn Clock,
    cancellation: &Cancellation,
    policy: &DeliveryPolicy,
    events: &dyn EventPublisher,
) -> Result<bool, StateError> {
    deliver(repository, sinks, clock, cancellation, policy, Some(events)).await
}
async fn deliver(
    repository: &dyn DeliveryRepository,
    sinks: &Sinks,
    clock: &dyn Clock,
    cancellation: &Cancellation,
    policy: &DeliveryPolicy,
    events: Option<&dyn EventPublisher>,
) -> Result<bool, StateError> {
    if cancellation.is_cancelled() {
        return Ok(false);
    }
    let now = clock.now()?;
    let worker = InstanceId::new("delivery-worker").unwrap();
    let Some(lease) = repository
        .claim_due(&worker, now, 30, 1)
        .await?
        .into_iter()
        .next()
    else {
        return Ok(false);
    };
    let Some(sink) = sinks.get(&lease.sink) else {
        repository.fail(&lease, now).await?;
        return Ok(true);
    };
    if lease.attempt > 5 || lease.notification.signal.expires_at <= now {
        repository.fail(&lease, now).await?;
        return Ok(true);
    }
    let mut notification = lease.notification.clone();
    notification.links = links::for_signal(&notification.signal);
    let ctx = RequestContext {
        priority: provider_api::RequestPriority::default(),
        deadline: Instant::now() + Duration::from_secs(10),
        cancellation: Cancellation::default(),
    };
    let work = async {
        tokio::time::timeout(Duration::from_secs(10), sink.send(&notification, &ctx))
            .await
            .unwrap_or(Err(NotificationError::UnknownOutcome))
    };
    let result = AssertUnwindSafe(work)
        .catch_unwind()
        .await
        .unwrap_or(Err(NotificationError::UnknownOutcome));
    let delivered = result.is_ok();
    let completed_at = clock.now()?;
    settle(repository, &lease, result, completed_at, policy).await?;
    if delivered
        && let Some(events) = events
        && let Ok(event) = EventEnvelope::new(
            completed_at,
            format!("delivery:{}", lease.delivery_id.as_str()),
            u64::from(lease.attempt),
            LifecycleEvent::NotificationDelivered {
                delivery: lease.delivery_id.clone(),
            },
        )
    {
        let _ = catch_unwind(AssertUnwindSafe(|| events.publish(event)));
    }
    Ok(true)
}
async fn settle(
    repository: &dyn DeliveryRepository,
    lease: &DeliveryLease,
    result: Result<DeliveryReceipt, NotificationError>,
    now: TimestampMs,
    policy: &DeliveryPolicy,
) -> Result<(), StateError> {
    match result {
        Ok(receipt) => repository.complete(lease, receipt, now).await,
        Err(
            NotificationError::Unauthorized
            | NotificationError::ForbiddenDestination
            | NotificationError::InvalidPayload,
        ) => repository.fail(lease, now).await,
        Err(_) if lease.attempt >= 5 => repository.fail(lease, now).await,
        Err(NotificationError::UnknownOutcome) if !policy.retry_unknown => {
            repository
                .retry(lease, lease.notification.signal.expires_at, true, now)
                .await
        }
        Err(error) => {
            let unknown = matches!(error, NotificationError::UnknownOutcome);
            let delay = match error {
                NotificationError::RateLimited { retry_after_ms } => {
                    retry_after_ms.clamp(1, 3_600_000)
                }
                _ => backoff_ms(lease),
            };
            let due = now
                .checked_add(delay as i64)
                .map_err(|_| StateError::InvalidRequest)?;
            repository.retry(lease, due, unknown, now).await
        }
    }
}
fn backoff_ms(lease: &DeliveryLease) -> u64 {
    let ceiling = (1000u64 << lease.attempt.saturating_sub(1).min(6)).min(60_000);
    let hash = ContentHash::of(&(lease.delivery_id.clone(), lease.attempt))
        .expect("serializable identifiers");
    let value = u64::from_str_radix(&hash.as_str()[..16], 16).expect("hex hash");
    1 + value % ceiling
}
