use super::*;
use test_support::market;
use test_support::signal as fixtures;

fn target() -> DeliveryTarget {
    DeliveryTarget {
        sink: InstanceId::new("console").unwrap(),
        destination: "stdout".into(),
    }
}

#[test]
fn acceptance_checks_time_chain_limits_destination_and_duplicate_targets() {
    let draft = fixtures::draft();
    let now = TimestampMs::new(1000).unwrap();
    assert!(validate_acceptance(&draft, &[target()], now).is_ok());
    assert!(validate_acceptance(&draft, &[], now).is_err());
    assert!(validate_acceptance(&draft, &[target(), target()], now).is_err());
    assert!(validate_acceptance(&draft, &vec![target(); 17], now).is_err());
    for destination in ["", &"x".repeat(257)] {
        let mut target = target();
        target.destination = destination.into();
        assert!(validate_acceptance(&draft, &[target], now).is_err());
    }

    for field in 0..8 {
        let mut invalid = draft.clone();
        match field {
            0 => invalid.signal.created_at = TimestampMs::new(1001).unwrap(),
            1 => invalid.signal.expires_at = now,
            2 => invalid.cooldown_secs = 604_801,
            3 => {
                invalid.signal.pool = PoolId::address(
                    ChainId::Evm { chain_id: 2 },
                    invalid.signal.pool.as_address().unwrap(),
                )
                .unwrap()
            }
            4 => invalid.signal.reasons = vec!["reason".into(); 33],
            5 => invalid.signal.warnings = vec!["warning".into(); 33],
            6 => {
                invalid.signal.facts = vec![
                    signal_api::SignalFact::Text {
                        label: "x".into(),
                        value: "y".into()
                    };
                    33
                ]
            }
            _ => invalid.signal.symbol = Some("x".repeat(16_384)),
        }

        assert_eq!(
            validate_acceptance(&invalid, &[target()], now),
            Err(StateError::InvalidRequest)
        );
    }
}

fn checkpoint() -> MonitorCheckpoint {
    MonitorCheckpoint {
        schema_version: 1,
        scope: ContentHash::of_bytes(b"scope"),
        saved_at: market::timestamp(600_000),
        cursor: None,
        next_discovery: market::timestamp(660_000),
        sequence: 1,
        watch: vec![WatchRecord {
            candidate: market::candidate(),
            next_due: market::timestamp(660_000),
            last_seen: market::timestamp(600_000),
            order: 1,
        }],
    }
}

#[test]
fn checkpoint_validation_checks_bounds_unique_tokens_orders_and_chain_consistency() {
    let valid = checkpoint();
    assert!(valid.validate().is_ok());
    for field in 0..10 {
        let mut invalid = valid.clone();
        match field {
            0 => invalid.schema_version = 2,
            1 => invalid.watch = vec![valid.watch[0].clone(); 1001],
            2 => {
                invalid.cursor = Some(discovery_api::DiscoveryCursor {
                    query_digest: valid.scope.clone(),
                    value: "x".repeat(4097),
                })
            }
            3 => invalid.watch[0].last_seen = market::timestamp(600_001),
            4 => invalid.watch[0].order = 2,
            5 => invalid.watch.push(valid.watch[0].clone()),
            6 => {
                let mut duplicate_order = valid.watch[0].clone();
                duplicate_order.candidate.token = market::token(3);
                invalid.watch.push(duplicate_order);
            }
            7 => invalid.watch[0].candidate.pool_hints = vec![market::pool_id(10); 129],
            8 => invalid.watch[0].candidate.symbol = Some("x".repeat(257)),
            _ => {
                invalid.watch[0].candidate.pool_hints = vec![
                    PoolId::address(
                        ChainId::Evm { chain_id: 2 },
                        "0x1111111111111111111111111111111111111111",
                    )
                    .unwrap(),
                ]
            }
        }

        assert_eq!(invalid.validate(), Err(StateError::InvalidRequest));
    }
}
