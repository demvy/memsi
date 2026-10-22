use super::*;
use std::{collections::BTreeMap, path::Path};

fn preset() -> AppConfig {
    AppConfig::parse(include_str!("research-preset.toml")).unwrap()
}

#[test]
fn preset_defaults_round_trip_and_digest_are_stable() {
    let config = preset();
    assert!(config.validate().is_ok());
    let encoded = toml::to_string(&config).unwrap();
    let decoded = AppConfig::parse(&encoded).unwrap();
    assert_eq!(decoded.digest(), config.digest());
    assert_eq!(
        config.instances().count(),
        config.plugins.len() + config.strategies.len() + config.notifications.len()
    );
    let defaults = MonitorConfig::default();
    assert_eq!(defaults.candidate_workers, 8);
    assert_eq!(defaults.min_recheck_secs, 0);
    assert_eq!(InstanceMode::default(), InstanceMode::Active);
    assert!(
        !toml::to_string(&defaults)
            .unwrap()
            .contains("min_recheck_secs")
    );
    assert!(!encoded.contains("logging = true"));
}

#[test]
fn configuration_rejects_unknown_fields_versions_chains_and_duplicate_instances() {
    assert!(matches!(
        AppConfig::parse("not valid TOML"),
        Err(ConfigError::Parse)
    ));
    assert!(
        AppConfig::parse(&format!(
            "unknown = true\n{}",
            include_str!("research-preset.toml")
        ))
        .is_err()
    );
    for mode in ["paper", "live"] {
        let input = toml::to_string(&preset())
            .unwrap()
            .replace("mode = \"signal\"", &format!("mode = \"{mode}\""));
        assert!(AppConfig::parse(&input).is_err());
    }

    let mut config = preset();
    config.schema_version = 2;
    assert!(config.validate().is_err());
    config = preset();
    config.chain = domain::ChainId::Evm { chain_id: 0 };
    assert!(config.validate().is_err());
    config = preset();
    config.notifications.push(config.plugins[0].clone());
    assert!(config.validate().is_err());
}

#[test]
fn monitor_resource_limits_are_checked_independently() {
    macro_rules! invalid {
        ($($field:ident: [$($value:expr),+]),+ $(,)?) => {$(
            $(
                let mut config = preset();
                config.monitor.$field = $value;
                assert!(config.validate().is_err(), "{}={}", stringify!($field), $value);
            )+
        )+};
    }

    invalid!(
        watchlist_retention_minutes: [0, 10081],
        poll_interval_secs: [0, 86401],
        min_recheck_secs: [86401],
        candidate_workers: [0, 65],
        queue_capacity: [0, 10001],
        candidate_deadline_secs: [0, 301],
        max_candidates: [0, 1001],
        close_grace_ms: [60001],
    );
}

#[test]
fn all_network_names_aliases_ids_and_chains_match() {
    for (network, name, id, aliases) in [
        (
            Network::Solana,
            "solana",
            1399811149,
            vec!["sol", "solana-mainnet"],
        ),
        (
            Network::Robinhood,
            "robinhood",
            4663,
            vec!["robinhood-mainnet"],
        ),
        (Network::Base, "base", 8453, vec!["base-mainnet"]),
        (
            Network::EthereumMainnet,
            "ethereum-mainnet",
            1,
            vec!["eth", "ethereum", " Ethereum_Mainnet "],
        ),
        (
            Network::ArcMainnet,
            "arc-mainnet",
            5042,
            vec!["arc", "ARC MAINNET"],
        ),
    ] {
        assert_eq!(network.name(), name);
        assert_eq!(network.to_string(), name);
        assert_eq!(network.codex_id(), id);
        assert_eq!(name.parse::<Network>().unwrap(), network);
        for alias in aliases {
            assert_eq!(alias.parse::<Network>().unwrap(), network);
        }
        assert!(network.chain().validate().is_ok());
        if network != Network::Solana {
            assert_eq!(
                network.chain(),
                domain::ChainId::Evm {
                    chain_id: id as u64
                }
            );
        }
    }

    assert!("unknown".parse::<Network>().is_err());
}

#[test]
fn network_selection_updates_codex_and_is_atomic_on_validation_errors() {
    let mut config = preset();
    config.select_network(Network::Base).unwrap();
    assert_eq!(config.chain, Network::Base.chain());
    let provider = config
        .plugins
        .iter_mut()
        .find(|plugin| plugin.plugin.as_str() == "provider.codex")
        .unwrap();
    assert_eq!(provider.config["network_id"].as_integer(), Some(8453));
    provider.enabled = false;
    let before = config.digest();
    assert!(config.select_network(Network::Solana).is_err());
    assert_eq!(before, config.digest());

    config = preset();
    let mut other = config
        .plugins
        .iter()
        .find(|plugin| plugin.plugin.as_str() == "provider.codex")
        .unwrap()
        .clone();
    other.id = domain::InstanceId::new("other").unwrap();
    other.plugin = domain::PluginId::new("provider.other").unwrap();
    config.plugins.push(other);
    let before = config.digest();
    assert!(config.select_network(Network::Base).is_err());
    assert_eq!(before, config.digest());
    config.monitor.candidate_workers = 0;
    let before = config.digest();
    assert!(config.select_network(Network::Solana).is_err());
    assert_eq!(before, config.digest());
}

