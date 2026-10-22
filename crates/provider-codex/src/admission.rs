//! Shared bounded admission; queued Active precedes Shadow, in-flight work is untouched.
use provider_api::{ProviderError, RequestContext, RequestPriority};
use std::{
    collections::VecDeque,
    sync::{Mutex, MutexGuard},
    time::{Duration, Instant},
};

use tokio::sync::Notify;
const MAX_WAITERS: usize = 128;
const MAX_SHADOW_WAITERS: usize = 96;
struct Waiter {
    id: u64,
    priority: RequestPriority,
}

struct State {
    waiting: VecDeque<Waiter>,
    in_flight: usize,
    next_id: u64,
    next_start: Instant,
}

pub(super) struct Admission {
    state: Mutex<State>,
    changed: Notify,
    capacity: usize,
    interval: Duration,
}

impl Admission {
    pub fn new(capacity: usize, interval: Duration) -> Self {
        Self {
            state: Mutex::new(State {
                waiting: VecDeque::new(),
                in_flight: 0,
                next_id: 0,
                next_start: Instant::now(),
            }),
            changed: Notify::new(),
            capacity,
            interval,
        }
    }

    fn lock(&self) -> Result<MutexGuard<'_, State>, ProviderError> {
        self.state
            .lock()
            .map_err(|_| ProviderError::InvalidData("Codex admission unavailable".into()))
    }

    /// Delay all future admissions, without shortening an existing cooldown.
    pub fn cooldown(&self, delay: Duration) -> Result<(), ProviderError> {
        let until = Instant::now()
            .checked_add(delay)
            .ok_or(ProviderError::InvalidRequest)?;
        let mut state = self.lock()?;
        state.next_start = state.next_start.max(until);
        self.changed.notify_waiters();
        Ok(())
    }

    pub async fn acquire(&self, ctx: &RequestContext) -> Result<Permit<'_>, ProviderError> {
        ctx.check()?;
        let id = {
            let mut state = self.lock()?;
            if state.waiting.len() >= MAX_WAITERS
                || (!ctx.priority.is_active()
                    && state
                        .waiting
                        .iter()
                        .filter(|waiter| !waiter.priority.is_active())
                        .count()
                        >= MAX_SHADOW_WAITERS)
            {
                return Err(ProviderError::RateLimited {
                    retry_after_ms: self.interval.as_millis() as u64,
                });
            }

            let id = state.next_id;
            state.next_id = id.checked_add(1).ok_or(ProviderError::InvalidRequest)?;
            state.waiting.push_back(Waiter {
                id,
                priority: ctx.priority.clone(),
            });
            id
        };

        let mut ticket = Ticket {
            admission: self,
            id: Some(id),
        };

        self.changed.notify_waiters();
        loop {
            ctx.check()?;
            let changed = self.changed.notified();
            tokio::pin!(changed);
            changed.as_mut().enable();
            let next_start = {
                let mut state = self.lock()?;
                let selected = state
                    .waiting
                    .iter()
                    .position(|waiter| waiter.priority.is_active())
                    .unwrap_or(0);
                if state.in_flight < self.capacity
                    && state.next_start <= Instant::now()
                    && state
                        .waiting
                        .get(selected)
                        .is_some_and(|waiter| waiter.id == id)
                {
                    state.waiting.remove(selected);
                    state.in_flight += 1;
                    state.next_start = Instant::now() + self.interval;
                    ticket.id = None;
                    self.changed.notify_waiters();
                    return Ok(Permit { admission: self });
                }

                state.next_start
            };

            let wake = (Instant::now() + Duration::from_millis(25)).min(ctx.deadline);
            let wake = if next_start > Instant::now() {
                wake.min(next_start)
            } else {
                wake
            };

            tokio::select! {
                _ = &mut changed => (),
                _ = tokio::time::sleep_until(wake.into()) => ()
            }
        }
    }
}

struct Ticket<'a> {
    admission: &'a Admission,
    id: Option<u64>,
}

