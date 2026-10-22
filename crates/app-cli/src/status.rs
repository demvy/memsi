use clap::Args;
use std::{collections::BTreeMap, path::PathBuf};
#[derive(Args)]
pub struct StatusArgs {
    #[arg(long, default_value = "memes-finder.toml")]
    config: PathBuf,
}

pub fn run(args: StatusArgs) -> Result<(), Box<dyn std::error::Error>> {
    let environment =
        BTreeMap::from_iter(std::env::vars().filter(|(key, _)| key.starts_with("MEME__")));
    let config = app_config::AppConfig::load(&args.config, &environment, &BTreeMap::new())?;
    app_bootstrap::build(&config)?;
    let state = app_bootstrap::StateInspection::VolatileProcessStateUnavailable;
    let instances: Vec<_> = config
        .instances()
        .filter(|i| i.enabled)
        .map(|i| {
            Ok(app_api::EnabledInstance {
                instance: i.id.clone(),
                plugin: i.plugin.clone(),
                mode: serde_json::to_value(i.mode)?
                    .as_str()
                    .ok_or("invalid instance mode")?
                    .into(),
            })
        })
        .collect::<Result<Vec<_>, Box<dyn std::error::Error>>>()?;
    let status: app_api::StatusEnvelope<_> = app_api::StatusEnvelope::new(
        config.digest(),
        config.chain.clone(),
        serde_json::to_value(config.mode)?
            .as_str()
            .ok_or("invalid app mode")?
            .into(),
        instances,
        state,
    );
    println!("{}", serde_json::to_string_pretty(&status)?);
    Ok(())
}
