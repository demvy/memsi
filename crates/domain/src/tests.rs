use super::*;
use serde::{Serialize, de::DeserializeOwned};

fn round_trip<T>(value: &T)
where
    T: Serialize + DeserializeOwned + PartialEq + std::fmt::Debug,
{
    let encoded = serde_json::to_string(value).unwrap();
    assert_eq!(&serde_json::from_str::<T>(&encoded).unwrap(), value);
}

fn chain() -> ChainId {
    ChainId::Evm { chain_id: 1 }
}

fn address(digit: char) -> String {
    format!("0x{}", digit.to_string().repeat(40))
}

#[test]
fn identifiers_validate_boundaries_and_preserve_conversions() {
    macro_rules! check {
        ($($kind:ty),+) => {$(
            for text in ["a", "0", "provider.codex", "a_b-c", &"a".repeat(96)] {
                let identifier = <$kind>::new(text).unwrap();
                assert_eq!(identifier.as_str(), text);
                assert_eq!(identifier.to_string(), text);
                assert_eq!(String::from(identifier.clone()), text);
                assert_eq!(<$kind>::try_from(text.to_owned()).unwrap(), identifier);
                round_trip(&identifier);
            }

            for text in ["", "A", "_a", "a b", "a/b", "é", &"a".repeat(97)] {
                assert!(<$kind>::new(text).is_err(), "accepted {text:?}");
                assert!(serde_json::from_value::<$kind>(serde_json::json!(text)).is_err());
            }
        )+};
    }

    check!(
        PluginId,
        InstanceId,
        CapabilityId,
        SchemaId,
        ProviderId,
        ProtocolId,
        BindingId
    );
}

#[test]
fn numeric_wrappers_reject_nonfinite_and_out_of_range_values() {
    for value in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY, -1.0] {
        assert!(NonNegative::new(value).is_err());
        assert!(PositivePrice::new(value).is_err());
        assert!(UnitInterval::new(value).is_err());
    }

    for value in [0.0, 0.5, 1.0] {
        let number = NonNegative::try_from(value).unwrap();
        assert_eq!(number.get(), value);
        assert_eq!(f64::from(number), value);
        round_trip(&number);

        let fraction = UnitInterval::try_from(value).unwrap();
        assert_eq!(fraction.get(), value);
        assert_eq!(f64::from(fraction), value);
        round_trip(&fraction);
    }

    let price = PositivePrice::try_from(2.5).unwrap();
    assert_eq!(price.get(), 2.5);
    assert_eq!(f64::from(price), 2.5);
    round_trip(&price);
    assert!(PositivePrice::new(0.0).is_err());
    assert!(UnitInterval::new(1.01).is_err());
    assert!(!NonNegative::new(-0.0).unwrap().get().is_sign_negative());
}

#[test]
fn timestamps_and_resolutions_check_arithmetic_boundaries() {
    assert!(TimestampMs::new(-1).is_err());
    assert!(TimestampMs::from_seconds(i64::MAX).is_err());
    assert!(TimestampMs::from_seconds(-1).is_err());
    assert!(TimestampMs::new(i64::MAX).unwrap().checked_add(1).is_err());
    assert!(TimestampMs::new(0).unwrap().checked_add(-1).is_err());

    let timestamp = TimestampMs::from_seconds(125).unwrap();
    assert_eq!(timestamp.get(), 125_000);
    assert_eq!(i64::from(timestamp), 125_000);
    assert_eq!(TimestampMs::try_from(125_000).unwrap(), timestamp);
    assert_eq!(timestamp.checked_add(1).unwrap().get(), 125_001);
    round_trip(&timestamp);

    for seconds in [1, 60, 604_800] {
        let resolution = Resolution::try_from(seconds).unwrap();
        assert_eq!(resolution.seconds(), seconds);
        assert_eq!(resolution.millis(), seconds as i64 * 1000);
        assert_eq!(u64::from(resolution), seconds);
        round_trip(&resolution);
    }

    assert!(Resolution::new(0).is_err());
    assert!(Resolution::new(604_801).is_err());
    let resolution = Resolution::new(60).unwrap();
    assert_eq!(
        timestamp.closed_cutoff(resolution, 5000).unwrap().get(),
        120_000
    );
    assert_eq!(
        timestamp.closed_cutoff(resolution, 5001).unwrap().get(),
        60_000
    );
    assert!(timestamp.closed_cutoff(resolution, 125_001).is_err());
    assert!(timestamp.closed_cutoff(resolution, u64::MAX).is_err());
}