#[test]
fn strategy_logging_defaults_to_enabled_for_unknown_instances() {
    let mut config = preset();
    let instance = config.strategies[0].id.clone();
    assert!(config.strategy_logging(&instance));
    config.strategies[0].logging = false;
    assert!(!config.strategy_logging(&instance));
    assert!(config.strategy_logging(&domain::InstanceId::new("unknown").unwrap()));
}

#[test]
fn secret_references_validate_names_and_round_trip() {
    for text in [
        "env:API_KEY",
        "env:A1_",
        &format!("env:{}", "A".repeat(128)),
    ] {
        let reference = SecretRef::new(text.into()).unwrap();
        let encoded = toml::Value::try_from(reference).unwrap();
        let decoded: SecretRef = encoded.try_into().unwrap();
        assert_eq!(String::from(decoded), text);
        assert!(SecretRef::try_from(text.to_owned()).is_ok());
    }

    for text in [
        "",
        "literal-secret",
        "env:",
        "env:lower",
        "env:A-B",
        &format!("env:{}", "A".repeat(129)),
    ] {
        assert!(SecretRef::new(text.into()).is_err());
    }

    // A unique absent key avoids mutating process environment in parallel tests.
    let missing = format!("MEMSI_TEST_MISSING_{}_SECRET", std::process::id());
    assert!(
        SecretRef::new(format!("env:{missing}"))
            .unwrap()
            .resolve()
            .is_err()
    );
}

#[test]
fn api_usage_defaults_and_each_invalid_resource_bound_are_checked() {
    assert!(ApiUsageConfig::default().validate().is_ok());
    macro_rules! invalid {
        ($field:ident, $value:expr) => {{
            let mut config = ApiUsageConfig::default();
            config.$field = $value;
            assert!(config.validate().is_err(), stringify!($field));
        }};
    }

    invalid!(monthly_limit, 0);
    invalid!(monthly_limit, u64::MAX);
    invalid!(warn_at, 0);
    invalid!(warn_at, 10_000);
    invalid!(reset_day, 0);
    invalid!(reset_day, 32);
    invalid!(database, String::new());
    invalid!(initial_period, Some("2025-02-29".into()));
    invalid!(initial_used, BTreeMap::from([("primary".into(), 1)]));
    for (name, used) in [("", 1), ("bad-name", 1), ("primary", 10_001)] {
        let config = ApiUsageConfig {
            initial_period: Some("2024-02-29".into()),
            initial_used: BTreeMap::from([(name.into(), used)]),
            ..Default::default()
        };
        assert!(config.validate().is_err());
    }

    let config = ApiUsageConfig {
        initial_period: Some("2024-02-29".into()),
        initial_used: BTreeMap::from([("primary_1".into(), 10_000)]),
        ..Default::default()
    };
    assert!(config.validate().is_ok());
}

#[test]
fn loading_reports_missing_files_and_gives_cli_overrides_precedence() {
    assert!(matches!(
        AppConfig::load(
            Path::new("/nonexistent/memsi-test-config.toml"),
            &BTreeMap::new(),
            &BTreeMap::new()
        ),
        Err(ConfigError::Read)
    ));
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/research-preset.toml");
    let environment = BTreeMap::from([("MEME__MONITOR__POLL_INTERVAL_SECS".into(), "31".into())]);
    let cli = BTreeMap::from([
        ("MEME__MONITOR__POLL_INTERVAL_SECS".into(), "32".into()),
        ("MEME__MODE".into(), "observe".into()),
    ]);
    let config = AppConfig::load(&path, &environment, &cli).unwrap();
    assert_eq!(config.monitor.poll_interval_secs, 32);
    assert_eq!(config.mode, AppMode::Observe);
    for key in ["UNKNOWN", "MEME__UNKNOWN"] {
        assert!(matches!(
            AppConfig::load(
                &path,
                &BTreeMap::new(),
                &BTreeMap::from([(key.into(), "1".into())])
            ),
            Err(ConfigError::UnknownOverride(_))
        ));
    }

    for text in ["abc", "0", "-1"] {
        assert!(
            AppConfig::load(
                &path,
                &BTreeMap::new(),
                &BTreeMap::from([("MEME__MONITOR__POLL_INTERVAL_SECS".into(), text.into())])
            )
            .is_err()
        );
    }
}

