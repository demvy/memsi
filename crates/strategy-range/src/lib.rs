#![forbid(unsafe_code)]
mod config;
mod filters;
mod pump;
pub use pump::PumpSignal;
mod rules;
use analysis_api::{AnalysisBinding, CandleRequirement};
pub use config::{
    Profile, ResearchPolicy, StrategyConfig, ThresholdOverrides, Thresholds, TrendBinding,
};
use domain::*;
use plugin_api::{PluginDescriptor, PluginKind};
use strategy_api::*;
pub struct RangeStrategy {
    thresholds: Thresholds,
    instance: InstanceId,
    mode: StrategyMode,
    profile: Profile,
    config: StrategyConfig,
    digest: ContentHash,
}
pub fn descriptor(profile: Profile) -> PluginDescriptor {
    PluginDescriptor {
        id: PluginId::new(profile.plugin_id()).unwrap(),
        version: env!("CARGO_PKG_VERSION").into(),
        kind: PluginKind::Strategy,
        provides: vec![],
    }
}
impl RangeStrategy {
    pub fn new(
        instance: InstanceId,
        mode: StrategyMode,
        profile: Profile,
        mut config: StrategyConfig,
    ) -> Result<Self, StrategyError> {
        let thresholds = config
            .thresholds
            .as_ref()
            .map_or(profile.thresholds(), |v| v.apply(profile.thresholds()));
        thresholds.validate()?;
        if config.metrics_max_age_secs == 0 || config.metrics_max_age_secs > 86400 {
            return Err(StrategyError::InvalidResult);
        }
        for (metric, bounds) in &config.filters {
            bounds
                .validate(*metric)
                .map_err(|_| StrategyError::InvalidResult)?;
        }
        if let Some(signal) = &config.signal {
            if config.research.is_some() {
                return Err(StrategyError::InvalidResult);
            }
            signal.validate()?;
        }
        if let Some(policy) = &config.research {
            policy.validate()?;
        }
        config.quote_tokens.sort();
        config.quote_tokens.dedup();
        config.protocols.sort();
        config.protocols.dedup();
        if !config.research_only && (config.quote_tokens.is_empty() || config.protocols.is_empty())
        {
            return Err(StrategyError::MissingInput);
        }
        let digest = ContentHash::of(&(profile, thresholds, &config))
            .map_err(|_| StrategyError::InvalidResult)?;
        Ok(Self {
            thresholds,
            instance,
            mode,
            profile,
            config,
            digest,
        })
    }
    fn binding(name: &str, instance: &InstanceId, schema: &str) -> AnalysisBinding {
        AnalysisBinding {
            name: BindingId::new(name).unwrap(),
            instance: instance.clone(),
            schema: SchemaId::new(schema).unwrap(),
            major: 1,
            required: true,
        }
    }
}
impl Strategy for RangeStrategy {
    fn descriptor(&self) -> PluginDescriptor {
        descriptor(self.profile)
    }
    fn instance(&self) -> &InstanceId {
        &self.instance
    }
    fn mode(&self) -> StrategyMode {
        self.mode
    }
    fn config_digest(&self) -> &ContentHash {
        &self.digest
    }
    fn prefilter(&self, candidate: &Candidate, _: TimestampMs) -> PrefilterDecision {
        if self.config.research.as_ref().is_some_and(|policy| {
            !policy.allowed_chains.is_empty()
                && !policy.allowed_chains.contains(candidate.token.chain())
        }) {
            return PrefilterDecision::Reject(vec![DecisionReason {
                code: "CHAIN_NOT_ALLOWED".into(),
                field: "candidate.chain".into(),
                actual: None,
                expected: "configured chain".into(),
            }]);
        }
        PrefilterDecision::Continue
    }
    fn requirements(&self) -> StrategyRequirements {
        let thresholds = self.thresholds;
        let mut analyses = vec![
            Self::binding("range", &self.config.range_analyzer, "analysis.range"),
            Self::binding("volume", &self.config.volume_analyzer, "analysis.volume"),
        ];
        if let Some(trend) = &self.config.trend {
            let mut binding = Self::binding("trend", &trend.analyzer, "analysis.trend");
            binding.required = trend.required;
            analyses.push(binding);
        }
        StrategyRequirements {
            pool: PoolSelectionPolicy {
                concentrated_only: !self.config.research_only,
                min_liquidity_usd: NonNegative::new(thresholds.liquidity_usd).unwrap(),
                min_pool_age_secs: thresholds.age_secs,
                quote_tokens: self.config.quote_tokens.clone(),
                max_probes: 5,
            },
            candles: CandleRequirement {
                resolution: Resolution::new(thresholds.resolution_secs).unwrap(),
                min_bars: self
                    .config
                    .signal
                    .as_ref()
                    .map_or(270, |s| s.required_bars().max(20)),
                desired_bars: self
                    .config
                    .signal
                    .as_ref()
                    .map_or(300, |s| s.required_bars().max(s.context_bars)),
                price_unit: PriceUnit::UsdPerToken,
                require_volume: true,
            },
            analyses,
        }
    }
    fn discovery_token_filters(&self) -> std::collections::BTreeMap<TokenMetric, MetricBounds> {
        self.config.filters.clone()
    }
    fn uses_token_metrics(&self) -> bool {
        self.config
            .filters
            .keys()
            .any(|m| *m != TokenMetric::PoolLiquidityUsd)
    }
    fn required_token_metrics(&self) -> Option<Vec<TokenMetric>> {
        Some(
            self.config
                .filters
                .keys()
                .copied()
                .filter(|m| *m != TokenMetric::PoolLiquidityUsd)
                .chain([TokenMetric::ProviderSeenAgeSecs])
                .collect(),
        )
    }
    fn screen_token_metrics(&self, candidate: &Candidate, now: TimestampMs) -> MetricScreen {
        filters::screen(candidate, now, &self.config)
    }
    fn token_filter_recheck_secs(&self) -> u64 {
        match self.profile {
            Profile::High => 900,
            Profile::Medium => 1200,
            Profile::Low => 1800,
        }
    }
    fn next_market_check(&self, now: TimestampMs, close_grace_ms: u64) -> Option<TimestampMs> {
        self.config.signal.as_ref()?;
        let interval = i64::try_from(self.thresholds.resolution_secs)
            .ok()?
            .checked_mul(1000)?;
        let grace = i64::try_from(close_grace_ms).ok()?;
        let closed = now.get().checked_sub(grace)?.div_euclid(interval);
        TimestampMs::new((closed + 1).checked_mul(interval)?.checked_add(grace)?).ok()
    }
    fn evaluate(&self, context: &StrategyContext<'_>) -> Result<StrategyDecision, StrategyError> {
        if context.pool.validate().is_err() || !context.pool.contains(&context.candidate.token) {
            return Err(StrategyError::InvalidResult);
        }
        if context.trend.is_some_and(|trend| {
            !trend.slope_log_per_bar.is_finite()
                || !(2..=1500).contains(&trend.window)
                || trend.resolution.seconds() != self.thresholds.resolution_secs
                || trend.price_unit != PriceUnit::UsdPerToken
        }) {
            return Err(StrategyError::InvalidResult);
        }
        let (verdict, reasons) = rules::evaluate(context, &self.config, self.thresholds);
        let after_secs = match (self.profile, verdict) {
            (_, Verdict::Unavailable) => 60,
            (Profile::Low, Verdict::Reject) => 1800,
            (Profile::Low, Verdict::Watch) => 120,
            (Profile::Low, Verdict::Match) => 300,
            (Profile::Medium, Verdict::Reject) => 1200,
            (Profile::Medium, Verdict::Watch) => 120,
            (Profile::Medium, Verdict::Match) => 300,
            (Profile::High, Verdict::Reject) => 900,
            (Profile::High, Verdict::Watch) => 60,
            (Profile::High, Verdict::Match) => 120,
        };
        let mut warnings = Vec::new();
        if self.config.trend.is_some() && context.trend.is_none() {
            warnings.push("TrendUnavailable".into());
        }
        if self.config.research_only {
            warnings.push("ResearchOnly".into());
        }
        Ok(StrategyDecision {
            instance: self.instance.clone(),
            plugin: PluginId::new(self.profile.plugin_id()).unwrap(),
            version: env!("CARGO_PKG_VERSION").into(),
            config_digest: self.digest.clone(),
            mode: self.mode,
            token: context.candidate.token.clone(),
            pool: Some(context.pool.pool.clone()),
            as_of: context.as_of,
            verdict,
            reasons,
            warnings,
            evidence: DecisionEvidence {
                token_metrics: (!context.candidate.metrics.is_empty())
                    .then(|| context.candidate.metrics.clone()),
                market: (self.config.research.is_some() || self.config.signal.is_some())
                    .then(|| Box::new(context.pool.clone())),
                candle_input_hash: None,
                cl_verification: context.pool.verification.clone(),
                input_snapshot_hash: Some(context.input_snapshot_hash.clone()),
                snapshot_hash: context.snapshot_hash.clone(),
                range: context.range.cloned(),
                volume: context.volume.cloned(),
                trend: context.trend.cloned(),
                cl_verified: context.pool.concentrated_at(context.as_of),
            },
            score: context.range.map(|range| range.confidence),
            watch: WatchDirective {
                after_secs,
                priority: 1,
            },
        })
    }
}
