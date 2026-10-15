use crate::shutdown::shutdown_signal;
use clap::Args;
use provider_api::Cancellation;
use std::{collections::BTreeMap, path::PathBuf, time::Duration};
#[derive(Args)]
pub struct MonitorArgs {
    /// Full structured per-candidate analysis diagnostics.
    #[arg(long)]
    pub debug: bool,
    #[arg(long, default_value = "memes-finder.toml")]
    pub config: PathBuf,
    /// Override settings, e.g. --set monitor.max_candidates=1. CLI wins over env.
    #[arg(long = "set", value_name = "KEY=VALUE")]
    pub overrides: Vec<String>,
    #[arg(long)]
    pub provider: Option<String>,
    /// Run one bounded discovery/evaluation/delivery cycle.
    #[arg(long)]
    pub once: bool,
}
pub async fn run(
    args: MonitorArgs,
    network: Option<app_config::Network>,
) -> Result<(), Box<dyn std::error::Error>> {
    super::startup::log(&args.config);
    let environment =
        BTreeMap::from_iter(std::env::vars().filter(|(key, _)| key.starts_with("MEME__")));
    let mut overrides = BTreeMap::new();
    for item in &args.overrides {
        let (key, value) = item.split_once('=').ok_or("expected --set KEY=VALUE")?;
        overrides.insert(
            format!("MEME__{}", key.replace('.', "__").to_ascii_uppercase()),
            value.into(),
        );
    }
    let mut config = app_config::AppConfig::load(&args.config, &environment, &overrides)?;
    if let Some(network) = network {
        config.select_network(network)?;
    }
    eprintln!(
        "{}",
        serde_json::json!({ "event" : "monitor_config_loaded",
        "discovery_interval_secs" : config.monitor.poll_interval_secs,
        "request_deadline_secs" : config.monitor.candidate_deadline_secs, "chain" :
        config.chain, })
    );
    let mut app = app_bootstrap::build(&config)?;
    #[allow(unused_mut)]
    let mut selected = app.take_monitor_provider(args.provider.as_deref())?;
    let provider_observer = selected
        .observations
        .clone()
        .map(|reader| (selected.id.clone(), reader));
    let services = app_services::Services::new(selected.market)?;
    eprintln!("data_origin=codex");
    let state: std::sync::Arc<dyn state_api::StateStore> =
        std::sync::Arc::from(app.state.open().await?);
    let clock = signal_service::SystemClock;
    let dependencies = monitor_runtime::Dependencies {
        config: &config,
        services: &services,
        source: selected.source.as_ref(),
        source_instance: selected.id,
        strategies: &app.strategies,
        analyzers: &app.analyzers,
        state: state.as_ref(),
        sinks: &app.sinks,
        targets: &app.delivery_targets,
        clock: &clock,
    };
    let mut monitor = monitor_runtime::Monitor::new(dependencies)?
        .with_events(app.events.clone())
        .with_diagnostics(std::sync::Arc::new(crate::discovery_logging::Console::new(
            args.debug, &config,
        )));
    let cancellation = Cancellation::default();
    eprintln!(
        "state_durability={:?} mode={:?}",
        state.durability(),
        config.mode
    );
    {
        let market_work = Box::pin(async {
            market(
                &mut monitor,
                &cancellation,
                args.once,
                provider_observer
                    .as_ref()
                    .map(|(_, reader)| reader.as_ref()),
            )
            .await
        });
        let market_work = crate::quota::supervise(
            market_work,
            provider_observer
                .as_ref()
                .map(|(_, reader)| reader.as_ref()),
            &app.sinks,
            &cancellation,
        );
        let work = market_work;
        tokio::pin!(work);
        tokio::select! {
            biased; interrupted = shutdown_signal() => { interrupted ?; cancellation
            .cancel();
            eprintln!("shutdown requested: settling in-flight work (up to 15 seconds)");
            tokio::select! { result = tokio::time::timeout(Duration::from_secs(15), & mut
            work) => { match result { Ok(result) => result ?, Err(_) => return
            Err("shutdown deadline exceeded; in-flight delivery outcomes may be unknown"
            .into()), } } interrupted = shutdown_signal() => { interrupted ?; return
            Err("forced shutdown; in-flight delivery outcome may be unknown".into()); } }
            } result = & mut work => result ?,
        }
    }
    let stats = state.stats().await?;
    if stats.durability == "volatile" && stats.pending > 0 {
        eprintln!(
            "warning: {} pending deliveries will be lost with volatile state",
            stats.pending
        );
    }
    if let Some((instance, reader)) = &provider_observer
        && let Some(transport) = reader.transport_observations()
    {
        eprintln!(
            "transport {}",
            serde_json::to_string(&serde_json::json!({ "provider" :
            instance, "observations" : transport }))?
        );
    }
    eprintln!("shutdown {}", serde_json::to_string(&stats)?);
    Ok(())
}
async fn market(
    monitor: &mut monitor_runtime::Monitor<'_>,
    cancellation: &Cancellation,
    once: bool,
    observer: Option<&dyn provider_api::TransportObservationReader>,
) -> Result<(), Box<dyn std::error::Error>> {
    let mut requests = crate::request_logging::Counter::new(observer);
    if once {
        let report = monitor.cycle(cancellation).await?;
        requests.log();
        eprintln!("{}", serde_json::to_string(&report)?);
    } else {
        monitor
            .run(cancellation, |report| {
                if report.discovered > 0 || report.evaluated > 0 || !report.errors.is_empty() {
                    requests.log();
                    match serde_json::to_string(report) {
                        Ok(json) => eprintln!("{json}"),
                        Err(error) => {
                            eprintln!("cycle report serialization failed: {error}")
                        }
                    }
                }
            })
            .await?;
    }
    Ok(())
}
