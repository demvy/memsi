//! Exact Codex token-level fields; unsupported windows/labels stay unknown.
use crate::{CodexProvider, parse, provider::now};
use domain::*;
use provider_api::*;
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};
const QUERY: &str = "query TokenMetrics($tokens: [String!], $limit: Int!) { filterTokens(tokens: $tokens, limit: $limit, useAggregatedStats: true) { results { circulatingMarketCap marketCap totalLiquidityUsd createdAt volume5m volume1 volume24 totalFees24 devHeldPercentage insiderHeldPercentage top10HoldersPercent sniperCount sniperHeldPercentage bundlerCount bundlerHeldPercentage token { address networkId createdAt } } } }";
impl CodexProvider {
    pub(crate) async fn read_token_metrics(
        &self,
        token: &TokenId,
        ctx: &RequestContext,
    ) -> Result<TokenMetrics, ProviderError> {
        self.read_token_metrics_selected(token, None, ctx).await
    }

    pub(crate) async fn read_token_metrics_selected(
        &self,
        token: &TokenId,
        metrics: Option<&[TokenMetric]>,
        ctx: &RequestContext,
    ) -> Result<TokenMetrics, ProviderError> {
        Ok(self
            .read_token_metrics_batch(std::slice::from_ref(token), metrics, ctx)
            .await?
            .remove(token)
            .unwrap_or_default())
    }

    pub(crate) async fn read_token_metrics_batch(
        &self,
        tokens: &[TokenId],
        metrics: Option<&[TokenMetric]>,
        ctx: &RequestContext,
    ) -> Result<BTreeMap<TokenId, TokenMetrics>, ProviderError> {
        self.read_token_metrics_batch_at(
            tokens,
            metrics,
            ctx,
            "https://graph.codex.io/graphql",
            || self.resolve_credential(),
        )
        .await
    }

    async fn read_token_metrics_batch_at(
        &self,
        tokens: &[TokenId],
        metrics: Option<&[TokenMetric]>,
        ctx: &RequestContext,
        endpoint: &str,
        credential: impl FnOnce() -> Result<String, ProviderError>,
    ) -> Result<BTreeMap<TokenId, TokenMetrics>, ProviderError> {
        ctx.check()?;
        if tokens.is_empty() || tokens.len() > 100 {
            return Err(ProviderError::InvalidRequest);
        }

        let requested: BTreeSet<_> = tokens.iter().cloned().collect();
        if requested.len() != tokens.len() {
            return Err(ProviderError::InvalidRequest);
        }

        for token in tokens {
            self.check_chain(token.chain())?;
        }

        let query = match metrics {
            None => QUERY.to_owned(),
            Some(metrics) => match selected_query(metrics) {
                Some(query) => query,
                None => {
                    return Ok(tokens
                        .iter()
                        .cloned()
                        .map(|token| (token, TokenMetrics::default()))
                        .collect());
                }
            },
        };

        let symbols: Vec<_> = tokens
            .iter()
            .map(|token| format!("{}:{}", token.address(), self.config.network_id))
            .collect();
        let data = self
            .query_at(
                &query,
                json!({ "tokens" : symbols, "limit" : tokens.len() }),
                ctx,
                endpoint,
                credential,
            )
            .await?;
        parse_batch(
            &data,
            &requested,
            &self.chain,
            self.config.network_id,
            now()?,
        )
    }
}

/// Stable allowlisted projection; derived metrics add only their exact dependencies.
fn selected_query(metrics: &[TokenMetric]) -> Option<String> {
    use TokenMetric::*;
    let mut fields = BTreeSet::new();
    let mut token_created = false;
    for metric in metrics {
        match metric {
            MarketCapUsd => {
                fields.insert("circulatingMarketCap");
            }

            FdvUsd => {
                fields.insert("marketCap");
            }

            TotalLiquidityUsd => {
                fields.insert("totalLiquidityUsd");
            }

            TopPoolAgeSecs => {
                fields.insert("createdAt");
            }

            ProviderSeenAgeSecs => {
                token_created = true;
            }

            Volume5mUsd => {
                fields.insert("volume5m");
            }

            Volume1hUsd => {
                fields.insert("volume1");
            }

            Volume24hUsd => {
                fields.insert("volume24");
            }

            Volume5mToLiquidity => {
                fields.extend(["volume5m", "totalLiquidityUsd"]);
            }

            InsiderSupplyPct => {
                fields.insert("insiderHeldPercentage");
            }

            Volume1hToLiquidity => {
                fields.extend(["volume1", "totalLiquidityUsd"]);
            }

            TotalFees24hUsd => {
                fields.insert("totalFees24");
            }

            DevSupplyPct => {
                fields.insert("devHeldPercentage");
            }

            Top10HoldersSupplyPct => {
                fields.insert("top10HoldersPercent");
            }

            SniperCount => {
                fields.insert("sniperCount");
            }

            SniperSupplyPct => {
                fields.insert("sniperHeldPercentage");
            }

            BundlerCount => {
                fields.insert("bundlerCount");
            }

            BundlerSupplyPct => {
                fields.insert("bundlerHeldPercentage");
            }

            PoolLiquidityUsd
            | TokenAgeSecs
            | TradingAgeSecs
            | Volume1mUsd
            | Volume6hUsd
            | TotalFeesUsd
            | TotalFeesNative
            | Top10PercentHoldersSupplyPct
            | PhishingWalletPct => {}
        }
    }

    if fields.is_empty() && !token_created {
        return None;
    }

    let fields = fields.into_iter().collect::<Vec<_>>().join(" ");
    let created = if token_created { " createdAt" } else { "" };
    Some(format!(
        "query TokenMetrics($tokens: [String!], $limit: Int!) {{ filterTokens(tokens: $tokens, limit: $limit, useAggregatedStats: true) {{ results {{ {fields} token {{ address networkId{created} }} }} }} }}"
    ))
}

