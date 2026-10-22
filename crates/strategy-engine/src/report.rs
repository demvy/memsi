use domain::{InstanceId, PoolId};
use serde::Serialize;
use strategy_api::{DecisionReason, StrategyDecision};
#[derive(Debug, Serialize)]
pub struct ProbeTrace {
    pub pool: PoolId,
    pub reason: String,
}

#[derive(Debug, Serialize)]
pub struct StrategyReport {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub prefilter_rejection: Option<Vec<DecisionReason>>,
    pub instance: InstanceId,
    pub decision: Option<StrategyDecision>,
    pub unavailable: Option<String>,
    pub trace: Vec<ProbeTrace>,
}

impl StrategyReport {
    pub(crate) fn unavailable(instance: &InstanceId, reason: impl ToString) -> Self {
        Self {
            prefilter_rejection: None,
            instance: instance.clone(),
            decision: None,
            unavailable: Some(reason.to_string()),
            trace: vec![],
        }
    }

    pub(crate) fn record(&mut self, pool: &PoolId, reason: impl ToString) {
        self.trace.push(ProbeTrace {
            pool: pool.clone(),
            reason: reason.to_string(),
        });
    }
}

/// Token-only screening has no pool or candle evidence and cannot emit a signal.
pub fn screen_token_metrics(
    strategy: &dyn strategy_api::Strategy,
    candidate: &domain::Candidate,
    as_of: domain::TimestampMs,
) -> Option<StrategyReport> {
    use strategy_api::MetricScreen;
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        strategy.screen_token_metrics(candidate, as_of)
    })) {
        Ok(MetricScreen::Continue) => None,
        Ok(MetricScreen::Reject(reasons)) => Some(StrategyReport {
            instance: strategy.instance().clone(),
            prefilter_rejection: Some(reasons),
            decision: None,
            unavailable: None,
            trace: vec![],
        }),
        Ok(MetricScreen::Unavailable(reasons)) => Some(StrategyReport::unavailable(
            strategy.instance(),
            format!(
                "token metrics unavailable: {}",
                reasons
                    .iter()
                    .map(|r| format!("{} {} expected {}", r.code, r.field, r.expected))
                    .collect::<Vec<_>>()
                    .join("; ")
            ),
        )),
        Err(_) => Some(StrategyReport::unavailable(
            strategy.instance(),
            "token metrics screening panic",
        )),
    }
}
