#![forbid(unsafe_code)]
mod incremental;
pub mod selection;
use domain::*;
use futures::FutureExt;
use provider_api::*;
use std::{
    collections::BTreeMap,
    panic::AssertUnwindSafe,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
    time::{Duration, Instant},
};

use tokio::sync::{Mutex, watch};
type Outcome = Result<Arc<CandleSeries>, ProviderError>;
struct Entry {
    priority: RequestPriority,
    rx: watch::Receiver<Option<Outcome>>,
    created: Instant,
}

#[derive(Default)]
struct Counters {
    hit: AtomicU64,
    join: AtomicU64,
    upstream: AtomicU64,
}

#[derive(Debug, serde::Serialize)]
pub struct CacheStats {
    pub cache_hits: u64,
    pub singleflight_joins: u64,
    pub upstream_attempts: u64,
}

/// One hub owns one configured provider; cached data never crosses providers.
pub struct DataHub {
    provider: Arc<dyn CandleProvider>,
    entries: Mutex<BTreeMap<ContentHash, Entry>>,
    capacity: usize,
    ttl: Duration,
    upstream_timeout: Duration,
    counts: Counters,
}

impl DataHub {
    pub fn new(
        provider: Arc<dyn CandleProvider>,
        capacity: usize,
        ttl: Duration,
        upstream_timeout: Duration,
    ) -> Result<Self, ProviderError> {
        if capacity == 0
            || capacity > 1024
            || ttl.is_zero()
            || ttl > Duration::from_secs(3600)
            || upstream_timeout.is_zero()
            || upstream_timeout > Duration::from_secs(300)
        {
            return Err(ProviderError::InvalidRequest);
        }

        Ok(Self {
            provider,
            entries: Mutex::new(BTreeMap::new()),
            capacity,
            ttl,
            upstream_timeout,
            counts: Counters::default(),
        })
    }

    pub fn stats(&self) -> CacheStats {
        CacheStats {
            cache_hits: self.counts.hit.load(Ordering::Relaxed),
            singleflight_joins: self.counts.join.load(Ordering::Relaxed),
            upstream_attempts: self.counts.upstream.load(Ordering::Relaxed),
        }
    }

