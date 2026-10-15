#![forbid(unsafe_code)]
use domain::*;
use serde::{Deserialize, Serialize};
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SignalKind {
    ResearchMatch,
    LpCandidate,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum SignalFact {
    Money {
        label: String,
        usd: NonNegative,
        window_secs: Option<u64>,
    },
    Range {
        lower: PositivePrice,
        upper: PositivePrice,
        unit: PriceUnit,
    },
    Confidence {
        value: UnitInterval,
    },
    Text {
        label: String,
        value: String,
    },
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Signal {
    pub id: ContentHash,
    pub created_at: TimestampMs,
    pub expires_at: TimestampMs,
    pub strategy: InstanceId,
    pub config_digest: ContentHash,
    pub token: TokenId,
    pub pool: PoolId,
    pub kind: SignalKind,
    pub symbol: Option<String>,
    pub facts: Vec<SignalFact>,
    pub reasons: Vec<String>,
    pub warnings: Vec<String>,
    pub snapshot_hash: ContentHash,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SignalLink {
    pub label: String,
    pub url: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SignalDraft {
    pub signal: Signal,
    pub dedupe_key: ContentHash,
    pub cooldown_secs: u64,
}
