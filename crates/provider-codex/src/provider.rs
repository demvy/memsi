use crate::{CodexConfig, admission::Admission, observations, retry_after};
use domain::*;
use provider_api::*;
use serde::Deserialize;
use serde_json::{Value, json};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
pub struct CodexProvider {
    pub(crate) pool_tokens: crate::pool_tokens::PoolTokens,
    pub(crate) config: CodexConfig,
    pub(crate) usage: Option<crate::usage::UsageLedger>,
    pub(crate) chain: ChainId,
    pub(crate) client: reqwest::Client,
    pub(crate) admission: Admission,
    pub(crate) observations: std::sync::Arc<observations::Observations>,
}

pub fn descriptor() -> plugin_api::PluginDescriptor {
    plugin_api::PluginDescriptor {
        id: PluginId::new("provider.codex").unwrap(),
        version: env!("CARGO_PKG_VERSION").into(),
        kind: plugin_api::PluginKind::DataProvider,
        provides: ["discovery.candidates", "data.pools", "data.candles"]
            .into_iter()
            .map(|id| plugin_api::Capability::pure(CapabilityId::new(id).unwrap(), 1))
            .collect(),
    }
}

pub(crate) fn now() -> Result<TimestampMs, ProviderError> {
    let ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| ProviderError::InvalidData("clock before epoch".into()))?
        .as_millis();
    TimestampMs::new(i64::try_from(ms).map_err(|_| ProviderError::InvalidRequest)?)
        .map_err(Into::into)
}

impl CodexProvider {
    pub fn new(chain: ChainId, config: CodexConfig) -> Result<Self, ProviderError> {
        chain.validate()?;
        config.validate()?;
        let mapped = match &chain {
            ChainId::Evm { chain_id } => *chain_id == config.network_id as u64,
            ChainId::Solana { genesis_hash } => {
                genesis_hash == "5eykt4UsFv8P8NJdTREpY1vzqKqZKvdpKuc147dw2N9d"
                    && config.network_id == 1399811149
            }
        };

        if !mapped {
            return Err(ProviderError::InvalidRequest);
        }

        let client = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .timeout(Duration::from_secs(30))
            .build()
            .map_err(|_| ProviderError::Transport)?;
        Ok(Self {
            pool_tokens: Default::default(),
            usage: config.usage.clone().map(crate::usage::UsageLedger::new),
            observations: std::sync::Arc::new(observations::Observations::default()),
            admission: Admission::new(
                config.concurrency,
                Duration::from_millis(config.min_interval_ms),
            ),
            config,
            chain,
            client,
        })
    }

    pub(crate) async fn usage_status(&self) -> Result<Option<ApiUsageStatus>, ProviderError> {
        let Some(ledger) = &self.usage else {
            return Ok(None);
        };

        let mut keys = Vec::new();
        for reference in std::iter::once(&self.config.api_key).chain(&self.config.fallback_api_keys)
        {
            let Ok(key) = reference.resolve() else {
                continue;
            };

            let usage = ledger.status(&key).await?;
            if !keys
                .iter()
                .any(|existing: &ApiKeyUsage| existing.key_id == usage.key_id)
            {
                keys.push(usage);
            }
        }

        if keys.is_empty() {
            return Err(ProviderError::Unauthorized);
        }

        let exhausted = keys.iter().all(|usage| usage.used >= usage.limit);
        Ok(Some(ApiUsageStatus { keys, exhausted }))
    }

    pub fn transport_observations(&self) -> Option<TransportObservations> {
        self.observations.snapshot()
    }

    pub(crate) fn check_chain(&self, chain: &ChainId) -> Result<(), ProviderError> {
        if chain != &self.chain {
            return Err(ProviderError::Unsupported("chain".into()));
        }

        Ok(())
    }

    pub(crate) fn symbol(&self, pool: &PoolId) -> Result<String, ProviderError> {
        self.check_chain(pool.chain())?;
        Ok(format!(
            "{}:{}",
            pool.as_address()
                .ok_or_else(|| ProviderError::Unsupported("opaque pool identifier".into()))?,
            self.config.network_id
        ))
    }

    pub(crate) fn resolve_credential(&self) -> Result<String, ProviderError> {
        std::iter::once(&self.config.api_key)
            .chain(&self.config.fallback_api_keys)
            .find_map(|reference| reference.resolve().ok())
            .ok_or(ProviderError::Unauthorized)
    }

    pub(crate) async fn query(
        &self,
        query: &str,
        variables: Value,
        ctx: &RequestContext,
    ) -> Result<Value, ProviderError> {
        self.query_at(
            query,
            variables,
            ctx,
            "https://graph.codex.io/graphql",
            || self.resolve_credential(),
        )
        .await
    }

    pub(crate) async fn query_at(
        &self,
        query: &str,
        variables: Value,
        ctx: &RequestContext,
        endpoint: &str,
        credential: impl FnOnce() -> Result<String, ProviderError>,
    ) -> Result<Value, ProviderError> {
        let data = self
            .query_raw_at(query, variables, ctx, endpoint, credential)
            .await?;
        serde_json::from_str(data.get()).map_err(|_| ProviderError::InvalidSchema("JSON".into()))
    }