struct CompactFiles(std::path::PathBuf);

impl CompactFiles {
    fn new(strategies: &str) -> Self {
        static NEXT_DIRECTORY: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let directory = std::env::temp_dir().join(format!(
            "memsi-config-test-{}-{}",
            std::process::id(),
            NEXT_DIRECTORY.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        ));
        std::fs::create_dir(&directory).unwrap();
        std::fs::write(directory.join("strategies.toml"), strategies).unwrap();
        Self(directory)
    }

    fn path(&self) -> std::path::PathBuf {
        self.0.join("program.toml")
    }
}

impl Drop for CompactFiles {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[test]
fn compact_legacy_profiles_expand_and_apply_thresholds_and_monitor_settings() {
    let profile = "liquidity_usd=1000\nvolume_24h_usd=2000\nage_secs=60\nconfidence=0.8\nmax_width_pct=20\nrelative_volume=1.5\nmax_position_pct=40\n";
    let files = CompactFiles::new(&format!("[low]\n{profile}\n[high]\n{profile}"));
    let input: toml::Value = toml::from_str("strategies_file='strategies.toml'\nwatchlist_retention_minutes=40\n[monitor]\ncandidate_workers=3\n").unwrap();
    let expanded = crate::simple::expand(input, &files.path()).unwrap();
    let config: AppConfig = expanded.try_into().unwrap();
    assert_eq!(config.monitor.watchlist_retention_minutes, 40);
    assert_eq!(config.monitor.candidate_workers, 3);
    assert_eq!(config.monitor.poll_interval_secs, 60);
    for strategy in &config.strategies {
        assert_eq!(
            strategy.config["thresholds"]["liquidity_usd"].as_float(),
            Some(1000.0)
        );
        assert_eq!(
            strategy.config["research"]["max_position_pct"].as_float(),
            Some(40.0)
        );
    }

    assert!(config.validate().is_ok());
}

fn screener_input() -> toml::Value {
    toml::from_str("kind='pump_high'\ntimeframe_secs=60\n[signal]\nmin_gain_pct=5\n[profiles.high.filters]\nmarket_cap_usd={gt=1000}\n").unwrap()
}

#[test]
fn compact_screener_defaults_inherit_filters_and_logging_and_reject_duplicate_filters() {
    let mut config = preset();
    crate::screener::apply(screener_input(), &mut config).unwrap();
    assert_eq!(config.monitor.poll_interval_secs, 180);
    assert_eq!(config.strategies.len(), 1);
    assert!(config.strategies[0].logging);
    assert_eq!(
        config.strategies[0].config["metrics_max_age_secs"].as_integer(),
        Some(60)
    );
    assert!(config.strategies[0].config.get("research").is_none());
    assert_eq!(
        config
            .plugins
            .iter()
            .find(|plugin| plugin.id.as_str() == "volume")
            .unwrap()
            .config["recent_bars"]
            .as_integer(),
        Some(3)
    );
    let mut duplicate = screener_input();
    let common_filters = duplicate["profiles"]["high"]["filters"].clone();
    duplicate
        .as_table_mut()
        .unwrap()
        .insert("filters".into(), common_filters);
    assert!(crate::screener::apply(duplicate, &mut preset()).is_err());
    let mut invalid = screener_input();
    invalid["kind"] = toml::Value::String("unknown".into());
    assert!(crate::screener::apply(invalid, &mut preset()).is_err());
    let mut invalid = screener_input();
    invalid.as_table_mut().unwrap().remove("timeframe_secs");
    assert!(crate::screener::apply(invalid, &mut preset()).is_err());
}

#[test]
fn strategy_sets_namespace_bindings_and_reject_duplicate_or_disabled_definitions() {
    let source = include_str!("../../../strategies.toml");
    let mut input: toml::Value = toml::from_str(source).unwrap();
    input.as_table_mut().unwrap().remove("metrics_max_age_secs");
    let mut config = preset();
    crate::strategy_set::apply(input.clone(), &mut config).unwrap();
    assert!(!config.strategies.is_empty());
    assert!(config.strategies.iter().all(|strategy| {
        strategy.config["range_analyzer"]
            .as_str()
            .unwrap()
            .starts_with("pump-")
    }));
    let mut duplicate = input.clone();
    let definitions = duplicate["strategies"].as_array_mut().unwrap();
    definitions.push(definitions[0].clone());
    assert!(crate::strategy_set::apply(duplicate, &mut preset()).is_err());
    let mut disabled = input;
    for definition in disabled["strategies"].as_array_mut().unwrap() {
        definition
            .as_table_mut()
            .unwrap()
            .insert("enabled".into(), toml::Value::Boolean(false));
    }
    assert!(crate::strategy_set::apply(disabled, &mut preset()).is_err());
}
