#![forbid(unsafe_code)]
use analysis_api::Analyzer;
use app_config::{AppConfig, AppMode};
use domain::InstanceId;
use plugin_api::{ConfiguredPlugin, PluginCatalog, RegistryError};
use std::collections::BTreeMap;
use strategy_api::{Strategy, StrategyMode};
#[derive(Debug, thiserror::Error)]
pub enum BootstrapError {
    #[error(transparent)]
    Plan(#[from] strategy_engine::planner::PlanError),
    #[error(transparent)]
    Config(#[from] app_config::ConfigError),
    #[error(transparent)]
    Registry(#[from] RegistryError),
    #[error("invalid configuration for instance {0}")]
    PluginConfig(InstanceId),
    #[error("plugin {0} is in an incompatible configuration section")]
    Section(InstanceId),
    #[error("this build supports observe and signal modes only")]
    UnsupportedMode,
}
/// Offline construction: no credentials, sockets, workers or global registries.
pub struct Application {
    pub events: std::sync::Arc<monitor_runtime::events::EventRouter>,
    pub state: StatePlan,
    pub sinks: BTreeMap<InstanceId, std::sync::Arc<dyn notification_api::NotificationSink>>,
    pub delivery_targets: Vec<state_api::DeliveryTarget>,
    pub strategies: BTreeMap<InstanceId, Box<dyn Strategy>>,
    #[cfg(feature = "codex")]
    pub providers: BTreeMap<InstanceId, std::sync::Arc<provider_codex::CodexProvider>>,
    pub analyzers: BTreeMap<InstanceId, Box<dyn Analyzer>>,
    pub initialization_order: Vec<InstanceId>,
}
pub fn catalog() -> Result<PluginCatalog, RegistryError> {
    #[allow(unused_mut)]
    let mut catalog = PluginCatalog::default();
    #[cfg(feature = "range")]
    catalog.register(analyzer_range::RangeAnalyzer::descriptor_static())?;
    #[cfg(feature = "trend")]
    catalog.register(analyzer_trend::TrendAnalyzer::descriptor_static())?;
    #[cfg(feature = "volume")]
    catalog.register(analyzer_volume::VolumeAnalyzer::descriptor_static())?;
    #[cfg(feature = "codex")]
    catalog.register(provider_codex::descriptor())?;
    #[cfg(feature = "low-risk")]
    catalog.register(strategy_low_risk::descriptor())?;
    #[cfg(feature = "high-risk")]
    catalog.register(strategy_high_risk::descriptor())?;
    #[cfg(feature = "console")]
    catalog.register(notification_console::descriptor())?;
    #[cfg(feature = "telegram")]
    catalog.register(notification_telegram::descriptor())?;
    Ok(catalog)
}
pub fn build(config: &AppConfig) -> Result<Application, BootstrapError> {
    config.validate()?;
    if !matches!(config.mode, AppMode::Observe | AppMode::Signal) {
        return Err(BootstrapError::UnsupportedMode);
    }
    let catalog = catalog()?;
    #[allow(unused_mut)]
    let mut analyzers: BTreeMap<InstanceId, Box<dyn Analyzer>> = BTreeMap::new();
    #[cfg(feature = "codex")]
    let mut providers = BTreeMap::new();
    let mut state = None;
    let mut configured = Vec::new();
    let mut strategies = BTreeMap::new();
    let mut sinks = BTreeMap::new();
    let mut delivery_targets = Vec::new();
    for instance in config.instances().filter(|i| i.enabled) {
        let descriptor = catalog.get(&instance.plugin)?.clone();
        if descriptor.kind == plugin_api::PluginKind::NotificationSink {
            if !config
                .notifications
                .iter()
                .any(|entry| entry.id == instance.id)
            {
                return Err(BootstrapError::Section(instance.id.clone()));
            }
            let (sink, destination) = build_sink(instance)?;
            configured.push(ConfiguredPlugin {
                instance: instance.id.clone(),
                descriptor,
                config_digest: domain::ContentHash::of(&instance.config)
                    .map_err(|_| BootstrapError::PluginConfig(instance.id.clone()))?,
                requires: vec![],
            });
            delivery_targets.push(state_api::DeliveryTarget {
                sink: instance.id.clone(),
                destination,
            });
            sinks.insert(instance.id.clone(), sink);
            continue;
        }
        if descriptor.kind == plugin_api::PluginKind::Strategy {
            if !config
                .strategies
                .iter()
                .any(|entry| entry.id == instance.id)
            {
                return Err(BootstrapError::Section(instance.id.clone()));
            }
            let strategy = build_strategy(instance)?;
            let requires: Vec<_> = strategy
                .requirements()
                .analyses
                .into_iter()
                .map(|binding| plugin_api::Requirement {
                    binding: binding.name,
                    provider: binding.instance,
                    capability: plugin_api::Capability::pure(
                        domain::CapabilityId::new(binding.schema.as_str())
                            .expect("valid schema identifier"),
                        binding.major,
                    ),
                })
                .collect();
            configured.push(ConfiguredPlugin {
                instance: instance.id.clone(),
                descriptor,
                config_digest: strategy.config_digest().clone(),
                requires,
            });
            strategies.insert(instance.id.clone(), strategy);
            continue;
        }
        if !config.plugins.iter().any(|p| p.id == instance.id) {
            return Err(BootstrapError::Section(instance.id.clone()));
        }
        if descriptor.kind == plugin_api::PluginKind::StateStore {
            if state.is_some() {
                return Err(BootstrapError::PluginConfig(instance.id.clone()));
            }
            state = Some(build_state_plan(instance)?);
            configured.push(ConfiguredPlugin {
                instance: instance.id.clone(),
                descriptor,
                config_digest: domain::ContentHash::of(&instance.config)
                    .map_err(|_| BootstrapError::PluginConfig(instance.id.clone()))?,
                requires: vec![],
            });
            continue;
        }
        #[cfg(feature = "codex")]
        let invalid = || BootstrapError::PluginConfig(instance.id.clone());
        #[cfg(feature = "codex")]
        if instance.plugin.as_str() == "provider.codex" {
            let provider = provider_codex::CodexProvider::new(
                config.chain.clone(),
                instance.config.clone().try_into().map_err(|_| invalid())?,
            )
            .map_err(|_| invalid())?;
            configured.push(ConfiguredPlugin {
                instance: instance.id.clone(),
                descriptor,
                config_digest: domain::ContentHash::of(&instance.config).map_err(|_| invalid())?,
                requires: vec![],
            });
            providers.insert(instance.id.clone(), std::sync::Arc::new(provider));
            continue;
        }
        let analyzer = build_analyzer(instance)?;
        configured.push(ConfiguredPlugin {
            instance: instance.id.clone(),
            descriptor,
            config_digest: analyzer.config_digest().clone(),
            requires: vec![],
        });
        analyzers.insert(instance.id.clone(), analyzer);
    }
    if config.mode == AppMode::Signal && (sinks.is_empty() || strategies.is_empty()) {
        return Err(BootstrapError::Config(app_config::ConfigError::Validation(
            "signal mode requires enabled strategies and notification sinks".into(),
        )));
    }
    let initialization_order = plugin_api::validate_instances(&configured)?;
    for strategy in strategies.values() {
        strategy_engine::planner::plan(strategy.as_ref(), &analyzers)?;
    }
    Ok(Application {
        events: std::sync::Arc::new(monitor_runtime::events::EventRouter::default()),
        state: state.unwrap_or_default(),
        sinks,
        delivery_targets,
        strategies,
        analyzers,
        initialization_order,
        #[cfg(feature = "codex")]
        providers,
    })
}
fn build_analyzer(
    instance: &app_config::PluginInstance,
) -> Result<Box<dyn Analyzer>, BootstrapError> {
    #[allow(unused_variables)]
    let invalid = || BootstrapError::PluginConfig(instance.id.clone());
    match instance.plugin.as_str() {
        #[cfg(feature = "range")]
        "analyzer.range" => Ok(Box::new(
            analyzer_range::RangeAnalyzer::new(
                instance.id.clone(),
                instance.config.clone().try_into().map_err(|_| invalid())?,
            )
            .map_err(|_| invalid())?,
        )),
        #[cfg(feature = "volume")]
        "analyzer.volume" => Ok(Box::new(
            analyzer_volume::VolumeAnalyzer::new(
                instance.id.clone(),
                instance.config.clone().try_into().map_err(|_| invalid())?,
            )
            .map_err(|_| invalid())?,
        )),
        #[cfg(feature = "trend")]
        "analyzer.trend" => Ok(Box::new(
            analyzer_trend::TrendAnalyzer::new(
                instance.id.clone(),
                instance.config.clone().try_into().map_err(|_| invalid())?,
            )
            .map_err(|_| invalid())?,
        )),
        _ => Err(RegistryError::NotCompiled(instance.plugin.clone()).into()),
    }
}
fn build_strategy(
    instance: &app_config::PluginInstance,
) -> Result<Box<dyn Strategy>, BootstrapError> {
    let mode = match instance.mode {
        app_config::InstanceMode::Active => StrategyMode::Active,
        app_config::InstanceMode::Shadow => StrategyMode::Shadow,
    };
    let invalid = || BootstrapError::PluginConfig(instance.id.clone());
    match instance.plugin.as_str() {
        #[cfg(feature = "low-risk")]
        "strategy.range-low-risk" => Ok(Box::new(
            strategy_low_risk::build(
                instance.id.clone(),
                mode,
                instance.config.clone().try_into().map_err(|_| invalid())?,
            )
            .map_err(|_| invalid())?,
        )),
        #[cfg(feature = "high-risk")]
        "strategy.range-high-risk" => Ok(Box::new(
            strategy_high_risk::build(
                instance.id.clone(),
                mode,
                instance.config.clone().try_into().map_err(|_| invalid())?,
            )
            .map_err(|_| invalid())?,
        )),
        _ => {
            let _ = (mode, invalid);
            Err(RegistryError::NotCompiled(instance.plugin.clone()).into())
        }
    }
}
type BuiltSink = (
    std::sync::Arc<dyn notification_api::NotificationSink>,
    String,
);
fn build_sink(instance: &app_config::PluginInstance) -> Result<BuiltSink, BootstrapError> {
    let invalid = || BootstrapError::PluginConfig(instance.id.clone());
    match instance.plugin.as_str() {
        #[cfg(feature = "console")]
        "notification.console" => {
            let config = instance.config.clone().try_into().map_err(|_| invalid())?;
            Ok((
                std::sync::Arc::new(notification_console::ConsoleSink::new(
                    instance.id.clone(),
                    config,
                )),
                "stdout".into(),
            ))
        }
        #[cfg(feature = "telegram")]
        "notification.telegram" => {
            let config = instance.config.clone().try_into().map_err(|_| invalid())?;
            let sink = notification_telegram::TelegramSink::new(instance.id.clone(), config)
                .map_err(|_| invalid())?;
            let destination = sink.destination().to_owned();
            Ok((std::sync::Arc::new(sink), destination))
        }
        _ => {
            let _ = invalid;
            Err(RegistryError::NotCompiled(instance.plugin.clone()).into())
        }
    }
}
#[derive(Default)]
pub enum StatePlan {
    #[default]
    Memory,
}
impl StatePlan {
    /// Runtime startup only; offline build/config-check never opens a database.
    pub async fn open(&self) -> Result<Box<dyn state_api::StateStore>, state_api::StateError> {
        match self {
            Self::Memory => Ok(Box::new(
                state_memory::MemoryState::new(Default::default())?,
            )),
        }
    }
}
fn build_state_plan(instance: &app_config::PluginInstance) -> Result<StatePlan, BootstrapError> {
    Err(RegistryError::NotCompiled(instance.plugin.clone()).into())
}
impl Application {
    pub fn market<P: provider_api::MarketProvider + 'static>(
        &self,
        provider: std::sync::Arc<P>,
    ) -> Result<std::sync::Arc<dyn provider_api::MarketProvider>, provider_api::ProviderError> {
        Ok(provider)
    }
}
pub struct MonitorProvider {
    pub observations: Option<std::sync::Arc<dyn provider_api::TransportObservationReader>>,
    pub id: InstanceId,
    pub market: std::sync::Arc<dyn provider_api::MarketProvider>,
    pub source: std::sync::Arc<dyn discovery_api::CandidateSource>,
}
impl Application {
    /// Select a discovery-capable market provider, without touching network or credentials.
    pub fn take_monitor_provider(
        &mut self,
        selected: Option<&str>,
    ) -> Result<MonitorProvider, BootstrapError> {
        #[allow(unused_mut)]
        let mut ids = Vec::new();
        #[cfg(feature = "codex")]
        ids.extend(self.providers.keys().cloned());
        let id = match selected {
            Some(id) => InstanceId::new(id).map_err(|_| {
                BootstrapError::Config(app_config::ConfigError::Validation(
                    "invalid monitor provider ID".into(),
                ))
            })?,
            None if ids.len() == 1 => ids.pop().unwrap(),
            _ => {
                return Err(BootstrapError::Config(app_config::ConfigError::Validation(
                    "select an enabled monitor provider with --provider".into(),
                )));
            }
        };
        #[cfg(feature = "codex")]
        if let Some(provider) = self.providers.remove(&id) {
            let market = self
                .market(provider.clone())
                .map_err(|_| BootstrapError::PluginConfig(id.clone()))?;
            return Ok(MonitorProvider {
                id,
                market,
                source: provider.clone(),
                observations: Some(provider),
            });
        }
        Err(BootstrapError::PluginConfig(id))
    }
}
#[derive(serde::Serialize)]
#[serde(tag = "observation", rename_all = "snake_case")]
pub enum StateInspection {
    VolatileProcessStateUnavailable,
}
impl StatePlan {
    /// Read counters without claiming work, migration or exclusive ownership.
    pub async fn inspect(&self) -> Result<StateInspection, state_api::StateError> {
        match self {
            Self::Memory => Ok(StateInspection::VolatileProcessStateUnavailable),
        }
    }
}
