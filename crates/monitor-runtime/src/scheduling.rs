use super::*;
/// Explicit program-level TTL since the token last appeared in discovery.
pub(super) fn watch_retention_ms(config: &app_config::MonitorConfig) -> i64 {
    config.watchlist_retention_minutes as i64 * 60_000
}

pub(super) fn expire_watch(
    watch: &mut BTreeMap<TokenId, WatchRecord>,
    now: TimestampMs,
    retention_ms: i64,
) -> usize {
    let before = watch.len();
    watch.retain(|_, entry| now.get() - entry.last_seen.get() < retention_ms);
    before - watch.len()
}

impl Monitor<'_> {
    pub(super) fn screened_watchlist(&self) -> bool {
        self.dependencies
            .strategies
            .values()
            .any(|s| s.uses_token_metrics())
    }

    pub(super) async fn discover(
        &mut self,
        now: TimestampMs,
        cancellation: &Cancellation,
        report: &mut CycleReport,
    ) -> Result<(), StateError> {
        self.next_discovery = now
            .checked_add(self.dependencies.config.monitor.poll_interval_secs as i64 * 1000)
            .map_err(|_| StateError::InvalidRequest)?;
        let context = request_context(self.dependencies.config, cancellation);
        let duration =
            Duration::from_secs(self.dependencies.config.monitor.candidate_deadline_secs);
        let work = async {
            tokio::time::timeout(
                duration,
                self.dependencies.source.poll(
                    &self.query,
                    if self.screened_watchlist() {
                        None
                    } else {
                        self.cursor.as_ref()
                    },
                    &context,
                ),
            )
            .await
        };

        let started = Instant::now();
        let batch = match AssertUnwindSafe(work)
            .catch_unwind()
            .await
            .unwrap_or(Ok(Err(provider_api::ProviderError::WorkerPanicked)))
        {
            Ok(Ok(batch)) => batch,
            Ok(Err(error)) => {
                if matches!(error, provider_api::ProviderError::WorkerPanicked) {
                    self.source_panics = self.source_panics.saturating_add(1);
                    report.worker_panics += 1;
                    if self.source_panics >= PANIC_LIMIT {
                        report
                            .errors
                            .push("discovery quarantined after repeated worker panics".into());
                    }
                }

                report.errors.push(error.to_string());
                return Ok(());
            }

            Err(_) => {
                report.errors.push("discovery deadline exceeded".into());
                return Ok(());
            }
        };

        if let Some(diagnostics) = &self.diagnostics {
            diagnostics.discovery(
                self.dependencies.clock.now()?,
                started.elapsed(),
                &batch.items,
            );
        }

        if batch.next_cursor.as_ref().is_some_and(|cursor| {
            cursor.query_digest != self.query.digest() || cursor.value.len() > 4096
        }) {
            report.errors.push("invalid discovery cursor".into());
            return Ok(());
        }

        self.source_panics = 0;
        self.cursor = if self.screened_watchlist() {
            None
        } else {
            batch.next_cursor
        };

        let capacity = self.dependencies.config.monitor.queue_capacity.min(1000);
        for candidate in batch.items.into_iter().take(self.query.max_items) {
            if candidate.token.chain() != &self.query.chain
                || candidate.pool_hints.len() > 128
                || candidate
                    .pool_hints
                    .iter()
                    .any(|pool| pool.chain() != candidate.token.chain())
                || candidate
                    .symbol
                    .as_ref()
                    .is_some_and(|symbol| symbol.len() > 256)
            {
                report.dropped += 1;
                continue;
            }

            report.discovered += 1;
            self.emit(
                LifecycleEvent::CandidateObserved {
                    token: candidate.token.clone(),
                },
                now,
                report,
            );
            if self.screened_watchlist() {
                if let Some(entry) = self.watch.get_mut(&candidate.token) {
                    entry.last_seen = now;
                }

                self.pending_screening
                    .insert(candidate.token.clone(), candidate);
                continue;
            }

            if let Some(entry) = self.watch.get_mut(&candidate.token) {
                entry.candidate = candidate;
                entry.last_seen = now;
            } else if self.watch.len() < capacity {
                self.sequence = self.sequence.checked_add(1).ok_or(StateError::Capacity)?;
                self.watch.insert(
                    candidate.token.clone(),
                    WatchRecord {
                        candidate,
                        next_due: now,
                        last_seen: now,
                        order: self.sequence,
                    },
                );
            } else {
                report.dropped += 1;
            }
        }

        Ok(())
    }

    pub(super) fn due_candidates(&mut self, now: TimestampMs) -> Vec<Candidate> {
        let fresh: Vec<_> = self.pending_screening.keys().take(100).cloned().collect();
        let mut candidates: Vec<_> = fresh
            .iter()
            .filter_map(|token| self.pending_screening.remove(token))
            .collect();
        if self.screened_watchlist() {
            return candidates;
        }

        let selected: std::collections::BTreeSet<_> =
            candidates.iter().map(|c| c.token.clone()).collect();
        let mut due: Vec<_> = self
            .watch
            .values()
            .filter(|entry| {
                !selected.contains(&entry.candidate.token)
                    && entry.next_due <= now
                    && self
                        .candidate_panics
                        .get(&entry.candidate.token)
                        .is_none_or(|count| *count < PANIC_LIMIT)
            })
            .collect();
        let batch_size = if self.screened_watchlist() {
            100
        } else {
            self.dependencies.config.monitor.candidate_workers
        };

        let batch_size = batch_size.saturating_sub(candidates.len());
        if due.len() > batch_size {
            due.select_nth_unstable_by(batch_size, |a, b| watch_order(a, b));
            due.truncate(batch_size);
        }

        due.sort_unstable_by(|a, b| watch_order(a, b));
        candidates.extend(due.into_iter().map(|entry| entry.candidate.clone()));
        candidates
    }
}

fn watch_order(a: &WatchRecord, b: &WatchRecord) -> std::cmp::Ordering {
    (a.next_due, a.order, &a.candidate.token).cmp(&(b.next_due, b.order, &b.candidate.token))
}

#[cfg(test)]
#[path = "scheduling_tests.rs"]
mod tests;
