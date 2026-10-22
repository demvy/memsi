use domain::{Candidate, TimestampMs};
use monitor_runtime::MonitorDiagnostics;
use std::{io::IsTerminal, time::Duration};
use time::{OffsetDateTime, format_description::well_known::Rfc3339};
pub(super) struct Console {
    color: bool,
    debug: bool,
    logging: std::collections::BTreeMap<domain::InstanceId, bool>,
}

impl Console {
    pub(super) fn new(debug: bool, config: &app_config::AppConfig, json_logging: bool) -> Self {
        Self {
            logging: config
                .strategies
                .iter()
                .filter(|s| s.enabled)
                .map(|s| (s.id.clone(), s.logging))
                .collect(),
            debug,
            color: !json_logging
                && std::io::stderr().is_terminal()
                && std::env::var_os("NO_COLOR").is_none(),
        }
    }
}

fn safe(text: &str) -> String {
    text.chars()
        .filter(|c| {
            !c.is_control() && !matches!(* c, '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}')
        })
        .take(1024)
        .collect()
}

fn label(candidate: &Candidate) -> String {
    format!(
        "{} CA={}",
        safe(candidate.symbol.as_deref().unwrap_or("?")),
        candidate.token.address()
    )
}

fn timestamp(at: TimestampMs) -> String {
    OffsetDateTime::from_unix_timestamp_nanos(i128::from(at.get()) * 1_000_000)
        .ok()
        .and_then(|t| t.format(&Rfc3339).ok())
        .unwrap_or_else(|| format!("{}ms UTC", at.get()))
}

fn styled(text: &str, color: bool) -> String {
    if color {
        format!("\x1b[31m{text}\x1b[0m")
    } else {
        text.into()
    }
}

fn reason(reason: &strategy_api::DecisionReason) -> String {
    format!(
        "{}: {}={}; expected {}",
        safe(&reason.code),
        safe(&reason.field),
        safe(reason.actual.as_deref().unwrap_or("unknown/stale")),
        safe(&reason.expected)
    )
}

impl MonitorDiagnostics for Console {
    fn discovery(&self, finished_at: TimestampMs, elapsed: Duration, candidates: &[Candidate]) {
        if !self.logging.is_empty() && self.logging.values().all(|enabled| !enabled) {
            return;
        }

        tracing::info!(
            "[discovery {}] received={} elapsed_ms={}",
            timestamp(finished_at),
            candidates.len(),
            elapsed.as_millis()
        );
        for candidate in candidates {
            tracing::info!("  {}", label(candidate));
        }
    }

    fn evaluation(&self, candidate: &Candidate, evaluation: &app_services::EvaluationReport) {
        for item in &evaluation.strategies {
            if self.logging.get(&item.instance) == Some(&false) {
                continue;
            }

            let prefix = format!(
                "[{}] {} profile={}",
                timestamp(evaluation.as_of),
                label(candidate),
                item.instance.as_str()
            );
            if !self.debug {
                let (status, code, negative) = if let Some(reasons) = &item.prefilter_rejection {
                    (
                        "REJECT".to_owned(),
                        reasons
                            .first()
                            .map_or("PREFILTER_REJECT", |r| r.code.as_str()),
                        true,
                    )
                } else if let Some(decision) = &item.decision {
                    (
                        format!("{:?}", decision.verdict),
                        decision
                            .reasons
                            .first()
                            .map_or("NO_SIGNAL", |r| r.code.as_str()),
                        matches!(
                            decision.verdict,
                            strategy_api::Verdict::Reject | strategy_api::Verdict::Unavailable
                        ),
                    )
                } else {
                    ("UNAVAILABLE".into(), "NO_ELIGIBLE_DATA", true)
                };

                tracing::info!(
                    "{}",
                    styled(
                        &format!("{prefix} {status} {}", safe(code)),
                        negative && self.color
                    )
                );
                continue;
            }

            if let Some(reasons) = &item.prefilter_rejection {
                for r in reasons {
                    tracing::info!(
                        "{}",
                        styled(&format!("{prefix} REJECT {}", reason(r)), self.color)
                    );
                }
            } else if let Some(decision) = &item.decision {
                let negative = matches!(
                    decision.verdict,
                    strategy_api::Verdict::Reject | strategy_api::Verdict::Unavailable
                );
                tracing::info!(
                    "{}",
                    styled(
                        &format!("{prefix} {:?}", decision.verdict),
                        negative && self.color
                    )
                );
                for r in &decision.reasons {
                    tracing::info!(
                        "{}",
                        styled(&format!("  {}", reason(r)), negative && self.color)
                    );
                }
            } else if let Some(error) = &item.unavailable {
                tracing::info!(
                    "{}",
                    styled(&format!("{prefix} UNAVAILABLE {}", safe(error)), self.color)
                );
            } else {
                tracing::info!(
                    "{}",
                    styled(
                        &format!("{prefix} UNAVAILABLE no eligible pool/analysis result"),
                        self.color
                    )
                );
            }

            for probe in &item.trace {
                tracing::info!(
                    "{}",
                    styled(
                        &format!(
                            "  pool={} {}",
                            probe.pool.as_address().unwrap_or("non-address pool"),
                            safe(&probe.reason)
                        ),
                        self.color
                    )
                );
            }
        }
    }

