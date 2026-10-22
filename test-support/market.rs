//! Deterministic market fixtures shared by unit tests; no network or system clock.

use domain::*;

pub fn timestamp(millis: i64) -> TimestampMs {
    TimestampMs::new(millis).unwrap()
}

pub fn chain() -> ChainId {
    ChainId::Evm { chain_id: 1 }
}

pub fn token(index: u64) -> TokenId {
    TokenId::new(chain(), &format!("0x{index:040x}")).unwrap()
}

pub fn pool_id(index: u64) -> PoolId {
    PoolId::address(chain(), &format!("0x{index:040x}")).unwrap()
}

pub fn observation(value: f64) -> Observation<NonNegative> {
    Observation {
        value: NonNegative::new(value).unwrap(),
        source: ProviderId::new("fixture").unwrap(),
        observed_at: Some(timestamp(600_000)),
        fetched_at: timestamp(600_000),
    }
}

pub fn pool(index: u64) -> PoolSnapshot {
    PoolSnapshot {
        pool: pool_id(index),
        token0: token(1),
        token1: token(2),
        kind: PoolKind::ConstantProduct,
        protocol: Some(ProtocolId::new("fixture").unwrap()),
        kind_verified: true,
        verification: None,
        liquidity_usd: Knowledge::known(observation(10_000.0)),
        volume_24h_usd: Knowledge::known(observation(1_000.0)),
        created_at: Knowledge::known(timestamp(0)),
    }
}

pub fn candidate() -> Candidate {
    Candidate {
        metrics: TokenMetrics::default(),
        token: token(1),
        pool_hints: vec![pool_id(10)],
        symbol: Some("TEST".into()),
        discovered_at: timestamp(600_000),
        source: ProviderId::new("fixture").unwrap(),
        liquidity_usd: Knowledge::known(observation(10_000.0)),
    }
}

pub fn series(prices: &[f64], volumes: &[f64]) -> CandleSeries {
    assert_eq!(prices.len(), volumes.len());
    let candles: Vec<_> = prices
        .iter()
        .zip(volumes)
        .enumerate()
        .map(|(index, (&price, &volume))| {
            Candle::new(
                timestamp(index as i64 * 60_000),
                price,
                price,
                price,
                price,
                Knowledge::known(NonNegative::new(volume).unwrap()),
            )
            .unwrap()
        })
        .collect();
    CandleSeries {
        pool: pool_id(10),
        priced_token: token(1),
        price_unit: PriceUnit::UsdPerToken,
        volume_unit: VolumeUnit::Usd,
        resolution: Resolution::new(60).unwrap(),
        end_exclusive: timestamp(prices.len() as i64 * 60_000),
        candles: candles.into(),
        provenance: SeriesProvenance {
            provider: ProviderId::new("fixture").unwrap(),
            contract_version: "1".into(),
            fetched_at: timestamp(prices.len() as i64 * 60_000),
            input_hash: ContentHash::of_bytes(b"fixture"),
        },
    }
}
