use super::*;
use domain::*;
use notification_api::*;
use provider_api::{Cancellation, RequestContext};
use state_api::*;
use std::sync::{Arc, Mutex};
use strategy_api::*;
#[path = "../../../test-support/decision.rs"]
mod fixtures;
#[path = "../../../test-support/signal.rs"]
mod signals;

struct FixedClock;
impl Clock for FixedClock {
    fn now(&self) -> Result<TimestampMs, StateError> {
        Ok(TimestampMs::new(600_000).unwrap())
    }
}

struct Sink {
    id: InstanceId,
    result: Result<(), NotificationError>,
    calls: Mutex<usize>,
}

#[async_trait::async_trait]
impl NotificationSink for Sink {
    fn id(&self) -> &InstanceId {
        &self.id
    }
    async fn send(
        &self,
        notification: &Notification,
        _: &RequestContext,
    ) -> Result<DeliveryReceipt, NotificationError> {
        assert!(!notification.links.is_empty());
        *self.calls.lock().unwrap() += 1;
        self.result.clone()?;
        Ok(DeliveryReceipt {
            remote_message_id: Some("42".into()),
            accepted_at: FixedClock.now().unwrap(),
        })
    }
}

#[tokio::test]
async fn acceptance_suppresses_shadow_and_nonmatches_and_deduplicates_active_matches() {
    let state = state_memory::MemoryState::new(Default::default()).unwrap();
    let targets = [DeliveryTarget {
        sink: InstanceId::new("console").unwrap(),
        destination: "stdout".into(),
    }];
    let now = FixedClock.now().unwrap();
    let mut decision = fixtures::decision();
    decision.mode = StrategyMode::Shadow;
    assert!(matches!(
        accept(&state, &decision, None, &targets, now)
            .await
            .unwrap(),
        Acceptance::Suppressed
    ));
    decision.mode = StrategyMode::Active;
    decision.verdict = Verdict::Watch;
    assert!(matches!(
        accept(&state, &decision, None, &targets, now)
            .await
            .unwrap(),
        Acceptance::Suppressed
    ));
    assert_eq!(state.stats().unwrap().signals, 0);
    decision.verdict = Verdict::Match;
    assert!(matches!(
        accept(&state, &decision, Some("A\nB"), &targets, now)
            .await
            .unwrap(),
        Acceptance::Accepted(_)
    ));
    assert!(matches!(
        accept(&state, &decision, None, &targets, now)
            .await
            .unwrap(),
        Acceptance::Suppressed
    ));
    let signals = state.list_signals(10).await.unwrap();
    assert_eq!(signals[0].symbol.as_deref(), Some("AB"));
    assert_eq!(signals[0].kind, signal_api::SignalKind::ResearchMatch);
    decision.as_of = TimestampMs::new(600_001).unwrap();
    assert_eq!(
        accept(&state, &decision, None, &targets, now)
            .await
            .unwrap_err(),
        StateError::InvalidRequest
    );
}

#[tokio::test]
async fn delivery_handles_success_permanent_failure_retry_and_unknown_outcome() {
    for outcome in [
        Ok(()),
        Err(NotificationError::Unauthorized),
        Err(NotificationError::Transport),
        Err(NotificationError::UnknownOutcome),
        Err(NotificationError::RateLimited {
            retry_after_ms: 1000,
        }),
    ] {
        let state = state_memory::MemoryState::new(Default::default()).unwrap();
        let sink = Arc::new(Sink {
            id: InstanceId::new("sink").unwrap(),
            result: outcome.clone(),
            calls: Mutex::new(0),
        });
        let targets = [DeliveryTarget {
            sink: sink.id.clone(),
            destination: "stdout".into(),
        }];
        accept(
            &state,
            &fixtures::decision(),
            None,
            &targets,
            FixedClock.now().unwrap(),
        )
        .await
        .unwrap();
        let sinks: delivery::Sinks =
            [(sink.id.clone(), sink.clone() as Arc<dyn NotificationSink>)].into();
        assert!(
            delivery::deliver_one(
                &state,
                &sinks,
                &FixedClock,
                &Cancellation::default(),
                &delivery::DeliveryPolicy::default()
            )
            .await
            .unwrap()
        );
        assert_eq!(*sink.calls.lock().unwrap(), 1);
        let stats = state.stats().unwrap();
        match outcome {
            Ok(()) => assert_eq!(stats.delivered, 1),
            Err(NotificationError::Unauthorized) => assert_eq!(stats.failed, 1),
            _ => assert_eq!(stats.pending, 1),
        }
    }
}

