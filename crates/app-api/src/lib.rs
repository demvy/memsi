#![forbid(unsafe_code)]
use domain::PluginId;
use domain::{ChainId, ContentHash, InstanceId};
use serde::Serialize;
/// Config identity only; deliberately excludes arbitrary instance config/credentials.
#[derive(Clone, Debug, Serialize)]
pub struct EnabledInstance {
    pub instance: InstanceId,
    pub plugin: PluginId,
    pub mode: String,
}

/// Shared CLI/HTTP shape. State and activity retain their explicit observation contracts.
#[derive(Serialize)]
pub struct StatusEnvelope<S, A> {
    pub schema_version: u16,
    pub config_digest: ContentHash,
    pub chain: ChainId,
    pub mode: String,
    pub enabled_instances: Vec<EnabledInstance>,
    pub provider_health: &'static str,
    pub runtime_status: &'static str,
    pub runtime: Option<event_api::RuntimeSnapshot>,
    pub runtime_activity: Option<A>,
    pub analysis_jobs: Option<AnalysisJobStats>,
    pub provider_transport: Option<ProviderTransportObservation>,
    pub state: S,
}

impl<S, A> StatusEnvelope<S, A> {
    pub fn new(
        config_digest: ContentHash,
        chain: ChainId,
        mode: String,
        enabled_instances: Vec<EnabledInstance>,
        state: S,
        runtime: Option<event_api::RuntimeSnapshot>,
        runtime_activity: Option<A>,
    ) -> Self {
        Self {
            schema_version: 1,
            config_digest,
            chain,
            mode,
            enabled_instances,
            provider_health: "not_observed",
            runtime_status: if runtime.is_some() {
                "observed"
            } else {
                "not_observed"
            },
            runtime,
            runtime_activity,
            analysis_jobs: None,
            provider_transport: None,
            state,
        }
    }

    pub fn with_provider_transport(
        mut self,
        observation: Option<ProviderTransportObservation>,
    ) -> Self {
        self.provider_transport = observation;
        self
    }

    pub fn with_analysis_jobs(mut self, jobs: Option<AnalysisJobStats>) -> Self {
        self.analysis_jobs = jobs;
        self
    }
}

/// Compiled catalog only, not a claim that every plugin is enabled or healthy.
#[derive(Serialize)]
pub struct PluginCatalogEnvelope {
    pub schema_version: u16,
    pub plugins: Vec<plugin_api::PluginDescriptor>,
}

impl PluginCatalogEnvelope {
    pub fn new(plugins: Vec<plugin_api::PluginDescriptor>) -> Self {
        Self {
            schema_version: 1,
            plugins,
        }
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct StrategyInfo {
    pub instance: InstanceId,
    pub plugin: PluginId,
    pub version: String,
    pub mode: strategy_api::StrategyMode,
    pub config_digest: ContentHash,
}

impl StrategyInfo {
    pub fn from_strategy(strategy: &dyn strategy_api::Strategy) -> Self {
        let descriptor = strategy.descriptor();
        Self {
            instance: strategy.instance().clone(),
            plugin: descriptor.id,
            version: descriptor.version,
            mode: strategy.mode(),
            config_digest: strategy.config_digest().clone(),
        }
    }
}

#[derive(Serialize)]
pub struct StrategyCatalogEnvelope {
    pub schema_version: u16,
    pub strategies: Vec<StrategyInfo>,
}

impl StrategyCatalogEnvelope {
    pub fn new(strategies: Vec<StrategyInfo>) -> Self {
        Self {
            schema_version: 1,
            strategies,
        }
    }
}

/// Static public failures only. Raw upstream/config/OS error strings do not belong here.
#[derive(Serialize)]
pub struct ErrorEnvelope {
    pub code: &'static str,
    pub message: &'static str,
    pub request_id: String,
    pub details: Option<()>,
}

impl ErrorEnvelope {
    pub fn new(code: &'static str, message: &'static str, request_id: String) -> Self {
        Self {
            code,
            message,
            request_id,
            details: None,
        }
    }
}

/// Bounded request for read-side analysis. Notifications need a separately authorized lane.
#[derive(Clone, Debug, serde::Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AnalysisJobRequest {
    pub token: domain::TokenId,
    #[serde(default)]
    pub pool: Option<domain::PoolId>,
    #[serde(default = "analysis_resolution")]
    pub resolution: domain::Resolution,
    #[serde(default)]
    pub analyzer_instances: Vec<InstanceId>,
    #[serde(default)]
    pub require_cl: bool,
    #[serde(default)]
    pub notify: bool,
}

fn analysis_resolution() -> domain::Resolution {
    domain::Resolution::new(300).expect("constant resolution")
}

impl AnalysisJobRequest {
    /// Validate host identity/capabilities before admitting work or fetching provider data.
    pub fn analyze_request(
        &self,
        chain: &ChainId,
        enabled: &std::collections::BTreeSet<InstanceId>,
        close_grace_ms: u64,
    ) -> Result<app_services::AnalyzeRequest, &'static str> {
        if self.token.chain() != chain
            || self.pool.as_ref().is_some_and(|pool| pool.chain() != chain)
        {
            return Err("analysis chain mismatch");
        }

        if self.notify {
            return Err("analysis notifications are not authorized");
        }

        if close_grace_ms > 60000
            || enabled.is_empty()
            || enabled.len() > 64
            || self.analyzer_instances.len() > 64
        {
            return Err("invalid analysis bounds");
        }

        let mut seen = std::collections::BTreeSet::new();
        for id in &self.analyzer_instances {
            if !enabled.contains(id) || !seen.insert(id) {
                return Err("unknown or duplicate analyzer instance");
            }
        }

        let mut policy = data_hub::selection::SelectionPolicy::default();
        if self.require_cl {
            policy.purpose = data_hub::selection::Purpose::ConcentratedLp;
        }

        Ok(app_services::AnalyzeRequest {
            token: self.token.clone(),
            pool: self.pool.clone(),
            resolution: self.resolution,
            analyzer_instances: self.analyzer_instances.clone(),
            policy,
            close_grace_ms,
        })
    }
}

/// One locked registry observation, not provider/worker health or durable history.
#[derive(Clone, Debug, Serialize)]
pub struct AnalysisJobStats {
    pub schema_version: u16,
    pub closed: bool,
    pub capacity: usize,
    pub worker_capacity: usize,
    pub pending: usize,
    pub running: usize,
    pub completed: usize,
    pub failed: usize,
    pub admission_available: bool,
}

#[derive(Serialize)]
pub struct ProviderTransportObservation {
    pub instance: InstanceId,
    pub counters: provider_api::TransportObservations,
}

#[cfg(test)]
mod tests;
