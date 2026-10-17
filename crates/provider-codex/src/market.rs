use crate::{
    CodexProvider, parse,
    provider::{descriptor, now},
};

use async_trait::async_trait;
use discovery_api::*;
use domain::*;
use provider_api::*;
use serde_json::{Value, json};
impl CodexProvider {
    pub(crate) async fn candle_side(
        &self,
        request: &CandleRequest,
        ctx: &RequestContext,
    ) -> Result<&'static str, ProviderError> {
        ctx.check()?;
        match self
            .pool_tokens
            .side(&request.pool, &request.priced_token)?
        {
            Some(side) => Ok(side),
            None => {
                let pool = self.pool(&request.pool, ctx).await?;
                self.pool_tokens.remember(&pool)?;
                self.pool_tokens
                    .side(&request.pool, &request.priced_token)?
                    .ok_or(ProviderError::InvalidRequest)
            }
        }
    }

    async fn pairs(
        &self,
        variables: Value,
        ctx: &RequestContext,
    ) -> Result<PoolDiscoveryResult, ProviderError> {
        let data = self.query(PAIRS, variables, ctx).await?;
        let rows = data
            .pointer("/filterPairs/results")
            .and_then(Value::as_array)
            .ok_or_else(|| ProviderError::InvalidSchema("pair results".into()))?;
        if rows.len() > 100 {
            return Err(ProviderError::InvalidSchema("pair count".into()));
        }

        let fetched = now()?;
        let pools = rows
            .iter()
            .map(|v| parse::pool(v, &self.chain, self.config.network_id, fetched))
            .collect::<Result<Vec<_>, _>>()?;
        for pool in &pools {
            self.pool_tokens.remember(pool)?;
        }

        Ok(PoolDiscoveryResult {
            pools,
            coverage: Coverage {
                complete: false,
                truncated: rows.len() == 100,
                warnings: vec![
                    "Provider-ranked first page; pool enumeration is not exhaustive".into(),
                    "Liquidity is provider-reported backing, not verified pool TVL".into(),
                ],
            },
        })
    }
}

const PAIRS: &str = "query Pairs($filters: PairFilters, $pairs: [String]) { filterPairs(filters: $filters, pairs: $pairs, limit: 100, rankings: [{attribute: liquidity, direction: DESC}]) { results { liquidity volumeUSD24 createdAt pair { address networkId protocol token0 token1 } } } }";
const TOKENS: &str = "query Tokens($filters: TokenFilters, $limit: Int, $offset: Int) { filterTokens(filters: $filters, limit: $limit, offset: $offset, useAggregatedStats: true, rankings: [{attribute: volume1, direction: DESC}]) { results { liquidity token { address networkId symbol } } } }";
const BARS: &str = r#"query Bars($symbol: String!, $from: Int!, $to: Int!, $resolution: String!, $quoteToken: QuoteToken) { getBars(symbol: $symbol, symbolType: POOL, from: $from, to: $to, resolution: $resolution, currencyCode: "USD", quoteToken: $quoteToken, removeEmptyBars: false, removeLeadingNullValues: true) { s t o h l c buyVolume sellVolume } }"#;
#[async_trait]
impl PoolDiscoveryProvider for CodexProvider {
    async fn token_metrics(
        &self,
        token: &TokenId,
        ctx: &RequestContext,
    ) -> Result<TokenMetrics, ProviderError> {
        self.read_token_metrics(token, ctx).await
    }

    async fn token_metrics_selected(
        &self,
        token: &TokenId,
        metrics: &[TokenMetric],
        ctx: &RequestContext,
    ) -> Result<TokenMetrics, ProviderError> {
        self.read_token_metrics_selected(token, Some(metrics), ctx)
            .await
    }

    async fn token_metrics_batch(
        &self,
        tokens: &[TokenId],
        metrics: Option<&[TokenMetric]>,
        ctx: &RequestContext,
    ) -> Result<std::collections::BTreeMap<TokenId, TokenMetrics>, ProviderError> {
        self.read_token_metrics_batch(tokens, metrics, ctx).await
    }

    fn descriptor(&self) -> plugin_api::PluginDescriptor {
        descriptor()
    }

    async fn pools_for_token(
        &self,
        token: &TokenId,
        ctx: &RequestContext,
    ) -> Result<PoolDiscoveryResult, ProviderError> {
        self.check_chain(token.chain())?;
        let variables = json!({
            "filters": {
                "network": [self.config.network_id],
                "tokenAddress": [token.address()],
            }
        });
        let result = self.pairs(variables, ctx).await?;
        if result.pools.iter().any(|p| !p.contains(token)) {
            return Err(ProviderError::InvalidData(
                "pair does not contain requested token".into(),
            ));
        }

        Ok(result)
    }
}

#[async_trait]
impl PoolMetadataProvider for CodexProvider {
    async fn pool(
        &self,
        pool: &PoolId,
        ctx: &RequestContext,
    ) -> Result<PoolSnapshot, ProviderError> {
        let symbol = self.symbol(pool)?;
        self.pairs(json!({ "pairs" : [symbol] }), ctx)
            .await?
            .pools
            .into_iter()
            .find(|p| &p.pool == pool)
            .ok_or(ProviderError::NotFound)
    }
}

#[async_trait]
impl CandleProvider for CodexProvider {
    fn supports_closed_candle_deltas(&self) -> bool {
        true
    }

