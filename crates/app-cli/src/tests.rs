use super::*;

#[test]
fn error_json_is_parsed_as_a_global_option() {
    for arguments in [
        vec!["memes_finder", "--error-json", "plugins"],
        vec!["memes_finder", "plugins", "--error-json"],
    ] {
        assert!(Cli::try_parse_from(arguments).unwrap().error_json);
    }

    assert!(
        !Cli::try_parse_from(["memes_finder", "plugins"])
            .unwrap()
            .error_json
    );
}

#[test]
fn command_parser_accepts_offline_commands_and_validates_networks() {
    for arguments in [
        vec!["memes_finder", "plugins", "--envelope"],
        vec!["memes_finder", "status"],
        vec!["memes_finder", "strategies"],
        vec!["memes_finder", "--network", "eth", "config-check"],
    ] {
        assert!(Cli::try_parse_from(arguments).is_ok());
    }

    assert!(Cli::try_parse_from(["memes_finder", "--network", "unknown", "config-check"]).is_err());
    assert!(Cli::try_parse_from(["memes_finder", "unknown-command"]).is_err());
    let unsupported =
        Cli::try_parse_from(["memes_finder", "--network", "base", "plugins"]).unwrap();
    assert!(run(unsupported).is_err());
}

#[cfg(feature = "codex")]
#[test]
fn market_argument_defaults_and_request_context_are_valid() {
    let cli = Cli::try_parse_from([
        "memes_finder",
        "analyze-token",
        "--token",
        "0x1111111111111111111111111111111111111111",
    ])
    .unwrap();
    let Command::AnalyzeToken(arguments) = cli.command else {
        panic!("expected analyze-token")
    };
    assert_eq!(arguments.resolution_secs, 300);
    assert!(arguments.analyzer.is_empty());
    assert!(!arguments.require_cl);
    let config =
        app_config::AppConfig::parse(include_str!("../../app-config/src/research-preset.toml"))
            .unwrap();
    assert!(live::context(&config).check().is_ok());
    assert!(runtime().is_ok());
}
