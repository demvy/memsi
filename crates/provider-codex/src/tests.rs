use super::*;
use discovery_api::{CandidateSource, DiscoveryQuery};
use domain::*;
use provider_api::*;
use serde_json::json;
use std::time::{Duration, Instant};
#[path = "../../../test-support/market.rs"]
mod fixtures;

fn config() -> CodexConfig {
    serde_json::from_value(json!({"api_key":"env:MEMSI_TEST_ABSENT_CODEX_KEY", "network_id":1}))
        .unwrap()
}

fn provider() -> CodexProvider {
    CodexProvider::new(fixtures::chain(), config()).unwrap()
}

fn context() -> RequestContext {
    RequestContext {
        priority: RequestPriority::default(),
        deadline: Instant::now() + Duration::from_secs(60),
        cancellation: Cancellation::default(),
    }
}

fn request() -> CandleRequest {
    CandleRequest {
        pool: fixtures::pool_id(10),
        priced_token: fixtures::token(1),
        price_unit: PriceUnit::UsdPerToken,
        resolution: Resolution::new(60).unwrap(),
        from_inclusive: fixtures::timestamp(0),
        end_exclusive: fixtures::timestamp(120_000),
        min_bars: 2,
        max_bars: 2,
    }
}

#[test]
fn codex_configuration_and_chain_mapping_are_validated_without_resolving_secrets() {
    let config = config();
    assert_eq!(config.concurrency, 2);
    assert_eq!(config.min_interval_ms, 500);
    assert!(config.validate().is_ok());
    for field in 0..7 {
        let mut invalid = config.clone();
        match field {
            0 => invalid.network_id = 0,
            1 => invalid.concurrency = 0,
            2 => invalid.concurrency = 9,
            3 => invalid.min_interval_ms = 99,
            4 => invalid.min_interval_ms = 60001,
            5 => invalid.fallback_api_keys = vec![config.api_key.clone(); 9],
            _ => {
                invalid.usage = Some(app_config::ApiUsageConfig {
                    monthly_limit: 0,
                    ..Default::default()
                })
            }
        }

        assert_eq!(invalid.validate(), Err(ProviderError::InvalidRequest));
    }

    assert!(CodexProvider::new(ChainId::Evm { chain_id: 2 }, config.clone()).is_err());
    let mut solana_config = config;
    solana_config.network_id = 1399811149;
    assert!(CodexProvider::new(app_config::Network::Solana.chain(), solana_config).is_ok());
    let provider = provider();
    assert_eq!(
        provider.symbol(&fixtures::pool_id(10)).unwrap(),
        format!("{}:1", fixtures::pool_id(10).as_address().unwrap())
    );
    assert!(provider.check_chain(&ChainId::Evm { chain_id: 2 }).is_err());
    let opaque = PoolId::new(
        fixtures::chain(),
        PoolLocator::Opaque {
            protocol: ProtocolId::new("test").unwrap(),
            manager: fixtures::token(1).address().into(),
            key: "key".into(),
        },
    )
    .unwrap();
    assert!(provider.symbol(&opaque).is_err());
    assert!(provider::now().unwrap().get() > 0);
    assert_eq!(provider.transport_observations().unwrap().started, 0);
    assert!(provider.supports_closed_candle_deltas());
    assert_eq!(
        PoolDiscoveryProvider::descriptor(&provider).id,
        descriptor().id
    );
    assert_eq!(CandidateSource::descriptor(&provider).id, descriptor().id);
}

#[tokio::test]
async fn cancelled_requests_cover_provider_ports_without_network_or_credentials() {
    let provider = provider();
    let context = context();
    context.cancellation.cancel();
    assert_eq!(
        provider
            .query("test", json!({}), &context)
            .await
            .unwrap_err(),
        ProviderError::Cancelled
    );
    assert_eq!(
        provider
            .token_metrics(&fixtures::token(1), &context)
            .await
            .unwrap_err(),
        ProviderError::Cancelled
    );
    assert_eq!(
        provider
            .token_metrics_selected(&fixtures::token(1), &[], &context)
            .await
            .unwrap_err(),
        ProviderError::Cancelled
    );
    assert_eq!(
        provider
            .token_metrics_batch(&[fixtures::token(1)], None, &context)
            .await
            .unwrap_err(),
        ProviderError::Cancelled
    );
    assert_eq!(
        provider
            .pools_for_token(&fixtures::token(1), &context)
            .await
            .unwrap_err(),
        ProviderError::Cancelled
    );
    assert_eq!(
        provider
            .pool(&fixtures::pool_id(10), &context)
            .await
            .unwrap_err(),
        ProviderError::Cancelled
    );
    assert_eq!(
        provider.candles(&request(), &context).await.unwrap_err(),
        ProviderError::Cancelled
    );
    let query = DiscoveryQuery::union(fixtures::chain(), &[None], 10).unwrap();
    assert_eq!(
        provider.poll(&query, None, &context).await.unwrap_err(),
        ProviderError::Cancelled
    );
    assert!(provider.usage_status().await.unwrap().is_none());
    assert_eq!(provider.transport_observations().unwrap().in_flight, 0);
}

#[tokio::test]
async fn unsupported_metrics_and_cached_pool_sides_do_not_require_http() {
    let provider = provider();
    let context = context();
    let metrics = provider
        .token_metrics_selected(
            &fixtures::token(1),
            &[TokenMetric::TotalFeesNative],
            &context,
        )
        .await
        .unwrap();
    assert!(metrics.is_empty());
    provider.pool_tokens.remember(&fixtures::pool(10)).unwrap();
    assert_eq!(
        provider.candle_side(&request(), &context).await.unwrap(),
        "token0"
    );
    let mut second = request();
    second.priced_token = fixtures::token(2);
    assert_eq!(
        provider.candle_side(&second, &context).await.unwrap(),
        "token1"
    );
    assert_eq!(provider.transport_observations().unwrap().http_attempts, 0);
    assert_eq!(
        provider
            .query_at(
                "query",
                json!({}),
                &context,
                "http://unused.invalid",
                || Err(ProviderError::Unauthorized)
            )
            .await
            .unwrap_err(),
        ProviderError::Unauthorized
    );
    assert_eq!(provider.transport_observations().unwrap().http_attempts, 0);
}