    pub async fn candles(&self, request: &CandleRequest, ctx: &RequestContext) -> Outcome {
        request.validate()?;
        ctx.check()?;
        let key = ContentHash::of(request).map_err(|_| ProviderError::InvalidRequest)?;
        let mut entries = self.entries.lock().await;
        let prefix = if self.provider.supports_closed_candle_deltas() {
            entries
                .values()
                .filter(|entry| entry.created.elapsed() < self.ttl)
                .filter_map(|entry| match &*entry.rx.borrow() {
                    Some(Ok(series)) if incremental::can_extend(series, request) => {
                        Some(series.clone())
                    }

                    _ => None,
                })
                .max_by_key(|series| series.end_exclusive)
        } else {
            None
        };

        entries.retain(|_, e| match &*e.rx.borrow() {
            None => e.rx.has_changed().is_ok(),
            Some(Ok(_)) => e.created.elapsed() < self.ttl,
            Some(Err(_)) => false,
        });
        let mut rx = if let Some(entry) = entries.get(&key) {
            if ctx.priority.is_active() {
                entry.priority.promote_active();
            }

            if entry.rx.borrow().is_some() {
                self.counts.hit.fetch_add(1, Ordering::Relaxed);
            } else {
                self.counts.join.fetch_add(1, Ordering::Relaxed);
            }

            entry.rx.clone()
        } else {
            let shadow_limit = (self.capacity * 3 / 4).max(1);
            if !ctx.priority.is_active()
                && entries
                    .values()
                    .filter(|entry| !entry.priority.is_active() && entry.rx.borrow().is_none())
                    .count()
                    >= shadow_limit
            {
                return Err(ProviderError::RateLimited {
                    retry_after_ms: 100,
                });
            }

            if entries.len() >= self.capacity {
                let victim = entries
                    .iter()
                    .filter(|(_, e)| e.rx.borrow().is_some())
                    .min_by_key(|(_, e)| e.created)
                    .map(|(k, _)| k.clone());
                if let Some(victim) = victim {
                    entries.remove(&victim);
                } else {
                    return Err(ProviderError::RateLimited {
                        retry_after_ms: 100,
                    });
                }
            }

            let (tx, rx) = watch::channel(None);
            let priority = ctx.priority.independent();
            entries.insert(
                key,
                Entry {
                    priority: priority.clone(),
                    rx: rx.clone(),
                    created: Instant::now(),
                },
            );
            let provider = self.provider.clone();
            let request = request.clone();
            let timeout = self.upstream_timeout;
            self.counts.upstream.fetch_add(1, Ordering::Relaxed);
            tokio::spawn(async move {
                let context = RequestContext {
                    priority,
                    deadline: Instant::now() + timeout,
                    cancellation: Cancellation::default(),
                };

                let work = async {
                    let upstream = prefix.as_ref().map_or_else(
                        || Ok(request.clone()),
                        |series| incremental::delta_request(series, &request),
                    )?;
                    match tokio::time::timeout(timeout, provider.candles(&upstream, &context)).await
                    {
                        Ok(Ok(series)) => {
                            let series = validate_response(series, &upstream)?;
                            let series = match &prefix {
                                Some(prefix) => {
                                    incremental::merge(prefix, series, &upstream, &request)?
                                }

                                None => series,
                            };

                            validate_response(series, &request).map(Arc::new)
                        }

                        Ok(Err(e)) => Err(e),
                        Err(_) => Err(ProviderError::Timeout),
                    }
                };

                let result = AssertUnwindSafe(work)
                    .catch_unwind()
                    .await
                    .unwrap_or(Err(ProviderError::WorkerPanicked));
                let _ = tx.send(Some(result));
            });
            rx
        };

        drop(entries);
        loop {
            ctx.check()?;
            if let Some(result) = rx.borrow().clone() {
                return result;
            }

            tokio::select! {
                changed = rx.changed() => {
                    if changed.is_err() {
                        return Err(ProviderError::Transport);
                    }
                },
                _ = tokio::time::sleep(Duration::from_millis(10)) => {}
            }
        }
    }
}

fn validate_response(
    mut series: CandleSeries,
    r: &CandleRequest,
) -> Result<CandleSeries, ProviderError> {
    series.validate(r.min_bars)?;
    if series.pool != r.pool
        || series.priced_token != r.priced_token
        || series.price_unit != r.price_unit
        || series.resolution != r.resolution
        || series.end_exclusive != r.end_exclusive
        || series.candles.len() > r.max_bars
        || series.candles[0].open_time < r.from_inclusive
    {
        return Err(ProviderError::InvalidData(
            "response does not match candle request".into(),
        ));
    }

    series.provenance.input_hash = ContentHash::of(&(r, &series.candles))
        .map_err(|_| ProviderError::InvalidData("input hash".into()))?;
    Ok(series)
}

#[async_trait::async_trait]
impl CandleObservationReader for DataHub {
    async fn candle_observations(
        &self,
        token: &TokenId,
        limit: usize,
    ) -> Result<CandleObservations, ProviderError> {
        if !(1..=16).contains(&limit) {
            return Err(ProviderError::InvalidRequest);
        }

        let entries = self.entries.lock().await;
        let mut series: Vec<_> = entries
            .values()
            .filter_map(|entry| {
                if entry.created.elapsed() >= self.ttl {
                    return None;
                }

                match &*entry.rx.borrow() {
                    Some(Ok(series)) if &series.priced_token == token => Some(series.clone()),
                    _ => None,
                }
            })
            .collect();
        drop(entries);
        series.sort_by(|a, b| {
            (
                b.end_exclusive,
                b.provenance.fetched_at,
                &b.provenance.input_hash,
            )
                .cmp(&(
                    a.end_exclusive,
                    a.provenance.fetched_at,
                    &a.provenance.input_hash,
                ))
        });
        let observed_series = series.len();
        series.truncate(limit);
        Ok(CandleObservations {
            truncated: observed_series > series.len(),
            observed_series,
            series,
        })
    }
}

#[cfg(test)]
mod tests;
