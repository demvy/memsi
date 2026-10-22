use domain::*;
use provider_api::*;
use serde_json::Value;
fn schema(field: &str) -> ProviderError {
    ProviderError::InvalidSchema(field.into())
}

fn string<'a>(v: &'a Value, field: &str) -> Result<&'a str, ProviderError> {
    v.get(field)
        .and_then(Value::as_str)
        .ok_or_else(|| schema(field))
}

fn network(v: &Value, expected: i32) -> Result<(), ProviderError> {
    if v.get("networkId").and_then(Value::as_i64) != Some(expected as i64) {
        return Err(ProviderError::InvalidData(
            "network identity mismatch".into(),
        ));
    }

    Ok(())
}

fn number(v: &Value) -> Result<NonNegative, ProviderError> {
    let n = if let Some(s) = v.as_str() {
        s.parse::<f64>().map_err(|_| schema("numeric string"))?
    } else {
        v.as_f64().ok_or_else(|| schema("number"))?
    };

    Ok(NonNegative::new(n)?)
}

pub(crate) fn observation(
    v: &Value,
    field: &str,
    fetched: TimestampMs,
) -> Result<Knowledge<Observation<NonNegative>>, ProviderError> {
    match v.get(field) {
        None | Some(Value::Null) => Ok(Knowledge::unknown()),
        Some(value) => Ok(Knowledge::known(Observation {
            value: number(value)?,
            source: ProviderId::new("codex").unwrap(),
            observed_at: None,
            fetched_at: fetched,
        })),
    }
}

pub(super) fn pool(
    v: &Value,
    chain: &ChainId,
    id: i32,
    fetched: TimestampMs,
) -> Result<PoolSnapshot, ProviderError> {
    let p = v.get("pair").ok_or_else(|| schema("pair"))?;
    network(p, id)?;
    let out = PoolSnapshot {
        pool: PoolId::address(chain.clone(), string(p, "address")?)?,
        token0: TokenId::new(chain.clone(), string(p, "token0")?)?,
        token1: TokenId::new(chain.clone(), string(p, "token1")?)?,
        kind: PoolKind::Unknown,
        protocol: p
            .get("protocol")
            .and_then(Value::as_str)
            .map(|s| ProtocolId::new(s.to_ascii_lowercase()))
            .transpose()?,
        kind_verified: false,
        verification: None,
        liquidity_usd: observation(v, "liquidity", fetched)?,
        volume_24h_usd: observation(v, "volumeUSD24", fetched)?,
        created_at: match v.get("createdAt") {
            None | Some(Value::Null) => Knowledge::unknown(),
            Some(t) => Knowledge::known(TimestampMs::from_seconds(
                t.as_i64().ok_or_else(|| schema("createdAt"))?,
            )?),
        },
    };

    out.validate()?;
    Ok(out)
}

pub(super) fn candidate(
    v: &Value,
    chain: &ChainId,
    id: i32,
    fetched: TimestampMs,
) -> Result<Candidate, ProviderError> {
    let t = v.get("token").ok_or_else(|| schema("token"))?;
    network(t, id)?;
    let symbol = t
        .get("symbol")
        .and_then(Value::as_str)
        .filter(|s| s.len() <= 128)
        .map(str::to_owned);
    Ok(Candidate {
        metrics: Default::default(),
        token: TokenId::new(chain.clone(), string(t, "address")?)?,
        pool_hints: vec![],
        symbol,
        discovered_at: fetched,
        source: ProviderId::new("codex").unwrap(),
        liquidity_usd: observation(v, "liquidity", fetched)?,
    })
}

pub(super) fn bars(
    v: &Value,
    r: &CandleRequest,
    fetched: TimestampMs,
) -> Result<CandleSeries, ProviderError> {
    r.validate()?;
    match v.get("s").and_then(Value::as_str) {
        Some("no_data") => {
            return Err(ProviderError::InsufficientHistory {
                have: 0,
                need: r.min_bars,
            });
        }

        Some("ok") => {}
        _ => return Err(schema("bars status")),
    }

    let arrays = ["t", "o", "h", "l", "c", "buyVolume", "sellVolume"].map(|key| {
        v.get(key)
            .and_then(Value::as_array)
            .ok_or_else(|| schema(key))
    });
    let [t, o, h, l, c, buy, sell] = arrays;
    let (t, o, h, l, c, buy, sell) = (t?, o?, h?, l?, c?, buy?, sell?);
    if t.len() > 1500 || [o, h, l, c, buy, sell].iter().any(|a| a.len() != t.len()) {
        return Err(schema("parallel array lengths"));
    }

    let mut candles = Vec::new();
    for i in 0..t.len() {
        let time =
            TimestampMs::from_seconds(t[i].as_i64().ok_or_else(|| schema("bar timestamp"))?)?;
        if time < r.from_inclusive || time >= r.end_exclusive {
            continue;
        }

        let volume_unknown = buy[i].is_null() || sell[i].is_null();
        let volume = if volume_unknown {
            Knowledge::unknown()
        } else {
            Knowledge::known(NonNegative::new(
                number(&buy[i])?.get() + number(&sell[i])?.get(),
            )?)
        };

        let mut candle = Candle::new(
            time,
            number(&o[i])?.get(),
            number(&h[i])?.get(),
            number(&l[i])?.get(),
            number(&c[i])?.get(),
            volume,
        )?;
        candle.buy_volume = if volume_unknown {
            Knowledge::unknown()
        } else {
            Knowledge::known(number(&buy[i])?)
        };

        candles.push(candle);
    }

    if candles.len() > r.max_bars {
        return Err(schema("requested bar bound"));
    }

    if candles.len() < r.min_bars {
        return Err(ProviderError::InsufficientHistory {
            have: candles.len(),
            need: r.min_bars,
        });
    }

    let input_hash = ContentHash::of(&(r, &candles)).map_err(|_| schema("input hash"))?;
    let out = CandleSeries {
        pool: r.pool.clone(),
        priced_token: r.priced_token.clone(),
        price_unit: r.price_unit.clone(),
        volume_unit: VolumeUnit::Usd,
        resolution: r.resolution,
        end_exclusive: r.end_exclusive,
        candles: candles.into(),
        provenance: SeriesProvenance {
            provider: ProviderId::new("codex").unwrap(),
            contract_version: "codex-graphql-2026-09-26".into(),
            fetched_at: fetched,
            input_hash,
        },
    };

    out.validate(r.min_bars)?;
    Ok(out)
}
