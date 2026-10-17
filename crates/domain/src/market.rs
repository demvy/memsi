use crate::*;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum PriceUnit {
    UsdPerToken,
    QuotePerToken { quote: TokenId },
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum VolumeUnit {
    Usd,
    Token { token: TokenId },
    Unknown,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Observation<T> {
    pub value: T,
    pub source: ProviderId,
    pub observed_at: Option<TimestampMs>,
    pub fetched_at: TimestampMs,
}

impl<T> Observation<T> {
    pub fn is_fresh(&self, now: TimestampMs, max_age_ms: u64, require_source_time: bool) -> bool {
        let t = match self.observed_at {
            Some(t) => t,
            None if !require_source_time => self.fetched_at,
            None => return false,
        };

        let delta = now.get() - t.get();
        delta >= 0 && delta as u64 <= max_age_ms
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PoolKind {
    Unknown,
    ConstantProduct,
    ConcentratedTicks,
    ConcentratedBins,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PoolSnapshot {
    pub pool: PoolId,
    pub token0: TokenId,
    pub token1: TokenId,
    pub kind: PoolKind,
    pub protocol: Option<ProtocolId>,
    pub kind_verified: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub verification: Option<PoolVerificationEvidence>,
    pub liquidity_usd: Knowledge<Observation<NonNegative>>,
    pub volume_24h_usd: Knowledge<Observation<NonNegative>>,
    pub created_at: Knowledge<TimestampMs>,
}

impl PoolSnapshot {
    pub fn validate(&self) -> Result<(), DomainError> {
        if self.pool.chain() != self.token0.chain()
            || self.pool.chain() != self.token1.chain()
            || self.token0 == self.token1
        {
            return Err(DomainError::Market("pool token identity mismatch"));
        }

        if self.kind_verified && (self.kind == PoolKind::Unknown || self.protocol.is_none()) {
            return Err(DomainError::Market("verified pool needs protocol and kind"));
        }

        if let Some(proof) = &self.verification {
            proof.validate(self.pool.chain())?;
            if !self.kind_verified {
                return Err(DomainError::Market("proof requires verified kind"));
            }
        }

        Ok(())
    }

    pub fn contains(&self, token: &TokenId) -> bool {
        &self.token0 == token || &self.token1 == token
    }

    pub fn concentrated_at(&self, as_of: TimestampMs) -> bool {
        self.concentrated()
            && self
                .verification
                .as_ref()
                .is_none_or(|proof| proof.is_fresh(as_of, u64::MAX))
    }

    pub fn concentrated(&self) -> bool {
        self.kind_verified
            && matches!(
                self.kind,
                PoolKind::ConcentratedTicks | PoolKind::ConcentratedBins
            )
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Candidate {
    #[serde(default, skip_serializing_if = "std::collections::BTreeMap::is_empty")]
    pub metrics: TokenMetrics,
    pub token: TokenId,
    pub pool_hints: Vec<PoolId>,
    pub symbol: Option<String>,
    pub discovered_at: TimestampMs,
    pub source: ProviderId,
    pub liquidity_usd: Knowledge<Observation<NonNegative>>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(try_from = "RawCandle")]
pub struct Candle {
    pub open_time: TimestampMs,
    pub open: PositivePrice,
    pub high: PositivePrice,
    pub low: PositivePrice,
    pub close: PositivePrice,
    pub volume: Knowledge<NonNegative>,
    #[serde(
        default = "Knowledge::unknown",
        skip_serializing_if = "missing_buy_volume"
    )]
    pub buy_volume: Knowledge<NonNegative>,
}

fn missing_buy_volume(value: &Knowledge<NonNegative>) -> bool {
    matches!(
        value,
        Knowledge::Unknown {
            reason: MissingReason::NotProvided
        }
    )
}

#[derive(Deserialize)]
struct RawCandle {
    open_time: TimestampMs,
    open: PositivePrice,
    high: PositivePrice,
    low: PositivePrice,
    close: PositivePrice,
    volume: Knowledge<NonNegative>,
    #[serde(default = "Knowledge::unknown")]
    buy_volume: Knowledge<NonNegative>,
}

impl TryFrom<RawCandle> for Candle {
    type Error = DomainError;
    fn try_from(v: RawCandle) -> Result<Self, Self::Error> {
        let mut candle = Self::new(
            v.open_time,
            v.open.get(),
            v.high.get(),
            v.low.get(),
            v.close.get(),
            v.volume,
        )?;
        candle.buy_volume = v.buy_volume;
        Ok(candle)
    }
}

impl Candle {
    pub fn new(
        t: TimestampMs,
        o: f64,
        h: f64,
        l: f64,
        c: f64,
        v: Knowledge<NonNegative>,
    ) -> Result<Self, DomainError> {
        let out = Self {
            open_time: t,
            open: PositivePrice::new(o)?,
            high: PositivePrice::new(h)?,
            low: PositivePrice::new(l)?,
            close: PositivePrice::new(c)?,
            volume: v,
            buy_volume: Knowledge::unknown(),
        };

        if l > o || l > c || h < o || h < c {
            return Err(DomainError::Market("OHLC ordering"));
        }

        Ok(out)
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SeriesProvenance {
    pub provider: ProviderId,
    pub contract_version: String,
    pub fetched_at: TimestampMs,
    pub input_hash: ContentHash,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CandleSeries {
    pub pool: PoolId,
    pub priced_token: TokenId,
    pub price_unit: PriceUnit,
    pub volume_unit: VolumeUnit,
    pub resolution: Resolution,
    pub end_exclusive: TimestampMs,
    pub candles: Arc<[Candle]>,
    pub provenance: SeriesProvenance,
}

impl CandleSeries {
    pub fn validate(&self, min_bars: usize) -> Result<(), DomainError> {
        if min_bars == 0 || self.candles.len() < min_bars {
            return Err(DomainError::Market("insufficient history"));
        }

        if self.pool.chain() != self.priced_token.chain() {
            return Err(DomainError::Market("chain mismatch"));
        }

        if self.end_exclusive.get() % self.resolution.millis() != 0 {
            return Err(DomainError::Market("unaligned cutoff"));
        }

        for (i, c) in self.candles.iter().enumerate() {
            Candle::new(
                c.open_time,
                c.open.get(),
                c.high.get(),
                c.low.get(),
                c.close.get(),
                c.volume.clone(),
            )?;
            if c.open_time.get() % self.resolution.millis() != 0 {
                return Err(DomainError::Market("unaligned bar"));
            }

            let next = c.open_time.checked_add(self.resolution.millis())?;
            if i + 1 < self.candles.len() {
                if next != self.candles[i + 1].open_time {
                    return Err(DomainError::Market("noncontiguous history"));
                }
            } else if next != self.end_exclusive {
                return Err(DomainError::Market("stale or partial last bar"));
            }
        }

        Ok(())
    }
}

/// Evidence for pool structure, separate from market metrics and token safety.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PoolVerificationEvidence {
    pub source: ProviderId,
    pub method_version: String,
    pub state: ChainState,
    pub observed_at: TimestampMs,
    pub available_at: TimestampMs,
    pub input_hash: ContentHash,
    pub valid_until: TimestampMs,
}

impl PoolVerificationEvidence {
    pub fn validate(&self, chain: &ChainId) -> Result<(), DomainError> {
        let matching = match (chain, &self.state) {
            (ChainId::Solana { .. }, ChainState::Solana { .. }) => true,
            (ChainId::Evm { .. }, ChainState::Evm { block_hash, .. }) => {
                block_hash.len() == 66
                    && block_hash.starts_with("0x")
                    && block_hash[2..].bytes().all(|b| b.is_ascii_hexdigit())
            }

            _ => false,
        };

        if !matching
            || self.available_at > self.valid_until
            || self.observed_at > self.available_at
            || self.method_version.is_empty()
            || self.method_version.len() > 128
            || self.method_version.chars().any(char::is_control)
        {
            return Err(DomainError::Market("invalid pool verification evidence"));
        }

        Ok(())
    }

    pub fn is_fresh(&self, as_of: TimestampMs, max_age_ms: u64) -> bool {
        self.available_at <= as_of
            && as_of <= self.valid_until
            && self.observed_at <= self.available_at
            && (as_of.get() - self.observed_at.get()) as u64 <= max_age_ms
    }
}
