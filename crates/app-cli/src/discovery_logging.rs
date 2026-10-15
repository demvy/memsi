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
    pub(super) fn new(debug: bool, config: &app_config::AppConfig) -> Self {
        Self {
            logging: config
                .strategies
                .iter()
                .filter(|s| s.enabled)
                .map(|s| (s.id.clone(), s.logging))
                .collect(),
            debug,
            color: std::io::stderr().is_terminal() && std::env::var_os("NO_COLOR").is_none(),
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
        eprintln!(
            "[discovery {}] received={} elapsed_ms={}",
            timestamp(finished_at),
            candidates.len(),
            elapsed.as_millis()
        );
        for candidate in candidates {
            eprintln!("  {}", label(candidate));
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
                eprintln!(
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
                    eprintln!(
                        "{}",
                        styled(&format!("{prefix} REJECT {}", reason(r)), self.color)
                    );
                }
            } else if let Some(decision) = &item.decision {
                let negative = matches!(
                    decision.verdict,
                    strategy_api::Verdict::Reject | strategy_api::Verdict::Unavailable
                );
                eprintln!(
                    "{}",
                    styled(
                        &format!("{prefix} {:?}", decision.verdict),
                        negative && self.color
                    )
                );
                for r in &decision.reasons {
                    eprintln!(
                        "{}",
                        styled(&format!("  {}", reason(r)), negative && self.color)
                    );
                }
            } else if let Some(error) = &item.unavailable {
                eprintln!(
                    "{}",
                    styled(&format!("{prefix} UNAVAILABLE {}", safe(error)), self.color)
                );
            } else {
                eprintln!(
                    "{}",
                    styled(
                        &format!("{prefix} UNAVAILABLE no eligible pool/analysis result"),
                        self.color
                    )
                );
            }
            for probe in &item.trace {
                eprintln!(
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
        eprintln!(
            "{}",
            styled(
                &format!("{} UNAVAILABLE {}", label(candidate), safe(error)),
                self.color
            )
        );
    }
}
