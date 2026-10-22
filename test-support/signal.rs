//! Minimal signal fixture shared by notification and state tests.
use domain::*;
use signal_api::{Signal, SignalDraft, SignalKind};

pub fn signal() -> Signal {
    let chain = ChainId::Evm { chain_id: 1 };
    Signal {
        id: ContentHash::of_bytes(b"signal"),
        created_at: TimestampMs::new(1000).unwrap(),
        expires_at: TimestampMs::new(301_000).unwrap(),
        strategy: InstanceId::new("strategy").unwrap(),
        config_digest: ContentHash::of_bytes(b"config"),
        token: TokenId::new(chain.clone(), "0x1111111111111111111111111111111111111111").unwrap(),
        pool: PoolId::address(chain, "0x2222222222222222222222222222222222222222").unwrap(),
        kind: SignalKind::ResearchMatch,
        symbol: Some("TEST".into()),
        facts: vec![],
        reasons: vec![],
        warnings: vec![],
        snapshot_hash: ContentHash::of_bytes(b"snapshot"),
    }
}

pub fn draft() -> SignalDraft {
    SignalDraft {
        signal: signal(),
        dedupe_key: ContentHash::of_bytes(b"dedupe"),
        cooldown_secs: 60,
    }
}