#[test]
fn knowledge_distinguishes_missing_from_known_zero() {
    assert_eq!(Knowledge::known(0).as_ref(), Some(&0));
    assert_eq!(Knowledge::<u8>::unknown().as_ref(), None);
    round_trip(&Knowledge::known(0));
    round_trip(&Knowledge::<u8>::unknown());
}

#[test]
fn atomic_amounts_validate_uint256_and_add_without_precision_loss() {
    let maximum = "115792089237316195423570985008687907853269984665640564039457584007913129639935";
    for text in [
        "",
        "00",
        "01",
        "-1",
        "+1",
        "1.0",
        " 1",
        "١",
        &"9".repeat(79),
    ] {
        assert!(AtomicAmount::new(text).is_err(), "accepted {text:?}");
    }

    let limit = AtomicAmount::new(maximum).unwrap();
    assert_eq!(limit.as_str(), maximum);
    assert!(limit.checked_add(&AtomicAmount::from(1u64)).is_err());
    assert_eq!(limit.checked_add(&AtomicAmount::from(0u64)).unwrap(), limit);
    assert!(AtomicAmount::new(format!("{}936", &maximum[..maximum.len() - 3])).is_err());

    let amount = AtomicAmount::from(999u64);
    assert_eq!(
        amount
            .checked_add(&AtomicAmount::from(1u128))
            .unwrap()
            .as_str(),
        "1000"
    );
    assert!(AtomicAmount::from(0u64).is_zero());
    assert!(!amount.is_zero());
    assert!(AtomicAmount::from(10u64) > AtomicAmount::from(9u64));
    assert!(AtomicAmount::from(11u64) > AtomicAmount::from(10u64));
    assert_eq!(amount.partial_cmp(&amount), Some(std::cmp::Ordering::Equal));
    assert_eq!(amount.units(0), "999");
    assert_eq!(amount.units(2), "9.99");
    assert_eq!(amount.units(3), "0.999");
    assert_eq!(amount.units(5), "0.00999");
    assert_eq!(amount.to_string(), "999");
    assert_eq!(AtomicAmount::try_from("999".to_owned()).unwrap(), amount);
    assert_eq!(String::from(amount.clone()), "999");
    round_trip(&amount);
}

#[test]
fn chain_addresses_are_validated_and_canonicalized() {
    assert!(ChainId::Evm { chain_id: 0 }.validate().is_err());
    assert_eq!(chain().address(&address('A')).unwrap(), address('a'));
    for input in ["", "0x1234", &address('g'), &"a".repeat(40)] {
        assert!(chain().address(input).is_err());
    }

    let solana = ChainId::Solana {
        genesis_hash: bs58::encode([1u8; 32]).into_string(),
    };
    let public_key = bs58::encode([2u8; 32]).into_string();
    assert!(solana.validate().is_ok());
    assert_eq!(solana.address(&public_key).unwrap(), public_key);
    for input in ["0", "1", &"1".repeat(45)] {
        assert!(solana.address(input).is_err());
    }

    assert!(
        ChainId::Solana {
            genesis_hash: "1".into()
        }
        .validate()
        .is_err()
    );
    assert!(
        ChainId::Solana {
            genesis_hash: "0".repeat(65)
        }
        .validate()
        .is_err()
    );
}

#[test]
fn token_and_pool_identities_validate_all_locator_forms() {
    let token = TokenId::new(chain(), &address('A')).unwrap();
    assert_eq!(token.chain(), &chain());
    assert_eq!(token.address(), address('a'));
    round_trip(&token);

    let pool = PoolId::address(chain(), &address('B')).unwrap();
    assert_eq!(pool.chain(), &chain());
    assert_eq!(pool.as_address(), Some(address('b').as_str()));
    assert!(matches!(pool.locator(), PoolLocator::Address { .. }));
    round_trip(&pool);

    let locator = |key: &str| PoolLocator::Opaque {
        protocol: ProtocolId::new("uniswap.v4").unwrap(),
        manager: address('C'),
        key: key.into(),
    };
    let opaque = PoolId::new(chain(), locator("pool key")).unwrap();
    assert_eq!(opaque.as_address(), None);
    round_trip(&opaque);
    for key in ["", "\n", "é", &"a".repeat(257)] {
        assert!(PoolId::new(chain(), locator(key)).is_err());
    }

    assert!(TokenId::new(chain(), "bad").is_err());
    assert!(PoolId::address(chain(), "bad").is_err());
}