#[tokio::test]
async fn delivery_short_circuits_cancellation_empty_queue_and_missing_sink() {
    let state = state_memory::MemoryState::new(Default::default()).unwrap();
    let sinks = delivery::Sinks::new();
    let policy = delivery::DeliveryPolicy::default();
    assert!(
        !delivery::deliver_one(
            &state,
            &sinks,
            &FixedClock,
            &Cancellation::default(),
            &policy
        )
        .await
        .unwrap()
    );
    let targets = [DeliveryTarget {
        sink: InstanceId::new("missing").unwrap(),
        destination: "stdout".into(),
    }];
    accept(
        &state,
        &fixtures::decision(),
        None,
        &targets,
        FixedClock.now().unwrap(),
    )
    .await
    .unwrap();
    let cancelled = Cancellation::default();
    cancelled.cancel();
    assert!(
        !delivery::deliver_one(&state, &sinks, &FixedClock, &cancelled, &policy)
            .await
            .unwrap()
    );
    assert!(
        delivery::deliver_one(
            &state,
            &sinks,
            &FixedClock,
            &Cancellation::default(),
            &policy
        )
        .await
        .unwrap()
    );
    assert_eq!(state.stats().unwrap().failed, 1);
}

#[test]
fn signal_links_use_explicit_mainnet_hosts_and_omit_unknown_networks() {
    let mut signal = signals::signal();
    for (chain_id, expected) in [(1, "etherscan.io"), (8453, "basescan.org")] {
        signal.token = TokenId::new(ChainId::Evm { chain_id }, signal.token.address()).unwrap();
        let links = links::for_signal(&signal);
        assert_eq!(links.len(), 3);
        assert!(links[0].url.contains(expected));
        assert!(links[2].url.starts_with("https://dexscreener.com/"));
    }

    signal.token = TokenId::new(ChainId::Evm { chain_id: 42 }, signal.token.address()).unwrap();
    assert!(links::for_signal(&signal).is_empty());
    let solana = ChainId::Solana {
        genesis_hash: "5eykt4UsFv8P8NJdTREpY1vzqKqZKvdpKuc147dw2N9d".into(),
    };
    signal.token = TokenId::new(solana.clone(), "11111111111111111111111111111111").unwrap();
    signal.pool = PoolId::address(solana, "11111111111111111111111111111111").unwrap();
    assert_eq!(links::for_signal(&signal).len(), 3);
    assert!(SystemClock.now().unwrap().get() > 0);
}

#[test]
fn provider_token_age_uses_elapsed_time_and_ignores_missing_or_future_observations() {
    let mut decision = fixtures::decision();
    let mut facts = vec![];
    token_age::append(&mut facts, &decision);
    assert!(facts.is_empty());
    let observation = Observation {
        value: NonNegative::new(10.9).unwrap(),
        source: ProviderId::new("fixture").unwrap(),
        observed_at: None,
        fetched_at: TimestampMs::new(598_000).unwrap(),
    };
    decision.evidence.token_metrics =
        Some([(TokenMetric::ProviderSeenAgeSecs, observation)].into());
    token_age::append(&mut facts, &decision);
    assert!(matches!(&facts[0], signal_api::SignalFact::Text { value, .. } if value == "12"));
    decision
        .evidence
        .token_metrics
        .as_mut()
        .unwrap()
        .get_mut(&TokenMetric::ProviderSeenAgeSecs)
        .unwrap()
        .fetched_at = TimestampMs::new(600_001).unwrap();
    facts.clear();
    token_age::append(&mut facts, &decision);
    assert!(facts.is_empty());
}

