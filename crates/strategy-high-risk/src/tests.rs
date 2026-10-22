use super::*;
use strategy_api::Strategy;
use test_support::strategy_config as fixtures;

#[test]
fn profile_builder_preserves_instance_mode_and_descriptor() {
    let instance = InstanceId::new("test").unwrap();
    let strategy = build(instance.clone(), StrategyMode::Shadow, fixtures::config()).unwrap();
    assert_eq!(strategy.instance(), &instance);
    assert_eq!(strategy.mode(), StrategyMode::Shadow);
    assert_eq!(strategy.descriptor().id, descriptor().id);
    assert_eq!(descriptor().id.as_str(), "strategy.range-high-risk");

    let mut invalid = fixtures::config();
    invalid.metrics_max_age_secs = 0;
    assert!(build(instance, StrategyMode::Active, invalid).is_err());
}