#[test]
fn content_hashes_are_stable_validated_sha256_strings() {
    let hash = ContentHash::of_bytes(b"abc");
    assert_eq!(
        hash.as_str(),
        "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
    );
    assert_eq!(
        ContentHash::try_from(hash.as_str().to_owned()).unwrap(),
        hash
    );
    assert_eq!(String::from(hash.clone()), hash.as_str());
    round_trip(&hash);
    for invalid in ["", &"A".repeat(64), &"g".repeat(64), &"a".repeat(63)] {
        assert!(ContentHash::try_from(invalid.to_owned()).is_err());
    }

    assert_eq!(
        ContentHash::of(&serde_json::json!({"a": 1, "b": 2})).unwrap(),
        ContentHash::of(&serde_json::json!({"b": 2, "a": 1})).unwrap()
    );
    assert_ne!(ContentHash::of(&1).unwrap(), ContentHash::of(&2).unwrap());
}

#[test]
fn activities_require_locations_matching_the_transaction_chain() {
    let transaction = ChainTransaction::new(chain(), &format!("0X{}", "A".repeat(64))).unwrap();
    assert_eq!(transaction.chain(), &chain());
    assert_eq!(transaction.hash(), format!("0x{}", "a".repeat(64)));
    round_trip(&transaction);
    for invalid in ["", "0x12", &format!("0x{}", "g".repeat(64))] {
        assert!(ChainTransaction::new(chain(), invalid).is_err());
    }

    let location = ActivityLocation::EvmLog {
        log_ordinal: 2,
        effect_index: 1,
    };
    let activity = ActivityId::new(transaction.clone(), location.clone()).unwrap();
    assert_eq!(activity.transaction(), &transaction);
    assert_eq!(activity.location(), &location);
    round_trip(&activity);
    assert!(ActivityId::new(transaction.clone(), ActivityLocation::TransactionFee).is_ok());
    assert!(
        ActivityId::new(
            transaction,
            ActivityLocation::SolanaInstruction {
                instruction: 0,
                inner: None,
                effect_index: 0
            }
        )
        .is_err()
    );

    let solana = ChainId::Solana {
        genesis_hash: bs58::encode([1u8; 32]).into_string(),
    };
    let signature = bs58::encode([2u8; 64]).into_string();
    let transaction = ChainTransaction::new(solana.clone(), &signature).unwrap();
    round_trip(&transaction);
    assert!(
        ActivityId::new(
            transaction.clone(),
            ActivityLocation::SolanaInstruction {
                instruction: 1,
                inner: Some(2),
                effect_index: 3
            }
        )
        .is_ok()
    );
    assert!(ActivityId::new(transaction, location).is_err());
    for invalid in ["0", "1", &"1".repeat(89)] {
        assert!(ChainTransaction::new(solana.clone(), invalid).is_err());
    }
}

#[test]
fn metric_bounds_enforce_units_and_inclusive_or_exclusive_limits() {
    let closed = MetricBounds {
        gte: Some(1.0),
        lte: Some(2.0),
        ..Default::default()
    };
    assert!(closed.validate(TokenMetric::SniperCount).is_ok());
    for value in [1.0, 1.5, 2.0] {
        assert!(closed.accepts(value));
    }
    for value in [0.0, 3.0, f64::NAN, f64::INFINITY] {
        assert!(!closed.accepts(value));
    }
    let open = MetricBounds {
        gt: Some(1.0),
        lt: Some(2.0),
        ..Default::default()
    };
    assert!(open.validate(TokenMetric::MarketCapUsd).is_ok());
    assert!(!open.accepts(1.0));
    assert!(open.accepts(1.5));
    assert!(!open.accepts(2.0));

    for bounds in [
        MetricBounds::default(),
        MetricBounds {
            gt: Some(1.0),
            gte: Some(1.0),
            ..Default::default()
        },
        MetricBounds {
            lt: Some(1.0),
            lte: Some(1.0),
            ..Default::default()
        },
        MetricBounds {
            gt: Some(1.0),
            lte: Some(1.0),
            ..Default::default()
        },
        MetricBounds {
            gte: Some(2.0),
            lte: Some(1.0),
            ..Default::default()
        },
        MetricBounds {
            gt: Some(f64::NAN),
            ..Default::default()
        },
        MetricBounds {
            gt: Some(-1.0),
            ..Default::default()
        },
    ] {
        assert!(bounds.validate(TokenMetric::MarketCapUsd).is_err());
    }

    assert!(
        MetricBounds {
            gte: Some(1.0),
            lte: Some(1.0),
            ..Default::default()
        }
        .validate(TokenMetric::SniperCount)
        .is_ok()
    );
    assert!(
        MetricBounds {
            lte: Some(101.0),
            ..Default::default()
        }
        .validate(TokenMetric::DevSupplyPct)
        .is_err()
    );
    assert!(
        MetricBounds {
            gte: Some(0.5),
            ..Default::default()
        }
        .validate(TokenMetric::BundlerCount)
        .is_err()
    );
    assert!(TokenMetric::DevSupplyPct.is_percentage());
    assert!(!TokenMetric::MarketCapUsd.is_percentage());
    assert!(TokenMetric::BundlerCount.is_count());
    assert!(!TokenMetric::DevSupplyPct.is_count());
}