#[tokio::test]
async fn range_signal_builder_rejects_each_missing_or_inconsistent_evidence_field() {
    let state = state_memory::MemoryState::new(Default::default()).unwrap();
    let targets = [DeliveryTarget {
        sink: InstanceId::new("sink").unwrap(),
        destination: "stdout".into(),
    }];
    for field in 0..7 {
        let mut decision = fixtures::decision();
        match field {
            0 => decision.pool = None,
            1 => decision.evidence.range = None,
            2 => decision.evidence.range.as_mut().unwrap().range = None,
            3 => decision.evidence.volume = None,
            4 => {
                decision
                    .evidence
                    .range
                    .as_mut()
                    .unwrap()
                    .range
                    .as_mut()
                    .unwrap()
                    .latest_inside_bounds = false
            }
            5 => decision.as_of = TimestampMs::new(539_999).unwrap(),
            _ => {
                decision.evidence.market.as_mut().unwrap().pool = PoolId::address(
                    ChainId::Evm { chain_id: 1 },
                    "0x9999999999999999999999999999999999999999",
                )
                .unwrap()
            }
        }

        assert_eq!(
            accept(&state, &decision, None, &targets, FixedClock.now().unwrap())
                .await
                .unwrap_err(),
            StateError::InvalidRequest
        );
    }

    assert_eq!(state.stats().unwrap().signals, 0);
}

#[test]
fn pump_signal_builder_uses_candle_event_identity_and_rejects_incomplete_evidence() {
    let mut decision = fixtures::decision();
    decision.evidence.range = None;
    decision.reasons.push(DecisionReason {
        code: "PUMP_HIGH".into(),
        field: "signal.pump".into(),
        actual: Some("gain=5;end=600000".into()),
        expected: "pump".into(),
    });
    let first = pump_signal::build(&decision, Some("A\nB"), FixedClock.now().unwrap()).unwrap();
    let later = pump_signal::build(&decision, None, TimestampMs::new(600_001).unwrap()).unwrap();
    assert_eq!(first.signal.id, later.signal.id);
    assert_eq!(first.signal.id, first.dedupe_key);
    assert_eq!(first.signal.created_at.get(), 600_000);
    assert_eq!(first.signal.expires_at.get(), 1_500_000);
    assert_eq!(first.cooldown_secs, 0);
    assert_eq!(first.signal.symbol.as_deref(), Some("AB"));
    for field in 0..5 {
        let mut invalid = decision.clone();
        match field {
            0 => invalid.warnings.clear(),
            1 => invalid.evidence.candle_input_hash = None,
            2 => invalid.evidence.volume = None,
            3 => invalid.reasons[0].actual = None,
            _ => invalid.reasons[0].actual = Some("end=600001".into()),
        }

        assert!(pump_signal::build(&invalid, None, FixedClock.now().unwrap()).is_err());
    }
}

#[tokio::test]
async fn successful_delivery_emits_a_valid_notification_delivered_event() {
    struct Events(Mutex<Vec<event_api::EventEnvelope>>);
    impl event_api::EventPublisher for Events {
        fn publish(&self, event: event_api::EventEnvelope) -> event_api::PublishReport {
            self.0.lock().unwrap().push(event);
            event_api::PublishReport {
                accepted_subscribers: 1,
                dropped_subscribers: 0,
            }
        }
    }

    let state = state_memory::MemoryState::new(Default::default()).unwrap();
    let sink = Arc::new(Sink {
        id: InstanceId::new("sink").unwrap(),
        result: Ok(()),
        calls: Mutex::new(0),
    });
    let targets = [DeliveryTarget {
        sink: sink.id.clone(),
        destination: "stdout".into(),
    }];
    accept(
        &state,
        &fixtures::decision(),
        None,
        &targets,
        FixedClock.now().unwrap(),
    )
    .await
    .unwrap();
    let sinks: delivery::Sinks = [(sink.id.clone(), sink as Arc<dyn NotificationSink>)].into();
    let events = Events(Mutex::new(vec![]));
    assert!(
        delivery::deliver_one_with_events(
            &state,
            &sinks,
            &FixedClock,
            &Cancellation::default(),
            &delivery::DeliveryPolicy::default(),
            &events
        )
        .await
        .unwrap()
    );
    let events = events.0.lock().unwrap();
    assert_eq!(events.len(), 1);
    assert!(events[0].validate().is_ok());
    assert!(matches!(
        events[0].payload,
        event_api::LifecycleEvent::NotificationDelivered { .. }
    ));
}
