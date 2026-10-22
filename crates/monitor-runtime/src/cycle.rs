use super::*;
use crate::scheduling::{expire_watch, watch_retention_ms};
impl Monitor<'_> {
    pub async fn cycle(
        &mut self,
        cancellation: &Cancellation,
    ) -> Result<CycleReport, MonitorError> {
        self.cycle_with_delivery(cancellation, true).await
    }

    pub(super) async fn cycle_with_delivery(
        &mut self,
        cancellation: &Cancellation,
        deliver: bool,
    ) -> Result<CycleReport, MonitorError> {
        let now = self.dependencies.clock.now()?;
        let mut report = CycleReport {
            discovered: 0,
            dropped: 0,
            watched: 0,
            evaluated: 0,
            accepted: 0,
            suppressed: 0,
            errors: vec![],
            muted_errors: 0,
            worker_panics: 0,
            discovery_quarantined: self.source_panics >= PANIC_LIMIT,
            quarantined_candidates: self
                .candidate_panics
                .values()
                .filter(|count| **count >= PANIC_LIMIT)
                .count(),
            state: self.dependencies.state.stats().await?,
        };

        if cancellation.is_cancelled() {
            return Ok(report);
        }

        self.restore(now).await?;
        if now >= self.next_discovery && self.source_panics < PANIC_LIMIT {
            self.discover(now, cancellation, &mut report).await?;
        }

        let retention_ms = watch_retention_ms(&self.dependencies.config.monitor);
        report.dropped += expire_watch(&mut self.watch, now, retention_ms);
        self.candidate_panics
            .retain(|token, _| self.watch.contains_key(token));
        if cancellation.is_cancelled() {
            self.checkpoint().await?;
            report.watched = self.watch.len();
            report.state = self.dependencies.state.stats().await?;
            self.panic_stats(&mut report);
            return Ok(report);
        }

        let candidates = self.due_candidates(now);
        let config = self.dependencies.config;
        let services = self.dependencies.services;
        let strategies = self.dependencies.strategies;
        let analyzers = self.dependencies.analyzers;
        let metric_context = request_context(config, cancellation);
        let metrics = services
            .prefetch_token_metrics(&candidates, strategies, &metric_context)
            .await;
        let metrics = &metrics;
        let jobs = candidates.into_iter().map(|candidate| async move {
            let context = request_context(config, cancellation);
            let timeout = Duration::from_secs(config.monitor.candidate_deadline_secs);
            let work = async {
                let prefetched = match metrics {
                    Ok(metrics) => metrics.get(&candidate.token).cloned().unwrap_or_default(),
                    Err(error) => return Ok(Err(error.clone())),
                };

                tokio::time::timeout(
                    timeout,
                    services.evaluate_with_metrics(
                        &candidate.token,
                        strategies,
                        analyzers,
                        config.monitor.close_grace_ms,
                        Some(&prefetched),
                        &context,
                    ),
                )
                .await
            };

            let result = AssertUnwindSafe(work)
                .catch_unwind()
                .await
                .unwrap_or(Ok(Err(provider_api::ProviderError::WorkerPanicked)));
            (candidate, result)
        });
        let mut results = stream::iter(jobs).buffer_unordered(config.monitor.candidate_workers);
        while let Some((candidate, result)) = results.next().await {
            self.process_result(candidate, result, cancellation, &mut report)
                .await?;
        }

        if deliver && self.dependencies.config.mode == AppMode::Signal {
            for _ in 0..16 {
                if !signal_service::delivery::deliver_one(
                    self.dependencies.state,
                    self.dependencies.sinks,
                    self.dependencies.clock,
                    cancellation,
                    &DeliveryPolicy::default(),
                )
                .await?
                {
                    break;
                }
            }
        }

        self.checkpoint().await?;
        report.watched = self.watch.len();
        report.state = self.dependencies.state.stats().await?;
        self.panic_stats(&mut report);
        Ok(report)
    }
}