fn candle(open_time: i64) -> Candle {
    Candle::new(
        TimestampMs::new(open_time).unwrap(),
        2.0,
        3.0,
        1.0,
        2.5,
        Knowledge::unknown(),
    )
    .unwrap()
}

fn series() -> CandleSeries {
    CandleSeries {
        pool: PoolId::address(chain(), &address('a')).unwrap(),
        priced_token: TokenId::new(chain(), &address('b')).unwrap(),
        price_unit: PriceUnit::UsdPerToken,
        volume_unit: VolumeUnit::Usd,
        resolution: Resolution::new(60).unwrap(),
        end_exclusive: TimestampMs::new(120_000).unwrap(),
        candles: vec![candle(0), candle(60_000)].into(),
        provenance: SeriesProvenance {
            provider: ProviderId::new("test").unwrap(),
            contract_version: "1".into(),
            fetched_at: TimestampMs::new(120_000).unwrap(),
            input_hash: ContentHash::of_bytes(b"series"),
        },
    }
}

#[test]
fn candles_reject_invalid_prices_and_ohlc_ordering() {
    let valid = candle(0);
    round_trip(&valid);
    let mut encoded = serde_json::to_value(&valid).unwrap();
    assert!(encoded.get("buy_volume").is_none());
    encoded["buy_volume"] = serde_json::json!({"status": "known", "value": 1.0});
    let decoded: Candle = serde_json::from_value(encoded.clone()).unwrap();
    assert_eq!(decoded.buy_volume.as_ref().unwrap().get(), 1.0);
    assert!(
        serde_json::to_value(decoded)
            .unwrap()
            .get("buy_volume")
            .is_some()
    );
    encoded["low"] = serde_json::json!(4.0);
    assert!(serde_json::from_value::<Candle>(encoded).is_err());

    for (open, high, low, close) in [
        (0.0, 3.0, 1.0, 2.0),
        (2.0, 1.0, 1.0, 2.0),
        (2.0, 3.0, 2.5, 2.0),
        (2.0, 3.0, 1.0, 4.0),
        (2.0, 3.0, 1.0, 0.5),
        (2.0, f64::NAN, 1.0, 2.0),
    ] {
        assert!(
            Candle::new(
                TimestampMs::new(0).unwrap(),
                open,
                high,
                low,
                close,
                Knowledge::unknown()
            )
            .is_err()
        );
    }
}

#[test]
fn candle_series_require_aligned_contiguous_closed_history() {
    let valid = series();
    assert!(valid.validate(2).is_ok());
    assert!(valid.validate(0).is_err());
    assert!(valid.validate(3).is_err());
    let mut invalid = valid.clone();
    invalid.priced_token = TokenId::new(ChainId::Evm { chain_id: 2 }, &address('b')).unwrap();
    assert!(invalid.validate(1).is_err());
    invalid = valid.clone();
    invalid.end_exclusive = TimestampMs::new(120_001).unwrap();
    assert!(invalid.validate(1).is_err());
    invalid = valid.clone();
    invalid.candles = vec![candle(1), candle(60_000)].into();
    assert!(invalid.validate(1).is_err());
    invalid.candles = vec![candle(0), candle(120_000)].into();
    assert!(invalid.validate(1).is_err());
    invalid.candles = vec![candle(0)].into();
    assert!(invalid.validate(1).is_err());
    let mut bad_candle = candle(60_000);
    bad_candle.high = PositivePrice::new(1.0).unwrap();
    invalid.candles = vec![candle(0), bad_candle].into();
    assert!(invalid.validate(1).is_err());
}

#[test]
fn observations_distinguish_source_time_fallback_and_future_data() {
    let mut observation = Observation {
        value: 1,
        source: ProviderId::new("test").unwrap(),
        observed_at: Some(TimestampMs::new(100).unwrap()),
        fetched_at: TimestampMs::new(150).unwrap(),
    };
    assert!(observation.is_fresh(TimestampMs::new(200).unwrap(), 100, true));
    assert!(!observation.is_fresh(TimestampMs::new(201).unwrap(), 100, true));
    assert!(!observation.is_fresh(TimestampMs::new(99).unwrap(), 100, false));
    observation.observed_at = None;
    assert!(!observation.is_fresh(TimestampMs::new(200).unwrap(), 100, true));
    assert!(observation.is_fresh(TimestampMs::new(200).unwrap(), 50, false));
    assert!(!observation.is_fresh(TimestampMs::new(149).unwrap(), 100, false));
}