    pub(crate) async fn query_raw_at(
        &self,
        query: &str,
        variables: Value,
        ctx: &RequestContext,
        endpoint: &str,
        credential: impl FnOnce() -> Result<String, ProviderError>,
    ) -> Result<Box<serde_json::value::RawValue>, ProviderError> {
        let guard = self.observations.begin();
        let result = self
            .query_raw_inner(query, variables, ctx, endpoint, credential)
            .await;
        guard.finish(&result);
        result
    }

    async fn query_raw_inner(
        &self,
        query: &str,
        variables: Value,
        ctx: &RequestContext,
        endpoint: &str,
        credential: impl FnOnce() -> Result<String, ProviderError>,
    ) -> Result<Box<serde_json::value::RawValue>, ProviderError> {
        ctx.check()?;
        let work = async {
            let mut key = None;
            let mut credential = Some(credential);
            let mut attempt = 0u32;
            let (_permit, mut response) = loop {
                let permit = self.admission.acquire(ctx).await?;
                ctx.check()?;
                if key.is_none() {
                    key = Some(credential.take().expect("credential resolved once")()?);
                }

                if let Some(ledger) = &self.usage {
                    if !ledger
                        .charge(key.as_ref().ok_or(ProviderError::Unauthorized)?)
                        .await?
                    {
                        let candidates = std::iter::once(&self.config.api_key)
                            .chain(&self.config.fallback_api_keys)
                            .filter_map(|reference| reference.resolve().ok())
                            .collect::<Vec<_>>();
                        key = Some(ledger.charge_first(candidates).await?);
                    }

                    ctx.check()?;
                }

                self.observations.attempt(attempt > 0);
                let started = std::time::Instant::now();
                let result = self
                    .client
                    .post(endpoint)
                    .header(reqwest::header::AUTHORIZATION, key.as_ref().unwrap())
                    .json(&json!({ "query" : query, "variables" : variables }))
                    .send()
                    .await;
                let transient = match &result {
                    Ok(response) => matches!(response.status().as_u16(), 502..=504),
                    Err(error) => error.is_timeout() || error.is_connect(),
                };

                let retrying = transient && attempt < 2;
                match &result {
                    Err(error) => crate::diagnostics::failure(
                        crate::diagnostics::category(error),
                        None,
                        attempt,
                        started.elapsed(),
                        retrying,
                    ),
                    Ok(response) if response.status().as_u16() != 200 => {
                        crate::diagnostics::failure(
                            "http_status",
                            Some(response.status().as_u16()),
                            attempt,
                            started.elapsed(),
                            retrying,
                        );
                    }

                    _ => {}
                }

                if retrying {
                    drop(result);
                    drop(permit);
                    let jitter = SystemTime::now()
                        .duration_since(UNIX_EPOCH)
                        .unwrap_or_default()
                        .subsec_millis()
                        % 100;
                    tokio::time::sleep(Duration::from_millis(
                        (250u64 << attempt) + u64::from(jitter),
                    ))
                    .await;
                    ctx.check()?;
                    attempt += 1;
                    continue;
                }

                let response = result.map_err(|error| {
                    if error.is_timeout() {
                        ProviderError::Timeout
                    } else {
                        ProviderError::Transport
                    }
                })?;
                break (permit, response);
            };

            match response.status().as_u16() {
                401 | 403 => return Err(ProviderError::Unauthorized),
                429 => {
                    self.observations.rate_limited();
                    let delay = retry_after::delay(
                        response
                            .headers()
                            .get("retry-after")
                            .and_then(|h| h.to_str().ok()),
                        SystemTime::now(),
                    );
                    let retry_after_ms = u64::try_from(delay.as_millis()).unwrap_or(u64::MAX);
                    self.admission
                        .cooldown(Duration::from_millis(retry_after_ms))?;
                    return Err(ProviderError::RateLimited { retry_after_ms });
                }

                200 => {}
                _ => return Err(ProviderError::Transport),
            }

            let mut body = Vec::new();
            let body_started = std::time::Instant::now();
            while let Some(chunk) = response.chunk().await.map_err(|_| {
                crate::diagnostics::failure(
                    "response_body",
                    Some(200),
                    attempt,
                    body_started.elapsed(),
                    false,
                );
                ProviderError::Transport
            })? {
                ctx.check()?;
                if body.len() + chunk.len() > 4 * 1024 * 1024 {
                    return Err(ProviderError::InvalidSchema("body exceeds limit".into()));
                }

                body.extend_from_slice(&chunk);
            }

            #[derive(Deserialize)]
            struct Envelope {
                errors: Option<Value>,
                data: Option<Box<serde_json::value::RawValue>>,
            }

            let envelope: Envelope = serde_json::from_slice(&body)
                .map_err(|_| ProviderError::InvalidSchema("JSON".into()))?;
            if envelope
                .errors
                .as_ref()
                .is_some_and(|v| !v.is_null() && v.as_array().is_none_or(|a| !a.is_empty()))
            {
                return Err(ProviderError::InvalidSchema(
                    "GraphQL operation failed".into(),
                ));
            }

            envelope
                .data
                .filter(|data| data.get().trim_start().starts_with('{'))
                .ok_or_else(|| ProviderError::InvalidSchema("missing data".into()))
        };

        tokio::select! {
            biased;
            _ = ctx.cancellation.cancelled() => Err(ProviderError::Cancelled),
            result = tokio::time::timeout_at(ctx.deadline.into(), work) => {
                result.map_err(|_| ProviderError::Timeout)?
            }
        }
    }
}