impl Drop for Ticket<'_> {
    fn drop(&mut self) {
        if let Some(id) = self.id
            && let Ok(mut state) = self.admission.state.lock()
        {
            state.waiting.retain(|waiter| waiter.id != id);
            self.admission.changed.notify_waiters();
        }
    }
}

pub(super) struct Permit<'a> {
    admission: &'a Admission,
}

impl Drop for Permit<'_> {
    fn drop(&mut self) {
        if let Ok(mut state) = self.admission.state.lock() {
            state.in_flight -= 1;
            self.admission.changed.notify_waiters();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use provider_api::Cancellation;
    use std::{
        future::Future,
        task::{Context, Poll, Waker},
    };

    fn context(priority: RequestPriority) -> RequestContext {
        RequestContext {
            priority,
            deadline: Instant::now() + Duration::from_secs(60),
            cancellation: Cancellation::default(),
        }
    }

    #[tokio::test]
    async fn active_waiters_precede_shadow_and_dropped_tickets_release_queue_slots() {
        let admission = Admission::new(1, Duration::ZERO);
        let first_context = context(RequestPriority::default());
        let first = admission.acquire(&first_context).await.unwrap();
        let shadow_context = context(RequestPriority::shadow());
        let active_context = context(RequestPriority::default());
        let mut shadow = Box::pin(admission.acquire(&shadow_context));
        let mut active = Box::pin(admission.acquire(&active_context));
        let mut task_context = Context::from_waker(Waker::noop());
        assert!(shadow.as_mut().poll(&mut task_context).is_pending());
        assert!(active.as_mut().poll(&mut task_context).is_pending());
        assert_eq!(admission.lock().unwrap().waiting.len(), 2);
        drop(first);
        assert!(shadow.as_mut().poll(&mut task_context).is_pending());
        let Poll::Ready(Ok(permit)) = active.as_mut().poll(&mut task_context) else {
            panic!("active request should be admitted first")
        };
        assert_eq!(admission.lock().unwrap().in_flight, 1);
        drop(permit);
        drop(shadow);
        assert!(admission.lock().unwrap().waiting.is_empty());
        assert_eq!(admission.lock().unwrap().in_flight, 0);
    }

    #[tokio::test]
    async fn queued_cancellation_removes_ticket_and_cooldown_never_shortens() {
        let admission = Admission::new(1, Duration::ZERO);
        let context = context(RequestPriority::default());
        let first = admission.acquire(&context).await.unwrap();
        let mut waiting = Box::pin(admission.acquire(&context));
        let mut task_context = Context::from_waker(Waker::noop());
        assert!(waiting.as_mut().poll(&mut task_context).is_pending());
        context.cancellation.cancel();
        assert!(matches!(waiting.await, Err(ProviderError::Cancelled)));
        assert!(admission.lock().unwrap().waiting.is_empty());
        drop(first);
        admission.cooldown(Duration::from_secs(60)).unwrap();
        let original = admission.lock().unwrap().next_start;
        admission.cooldown(Duration::from_secs(1)).unwrap();
        assert_eq!(admission.lock().unwrap().next_start, original);
    }

    #[tokio::test]
    async fn queue_bounds_reject_shadow_overflow_and_id_exhaustion() {
        let admission = Admission::new(1, Duration::from_millis(100));
        {
            let mut state = admission.lock().unwrap();
            for id in 0..MAX_SHADOW_WAITERS {
                state.waiting.push_back(Waiter {
                    id: id as u64,
                    priority: RequestPriority::shadow(),
                });
            }
        }

        assert!(matches!(
            admission.acquire(&context(RequestPriority::shadow())).await,
            Err(ProviderError::RateLimited {
                retry_after_ms: 100
            })
        ));
        {
            let mut state = admission.lock().unwrap();
            state.waiting.clear();
            state.next_id = u64::MAX;
        }

        assert!(matches!(
            admission
                .acquire(&context(RequestPriority::default()))
                .await,
            Err(ProviderError::InvalidRequest)
        ));
    }
}
