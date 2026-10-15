//! Pump alerts carry their own evidence and do not require a confirmed price range.
use domain::*;
use signal_api::*;
use state_api::*;
use strategy_api::*;
pub(super) fn build(
    decision: &StrategyDecision,
    symbol: Option<&str>,
    now: TimestampMs,
) -> Result<SignalDraft, StateError> {
    let pool = decision.pool.clone().ok_or(StateError::InvalidRequest)?;
    let reason = decision
        .reasons
        .iter()
        .find(|r| r.code == "PUMP_HIGH")
        .ok_or(StateError::InvalidRequest)?;
    if decision.as_of > now
        || now.get() - decision.as_of.get() > 60000
        || pool.chain() != decision.token.chain()
        || !decision.warnings.iter().any(|w| w == "ResearchOnly")
        || decision.evidence.candle_input_hash.is_none()
        || decision.evidence.volume.is_none()
        || reason.actual.is_none()
    {
        return Err(StateError::InvalidRequest);
    }
    let event_at = reason
        .actual
        .as_deref()
        .and_then(|text| {
            text.split(';')
                .find_map(|part| part.trim().strip_prefix("end="))
        })
        .and_then(|text| text.parse::<i64>().ok())
        .and_then(|ms| TimestampMs::new(ms).ok())
        .filter(|at| *at <= decision.as_of)
        .ok_or(StateError::InvalidRequest)?;
    let expires_at = event_at
        .checked_add(900000)
        .map_err(|_| StateError::InvalidRequest)?;
    if expires_at <= now {
        return Err(StateError::InvalidRequest);
    }
    let kind = SignalKind::ResearchMatch;
    let dedupe_key = ContentHash::of(&(
        1u8,
        &decision.instance,
        &decision.config_digest,
        &decision.token,
        &pool,
        &kind,
        event_at,
    ))
    .map_err(|_| StateError::InvalidRequest)?;
    let id = dedupe_key.clone();
    let mut facts = vec![SignalFact::Text {
        label: "Five-minute price growth".into(),
        value: reason.actual.clone().unwrap(),
    }];
    if let Some(metrics) = &decision.evidence.token_metrics {
        for (metric, observation) in metrics {
            if observation.fetched_at > decision.as_of
                || !observation.is_fresh(decision.as_of, 86400000, false)
            {
                return Err(StateError::InvalidRequest);
            }
            facts.push(SignalFact::Text {
                label: format!("{metric:?}"),
                value: observation.value.get().to_string(),
            });
        }
    }
    crate::token_age::append(&mut facts, decision);
    let signal = Signal {
        id,
        created_at: event_at,
        expires_at,
        strategy: decision.instance.clone(),
        config_digest: decision.config_digest.clone(),
        token: decision.token.clone(),
        pool,
        kind,
        symbol: symbol.map(|s| s.chars().filter(|c| !c.is_control()).take(64).collect()),
        facts,
        reasons: decision.reasons.iter().map(|r| r.code.clone()).collect(),
        warnings: decision.warnings.clone(),
        snapshot_hash: decision.evidence.snapshot_hash.clone(),
    };
    Ok(SignalDraft {
        signal,
        dedupe_key,
        cooldown_secs: 0,
    })
}
