use clap::Args;
use domain::*;
use provider_api::*;
use std::{
    collections::BTreeMap,
    path::PathBuf,
    time::{Duration, Instant},
};
#[derive(Args)]
pub struct MarketArgs {
    #[arg(long, default_value = "memes-finder.toml")]
    pub config: PathBuf,
    #[arg(long)]
    pub token: String,
    /// Configured provider instance (required when more than one is enabled).
    #[arg(long)]
    pub provider: Option<String>,
}
#[derive(Args)]
pub struct AnalyzeArgs {
    #[command(flatten)]
    pub market: MarketArgs,
    #[arg(long)]
    pub pool: Option<String>,
    #[arg(long, default_value_t = 300)]
    pub resolution_secs: u64,
    /// Analyzer instance IDs; repeat for several. Defaults to all enabled analyzers.
    #[arg(long)]
    pub analyzer: Vec<String>,
    /// Require independently verified concentrated liquidity pools.
    #[arg(long)]
    pub require_cl: bool,
}
struct LoadedApp {
    config: app_config::AppConfig,
    app: app_bootstrap::Application,
    token: TokenId,
    service: app_services::Services,
}
pub(super) struct ConfiguredMarket {
    pub config: app_config::AppConfig,
    pub app: app_bootstrap::Application,
    pub token: TokenId,
    pub provider_id: InstanceId,
}
pub(super) fn configure(args: &MarketArgs) -> Result<ConfiguredMarket, Box<dyn std::error::Error>> {
    let (config, app, id) = configure_provider(&args.config, args.provider.as_deref())?;
    let token = TokenId::new(config.chain.clone(), &args.token)?;
    Ok(ConfiguredMarket {
        config,
        app,
        token,
        provider_id: id,
    })
}
pub(super) fn configure_provider(
    path: &std::path::Path,
    selected: Option<&str>,
) -> Result<
    (
        app_config::AppConfig,
        app_bootstrap::Application,
        InstanceId,
    ),
    Box<dyn std::error::Error>,
> {
    let env = BTreeMap::from_iter(std::env::vars().filter(|(key, _)| key.starts_with("MEME__")));
    let config = app_config::AppConfig::load(path, &env, &BTreeMap::new())?;
    let app = app_bootstrap::build(&config)?;
    let id = match selected {
        Some(id) => InstanceId::new(id)?,
        None if app.providers.len() == 1 => app.providers.keys().next().unwrap().clone(),
        _ => return Err("select an enabled provider instance with --provider".into()),
    };
    if !app.providers.contains_key(&id) {
        return Err("provider instance is not enabled".into());
    }
    Ok((config, app, id))
}
fn load(args: &MarketArgs) -> Result<LoadedApp, Box<dyn std::error::Error>> {
    let ConfiguredMarket {
        config,
        mut app,
        token,
        provider_id: id,
    } = configure(args)?;
    let provider = app
        .providers
        .remove(&id)
        .ok_or("provider instance is not enabled")?;
    let service = app_services::Services::new(app.market(provider)?)?;
    Ok(LoadedApp {
        config,
        app,
        token,
        service,
    })
}
pub(super) fn context(config: &app_config::AppConfig) -> RequestContext {
    RequestContext {
        priority: provider_api::RequestPriority::default(),
        deadline: Instant::now() + Duration::from_secs(config.monitor.candidate_deadline_secs),
        cancellation: Cancellation::default(),
    }
}
pub async fn list(args: MarketArgs) -> Result<(), Box<dyn std::error::Error>> {
    let LoadedApp {
        config,
        token,
        service,
        ..
    } = load(&args)?;
    let result = service.list_pools(&token, &context(&config)).await?;
    println!("{}", serde_json::to_string_pretty(&result)?);
    Ok(())
}
pub async fn analyze(args: AnalyzeArgs) -> Result<(), Box<dyn std::error::Error>> {
    let LoadedApp {
        config,
        app,
        token,
        service,
    } = load(&args.market)?;
    let request = app_api::AnalysisJobRequest {
        pool: args
            .pool
            .map(|address| PoolId::address(config.chain.clone(), &address))
            .transpose()?,
        token,
        resolution: Resolution::new(args.resolution_secs)?,
        analyzer_instances: args
            .analyzer
            .into_iter()
            .map(InstanceId::new)
            .collect::<Result<_, _>>()?,
        require_cl: args.require_cl,
        notify: false,
    };
    let request = request.analyze_request(
        &config.chain,
        &app.analyzers.keys().cloned().collect(),
        config.monitor.close_grace_ms,
    )?;
    let result = service
        .analyze(&request, &app.analyzers, &context(&config))
        .await?;
    println!("{}", serde_json::to_string_pretty(&result)?);
    Ok(())
}
pub fn runtime() -> Result<tokio::runtime::Runtime, std::io::Error> {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
}
/// Evaluate all configured strategies; each strategy owns its timeframe and filters.
pub async fn evaluate(args: MarketArgs) -> Result<(), Box<dyn std::error::Error>> {
    let LoadedApp {
        config,
        app,
        token,
        service,
    } = load(&args)?;
    let report = service
        .evaluate_strategies(
            &token,
            &app.strategies,
            &app.analyzers,
            config.monitor.close_grace_ms,
            &context(&config),
        )
        .await?;
    println!("{}", serde_json::to_string_pretty(&report)?);
    Ok(())
}