fn proof() -> PoolVerificationEvidence {
    PoolVerificationEvidence {
        source: ProviderId::new("test").unwrap(),
        method_version: "v1".into(),
        state: ChainState::Evm {
            block_number: 1,
            block_hash: format!("0x{}", "a".repeat(64)),
        },
        observed_at: TimestampMs::new(100).unwrap(),
        available_at: TimestampMs::new(200).unwrap(),
        input_hash: ContentHash::of_bytes(b"proof"),
        valid_until: TimestampMs::new(300).unwrap(),
    }
}

#[test]
fn pool_verification_enforces_chain_method_and_time_consistency() {
    let valid = proof();
    assert!(valid.validate(&chain()).is_ok());
    assert!(valid.is_fresh(TimestampMs::new(200).unwrap(), 100));
    assert!(valid.is_fresh(TimestampMs::new(300).unwrap(), 200));
    assert!(!valid.is_fresh(TimestampMs::new(199).unwrap(), u64::MAX));
    assert!(!valid.is_fresh(TimestampMs::new(301).unwrap(), u64::MAX));
    assert!(!valid.is_fresh(TimestampMs::new(300).unwrap(), 199));
    round_trip(&valid);

    for method in ["", "\n", &"a".repeat(129)] {
        let mut invalid = valid.clone();
        invalid.method_version = method.into();
        assert!(invalid.validate(&chain()).is_err());
    }

    let mut invalid = valid.clone();
    invalid.observed_at = TimestampMs::new(201).unwrap();
    assert!(invalid.validate(&chain()).is_err());
    assert!(!invalid.is_fresh(TimestampMs::new(250).unwrap(), u64::MAX));
    invalid = valid.clone();
    invalid.available_at = TimestampMs::new(301).unwrap();
    assert!(invalid.validate(&chain()).is_err());
    invalid = valid.clone();
    invalid.state = ChainState::Evm {
        block_number: 1,
        block_hash: "bad".into(),
    };
    assert!(invalid.validate(&chain()).is_err());
    invalid.state = ChainState::Solana {
        slot: 1,
        commitment: SolanaCommitment::Finalized,
    };
    assert!(invalid.validate(&chain()).is_err());
    let solana = ChainId::Solana {
        genesis_hash: bs58::encode([1u8; 32]).into_string(),
    };
    assert!(invalid.validate(&solana).is_ok());
    assert!(valid.validate(&solana).is_err());
}

#[test]
fn pool_snapshots_validate_tokens_and_verified_kind() {
    let mut pool = PoolSnapshot {
        pool: PoolId::address(chain(), &address('a')).unwrap(),
        token0: TokenId::new(chain(), &address('b')).unwrap(),
        token1: TokenId::new(chain(), &address('c')).unwrap(),
        kind: PoolKind::ConcentratedTicks,
        protocol: Some(ProtocolId::new("test").unwrap()),
        kind_verified: true,
        verification: Some(proof()),
        liquidity_usd: Knowledge::unknown(),
        volume_24h_usd: Knowledge::unknown(),
        created_at: Knowledge::unknown(),
    };
    assert!(pool.validate().is_ok());
    assert!(pool.contains(&pool.token0));
    assert!(pool.contains(&pool.token1));
    assert!(!pool.contains(&TokenId::new(chain(), &address('d')).unwrap()));
    assert!(pool.concentrated());
    assert!(pool.concentrated_at(TimestampMs::new(200).unwrap()));
    assert!(!pool.concentrated_at(TimestampMs::new(301).unwrap()));
    pool.verification = None;
    assert!(pool.concentrated_at(TimestampMs::new(999).unwrap()));
    pool.kind = PoolKind::ConcentratedBins;
    assert!(pool.concentrated());
    pool.kind = PoolKind::Unknown;
    assert!(pool.validate().is_err());
    assert!(!pool.concentrated());
    pool.kind = PoolKind::ConstantProduct;
    pool.protocol = None;
    assert!(pool.validate().is_err());
    pool.kind_verified = false;
    assert!(pool.validate().is_ok());
    pool.verification = Some(proof());
    assert!(pool.validate().is_err());
    pool.verification = None;
    pool.token1 = pool.token0.clone();
    assert!(pool.validate().is_err());
    pool.token1 = TokenId::new(ChainId::Evm { chain_id: 2 }, &address('c')).unwrap();
    assert!(pool.validate().is_err());
}