    async fn candles(
        &self,
        request: &CandleRequest,
        ctx: &RequestContext,
    ) -> Result<CandleSeries, ProviderError> {
        request.validate()?;
        let symbol = self.symbol(&request.pool)?;
        if request.price_unit != PriceUnit::UsdPerToken {
            return Err(ProviderError::Unsupported("price units".into()));
        }

        if request.max_bars > 1500
            || (request.end_exclusive.get() - request.from_inclusive.get())
                / request.resolution.millis()
                > 1500
        {
            return Err(ProviderError::Unsupported("maximum 1500 bars".into()));
        }

        let resolution = match request.resolution.seconds() {
            1 => "1S",
            5 => "5S",
            15 => "15S",
            30 => "30S",
            60 => "1",
            300 => "5",
            900 => "15",
            1800 => "30",
            3600 => "60",
            14400 => "240",
            43200 => "720",
            86400 => "1D",
            604800 => "7D",
            _ => return Err(ProviderError::Unsupported("resolution".into())),
        };

        ctx.check()?;
        let side = self.candle_side(request, ctx).await?;
        let variables = json!({
            "symbol": symbol,
            "from": request.from_inclusive.get() / 1000,
            "to": request.end_exclusive.get() / 1000 - 1,
            "resolution": resolution,
            "quoteToken": side,
        });
        let data = self.query(BARS, variables, ctx).await?;
        parse::bars(
            data.get("getBars")
                .ok_or_else(|| ProviderError::InvalidSchema("bars".into()))?,
            request,
            now()?,
        )
    }
}

#[async_trait]
impl CandidateSource for CodexProvider {
    fn descriptor(&self) -> plugin_api::PluginDescriptor {
        descriptor()
    }

    async fn poll(
        &self,
        query: &DiscoveryQuery,
        cursor: Option<&DiscoveryCursor>,
        ctx: &RequestContext,
    ) -> Result<CandidateBatch, ProviderError> {
        self.check_chain(&query.chain)?;
        if query.max_items == 0 || query.max_items > 1000 {
            return Err(ProviderError::InvalidRequest);
        }

        let digest = query.digest();
        let offset = match cursor {
            None => 0,
            Some(c) if c.query_digest == digest => c
                .value
                .parse::<usize>()
                .map_err(|_| ProviderError::InvalidRequest)?,
            _ => return Err(ProviderError::InvalidRequest),
        };

        if offset > 10000 {
            return Err(ProviderError::InvalidRequest);
        }

        let limit = query.max_items.min(100);
        let filters = discovery_filters(query, self.config.network_id);
        let data = self
            .query(
                TOKENS,
                json!({ "filters" : filters, "limit" : limit, "offset" : offset }),
                ctx,
            )
            .await?;
        let rows = data
            .pointer("/filterTokens/results")
            .and_then(Value::as_array)
            .ok_or_else(|| ProviderError::InvalidSchema("token results".into()))?;
        if rows.len() > limit {
            return Err(ProviderError::InvalidSchema("token count".into()));
        }

        let fetched_at = now()?;
        let items = rows
            .iter()
            .map(|v| parse::candidate(v, &self.chain, self.config.network_id, fetched_at))
            .collect::<Result<_, _>>()?;
        let next_cursor = if rows.len() == limit && offset + limit <= 10000 {
            Some(DiscoveryCursor {
                query_digest: digest,
                value: (offset + limit).to_string(),
            })
        } else {
            None
        };

        Ok(CandidateBatch {
            items,
            next_cursor,
            fetched_at,
            coverage: Coverage {
                complete: false,
                truncated: rows.len() == limit,
                warnings: vec![
                    "Ranked offset pages are not a snapshot; deduplicate by token identity".into(),
                ],
            },
        })
    }
}

fn discovery_filters(query: &DiscoveryQuery, network_id: i32) -> Value {
    let mut filters = json!({
        "network": [network_id],
        "trendingIgnored": false,
        "potentialScam": false
    });
    for (name, bounds) in [
        ("liquidity", &query.liquidity_filter),
        ("marketCap", &query.market_cap_filter),
    ] {
        if let Some(bounds) = bounds {
            let mut value = serde_json::Map::new();
            for (key, bound) in [
                ("gt", bounds.gt),
                ("gte", bounds.gte),
                ("lt", bounds.lt),
                ("lte", bounds.lte),
            ] {
                if let Some(number) = bound {
                    value.insert(key.into(), json!(number));
                }
            }

            if !value.is_empty() {
                filters[name] = Value::Object(value);
            }
        }
    }

    if query.liquidity_filter.is_none()
        && let Some(min) = query.min_liquidity_usd
    {
        filters["liquidity"] = json!({ "gte" : min.get() });
    }

    filters
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn discovery_filters_use_explicit_bounds_before_legacy_minimum_and_keep_scam_filter() {
        let mut query = DiscoveryQuery::union(
            ChainId::Evm { chain_id: 1 },
            &[Some(NonNegative::new(100.0).unwrap())],
            100,
        )
        .unwrap();
        let filters = discovery_filters(&query, 1);
        assert_eq!(filters["liquidity"]["gte"], 100.0);
        assert_eq!(filters["potentialScam"], false);
        assert_eq!(filters["network"], json!([1]));
        query.liquidity_filter = Some(MetricBounds {
            gt: Some(200.0),
            lte: Some(1000.0),
            ..Default::default()
        });
        query.market_cap_filter = Some(MetricBounds {
            gte: Some(500.0),
            lt: Some(3000.0),
            ..Default::default()
        });
        let filters = discovery_filters(&query, 8453);
        assert_eq!(filters["liquidity"], json!({"gt":200.0,"lte":1000.0}));
        assert_eq!(filters["marketCap"], json!({"gte":500.0,"lt":3000.0}));
    }
}
