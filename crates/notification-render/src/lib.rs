#![forbid(unsafe_code)]
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
