use super::*;
use std::collections::BTreeSet;
use test_support::market as fixtures;
use test_support::strategy_config;

fn request() -> AnalysisJobRequest {
    serde_json::from_value(serde_json::json!({"token": fixtures::token(1)})).unwrap()
}

#[test]
fn job_request_defaults_and_validated_conversion_preserve_contract() {
    let mut request = request();
    assert_eq!(request.resolution.seconds(), 300);
    assert!(request.pool.is_none());
    assert!(!request.notify);
    assert!(!request.require_cl);
    let enabled = BTreeSet::from([InstanceId::new("range").unwrap()]);
    request.analyzer_instances = enabled.iter().cloned().collect();
    request.require_cl = true;
    let converted = request
        .analyze_request(&fixtures::chain(), &enabled, 5000)
        .unwrap();
    assert_eq!(converted.token, request.token);
    assert_eq!(converted.analyzer_instances, request.analyzer_instances);
    assert_eq!(converted.close_grace_ms, 5000);
    assert_eq!(
        converted.policy.purpose,
        data_hub::selection::Purpose::ConcentratedLp
    );
    assert!(
        serde_json::from_value::<AnalysisJobRequest>(
            serde_json::json!({"token": fixtures::token(1), "unknown":true})
        )
        .is_err()
    );
}

#[test]
fn job_request_rejects_wrong_chain_unauthorized_notifications_and_invalid_analyzers() {
    let enabled = BTreeSet::from([InstanceId::new("range").unwrap()]);
    for field in 0..7 {
        let mut invalid = request();
        let mut chain = fixtures::chain();
        let mut enabled = enabled.clone();
        let mut grace = 5000;
        match field {
            0 => chain = ChainId::Evm { chain_id: 2 },
            1 => invalid.notify = true,
            2 => grace = 60001,
            3 => enabled.clear(),
            4 => invalid.analyzer_instances = vec![InstanceId::new("unknown").unwrap()],
            5 => invalid.analyzer_instances = vec![InstanceId::new("range").unwrap(); 2],
            _ => invalid.analyzer_instances = vec![InstanceId::new("range").unwrap(); 65],
        }

        assert!(invalid.analyze_request(&chain, &enabled, grace).is_err());
    }
}

#[test]
fn status_envelope_builders_preserve_observation_state_and_version() {
    let digest = ContentHash::of_bytes(b"config");
    let status: StatusEnvelope<()> = StatusEnvelope::new(
        digest.clone(),
        fixtures::chain(),
        "observe".into(),
        vec![],
        (),
    );
    assert_eq!(status.schema_version, 1);
    assert_eq!(status.provider_health, "not_observed");
    let encoded = serde_json::to_value(status).unwrap();
    assert_eq!(encoded["config_digest"], digest.as_str());
}

#[test]
fn catalogs_and_strategy_info_expose_only_expected_identities() {
    let strategy = strategy_range::RangeStrategy::new(
        InstanceId::new("test").unwrap(),
        strategy_api::StrategyMode::Shadow,
        strategy_range::Profile::High,
        strategy_config::config(),
    )
    .unwrap();
    let info = StrategyInfo::from_strategy(&strategy);
    assert_eq!(info.instance.as_str(), "test");
    assert_eq!(info.plugin.as_str(), "strategy.range-high-risk");
    assert_eq!(info.mode, strategy_api::StrategyMode::Shadow);
    let catalog = StrategyCatalogEnvelope::new(vec![info]);
    assert_eq!(catalog.schema_version, 1);
    assert_eq!(catalog.strategies.len(), 1);
    let catalog = PluginCatalogEnvelope::new(vec![strategy_range::descriptor(
        strategy_range::Profile::High,
    )]);
    assert_eq!(catalog.schema_version, 1);
    assert_eq!(catalog.plugins.len(), 1);
}
