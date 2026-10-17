use super::*;
use domain::*;
use signal_api::*;
#[path = "../../../test-support/signal.rs"]
mod fixtures;

fn notification() -> Notification {
    Notification {
        delivery_id: ContentHash::of_bytes(b"delivery"),
        signal: fixtures::signal(),
        links: vec![],
        destination: "stdout".into(),
        part_index: 0,
    }
}

#[test]
fn text_rendering_cleans_controls_escapes_html_and_preserves_unicode() {
    let mut notification = notification();
    notification.signal.symbol = Some("<A&\"B>\n\u{202e}🦀".into());
    let html = telegram_html(&notification);
    assert!(html.contains("&lt;A&amp;&quot;B&gt;🦀"));
    assert!(!html.contains('\u{202e}'));
    let plain = plain_text(&notification);
    assert!(plain.contains("<A&\"B>🦀"));
    assert!(plain.contains("Research match"));
    assert!(summary_text(&notification).contains("RESEARCH_MATCH"));
    notification.signal.kind = SignalKind::LpCandidate;
    assert!(summary_text(&notification).contains("LP_CANDIDATE"));
    assert!(plain_text(&notification).contains("LP candidate"));
    notification.signal.kind = SignalKind::ResearchMatch;
    notification.signal.reasons.push("PUMP_HIGH".into());
    assert!(summary_text(&notification).contains("PUMP_HIGH"));
    assert_eq!(clean("a\nb\u{2066}c🦀d", 4), "abc🦀");
}

#[test]
fn token_age_requires_valid_provider_fact_and_uses_largest_whole_unit() {
    for (seconds, expected) in [
        (0.0, "less than a minute ago"),
        (60.0, "1 minute ago"),
        (120.0, "2 minutes ago"),
        (3600.0, "1 hour ago"),
        (86_400.0, "1 day ago"),
        (2_592_000.0, "1 month ago"),
        (31_536_000.0, "1 year ago"),
    ] {
        assert_eq!(human_age(seconds), expected);
    }

    let mut signal = fixtures::signal();
    assert!(token_age(&signal).starts_with("unknown"));
    for value in ["invalid", "NaN", "inf", "-1"] {
        signal.facts = vec![SignalFact::Text {
            label: "Token age seconds (provider)".into(),
            value: value.into(),
        }];
        assert!(token_age(&signal).starts_with("unknown"));
    }

    signal.facts = vec![SignalFact::Text {
        label: "Token age seconds (provider)".into(),
        value: "7200".into(),
    }];
    assert_eq!(token_age(&signal), "2 hours ago");
}

#[test]
fn fixed_link_mappings_support_known_chains_and_reject_unknown_networks() {
    let mut signal = fixtures::signal();
    for (id, gmgn, defined, name) in [
        (1, Some("eth"), Some("eth"), "Ethereum Mainnet"),
        (8453, Some("base"), Some("base"), "Base"),
        (4663, None, Some("robinhood"), "Robinhood"),
        (5042, None, Some("arc"), "Arc Mainnet"),
        (42, None, None, "EVM 42"),
    ] {
        signal.token = TokenId::new(ChainId::Evm { chain_id: id }, signal.token.address()).unwrap();
        assert_eq!(
            gmgn_url(&signal),
            gmgn.map(|network| format!(
                "https://gmgn.ai/{network}/token/{}",
                signal.token.address()
            ))
        );
        assert_eq!(
            defined_url(&signal),
            defined.map(|network| format!(
                "https://un.defined.fi/{network}/{}",
                signal.token.address()
            ))
        );
        assert_eq!(chain_name(&signal), name);
    }

    for (genesis, supported) in [
        ("5eykt4UsFv8P8NJdTREpY1vzqKqZKvdpKuc147dw2N9d", true),
        ("11111111111111111111111111111111", false),
    ] {
        signal.token = TokenId::new(
            ChainId::Solana {
                genesis_hash: genesis.into(),
            },
            "11111111111111111111111111111111",
        )
        .unwrap();
        assert_eq!(gmgn_url(&signal).is_some(), supported);
        assert_eq!(defined_url(&signal).is_some(), supported);
        assert_eq!(chain_name(&signal), "Solana");
    }
}

#[test]
fn detailed_rendering_includes_all_fact_types_warnings_and_only_allowed_links() {
    let mut notification = notification();
    notification.signal.warnings.push("RESEARCH_ONLY".into());
    notification.signal.reasons.push("RANGE_CONFIRMED".into());
    notification.signal.facts = vec![
        SignalFact::Money {
            label: "Liquidity".into(),
            usd: NonNegative::new(1000.0).unwrap(),
            window_secs: Some(60),
        },
        SignalFact::Money {
            label: "Fees".into(),
            usd: NonNegative::new(10.0).unwrap(),
            window_secs: None,
        },
        SignalFact::Range {
            lower: PositivePrice::new(1.0).unwrap(),
            upper: PositivePrice::new(2.0).unwrap(),
            unit: PriceUnit::UsdPerToken,
        },
        SignalFact::Range {
            lower: PositivePrice::new(1.0).unwrap(),
            upper: PositivePrice::new(2.0).unwrap(),
            unit: PriceUnit::QuotePerToken {
                quote: notification.signal.token.clone(),
            },
        },
        SignalFact::Confidence {
            value: UnitInterval::new(0.85).unwrap(),
        },
        SignalFact::Text {
            label: "Note".into(),
            value: "value".into(),
        },
    ];
    notification.links = vec![
        SignalLink {
            label: "Token".into(),
            url: "https://etherscan.io/token/valid".into(),
        },
        SignalLink {
            label: "Bad".into(),
            url: "https://example.com/".into(),
        },
        SignalLink {
            label: "Control".into(),
            url: "https://etherscan.io/\n".into(),
        },
    ];
    let text = plain_text(&notification);
    for expected in [
        "Warning: RESEARCH_ONLY",
        "+ range confirmed",
        "Liquidity: $1000 / 60s",
        "Fees: $10",
        "USD/token",
        "per token",
        "Confidence: 85.0%",
        "Note: value",
        "https://etherscan.io/token/valid",
    ] {
        assert!(text.contains(expected), "missing {expected}");
    }
    assert!(!text.contains("example.com"));
    assert!(!text.contains("Control:"));
    let mut long = "x".repeat(3498);
    append(&mut long, "🦀");
    assert_eq!(long.len(), 3498);
    append(&mut long, "a");
    assert_eq!(long.len(), 3500);
}
