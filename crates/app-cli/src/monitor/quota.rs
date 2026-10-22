//! Subscription lifecycle: warn, fail over in the provider, stop when all keys are spent.
use provider_api::{ApiKeyUsage, Cancellation, RequestContext, TransportObservationReader};
use signal_service::delivery::Sinks;
use std::{
    future::Future,
    time::{Duration, Instant},
};

type Error = Box<dyn std::error::Error>;
pub async fn supervise(
    work: impl Future<Output = Result<(), Error>>,
    reader: Option<&dyn TransportObservationReader>,
    sinks: &Sinks,
    cancellation: &Cancellation,
) -> Result<(), Error> {
    let Some(reader) = reader else {
        return work.await;
    };

    tokio::pin!(work);
    let stop = match inspect(reader, sinks).await {
        Ok(stop) => stop,
        Err(error) => {
            cancellation.cancel();
            return Err(error);
        }
    };

    if stop {
        cancellation.cancel();
        return tokio::time::timeout(Duration::from_secs(15), &mut work)
            .await
            .map_err(|_| "quota shutdown deadline exceeded")?;
    }

    let watch_quota = async {
        let mut tick = tokio::time::interval(Duration::from_millis(250));
        loop {
            tick.tick().await;
            if inspect(reader, sinks).await? {
                return Ok::<_, Error>(());
            }
        }
    };

    let mut watch_quota = Box::pin(watch_quota);
    tokio::select! {
        result = &mut watch_quota => {
            cancellation.cancel();
            result?;

            tracing::info!("parser stopped: all configured API keys have exhausted their quota");
            tokio::time::timeout(Duration::from_secs(15), &mut work)
                .await
                .map_err(|_| "quota shutdown deadline exceeded")?
        }
        result = &mut work => {
            drop(watch_quota);
            inspect(reader, sinks).await?;
            result
        }
    }
}

async fn inspect(reader: &dyn TransportObservationReader, sinks: &Sinks) -> Result<bool, Error> {
    let Some(status) = reader.api_usage().await? else {
        return Ok(false);
    };

    for usage in &status.keys {
        let exhausted = usage.used >= usage.limit;
        if usage.used < usage.warn_at || !reader.claim_usage_notice(usage, exhausted).await? {
            continue;
        }

        let text = message(usage, exhausted, status.exhausted);
        tracing::info!("{text}");
        let ctx = RequestContext {
            priority: Default::default(),
            deadline: Instant::now() + Duration::from_secs(15),
            cancellation: Default::default(),
        };

        for (id, sink) in sinks {
            match sink.send_notice(&text, &ctx).await {
                Ok(_) => tracing::info!("quota notice accepted by {id}"),
                Err(error) => {
                    tracing::warn!(sink = %id, error = %error, "quota notice failed; no automatic resend")
                }
            }
        }
    }

    Ok(status.exhausted)
}