/// Responses may be reordered or omit unavailable tokens, but cannot introduce other identities.
fn parse_batch(
    data: &Value,
    requested: &BTreeSet<TokenId>,
    chain: &ChainId,
    network: i32,
    fetched: TimestampMs,
) -> Result<BTreeMap<TokenId, TokenMetrics>, ProviderError> {
    let rows = data
        .pointer("/filterTokens/results")
        .and_then(Value::as_array)
        .ok_or_else(|| ProviderError::InvalidSchema("token metrics results".into()))?;
    if rows.len() > requested.len() {
        return Err(ProviderError::InvalidSchema("token metrics count".into()));
    }

    let mut output = BTreeMap::new();
    for row in rows {
        let identity = &row["token"];
        let token = TokenId::new(
            chain.clone(),
            identity["address"]
                .as_str()
                .ok_or_else(|| ProviderError::InvalidSchema("token metrics address".into()))?,
        )?;
        if identity["networkId"].as_i64() != Some(i64::from(network))
            || !requested.contains(&token)
            || output.contains_key(&token)
        {
            return Err(ProviderError::InvalidData("token metrics identity".into()));
        }

        output.insert(token, parse_metrics(row, fetched)?);
    }

    for token in requested {
        output.entry(token.clone()).or_default();
    }

    Ok(output)
}

