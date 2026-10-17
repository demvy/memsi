use domain::*;
use signal_api::*;
use state_api::*;
use strategy_api::*;
/// Called only with an engine-validated decision. Shadow and non-matches have no side effects.
pub async fn accept(
    repository: &dyn SignalRepository,
    decision: &StrategyDecision,
    symbol: Option<&str>,
    targets: &[DeliveryTarget],
    now: TimestampMs,
) -> Result<Acceptance, StateError> {
    if decision.mode != StrategyMode::Active || decision.verdict != Verdict::Match {
        return Ok(Acceptance::Suppressed);
    }

    let draft = build(decision, symbol, now)?;
    repository
        .accept_signal_and_deliveries(draft, targets, now)
        .await
}

fn build(
    decision: &StrategyDecision,
    symbol: Option<&str>,
    now: TimestampMs,
) -> Result<SignalDraft, StateError> {
    if decision.as_of > now || now.get() - decision.as_of.get() > 60_000 {
        return Err(StateError::InvalidRequest);
    }

    if decision.reasons.iter().any(|r| r.code == "PUMP_HIGH") {
        return crate::pump_signal::build(decision, symbol, now);
    }

    let pool = decision.pool.clone().ok_or(StateError::InvalidRequest)?;
    if pool.chain() != decision.token.chain() {
        return Err(StateError::InvalidRequest);
    }

    let range = decision
        .evidence
        .range
        .as_ref()
        .ok_or(StateError::InvalidRequest)?;
    let bounds = range.range.as_ref().ok_or(StateError::InvalidRequest)?;
    if decision.evidence.volume.is_none() || !bounds.latest_inside_bounds {
        return Err(StateError::InvalidRequest);
    }

    let research_only = decision
        .warnings
        .iter()
        .any(|warning| warning == "ResearchOnly");
    let kind = if decision.evidence.cl_verified && !research_only {
        SignalKind::LpCandidate
    } else {
        SignalKind::ResearchMatch
    };

    let cooldown_secs = match decision.plugin.as_str() {
        "strategy.range-low-risk" => 1800,
        "strategy.range-medium-risk" => 1200,
        _ => 900,
    };

    let dedupe_key = ContentHash::of(&(
        1u8,
        &decision.instance,
        &decision.config_digest,
        &decision.token,
        &pool,
        &kind,
    ))
    .map_err(|_| StateError::InvalidRequest)?;
    let id = ContentHash::of(&(
        1u8,
        &dedupe_key,
        decision.as_of,
        &decision.evidence.snapshot_hash,
    ))
    .map_err(|_| StateError::InvalidRequest)?;
    let mut expires_at = decision
        .as_of
        .checked_add(900_000)
        .map_err(|_| StateError::InvalidRequest)?;
    if decision.evidence.cl_verified
        && !research_only
        && let Some(proof) = &decision.evidence.cl_verification
    {
        proof
            .validate(pool.chain())
            .map_err(|_| StateError::InvalidRequest)?;
        if !proof.is_fresh(decision.as_of, u64::MAX) || !proof.is_fresh(now, u64::MAX) {
            return Err(StateError::InvalidRequest);
        }

        expires_at = expires_at.min(proof.valid_until);
    }

    if expires_at <= now {
        return Err(StateError::InvalidRequest);
    }

    let mut signal = Signal {
        id,
        created_at: now,
        expires_at,
        strategy: decision.instance.clone(),
        config_digest: decision.config_digest.clone(),
        token: decision.token.clone(),
        pool,
        kind,
        symbol: symbol.map(|value| {
            value
                .chars()
                .filter(|character| !character.is_control())
                .take(64)
                .collect()
        }),
        facts: vec![
            SignalFact::Range {
                lower: bounds.lower,
                upper: bounds.upper,
                unit: range.price_unit.clone(),
            },
            SignalFact::Confidence {
                value: range.confidence,
            },
        ],
        reasons: decision
            .reasons
            .iter()
            .map(|reason| reason.code.clone())
            .collect(),
        warnings: decision.warnings.clone(),
        snapshot_hash: decision.evidence.snapshot_hash.clone(),
    };

    if let Some(position) = &bounds.position {
        signal.facts.extend([
            SignalFact::Text {
                label: "Current price".into(),
                value: format!(
                    "{:.8e} {}",
                    position.current_price.get(),
                    if range.price_unit == PriceUnit::UsdPerToken {
                        "USD/token"
                    } else {
                        "quote/token"
                    }
                ),
            },
            SignalFact::Text {
                label: "Range position".into(),
                value: format!("{:.2}% ({:?})", position.raw_pct, position.zone),
            },
            SignalFact::Text {
                label: "Range width".into(),
                value: format!("{:.2}%", bounds.width_pct.get()),
            },
        ]);
    }

    if let Some(market) = &decision.evidence.market {
        if market.validate().is_err()
            || market.pool != signal.pool
            || !market.contains(&signal.token)
        {
            return Err(StateError::InvalidRequest);
        }

        signal.facts.push(SignalFact::Text {
            label: "DEX".into(),
            value: market
                .protocol
                .as_ref()
                .map_or_else(|| "unavailable".into(), |p| p.as_str().into()),
        });
        for (label, value, window_secs) in [
            ("Liquidity", &market.liquidity_usd, None),
            ("Volume", &market.volume_24h_usd, Some(86400)),
        ] {
            if let Some(observation) = value
                .as_ref()
                .filter(|o| o.is_fresh(decision.as_of, 60_000, false))
            {
                signal.facts.push(SignalFact::Money {
                    label: label.into(),
                    usd: observation.value,
                    window_secs,
                });
            } else {
                signal.facts.push(SignalFact::Text {
                    label: label.into(),
                    value: "unavailable".into(),
                });
            }
        }

        let age = market
            .created_at
            .as_ref()
            .filter(|created| **created <= decision.as_of)
            .map(|created| (decision.as_of.get() - created.get()) / 1000);
        signal.facts.push(SignalFact::Text {
            label: "Pool age (at analysis)".into(),
            value: age.map_or_else(
                || "unavailable".into(),
                |secs| format!("{}h {}m", secs / 3600, secs % 3600 / 60),
            ),
        });
    }

    if decision.evidence.market.is_none() {
        signal.facts.push(SignalFact::Text {
            label: "Market facts".into(),
            value: "not captured in this decision".into(),
        });
    }

    let volume = decision.evidence.volume.as_ref().unwrap();
    let (ratio, state) = volume
        .robust
        .as_ref()
        .map_or((volume.relative_volume, volume.state), |v| {
            (v.relative_volume, v.state)
        });
    signal.facts.push(SignalFact::Text {
        label: "Volume activity".into(),
        value: format!(
            "{state:?}; {} ratio {}",
            if volume.robust.is_some() {
                "median"
            } else {
                "mean"
            },
            ratio.map_or_else(|| "unavailable".into(), |r| format!("{:.2}x", r.get()))
        ),
    });
    if let Some(robust) = &volume.robust {
        signal.facts.push(SignalFact::Text {
            label: "Log-volume slope/bar".into(),
            value: format!(
                "short {:.4}; long {:.4}",
                robust.short_slope, robust.long_slope
            ),
        });
    }

    if let Some(trend) = &decision.evidence.trend {
        signal.facts.push(SignalFact::Text {
            label: "Price trend".into(),
            value: format!(
                "{:?}; slope {:.4}/bar",
                trend.direction, trend.slope_log_per_bar
            ),
        });
    }

    Ok(SignalDraft {
        signal,
        dedupe_key,
        cooldown_secs,
    })
}