fn message(usage: &ApiKeyUsage, exhausted: bool, stop: bool) -> String {
    let title = if exhausted {
        "🔴 Codex.io — пакет запитів вичерпано"
    } else {
        "🟡 Codex.io — пакет запитів скоро закінчиться"
    };

    let action = if stop {
        "⏸ Parser зупиняється: доступних резервних ключів немає. Додайте ключ іншої підписки або дочекайтесь нового періоду."
    } else if exhausted {
        "🔄 Наступні запити використовують доступний резервний ключ Codex.io."
    } else {
        "🔑 Підготуйте резервний ключ/provider даних. V1 підтримує резервні ключі Codex.io."
    };

    format!(
        "{title}\n📊 Локально враховано: {} / {} HTTP-спроб\n🗓 Період від {} UTC\nKey ID: {}\n{action}",
        usage.used,
        usage.limit,
        usage.period,
        usage.key_id.chars().take(12).collect::<String>()
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quota_messages_distinguish_warning_failover_and_stop_and_shorten_key_identity() {
        let usage = ApiKeyUsage {
            key_id: "abcdef0123456789".into(),
            period: "2025-01-01".into(),
            used: 9,
            limit: 10,
            warn_at: 9,
        };
        let warning = message(&usage, false, false);
        let failover = message(&usage, true, false);
        let stop = message(&usage, true, true);
        assert!(warning.contains("🟡"));
        assert!(failover.contains("🔄"));
        assert!(stop.contains("⏸"));
        assert!(warning.contains("9 / 10"));
        assert!(warning.contains("abcdef012345"));
        assert!(!warning.contains(&usage.key_id));
    }

    #[tokio::test]
    async fn supervisor_without_usage_reader_preserves_work_result() {
        let cancellation = Cancellation::default();
        assert!(
            supervise(async { Ok(()) }, None, &Sinks::new(), &cancellation)
                .await
                .is_ok()
        );
        assert!(!cancellation.is_cancelled());
        assert!(
            supervise(
                async { Err("work failed".into()) },
                None,
                &Sinks::new(),
                &cancellation
            )
            .await
            .is_err()
        );
    }
}

#[cfg(test)]
mod policy_tests {
    use super::*;
    use provider_api::{ApiUsageStatus, ProviderError, TransportObservations};
    use std::sync::atomic::{AtomicUsize, Ordering};

    struct Reader {
        status: Result<Option<ApiUsageStatus>, ProviderError>,
        claims: AtomicUsize,
    }

    #[async_trait::async_trait]
    impl TransportObservationReader for Reader {
        fn transport_observations(&self) -> Option<TransportObservations> {
            None
        }
        async fn api_usage(&self) -> Result<Option<ApiUsageStatus>, ProviderError> {
            self.status.clone()
        }
        async fn claim_usage_notice(
            &self,
            _: &ApiKeyUsage,
            _: bool,
        ) -> Result<bool, ProviderError> {
            Ok(self.claims.fetch_add(1, Ordering::Relaxed) == 0)
        }
    }

    fn usage_reader(used: u64, exhausted: bool) -> Reader {
        Reader {
            status: Ok(Some(ApiUsageStatus {
                keys: vec![ApiKeyUsage {
                    key_id: "key-identity".into(),
                    period: "2025-01-01".into(),
                    used,
                    limit: 10,
                    warn_at: 9,
                }],
                exhausted,
            })),
            claims: AtomicUsize::new(0),
        }
    }

    #[tokio::test]
    async fn quota_inspection_claims_only_warning_or_exhaustion_notices() {
        let reader = usage_reader(8, false);
        assert!(!inspect(&reader, &Sinks::new()).await.unwrap());
        assert_eq!(reader.claims.load(Ordering::Relaxed), 0);
        let reader = usage_reader(9, false);
        assert!(!inspect(&reader, &Sinks::new()).await.unwrap());
        assert_eq!(reader.claims.load(Ordering::Relaxed), 1);
        assert!(!inspect(&reader, &Sinks::new()).await.unwrap());
        let exhausted = usage_reader(10, true);
        assert!(inspect(&exhausted, &Sinks::new()).await.unwrap());
    }

    #[tokio::test]
    async fn exhaustion_and_accounting_errors_cancel_work_before_shutdown_settlement() {
        let exhausted = usage_reader(10, true);
        let cancellation = Cancellation::default();
        let work = async {
            assert!(cancellation.is_cancelled());
            Ok(())
        };
        assert!(
            supervise(work, Some(&exhausted), &Sinks::new(), &cancellation)
                .await
                .is_ok()
        );
        let failed = Reader {
            status: Err(ProviderError::UsageAccountingUnavailable),
            claims: AtomicUsize::new(0),
        };
        let cancellation = Cancellation::default();
        assert!(
            supervise(
                async { Ok(()) },
                Some(&failed),
                &Sinks::new(),
                &cancellation
            )
            .await
            .is_err()
        );
        assert!(cancellation.is_cancelled());
        let no_accounting = Reader {
            status: Ok(None),
            claims: AtomicUsize::new(0),
        };
        let cancellation = Cancellation::default();
        assert!(
            supervise(
                async { Ok(()) },
                Some(&no_accounting),
                &Sinks::new(),
                &cancellation
            )
            .await
            .is_ok()
        );
        assert!(!cancellation.is_cancelled());
    }
}