    fn failure(&self, candidate: &Candidate, error: &str) {
        if !self.logging.is_empty() && self.logging.values().all(|enabled| !enabled) {
            return;
        }

        tracing::info!(
            "{}",
            styled(
                &format!("{} UNAVAILABLE {}", label(candidate), safe(error)),
                self.color
            )
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn console_helpers_clean_controls_limit_lengths_and_format_timestamps() {
        assert_eq!(safe("A\nB\u{202e}🦀"), "AB🦀");
        assert_eq!(safe(&"x".repeat(1025)).len(), 1024);
        assert_eq!(
            timestamp(TimestampMs::new(0).unwrap()),
            "1970-01-01T00:00:00Z"
        );
        assert_eq!(styled("error", false), "error");
        assert_eq!(styled("error", true), "\x1b[31merror\x1b[0m");
        let reason = strategy_api::DecisionReason {
            code: "MISSING".into(),
            field: "volume".into(),
            actual: None,
            expected: "known".into(),
        };
        assert_eq!(
            super::reason(&reason),
            "MISSING: volume=unknown/stale; expected known"
        );
    }
}

#[cfg(test)]
mod diagnostic_tests {
    use super::*;

    #[test]
    fn disabled_diagnostic_policy_is_retained_and_labels_sanitize_provider_symbols() {
        let mut config = app_config::AppConfig::parse(include_str!(
            "../../../app-config/src/research-preset.toml"
        ))
        .unwrap();
        for strategy in &mut config.strategies {
            strategy.logging = false;
        }
        let console = Console::new(false, &config, false);
        assert!(!console.debug);
        assert!(console.logging.values().all(|enabled| !enabled));
        let candidate = domain::Candidate {
            metrics: Default::default(),
            token: domain::TokenId::new(
                domain::ChainId::Evm { chain_id: 1 },
                "0x1111111111111111111111111111111111111111",
            )
            .unwrap(),
            pool_hints: vec![],
            symbol: Some("A\nB".into()),
            discovered_at: TimestampMs::new(0).unwrap(),
            source: domain::ProviderId::new("test").unwrap(),
            liquidity_usd: domain::Knowledge::unknown(),
        };
        assert!(label(&candidate).starts_with("AB CA="));
        console.discovery(
            TimestampMs::new(0).unwrap(),
            Duration::ZERO,
            std::slice::from_ref(&candidate),
        );
        console.failure(&candidate, "test error");
        console.evaluation(
            &candidate,
            &app_services::EvaluationReport {
                schema_version: 1,
                token: candidate.token.clone(),
                as_of: TimestampMs::new(0).unwrap(),
                coverage: provider_api::Coverage {
                    complete: true,
                    truncated: false,
                    warnings: vec![],
                },
                strategies: vec![],
            },
        );
    }
}