#[test]
fn response_parsers_validate_identity_and_keep_unverified_pool_kind_unknown() {
    let raw = json!({"pair":{"address":fixtures::pool_id(10).as_address(), "networkId":1,"token0":fixtures::token(1).address(),"token1":fixtures::token(2).address(),"protocol":"UNISWAP"},"liquidity":"1000","volumeUSD24":2000,"createdAt":100});
    let pool = parse::pool(&raw, &fixtures::chain(), 1, fixtures::timestamp(600_000)).unwrap();
    assert_eq!(pool.kind, PoolKind::Unknown);
    assert!(!pool.kind_verified);
    assert_eq!(pool.protocol.unwrap().as_str(), "uniswap");
    assert_eq!(pool.liquidity_usd.as_ref().unwrap().value.get(), 1000.0);
    assert!(parse::pool(&raw, &fixtures::chain(), 2, fixtures::timestamp(600_000)).is_err());
    assert!(
        parse::pool(
            &json!({}),
            &fixtures::chain(),
            1,
            fixtures::timestamp(600_000)
        )
        .is_err()
    );
    let raw =
        json!({"token":{"address":fixtures::token(1).address(),"networkId":1,"symbol":"TEST"}});
    let candidate =
        parse::candidate(&raw, &fixtures::chain(), 1, fixtures::timestamp(600_000)).unwrap();
    assert_eq!(candidate.symbol.as_deref(), Some("TEST"));
    assert!(candidate.liquidity_usd.as_ref().is_none());
    assert!(parse::candidate(&raw, &fixtures::chain(), 2, fixtures::timestamp(600_000)).is_err());
    for value in [json!(-1), json!("NaN"), json!("bad"), json!(true)] {
        assert!(
            parse::observation(&json!({"value":value}), "value", fixtures::timestamp(0)).is_err()
        );
    }

    assert!(
        parse::observation(&json!({"value":null}), "value", fixtures::timestamp(0))
            .unwrap()
            .as_ref()
            .is_none()
    );
}

#[test]
fn bar_parser_combines_volume_validates_parallel_arrays_and_requires_closed_history() {
    let valid = json!({"s":"ok","t":[0,60],"o":[1,2],"h":[2,3],"l":[1,2],"c":[2,3],"buyVolume":[10,20],"sellVolume":[30,40]});
    let series = parse::bars(&valid, &request(), fixtures::timestamp(120_000)).unwrap();
    assert_eq!(series.candles[0].volume.as_ref().unwrap().get(), 40.0);
    assert_eq!(series.candles[0].buy_volume.as_ref().unwrap().get(), 10.0);
    assert!(series.validate(2).is_ok());
    assert!(matches!(
        parse::bars(&json!({"s":"no_data"}), &request(), fixtures::timestamp(0)),
        Err(ProviderError::InsufficientHistory { have: 0, need: 2 })
    ));
    for field in 0..5 {
        let mut invalid = valid.clone();
        match field {
            0 => invalid["s"] = json!("error"),
            1 => invalid["o"] = json!([1]),
            2 => invalid["t"] = json!([0, 61]),
            3 => invalid["c"] = json!([0, 3]),
            _ => invalid["t"] = json!([0, 120]),
        }

        assert!(parse::bars(&invalid, &request(), fixtures::timestamp(120_000)).is_err());
    }

    let mut missing_volume = valid;
    missing_volume["buyVolume"] = json!([null, 20]);
    let series = parse::bars(&missing_volume, &request(), fixtures::timestamp(120_000)).unwrap();
    assert!(series.candles[0].volume.as_ref().is_none());
    assert!(series.candles[0].buy_volume.as_ref().is_none());
}

#[test]
fn pool_identity_cache_rejects_token_changes_and_bounds_entries() {
    let cache = pool_tokens::PoolTokens::default();
    let pool = fixtures::pool(10);
    assert_eq!(cache.side(&pool.pool, &pool.token0).unwrap(), None);
    cache.remember(&pool).unwrap();
    cache.remember(&pool).unwrap();
    assert_eq!(
        cache.side(&pool.pool, &pool.token0).unwrap(),
        Some("token0")
    );
    assert_eq!(
        cache.side(&pool.pool, &pool.token1).unwrap(),
        Some("token1")
    );
    assert!(cache.side(&pool.pool, &fixtures::token(99)).is_err());
    let mut changed = pool;
    changed.token1 = fixtures::token(99);
    assert!(cache.remember(&changed).is_err());
    for index in 100..613 {
        cache.remember(&fixtures::pool(index)).unwrap();
    }
    assert_eq!(
        cache
            .side(&fixtures::pool_id(10), &fixtures::token(1))
            .unwrap(),
        None
    );
}

#[tokio::test]
async fn providers_without_usage_accounting_do_not_claim_notices() {
    let provider = provider();
    let usage = ApiKeyUsage {
        key_id: "key-identity".into(),
        period: "2025-01-01".into(),
        used: 10,
        limit: 10,
        warn_at: 9,
    };
    assert!(!provider.claim_usage_notice(&usage, true).await.unwrap());
    assert!(provider.api_usage().await.unwrap().is_none());
}
