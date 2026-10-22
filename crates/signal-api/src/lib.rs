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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn signal_fact_wire_tags_and_validated_numeric_values_remain_stable() {
        let fact = SignalFact::Confidence {
            value: UnitInterval::new(0.75).unwrap(),
        };
        let encoded = serde_json::to_value(fact).unwrap();
        assert_eq!(
            encoded,
            serde_json::json!({"kind":"confidence","value":0.75})
        );
        let decoded: SignalFact = serde_json::from_value(encoded).unwrap();
        assert!(matches!(decoded, SignalFact::Confidence { value } if value.get() == 0.75));
        assert!(
            serde_json::from_value::<SignalFact>(
                serde_json::json!({"kind":"confidence","value":2})
            )
            .is_err()
        );
        assert_eq!(
            serde_json::to_value(SignalKind::ResearchMatch).unwrap(),
            "research_match"
        );
        assert_eq!(
            serde_json::to_value(SignalKind::LpCandidate).unwrap(),
            "lp_candidate"
        );
    }
}
