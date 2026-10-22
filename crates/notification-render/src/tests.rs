use super::*;
use domain::*;
use test_support::signal as fixtures;

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
    assert!(summary_text(&notification).contains("RESEARCH_MATCH"));
    notification.signal.kind = SignalKind::LpCandidate;
    assert!(summary_text(&notification).contains("LP_CANDIDATE"));
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
