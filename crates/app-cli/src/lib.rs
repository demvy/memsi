#![forbid(unsafe_code)]
#[cfg(feature = "codex")]
mod live;
mod logging;
#[cfg(feature = "monitor")]
mod monitor;
mod status;
use clap::{CommandFactory, FromArgMatches, Parser, Subcommand};
use std::{collections::BTreeMap, path::PathBuf, process::ExitCode};
#[derive(Parser)]
#[command(
    version,
    about = "Compile-time plugins, Codex market data and token analysis"
)]
struct Cli {
    /// Mainnet: solana, robinhood, base, ethereum-mainnet (eth), arc-mainnet (arc).
    #[arg(long, global = true, value_name = "NETWORK")]
    network: Option<app_config::Network>,
    /// Emit JSON logs to stderr, with redacted errors on command failure.
    #[arg(long, global = true)]
    error_json: bool,
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Inspect configured identities offline.
    Status(status::StatusArgs),
    /// Discover, evaluate and deliver alerts.
    #[cfg(feature = "monitor")]
    Monitor(monitor::MonitorArgs),
    #[cfg(feature = "codex")]
    ListPools(live::MarketArgs),
    #[cfg(feature = "codex")]
    EvaluateToken(live::MarketArgs),
    #[cfg(feature = "codex")]
    AnalyzeToken(live::AnalyzeArgs),
    /// List only plugins compiled into this binary.
    Plugins {
        /// Emit the shared versioned HTTP catalog envelope.
        #[arg(long)]
        envelope: bool,
    },
    /// Read configured strategy identities/modes/digests offline.
    Strategies {
        #[arg(long, default_value = "memes-finder.toml")]
        config: PathBuf,
    },
    /// Validate and construct configured plugins without network or secrets.
    ConfigCheck {
        #[arg(long, default_value = "memes-finder.toml")]
        config: PathBuf,
    },
}

fn run(cli: Cli) -> Result<(), Box<dyn std::error::Error>> {
    if cli.network.is_some() {
        let supported = matches!(&cli.command, Command::ConfigCheck { .. });
        #[cfg(feature = "monitor")]
        let supported = supported || matches!(&cli.command, Command::Monitor(_));
        if !supported {
            return Err("--network is supported by monitor and config-check".into());
        }
    }

    match cli.command {
        Command::Status(args) => status::run(args)?,
        #[cfg(feature = "monitor")]
        Command::Monitor(args) => {
            runtime()?.block_on(monitor::run(args, cli.network, cli.error_json))?
        }
        #[cfg(feature = "codex")]
        Command::ListPools(args) => runtime()?.block_on(live::list(args))?,
        #[cfg(feature = "codex")]
        Command::EvaluateToken(args) => runtime()?.block_on(live::evaluate(args))?,
        #[cfg(feature = "codex")]
        Command::AnalyzeToken(args) => runtime()?.block_on(live::analyze(args))?,
        Command::Plugins { envelope } => {
            let plugins: Vec<_> = app_bootstrap::catalog()?.descriptors().cloned().collect();
            let output = if envelope {
                serde_json::to_string_pretty(&app_api::PluginCatalogEnvelope::new(plugins))?
            } else {
                serde_json::to_string_pretty(&plugins)?
            };

            println!("{output}");
        }

        Command::Strategies { config } => {
            let environment = std::env::vars()
                .filter(|(key, _)| key.starts_with("MEME__"))
                .collect();
            let config = app_config::AppConfig::load(&config, &environment, &BTreeMap::new())?;
            let app = app_bootstrap::build(&config)?;
            let strategies = app
                .strategies
                .values()
                .map(|s| app_api::StrategyInfo::from_strategy(s.as_ref()))
                .collect();
            println!(
                "{}",
                serde_json::to_string_pretty(&app_api::StrategyCatalogEnvelope::new(strategies))?
            );
        }

        Command::ConfigCheck { config } => {
            let environment: BTreeMap<_, _> = std::env::vars()
                .filter(|(key, _)| key.starts_with("MEME__"))
                .collect();
            let mut config = app_config::AppConfig::load(&config, &environment, &BTreeMap::new())?;
            if let Some(network) = cli.network {
                config.select_network(network)?;
            }

            let app = app_bootstrap::build(&config)?;
            if let Some(network) = cli.network {
                println!(
                    "Network: {network}; Codex network ID: {}",
                    network.codex_id()
                );
            }

            println!(
                "Configuration valid: {} enabled plugin instances (offline).",
                app.initialization_order.len()
            );
        }
    }

    Ok(())
}

#[cfg(feature = "monitor")]
fn runtime() -> Result<tokio::runtime::Runtime, std::io::Error> {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
}

/// Run the shared CLI with the public executable name used in help output.
pub fn entrypoint(binary_name: &'static str) -> ExitCode {
    // Parsing can fail before clap produces Cli, so detect the output mode first.
    let error_json = std::env::args_os().any(|argument| argument == "--error-json");
    logging::init(error_json);

    let parsed = Cli::command()
        .name(binary_name)
        .bin_name(binary_name)
        .try_get_matches()
        .and_then(|matches| Cli::from_arg_matches(&matches));
    let cli = match parsed {
        Ok(cli) => cli,
        Err(error) => {
            if error.exit_code() == 0 {
                let _ = error.print();
                return ExitCode::SUCCESS;
            }

            if error_json {
                print_json_error("invalid_request", "invalid command arguments");
            } else {
                let _ = error.print();
            }

            return ExitCode::FAILURE;
        }
    };

    match run(cli) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            if error_json {
                print_json_error("command_failed", "command failed");
            } else {
                tracing::error!(error = %error, "command failed");
            }

            ExitCode::FAILURE
        }
    }
}

fn print_json_error(code: &'static str, message: &'static str) {
    tracing::error!(
        code,
        message,
        request_id = %format!("cli-{}", std::process::id()),
    );
}

#[cfg(test)]
mod tests;
