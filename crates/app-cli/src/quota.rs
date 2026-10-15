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
        result = & mut watch_quota => { cancellation.cancel(); result ?;
        eprintln!("parser stopped: all configured API keys have exhausted their quota");
        tokio::time::timeout(Duration::from_secs(15), & mut work). await .map_err(| _ |
        "quota shutdown deadline exceeded") ? } result = & mut work => {
        drop(watch_quota); inspect(reader, sinks). await ?; result }
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
        eprintln!("{text}");
        let ctx = RequestContext {
            priority: Default::default(),
            deadline: Instant::now() + Duration::from_secs(15),
            cancellation: Default::default(),
        };
        for (id, sink) in sinks {
            match sink.send_notice(&text, &ctx).await {
                Ok(_) => eprintln!("quota notice accepted by {id}"),
                Err(error) => {
                    eprintln!("quota notice failed for {id}: {error}; no automatic resend")
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
