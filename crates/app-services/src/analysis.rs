use super::*;
#[derive(Debug, Serialize)]
pub struct ProbeFailure {
    pub pool: PoolId,
    pub error: String,
}

#[derive(Debug, Serialize)]
pub struct AnalyzerReport {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub market: Option<Box<PoolSnapshot>>,
    pub instance: InstanceId,
    pub config_digest: ContentHash,
    pub selected_pool: Option<PoolId>,
    pub input_hash: Option<ContentHash>,
    pub result: Option<AnalysisValue>,
    pub error: Option<String>,
    pub probes: Vec<ProbeFailure>,
}

#[derive(Debug, Serialize)]
pub struct AnalysisReport {
    pub schema_version: u16,
    pub token: TokenId,
    pub as_of: TimestampMs,
    pub purpose: Purpose,
    pub coverage: Coverage,
    pub excluded: Vec<Exclusion>,
    pub analyzers: Vec<AnalyzerReport>,
}

pub struct AnalyzeRequest {
    pub token: TokenId,
    pub pool: Option<PoolId>,
    pub resolution: Resolution,
    pub analyzer_instances: Vec<InstanceId>,
    pub policy: SelectionPolicy,
    pub close_grace_ms: u64,
}

impl Services {
    /// Manual analysis has no notification or trading side effects.
    pub async fn analyze(
        &self,
        request: &AnalyzeRequest,
        analyzers: &BTreeMap<InstanceId, Box<dyn Analyzer>>,
        ctx: &RequestContext,
    ) -> Result<AnalysisReport, ProviderError> {
        if request.close_grace_ms > 60000
            || request.policy.max_probes == 0
            || request.policy.max_probes > 20
        {
            return Err(ProviderError::InvalidRequest);
        }

        let selected: Vec<_> = if request.analyzer_instances.is_empty() {
            analyzers.values().map(Box::as_ref).collect()
        } else {
            request
                .analyzer_instances
                .iter()
                .map(|id| {
                    analyzers
                        .get(id)
                        .map(Box::as_ref)
                        .ok_or(ProviderError::InvalidRequest)
                })
                .collect::<Result<_, _>>()?
        };

        if selected.is_empty() {
            return Err(ProviderError::InvalidRequest);
        }

        let pools = if let Some(pool) = &request.pool {
            if pool.chain() != request.token.chain() {
                return Err(ProviderError::InvalidRequest);
            }

            let value = read_bounded(self.provider.pool(pool, ctx), ctx).await?;
            if &value.pool != pool {
                return Err(ProviderError::InvalidData("explicit pool mismatch".into()));
            }

            PoolDiscoveryResult {
                pools: vec![value],
                coverage: Coverage {
                    complete: false,
                    truncated: false,
                    warnings: vec!["Explicit pool only".into()],
                },
            }
        } else {
            self.list_pools(&request.token, ctx).await?
        };

        ctx.check()?;
        let as_of = self.evaluation_time()?;
        let cutoff = as_of.closed_cutoff(request.resolution, request.close_grace_ms)?;
        let plan = selection::rank(&request.token, pools.pools, &request.policy, as_of)?;
        let mut reports = Vec::new();
        for analyzer in selected {
            ctx.check()?;
            let requirement = analyzer.requirements(request.resolution, PriceUnit::UsdPerToken);
            if !requirement.dependencies.is_empty() {
                return Err(ProviderError::Unsupported(
                    "analyzer dependency planner".into(),
                ));
            }

            let n = requirement.candles.desired_bars;
            if n == 0
                || n > 1500
                || requirement.candles.min_bars > n
                || requirement.candles.resolution != request.resolution
                || requirement.candles.price_unit != PriceUnit::UsdPerToken
            {
                return Err(ProviderError::InvalidRequest);
            }

            let from = TimestampMs::new(
                cutoff
                    .get()
                    .checked_sub(n as i64 * request.resolution.millis())
                    .ok_or(ProviderError::InvalidRequest)?,
            )?;
            let mut report = AnalyzerReport {
                market: None,
                instance: analyzer.instance().clone(),
                config_digest: analyzer.config_digest().clone(),
                selected_pool: None,
                input_hash: None,
                result: None,
                error: None,
                probes: vec![],
            };

            for pool in plan.ranked.iter().take(request.policy.max_probes) {
                let candles = CandleRequest {
                    pool: pool.pool.clone(),
                    priced_token: request.token.clone(),
                    price_unit: PriceUnit::UsdPerToken,
                    resolution: request.resolution,
                    from_inclusive: from,
                    end_exclusive: cutoff,
                    min_bars: requirement.candles.min_bars,
                    max_bars: n,
                };

                match self.hub.candles(&candles, ctx).await {
                    Ok(series) => {
                        let artifacts = ArtifactStore::default();
                        match analyzer.analyze(&AnalysisContext {
                            candles: &series,
                            dependencies: &artifacts,
                            as_of,
                        }) {
                            Ok(value) => {
                                report.selected_pool = Some(pool.pool.clone());
                                report.market = Some(Box::new(pool.clone()));
                                report.input_hash = Some(series.provenance.input_hash.clone());
                                report.result = Some(value);
                                break;
                            }

                            Err(error) => report.probes.push(ProbeFailure {
                                pool: pool.pool.clone(),
                                error: format!("analysis: {error}"),
                            }),
                        }
                    }

                    Err(
                        e @ (ProviderError::Cancelled
                        | ProviderError::Timeout
                        | ProviderError::Unauthorized
                        | ProviderError::RateLimited { .. }),
                    ) => return Err(e),
                    Err(error) => report.probes.push(ProbeFailure {
                        pool: pool.pool.clone(),
                        error: error.to_string(),
                    }),
                }
            }

            if report.result.is_none() {
                report.error = Some(
                    if plan.ranked.is_empty() {
                        "no_eligible_pool"
                    } else {
                        "selection_unavailable"
                    }
                    .into(),
                );
            }

            reports.push(report);
        }

        Ok(AnalysisReport {
            schema_version: 1,
            token: request.token.clone(),
            as_of,
            purpose: request.policy.purpose,
            coverage: pools.coverage,
            excluded: plan.excluded,
            analyzers: reports,
        })
    }
}
