use domain::*;
use notification_api::DeliveryReceipt;
use signal_api::*;
use state_api::*;
use state_memory::{Limits, MemoryState};

fn time(ms: i64) -> TimestampMs {
    TimestampMs::new(ms).unwrap()
}
fn draft(id: &str, key: &str) -> SignalDraft {
    let chain = ChainId::Evm { chain_id: 1 };
    SignalDraft {
        signal: Signal {
            id: ContentHash::of_bytes(id.as_bytes()),
            created_at: time(0),
            expires_at: time(900_000),
            strategy: InstanceId::new("high").unwrap(),
            config_digest: ContentHash::of_bytes(b"config"),
            token: TokenId::new(chain.clone(), "0x1111111111111111111111111111111111111111")
                .unwrap(),
            pool: PoolId::address(chain, "0x2222222222222222222222222222222222222222").unwrap(),
            kind: SignalKind::ResearchMatch,
            symbol: Some("TEST".into()),
            facts: vec![],
            reasons: vec![],
            warnings: vec![],
            snapshot_hash: ContentHash::of_bytes(b"snapshot"),
        },
        dedupe_key: ContentHash::of_bytes(key.as_bytes()),
        cooldown_secs: 60,
    }
}
fn targets() -> Vec<DeliveryTarget> {
    vec![DeliveryTarget {
        sink: InstanceId::new("console").unwrap(),
        destination: "stdout".into(),
    }]
}
fn worker() -> InstanceId {
    InstanceId::new("worker").unwrap()
}

#[tokio::test]
async fn concurrent_acceptance_is_atomic_and_pending_blocks_after_cooldown() {
    let state = MemoryState::new(Limits::default()).unwrap();
    let targets = targets();
    let (first, second) = tokio::join!(
        state.accept_signal_and_deliveries(draft("one", "same"), &targets, time(0)),
        state.accept_signal_and_deliveries(draft("two", "same"), &targets, time(0))
    );
    assert_eq!(
        [first.unwrap(), second.unwrap()]
            .iter()
            .filter(|result| matches!(result, Acceptance::Accepted(_)))
            .count(),
        1
    );
    assert!(matches!(
        state
            .accept_signal_and_deliveries(draft("three", "same"), &targets, time(61_000))
            .await
            .unwrap(),
        Acceptance::Suppressed
    ));
    assert_eq!(state.stats().unwrap().signals, 1);
    assert_eq!(state.stats().unwrap().deliveries, 1);
}

#[tokio::test]
async fn capacity_failure_does_not_insert_signal_or_cooldown() {
    let state = MemoryState::new(Limits {
        signals: 1,
        deliveries: 1,
        dedupe_keys: 10,
        retry_abandoned_sends: false,
    })
    .unwrap();
    state
        .accept_signal_and_deliveries(draft("one", "one"), &targets(), time(0))
        .await
        .unwrap();
    assert_eq!(
        state
            .accept_signal_and_deliveries(draft("two", "two"), &targets(), time(1))
            .await
            .unwrap_err(),
        StateError::Capacity
    );
    assert_eq!(state.stats().unwrap().signals, 1);
    let lease = state
        .claim_due(&worker(), time(2), 30, 1)
        .await
        .unwrap()
        .remove(0);
    state
        .complete(
            &lease,
            DeliveryReceipt {
                remote_message_id: None,
                accepted_at: time(3),
            },
            time(3),
        )
        .await
        .unwrap();
    assert!(matches!(
        state
            .accept_signal_and_deliveries(draft("two", "two"), &targets(), time(4))
            .await
            .unwrap(),
        Acceptance::Accepted(_)
    ));
}