pub(crate) fn parse_metrics(
    value: &Value,
    fetched: TimestampMs,
) -> Result<TokenMetrics, ProviderError> {
    use TokenMetric::*;
    let mut output = TokenMetrics::default();
    for (metric, field) in [
        (MarketCapUsd, "circulatingMarketCap"),
        (FdvUsd, "marketCap"),
        (TotalLiquidityUsd, "totalLiquidityUsd"),
        (Volume5mUsd, "volume5m"),
        (Volume1hUsd, "volume1"),
        (Volume24hUsd, "volume24"),
        (TotalFees24hUsd, "totalFees24"),
        (DevSupplyPct, "devHeldPercentage"),
        (InsiderSupplyPct, "insiderHeldPercentage"),
        (Top10HoldersSupplyPct, "top10HoldersPercent"),
        (SniperCount, "sniperCount"),
        (SniperSupplyPct, "sniperHeldPercentage"),
        (BundlerCount, "bundlerCount"),
        (BundlerSupplyPct, "bundlerHeldPercentage"),
    ] {
        if let Some(observation) = parse::observation(value, field, fetched)?.as_ref() {
            let number = observation.value.get();
            if (metric.is_percentage() && number > 100.0)
                || (metric.is_count() && number.fract() != 0.0)
            {
                return Err(ProviderError::InvalidData("token metric units".into()));
            }

            output.insert(metric, observation.clone());
        }
    }

    for (metric, created) in [
        (TopPoolAgeSecs, value.get("createdAt")),
        (ProviderSeenAgeSecs, value.pointer("/token/createdAt")),
    ] {
        if let Some(created) = created.filter(|v| !v.is_null()) {
            let created =
                TimestampMs::from_seconds(created.as_i64().ok_or_else(|| {
                    ProviderError::InvalidSchema("token creation timestamp".into())
                })?)?;
            if created > fetched {
                return Err(ProviderError::InvalidData("future token creation".into()));
            }

            output.insert(
                metric,
                Observation {
                    value: NonNegative::new((fetched.get() - created.get()) as f64 / 1000.0)?,
                    source: ProviderId::new("codex").unwrap(),
                    observed_at: None,
                    fetched_at: fetched,
                },
            );
        }
    }

    for (volume_metric, ratio_metric) in [
        (Volume1hUsd, Volume1hToLiquidity),
        (Volume5mUsd, Volume5mToLiquidity),
    ] {
        if let (Some(volume), Some(liquidity)) =
            (output.get(&volume_metric), output.get(&TotalLiquidityUsd))
            && liquidity.value.get() > 0.0
        {
            output.insert(
                ratio_metric,
                Observation {
                    value: NonNegative::new(volume.value.get() / liquidity.value.get())?,
                    source: ProviderId::new("codex").unwrap(),
                    observed_at: None,
                    fetched_at: fetched,
                },
            );
        }
    }

    Ok(output)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fetched() -> TimestampMs {
        TimestampMs::new(600_000).unwrap()
    }

    #[test]
    fn metric_projection_is_stable_allowlisted_and_includes_exact_derived_dependencies() {
        assert!(selected_query(&[]).is_none());
        assert!(
            selected_query(&[TokenMetric::TotalFeesNative, TokenMetric::TradingAgeSecs]).is_none()
        );
        let query = selected_query(&[
            TokenMetric::Volume5mToLiquidity,
            TokenMetric::ProviderSeenAgeSecs,
        ])
        .unwrap();
        assert!(query.contains("totalLiquidityUsd volume5m"));
        assert!(query.contains("token { address networkId createdAt }"));
        assert_eq!(
            query,
            selected_query(&[
                TokenMetric::ProviderSeenAgeSecs,
                TokenMetric::Volume5mToLiquidity,
                TokenMetric::Volume5mToLiquidity
            ])
            .unwrap()
        );
        assert!(!query.contains("sniperCount"));
    }

    #[test]
    fn metrics_parse_exact_fields_units_ages_and_ratios_without_inventing_unsupported_facts() {
        let metrics = parse_metrics(&json!({"circulatingMarketCap":"1000","marketCap":2000,"totalLiquidityUsd":100,"volume5m":50,"volume1":200,"volume24":1000,"devHeldPercentage":10,"sniperCount":2,"createdAt":100,"token":{"createdAt":200}}), fetched()).unwrap();
        for (metric, value) in [
            (TokenMetric::MarketCapUsd, 1000.0),
            (TokenMetric::FdvUsd, 2000.0),
            (TokenMetric::Volume5mToLiquidity, 0.5),
            (TokenMetric::Volume1hToLiquidity, 2.0),
            (TokenMetric::TopPoolAgeSecs, 500.0),
            (TokenMetric::ProviderSeenAgeSecs, 400.0),
        ] {
            assert_eq!(metrics[&metric].value.get(), value);
        }

        assert!(!metrics.contains_key(&TokenMetric::TradingAgeSecs));
        assert!(!metrics.contains_key(&TokenMetric::TotalFeesNative));
        assert!(parse_metrics(&json!({}), fetched()).unwrap().is_empty());
        assert!(
            !parse_metrics(&json!({"totalLiquidityUsd":0,"volume1":1}), fetched())
                .unwrap()
                .contains_key(&TokenMetric::Volume1hToLiquidity)
        );
        for value in [
            json!({"devHeldPercentage":101}),
            json!({"sniperCount":1.5}),
            json!({"createdAt":601}),
            json!({"createdAt":"bad"}),
            json!({"token":{"createdAt":-1}}),
        ] {
            assert!(parse_metrics(&value, fetched()).is_err());
        }
    }

    #[test]
    fn batch_parser_fills_missing_tokens_and_rejects_foreign_duplicate_or_wrong_network_rows() {
        let chain = ChainId::Evm { chain_id: 1 };
        let token =
            TokenId::new(chain.clone(), "0x1111111111111111111111111111111111111111").unwrap();
        let requested = BTreeSet::from([token.clone()]);
        let empty = parse_batch(
            &json!({"filterTokens":{"results":[]}}),
            &requested,
            &chain,
            1,
            fetched(),
        )
        .unwrap();
        assert!(empty[&token].is_empty());
        let row = json!({"token":{"address":token.address(),"networkId":1}, "marketCap":100});
        let valid = parse_batch(
            &json!({"filterTokens":{"results":[row.clone()]}}),
            &requested,
            &chain,
            1,
            fetched(),
        )
        .unwrap();
        assert_eq!(valid[&token][&TokenMetric::FdvUsd].value.get(), 100.0);
        assert!(parse_batch(&json!({}), &requested, &chain, 1, fetched()).is_err());
        assert!(
            parse_batch(
                &json!({"filterTokens":{"results":[row.clone(),row.clone()]}}),
                &requested,
                &chain,
                1,
                fetched()
            )
            .is_err()
        );
        assert!(
            parse_batch(
                &json!({"filterTokens":{"results":[row]}}),
                &requested,
                &chain,
                2,
                fetched()
            )
            .is_err()
        );
    }
}
