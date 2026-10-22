use super::*;
impl Monitor<'_> {
    pub(super) async fn process_result(
        &mut self,
        candidate: Candidate,
        result: Result<
            Result<app_services::EvaluationReport, provider_api::ProviderError>,
            tokio::time::error::Elapsed,
        >,
        cancellation: &Cancellation,
        report: &mut CycleReport,
    ) -> Result<(), MonitorError> {
        report.evaluated += 1;
        let mut next_check_secs = 60;
        let mut panicked = false;
        let mut market_check = None;
        let mut admitted_watch = false;
        let any_logging = self
            .dependencies
            .strategies
            .keys()
            .any(|id| self.dependencies.config.strategy_logging(id));
        match result {
            Ok(Ok(evaluation)) => {
                if let Some(diagnostics) = &self.diagnostics {
                    diagnostics.evaluation(&candidate, &evaluation);
                }

                let next: Option<Vec<_>> = self
                    .dependencies
                    .strategies
                    .values()
                    .map(|strategy| {
                        strategy.next_market_check(
                            evaluation.as_of,
                            self.dependencies.config.monitor.close_grace_ms,
                        )
                    })
                    .collect();
                market_check = next.and_then(|times| times.into_iter().min());
                admitted_watch = evaluation.strategies.iter().any(|item| {
                    item.decision.as_ref().is_some_and(|d| {
                        matches!(
                            d.verdict,
                            strategy_api::Verdict::Watch | strategy_api::Verdict::Match
                        )
                    })
                });
                panicked = evaluation.strategies.iter().any(|item| {
                    item.trace.iter().any(|probe| {
                        probe.reason == provider_api::ProviderError::WorkerPanicked.to_string()
                    })
                });
                let log_panic = evaluation.strategies.iter().any(|item| {
                    self.dependencies.config.strategy_logging(&item.instance)
                        && item.trace.iter().any(|p| {
                            p.reason == provider_api::ProviderError::WorkerPanicked.to_string()
                        })
                });
                if panicked {
                    report.candidate_error(log_panic, "candidate provider worker panicked");
                }

                if !panicked {
                    self.candidate_panics.remove(&candidate.token);
                }

                next_check_secs = evaluation
                    .strategies
                    .iter()
                    .filter_map(|item| {
                        item.decision
                            .as_ref()
                            .map(|decision| decision.watch.after_secs)
                            .or_else(|| {
                                item.prefilter_rejection.as_ref().map(|_| {
                                    self.dependencies.strategies[&item.instance]
                                        .token_filter_recheck_secs()
                                })
                            })
                    })
                    .min()
                    .unwrap_or(60)
                    .clamp(10, 3600);
                self.accept_evaluation(evaluation, &candidate, cancellation, report)
                    .await?;
            }

            Ok(Err(error)) => {
                if fatal_provider_error(&error) {
                    return Err(error.into());
                }

                if any_logging && let Some(diagnostics) = &self.diagnostics {
                    diagnostics.failure(&candidate, &error.to_string());
                }

                panicked = matches!(error, provider_api::ProviderError::WorkerPanicked);
                report.candidate_error(any_logging, error.to_string());
            }

            Err(_) => {
                if any_logging && let Some(diagnostics) = &self.diagnostics {
                    diagnostics.failure(&candidate, "candidate deadline exceeded");
                }

                report.candidate_error(any_logging, "candidate deadline exceeded");
            }
        }

        if self.screened_watchlist() {
            self.update_screened_watch(&candidate, admitted_watch, report)?;
        }

        if panicked {
            report.worker_panics += 1;
            let count = self
                .candidate_panics
                .entry(candidate.token.clone())
                .or_default();
            *count = count.saturating_add(1);
            if *count >= PANIC_LIMIT {
                report.candidate_error(
                    any_logging,
                    "candidate quarantined after repeated worker panics",
                );
            }

            next_check_secs = next_check_secs.max(60);
        }

        if self.screened_watchlist() {
            return Ok(());
        }

        next_check_secs = next_check_secs.max(self.dependencies.config.monitor.min_recheck_secs);
        if let Some(entry) = self.watch.get_mut(&candidate.token) {
            entry.next_due = self
                .dependencies
                .clock
                .now()?
                .checked_add(next_check_secs as i64 * 1000)
                .map_err(|_| StateError::InvalidRequest)?;
            if !panicked && let Some(next) = market_check {
                let now = self.dependencies.clock.now()?;
                let floor = now
                    .checked_add(self.dependencies.config.monitor.min_recheck_secs as i64 * 1000)
                    .map_err(|_| StateError::InvalidRequest)?;
                let closed = next.max(floor);
                entry.next_due = if admitted_watch {
                    closed
                } else {
                    entry.next_due.max(closed)
                };
            }
        }

        Ok(())
    }

    fn update_screened_watch(
        &mut self,
        candidate: &Candidate,
        admitted: bool,
        report: &mut CycleReport,
    ) -> Result<(), StateError> {
        if !admitted {
            self.watch.remove(&candidate.token);
            report.dropped += 1;
            return Ok(());
        }

        if let Some(entry) = self.watch.get_mut(&candidate.token) {
            entry.candidate = candidate.clone();
            entry.next_due = self.next_discovery;
            return Ok(());
        }

        let capacity = self.dependencies.config.monitor.queue_capacity.min(1000);
        if self.watch.len() >= capacity {
            report.dropped += 1;
            return Ok(());
        }

        self.sequence = self.sequence.checked_add(1).ok_or(StateError::Capacity)?;
        self.watch.insert(
            candidate.token.clone(),
            WatchRecord {
                candidate: candidate.clone(),
                next_due: self.next_discovery,
                last_seen: self.dependencies.clock.now()?,
                order: self.sequence,
            },
        );

        Ok(())
    }

    async fn accept_evaluation(
        &self,
        evaluation: app_services::EvaluationReport,
        candidate: &Candidate,
        cancellation: &Cancellation,
        report: &mut CycleReport,
    ) -> Result<(), StateError> {
        for item in evaluation.strategies {
            let logging = self.dependencies.config.strategy_logging(&item.instance);
            if let Some(error) = item.unavailable {
                report.candidate_error(logging, error);
            }

            if self.dependencies.config.mode != AppMode::Signal || cancellation.is_cancelled() {
                continue;
            }

            if let Some(mut decision) = item.decision {
                if !decision
                    .warnings
                    .iter()
                    .any(|warning| warning == "PartialDiscoveryCoverage")
                {
                    decision.warnings.push("PartialDiscoveryCoverage".into());
                }

                let accepted_at = self.dependencies.clock.now()?;
                match signal_service::accept(
                    self.dependencies.state,
                    &decision,
                    candidate.symbol.as_deref(),
                    self.dependencies.targets,
                    accepted_at,
                )
                .await
                {
                    Ok(Acceptance::Accepted(_)) => report.accepted += 1,

                    Ok(Acceptance::Suppressed) => report.suppressed += 1,
                    Err(error) => report.candidate_error(logging, error.to_string()),
                }
            }
        }

        Ok(())
    }
}

impl CycleReport {
    fn candidate_error(&mut self, logging: bool, message: impl Into<String>) {
        if logging {
            self.errors.push(message.into());
        } else {
            self.muted_errors += 1;
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::tests::report;

    #[test]
    fn muted_diagnostics_still_count_toward_health_while_visible_messages_are_retained() {
        let mut report = report();
        let previous_muted = report.muted_errors;
        let previous_errors = report.errors.len();
        report.candidate_error(false, "muted diagnostic");
        report.candidate_error(true, "visible diagnostic");
        assert_eq!(report.muted_errors, previous_muted + 1);
        assert_eq!(report.errors.len(), previous_errors + 1);
        assert_eq!(report.errors.last().unwrap(), "visible diagnostic");
    }
}
