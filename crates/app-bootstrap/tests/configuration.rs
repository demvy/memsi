#![cfg(all(
    feature = "codex",
    feature = "range",
    feature = "volume",
    feature = "high-risk",
    feature = "console"
))]
use std::{collections::BTreeMap, path::Path};

#[test]
fn shipped_monitor_configuration_builds_without_opening_services() {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../memes-finder.toml");
    let config = app_config::AppConfig::load(
        &path,
        &BTreeMap::new(),
        &BTreeMap::from([("MEME__TELEGRAM".into(), "false".into())]),
    )
    .unwrap();
    let app = app_bootstrap::build(&config).unwrap();
    assert_eq!(app.strategies.len(), 1);
    assert_eq!(app.providers.len(), 1);
    assert_eq!(app.sinks.len(), 1);
    assert!(
        !app_bootstrap::catalog()
            .unwrap()
            .descriptors()
            .any(|p| p.id.as_str().contains("pump"))
    );
}

#[test]
fn removed_service_configuration_is_rejected_instead_of_ignored() {
    let preset = include_str!("../../app-config/src/research-preset.toml");
    for field in [
        "usd_price_provider",
        "native_liquidity_source",
        "pool_verifier",
        "token_risk_sources",
    ] {
        let value = if field == "token_risk_sources" {
            "[]"
        } else {
            "'legacy'"
        };

        assert!(
            app_config::AppConfig::parse(&format!("{field}={value}\n{preset}")).is_err(),
            "{field}"
        );
    }
}

#[test]
fn shipped_pump_profile_screens_small_caps_and_volume_ratio() {
    use domain::*;
    use strategy_api::MetricScreen;
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../memes-finder.toml");
    let config = app_config::AppConfig::load(
        &path,
        &BTreeMap::new(),
        &BTreeMap::from([("MEME__TELEGRAM".into(), "false".into())]),
    )
    .unwrap();
    let app = app_bootstrap::build(&config).unwrap();
    let strategy = &app.strategies[&InstanceId::new("pump-high").unwrap()];
    let now = TimestampMs::new(1_260_000).unwrap();
    let mut candidate = Candidate {
        token: TokenId::new(
            ChainId::Evm { chain_id: 1 },
            "0x1111111111111111111111111111111111111111",
        )
        .unwrap(),
        metrics: BTreeMap::new(),
        pool_hints: vec![],
        symbol: None,
        discovered_at: now,
        source: ProviderId::new("test").unwrap(),
        liquidity_usd: Knowledge::unknown(),
    };

    let observation = |value| Observation {
        value: NonNegative::new(value).unwrap(),
        source: ProviderId::new("test").unwrap(),
        observed_at: Some(now),
        fetched_at: now,
    };

    candidate
        .metrics
        .insert(TokenMetric::MarketCapUsd, observation(500_000.0));
    candidate
        .metrics
        .insert(TokenMetric::TotalLiquidityUsd, observation(40_000.0));
    candidate
        .metrics
        .insert(TokenMetric::Volume5mToLiquidity, observation(0.11));
    assert!(matches!(
        strategy.screen_token_metrics(&candidate, now),
        MetricScreen::Continue
    ));
    candidate
        .metrics
        .insert(TokenMetric::Volume5mToLiquidity, observation(0.09));
    assert!(matches!(
        strategy.screen_token_metrics(&candidate, now),
        MetricScreen::Reject(_)
    ));
    candidate
        .metrics
        .insert(TokenMetric::Volume5mToLiquidity, observation(1.2));
    candidate
        .metrics
        .insert(TokenMetric::MarketCapUsd, observation(5_000_000.0));
    assert!(matches!(
        strategy.screen_token_metrics(&candidate, now),
        MetricScreen::Reject(_)
    ));
}
