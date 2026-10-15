//! Provider-reported token facts. Missing fields are unknown, never inferred from unrelated metrics.
use crate::{DomainError, NonNegative, Observation};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TokenMetric {
    MarketCapUsd,
    FdvUsd,
    TotalLiquidityUsd,
    PoolLiquidityUsd,
    TokenAgeSecs,
    /// Time since the first on-chain token mint, independently sourced.
    TradingAgeSecs,
    ProviderSeenAgeSecs,
    TopPoolAgeSecs,
    #[serde(rename = "volume_1m_usd")]
    Volume1mUsd,
    #[serde(rename = "volume_5m_usd")]
    Volume5mUsd,
    #[serde(rename = "volume_1h_usd")]
    Volume1hUsd,
    #[serde(rename = "volume_6h_usd")]
    Volume6hUsd,
    #[serde(rename = "volume_24h_usd")]
    Volume24hUsd,
    #[serde(rename = "volume_1h_to_liquidity")]
    Volume1hToLiquidity,
    #[serde(rename = "volume_5m_to_liquidity")]
    Volume5mToLiquidity,
    TotalFeesUsd,
    /// Lifetime fees denominated in whole native chain coins (SOL/ETH/etc.), not USD.
    TotalFeesNative,
    #[serde(rename = "total_fees_24h_usd")]
    TotalFees24hUsd,
    DevSupplyPct,
    InsiderSupplyPct,
    Top10HoldersSupplyPct,
    Top10PercentHoldersSupplyPct,
    SniperCount,
    SniperSupplyPct,
    PhishingWalletPct,
    BundlerSupplyPct,
    BundlerCount,
}
impl TokenMetric {
    pub fn is_percentage(self) -> bool {
        matches!(
            self,
            Self::DevSupplyPct
                | Self::InsiderSupplyPct
                | Self::Top10HoldersSupplyPct
                | Self::Top10PercentHoldersSupplyPct
                | Self::SniperSupplyPct
                | Self::PhishingWalletPct
                | Self::BundlerSupplyPct
        )
    }
    pub fn is_count(self) -> bool {
        matches!(self, Self::SniperCount | Self::BundlerCount)
    }
}
pub type TokenMetrics = BTreeMap<TokenMetric, Observation<NonNegative>>;
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MetricBounds {
    pub gt: Option<f64>,
    pub gte: Option<f64>,
    pub lt: Option<f64>,
    pub lte: Option<f64>,
}
impl MetricBounds {
    pub fn validate(&self, metric: TokenMetric) -> Result<(), DomainError> {
        let values = [self.gt, self.gte, self.lt, self.lte];
        if values.iter().all(Option::is_none)
            || values.iter().flatten().any(|v| {
                !v.is_finite()
                    || *v < 0.0
                    || (metric.is_percentage() && *v > 100.0)
                    || (metric.is_count() && v.fract() != 0.0)
            })
            || (self.gt.is_some() && self.gte.is_some())
            || (self.lt.is_some() && self.lte.is_some())
        {
            return Err(DomainError::Market("invalid metric bounds"));
        }
        if let (Some(lower), Some(upper)) = (self.gt.or(self.gte), self.lt.or(self.lte))
            && (lower > upper || (lower == upper && (self.gt.is_some() || self.lt.is_some())))
        {
            return Err(DomainError::Market("empty metric interval"));
        }
        Ok(())
    }
    pub fn accepts(&self, value: f64) -> bool {
        value.is_finite()
            && self.gt.is_none_or(|v| value > v)
            && self.gte.is_none_or(|v| value >= v)
            && self.lt.is_none_or(|v| value < v)
            && self.lte.is_none_or(|v| value <= v)
    }
}
