#![forbid(unsafe_code)]
use domain::{ChainId, PriceUnit};
use notification_api::Notification;
use signal_api::{SignalFact, SignalKind};

mod links;
use links::chain_name;
pub use links::{defined_url, gmgn_url};

fn token_age(signal: &signal_api::Signal) -> String {
    let seconds = signal
        .facts
        .iter()
        .find_map(|f| match f {
            SignalFact::Text { label, value } if label == "Token age seconds (provider)" => {
                value.parse::<f64>().ok()
            }

            _ => None,
        })
        .filter(|v| v.is_finite() && *v >= 0.);
    seconds.map_or_else(
        || "unknown (provider did not return token creation time)".into(),
        human_age,
    )
}

fn human_age(seconds: f64) -> String {
    for (unit, span) in [
        ("year", 31_536_000.),
        ("month", 2_592_000.),
        ("day", 86_400.),
        ("hour", 3600.),
        ("minute", 60.),
    ] {
        if seconds >= span {
            let n = (seconds / span).floor() as u64;
            return format!("{n} {unit}{} ago", if n == 1 { "" } else { "s" });
        }
    }

    "less than a minute ago".into()
}

pub fn telegram_html(notification: &Notification) -> String {
    let signal = &notification.signal;
    let ca = html(&clean(signal.token.address(), 128));
    let contract = gmgn_url(signal).map_or(ca.clone(), |url| {
        format!("<a href=\"{}\">{ca}</a>", html(&url))
    });
    let pools = defined_url(signal).map_or("unavailable for this chain".into(), |url| {
        format!("<a href=\"{}\">Defined.fi</a>", html(&url))
    });
    format!(
        "<b>{}</b> · {}\nStrategy: {}\nChain: {}\nToken: {contract}\nCheckPools at: {pools}\nCreated: {}",
        signal_title(signal),
        html(&clean(signal.symbol.as_deref().unwrap_or("unknown"), 64)),
        html(&clean(signal.strategy.as_str(), 128)),
        chain_name(signal),
        html(&token_age(signal))
    )
}

pub fn summary_text(notification: &Notification) -> String {
    let signal = &notification.signal;
    format!(
        "{} · {}\nStrategy: {}\nChain: {}\nToken: {}\nCheckPools at: {}\nCreated: {}\n",
        signal_title(signal),
        clean(signal.symbol.as_deref().unwrap_or("unknown"), 64),
        clean(signal.strategy.as_str(), 128),
        chain_name(signal),
        gmgn_url(signal).unwrap_or_else(|| signal.token.address().into()),
        defined_url(signal).unwrap_or_else(|| "unavailable for this chain".into()),
        token_age(signal)
    )
}

fn signal_title(signal: &signal_api::Signal) -> &'static str {
    match signal.kind {
        SignalKind::LpCandidate => "LP_CANDIDATE",
        SignalKind::ResearchMatch if signal.reasons.iter().any(|r| r == "PUMP_HIGH") => "PUMP_HIGH",
        SignalKind::ResearchMatch => "RESEARCH_MATCH",
    }
}

fn html(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

pub fn plain_text(notification: &Notification) -> String {
    let signal = &notification.signal;
    let title = match signal.kind {
        SignalKind::ResearchMatch => "Research match",
        SignalKind::LpCandidate => "LP candidate",
    };

    let symbol = clean(signal.symbol.as_deref().unwrap_or("unknown ticker"), 64);
    let chain = match signal.token.chain() {
        ChainId::Evm { chain_id } => format!("EVM {chain_id}"),
        ChainId::Solana { .. } => "Solana".into(),
    };

    let pool = signal
        .pool
        .as_address()
        .map(str::to_owned)
        .unwrap_or_else(|| format!("{:?}", signal.pool.locator()));
    let mut text = format!(
        "{title} · {symbol}\nStrategy: {}\nChain: {chain}\nToken: {}\nPool: {pool}\nCreated (Unix ms): {}\nSignal: {}\n",
        signal.strategy,
        signal.token.address(),
        signal.created_at.get(),
        &signal.id.as_str()[..12]
    );
    for link in notification.links.iter().take(4) {
        let allowed = [
            "https://explorer.solana.com/",
            "https://etherscan.io/",
            "https://basescan.org/",
            "https://dexscreener.com/solana/",
            "https://dexscreener.com/ethereum/",
            "https://dexscreener.com/base/",
        ];
        if allowed.iter().any(|prefix| link.url.starts_with(prefix))
            && !link.url.chars().any(char::is_control)
            && link.url.len() < 512
        {
            append(
                &mut text,
                &format!("{}: {}", clean(&link.label, 32), link.url),
            );
        }
    }
    // Warnings precede optional facts so truncation cannot hide the research/risk label.
    for warning in &signal.warnings {
        append(&mut text, &format!("Warning: {}", clean(warning, 160)));
    }

    if !signal.reasons.is_empty() {
        append(&mut text, "Matched because:");
        for reason in signal.reasons.iter().take(12) {
            append(
                &mut text,
                &format!("+ {}", clean(reason, 160).replace('_', " ").to_lowercase()),
            );
        }
    }

    for fact in &signal.facts {
        let line = match fact {
            SignalFact::Money {
                label,
                usd,
                window_secs,
            } => format!(
                "{}: ${}{}",
                clean(label, 64),
                usd.get(),
                window_secs
                    .map(|seconds| format!(" / {seconds}s"))
                    .unwrap_or_default()
            ),
            SignalFact::Range { lower, upper, unit } => {
                let unit = match unit {
                    PriceUnit::UsdPerToken => "USD/token".into(),
                    PriceUnit::QuotePerToken { quote } => format!("{} per token", quote.address()),
                };

                format!("Range: {}–{} {unit}", lower.get(), upper.get())
            }

            SignalFact::Confidence { value } => format!("Confidence: {:.1}%", value.get() * 100.),
            SignalFact::Text { label, value } => {
                format!("{}: {}", clean(label, 64), clean(value, 256))
            }
        };

        append(&mut text, &line);
    }

    text
}

fn append(text: &mut String, line: &str) {
    if text.encode_utf16().count() + line.encode_utf16().count() < 3500 {
        text.push_str(line);
        text.push('\n');
    }
}

fn clean(value: &str, limit: usize) -> String {
    value
        .chars()
        .filter(|character| {
            !character.is_control()
                && !matches!(*character, '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}')
        })
        .take(limit)
        .collect()
}

#[cfg(test)]
mod tests;
