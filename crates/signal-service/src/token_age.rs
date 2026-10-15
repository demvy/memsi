//! Provider-reported token creation time, distinct from pool/signal creation.
use domain::TokenMetric;
use signal_api::SignalFact;
use strategy_api::StrategyDecision;
pub(super) fn append(facts: &mut Vec<SignalFact>, decision: &StrategyDecision) {
    let Some(age) = decision
        .evidence
        .token_metrics
        .as_ref()
        .and_then(|m| m.get(&TokenMetric::ProviderSeenAgeSecs))
    else {
        return;
    };
    if age.fetched_at > decision.as_of {
        return;
    }
    let seconds = age.value.get() + (decision.as_of.get() - age.fetched_at.get()) as f64 / 1000.;
    if seconds.is_finite() && seconds >= 0. {
        facts.push(SignalFact::Text {
            label: "Token age seconds (provider)".into(),
            value: seconds.floor().to_string(),
        });
    }
}
