//! In-process market provider for service and engine tests.
#![allow(dead_code)]
use async_trait::async_trait;
use domain::*;
use provider_api::*;
use std::sync::atomic::{AtomicUsize, Ordering};

#[path = "market.rs"]
pub mod market;

pub struct Provider {
    pub pool_calls: AtomicUsize,
    pub candle_calls: AtomicUsize,
    pub metrics_calls: AtomicUsize,
    pub pools: Vec<PoolSnapshot>,
    pub panic: bool,
}

impl Default for Provider {
    fn default() -> Self {
        Self {
            pool_calls: AtomicUsize::new(0),
            candle_calls: AtomicUsize::new(0),
            metrics_calls: AtomicUsize::new(0),
            pools: vec![market::pool(10)],
            panic: false,
        }
    }
}

#[async_trait]
impl CandleProvider for Provider {
    async fn candles(
        &self,
        request: &CandleRequest,
        ctx: &RequestContext,
    ) -> Result<CandleSeries, ProviderError> {
        ctx.check()?;
        self.candle_calls.fetch_add(1, Ordering::Relaxed);
        assert!(!self.panic, "simulated provider panic");
        let count = ((request.end_exclusive.get() - request.from_inclusive.get())
            / request.resolution.millis()) as usize;
        let candles: Vec<_> = (0..count)
            .map(|index| {
                let price = 1.5 + (index as f64 * 0.3).sin() * 0.1;
                Candle::new(
                    request
                        .from_inclusive
                        .checked_add(index as i64 * request.resolution.millis())
                        .unwrap(),
                    price,
                    price,
                    price,
                    price,
                    Knowledge::known(NonNegative::new(10.0).unwrap()),
                )
                .unwrap()
            })
            .collect();
        Ok(CandleSeries {
            pool: request.pool.clone(),
            priced_token: request.priced_token.clone(),
            price_unit: request.price_unit.clone(),
            volume_unit: VolumeUnit::Usd,
            resolution: request.resolution,
            end_exclusive: request.end_exclusive,
            candles: candles.into(),
            provenance: SeriesProvenance {
                provider: ProviderId::new("fixture").unwrap(),
                contract_version: "1".into(),
                fetched_at: request.end_exclusive,
                input_hash: ContentHash::of_bytes(b"fixture"),
            },
        })
    }
}

#[async_trait]
impl PoolDiscoveryProvider for Provider {
    fn descriptor(&self) -> plugin_api::PluginDescriptor {
        plugin_api::PluginDescriptor {
            id: PluginId::new("provider.fixture").unwrap(),
            version: "1".into(),
            kind: plugin_api::PluginKind::DataProvider,
            provides: vec![],
        }
    }
    async fn token_metrics(
        &self,
        _: &TokenId,
        ctx: &RequestContext,
    ) -> Result<TokenMetrics, ProviderError> {
        ctx.check()?;
        self.metrics_calls.fetch_add(1, Ordering::Relaxed);
        Ok([(TokenMetric::MarketCapUsd, market::observation(500_000.0))].into())
    }
    async fn pools_for_token(
        &self,
        _: &TokenId,
        ctx: &RequestContext,
    ) -> Result<PoolDiscoveryResult, ProviderError> {
        ctx.check()?;
        self.pool_calls.fetch_add(1, Ordering::Relaxed);
        assert!(!self.panic, "simulated provider panic");
        Ok(PoolDiscoveryResult {
            pools: self.pools.clone(),
            coverage: Coverage {
                complete: true,
                truncated: false,
                warnings: vec![],
            },
        })
    }
}

#[async_trait]
impl PoolMetadataProvider for Provider {
    async fn pool(
        &self,
        pool: &PoolId,
        ctx: &RequestContext,
    ) -> Result<PoolSnapshot, ProviderError> {
        ctx.check()?;
        self.pools
            .iter()
            .find(|candidate| &candidate.pool == pool)
            .cloned()
            .ok_or(ProviderError::NotFound)
    }
}