#[tokio::test]
async fn stale_lease_cannot_acknowledge_new_attempt() {
    let state = MemoryState::new(Limits {
        retry_abandoned_sends: true,
        ..Limits::default()
    })
    .unwrap();
    state
        .accept_signal_and_deliveries(draft("one", "one"), &targets(), time(0))
        .await
        .unwrap();
    let old = state
        .claim_due(&worker(), time(0), 1, 1)
        .await
        .unwrap()
        .remove(0);
    let current = state
        .claim_due(&worker(), time(1001), 30, 1)
        .await
        .unwrap()
        .remove(0);
    assert_eq!(current.attempt, 2);
    assert_ne!(old.lease_token, current.lease_token);
    assert_eq!(
        state
            .complete(
                &old,
                DeliveryReceipt {
                    remote_message_id: None,
                    accepted_at: time(1002)
                },
                time(1002)
            )
            .await
            .unwrap_err(),
        StateError::Conflict
    );
    state
        .complete(
            &current,
            DeliveryReceipt {
                remote_message_id: None,
                accepted_at: time(1003),
            },
            time(1003),
        )
        .await
        .unwrap();
    assert_eq!(state.stats().unwrap().delivered, 1);
}

#[tokio::test]
async fn unknown_outcome_is_parked_and_expiry_releases_it() {
    let state = MemoryState::new(Limits::default()).unwrap();
    state
        .accept_signal_and_deliveries(draft("one", "one"), &targets(), time(0))
        .await
        .unwrap();
    let lease = state
        .claim_due(&worker(), time(0), 30, 1)
        .await
        .unwrap()
        .remove(0);
    state
        .retry(&lease, time(900_000), true, time(1))
        .await
        .unwrap();
    assert!(matches!(
        state.delivery_status(&lease.delivery_id).unwrap(),
        DeliveryStatus::UnknownOutcome
    ));
    assert!(
        state
            .claim_due(&worker(), time(31_000), 30, 1)
            .await
            .unwrap()
            .is_empty()
    );
    assert!(
        state
            .claim_due(&worker(), time(900_000), 30, 1)
            .await
            .unwrap()
            .is_empty()
    );
    assert!(matches!(
        state.delivery_status(&lease.delivery_id).unwrap(),
        DeliveryStatus::Expired
    ));
}

#[tokio::test]
async fn completed_destination_is_not_retried_with_failed_destination() {
    let state = MemoryState::new(Limits::default()).unwrap();
    let mut targets = targets();
    targets.push(DeliveryTarget {
        sink: InstanceId::new("telegram").unwrap(),
        destination: "chat".into(),
    });
    state
        .accept_signal_and_deliveries(draft("one", "one"), &targets, time(0))
        .await
        .unwrap();
    let leases = state.claim_due(&worker(), time(0), 30, 2).await.unwrap();
    state
        .complete(
            &leases[0],
            DeliveryReceipt {
                remote_message_id: None,
                accepted_at: time(1),
            },
            time(1),
        )
        .await
        .unwrap();
    state
        .retry(&leases[1], time(5000), false, time(1))
        .await
        .unwrap();
    let retry = state.claim_due(&worker(), time(5000), 30, 2).await.unwrap();
    assert_eq!(retry.len(), 1);
    assert_eq!(retry[0].delivery_id, leases[1].delivery_id);
    assert_eq!(state.stats().unwrap().delivered, 1);
}

async fn pagination_contract(state: &dyn SignalRepository) {
    for (label, at) in [("first", 0), ("second", 100), ("third", 100)] {
        let mut item = draft(label, label);
        item.signal.created_at = time(at);
        state
            .accept_signal_and_deliveries(item, &targets(), time(at))
            .await
            .unwrap();
    }
    let full = state.list_signals(10).await.unwrap();
    let page = state.signal_history(None, 1).await.unwrap();
    assert_eq!(page[0].id, full[0].id);
    let cursor = SignalCursor {
        created_at: page[0].created_at,
        id: page[0].id.clone(),
    };
    let second = state.signal_history(Some(&cursor), 1).await.unwrap();
    assert_eq!(second[0].id, full[1].id);
    let cursor = SignalCursor {
        created_at: second[0].created_at,
        id: second[0].id.clone(),
    };
    let last = state.signal_history(Some(&cursor), 200).await.unwrap();
    assert_eq!(last.len(), 1);
    assert_eq!(last[0].id, full[2].id);
    assert_eq!(
        state.signal_history(None, 0).await.unwrap_err(),
        StateError::InvalidRequest
    );
    assert_eq!(
        state.signal_history(None, 201).await.unwrap_err(),
        StateError::InvalidRequest
    );
}

#[tokio::test]
async fn stable_signal_keyset_pagination_handles_equal_times() {
    pagination_contract(&MemoryState::new(Default::default()).unwrap()).await;
}
