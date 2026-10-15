//! Stable transaction-local identities, independent of capture time or block inclusion.
use crate::{ChainId, DomainError};
use serde::{Deserialize, Serialize};
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(try_from = "RawTransaction")]
pub struct ChainTransaction {
    chain: ChainId,
    hash: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawTransaction {
    chain: ChainId,
    hash: String,
}
impl ChainTransaction {
    pub fn new(chain: ChainId, hash: &str) -> Result<Self, DomainError> {
        chain.validate()?;
        let invalid = || DomainError::Identity("transaction hash/signature".into());
        let hash = match &chain {
            ChainId::Evm { .. } => {
                let hex = hash
                    .strip_prefix("0x")
                    .or_else(|| hash.strip_prefix("0X"))
                    .ok_or_else(invalid)?;
                if hex.len() != 64 || !hex.bytes().all(|byte| byte.is_ascii_hexdigit()) {
                    return Err(invalid());
                }
                format!("0x{}", hex.to_ascii_lowercase())
            }
            ChainId::Solana { .. } => {
                if hash.len() > 88 {
                    return Err(invalid());
                }
                let bytes = bs58::decode(hash).into_vec().map_err(|_| invalid())?;
                if bytes.len() != 64 {
                    return Err(invalid());
                }
                bs58::encode(bytes).into_string()
            }
        };
        Ok(Self { chain, hash })
    }
    pub fn chain(&self) -> &ChainId {
        &self.chain
    }
    pub fn hash(&self) -> &str {
        &self.hash
    }
}
impl TryFrom<RawTransaction> for ChainTransaction {
    type Error = DomainError;
    fn try_from(raw: RawTransaction) -> Result<Self, Self::Error> {
        Self::new(raw.chain, &raw.hash)
    }
}
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum ActivityLocation {
    /// Ordinal in this transaction's receipt.logs, NOT the block-global RPC logIndex.
    EvmLog { log_ordinal: u32, effect_index: u16 },
    /// `inner` is an ordinal in the outer instruction's flat innerInstructions list.
    SolanaInstruction {
        instruction: u16,
        inner: Option<u16>,
        effect_index: u16,
    },
    /// One aggregate network fee per transaction, including failed transactions.
    TransactionFee,
}
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(try_from = "RawActivity")]
pub struct ActivityId {
    transaction: ChainTransaction,
    location: ActivityLocation,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawActivity {
    transaction: ChainTransaction,
    location: ActivityLocation,
}
impl ActivityId {
    pub fn new(
        transaction: ChainTransaction,
        location: ActivityLocation,
    ) -> Result<Self, DomainError> {
        let valid = matches!(
            (transaction.chain(), &location),
            (_, ActivityLocation::TransactionFee)
                | (ChainId::Evm { .. }, ActivityLocation::EvmLog { .. })
                | (
                    ChainId::Solana { .. },
                    ActivityLocation::SolanaInstruction { .. }
                )
        );
        if !valid {
            return Err(DomainError::Identity(
                "activity location/chain mismatch".into(),
            ));
        }
        Ok(Self {
            transaction,
            location,
        })
    }
    pub fn transaction(&self) -> &ChainTransaction {
        &self.transaction
    }
    pub fn location(&self) -> &ActivityLocation {
        &self.location
    }
}
impl TryFrom<RawActivity> for ActivityId {
    type Error = DomainError;
    fn try_from(raw: RawActivity) -> Result<Self, Self::Error> {
        Self::new(raw.transaction, raw.location)
    }
}
