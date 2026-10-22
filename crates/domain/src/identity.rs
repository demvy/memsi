use crate::DomainError;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::fmt;
macro_rules! id {
    ($($name:ident),+) => {
        $(
            #[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
            #[serde(try_from = "String", into = "String")]
            pub struct $name(String);

            impl $name {
                pub fn new(value: impl Into<String>) -> Result<Self, DomainError> {
                    let value = value.into();

                    if value.is_empty()
                        || value.len() > 96
                        || !value.as_bytes()[0].is_ascii_lowercase()
                            && !value.as_bytes()[0].is_ascii_digit()
                        || !value.bytes().all(|c|
                            c.is_ascii_lowercase() || c.is_ascii_digit() || b"._-".contains(&c)
                        )
                    {
                        return Err(DomainError::Identity(stringify!($name).into()));
                    }

                    Ok(Self(value))
                }

                pub fn as_str(&self) -> &str {
                    &self.0
                }
            }

            impl TryFrom<String> for $name {
                type Error = DomainError;

                fn try_from(v: String) -> Result<Self, Self::Error> {
                    Self::new(v)
                }
            }

            impl From<$name> for String {
                fn from(v: $name) -> Self {
                    v.0
                }
            }

            impl fmt::Display for $name {
                fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                    f.write_str(&self.0)
                }
            }
        )+
    };
}

id!(
    PluginId,
    InstanceId,
    CapabilityId,
    SchemaId,
    ProviderId,
    ProtocolId,
    BindingId
);
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(tag = "family", rename_all = "snake_case")]
pub enum ChainId {
    Evm { chain_id: u64 },
    Solana { genesis_hash: String },
}

impl ChainId {
    pub fn validate(&self) -> Result<(), DomainError> {
        match self {
            Self::Evm { chain_id } if *chain_id > 0 => Ok(()),
            Self::Solana { genesis_hash }
                if genesis_hash.len() <= 64
                    && bs58::decode(genesis_hash)
                        .into_vec()
                        .is_ok_and(|b| b.len() == 32) =>
            {
                Ok(())
            }

            _ => Err(DomainError::Identity("invalid chain identity".into())),
        }
    }

    pub fn address(&self, input: &str) -> Result<String, DomainError> {
        self.validate()?;
        match self {
            Self::Evm { .. } => {
                let hex = input
                    .strip_prefix("0x")
                    .or_else(|| input.strip_prefix("0X"))
                    .ok_or_else(|| DomainError::Identity("EVM prefix".into()))?;
                if hex.len() != 40 || !hex.bytes().all(|b| b.is_ascii_hexdigit()) {
                    return Err(DomainError::Identity("EVM address length/hex".into()));
                }

                Ok(format!("0x{}", hex.to_ascii_lowercase()))
            }

            Self::Solana { .. } => {
                if input.len() > 44 {
                    return Err(DomainError::Identity("Solana address length".into()));
                }

                let bytes = bs58::decode(input)
                    .into_vec()
                    .map_err(|_| DomainError::Identity("Solana base58".into()))?;
                if bytes.len() != 32 {
                    return Err(DomainError::Identity(
                        "Solana address must decode to 32 bytes".into(),
                    ));
                }

                Ok(bs58::encode(bytes).into_string())
            }
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(try_from = "RawTokenId")]
pub struct TokenId {
    chain: ChainId,
    address: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawTokenId {
    chain: ChainId,
    address: String,
}

impl TryFrom<RawTokenId> for TokenId {
    type Error = DomainError;
    fn try_from(v: RawTokenId) -> Result<Self, Self::Error> {
        Self::new(v.chain, &v.address)
    }
}

impl TokenId {
    pub fn new(chain: ChainId, address: &str) -> Result<Self, DomainError> {
        let address = chain.address(address)?;
        Ok(Self { chain, address })
    }

    pub fn chain(&self) -> &ChainId {
        &self.chain
    }

    pub fn address(&self) -> &str {
        &self.address
    }
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum PoolLocator {
    Address {
        address: String,
    },
    Opaque {
        protocol: ProtocolId,
        manager: String,
        key: String,
    },
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(try_from = "RawPoolId")]
pub struct PoolId {
    chain: ChainId,
    locator: PoolLocator,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawPoolId {
    chain: ChainId,
    locator: PoolLocator,
}

impl TryFrom<RawPoolId> for PoolId {
    type Error = DomainError;
    fn try_from(v: RawPoolId) -> Result<Self, Self::Error> {
        Self::new(v.chain, v.locator)
    }
}

impl PoolId {
    pub fn new(chain: ChainId, locator: PoolLocator) -> Result<Self, DomainError> {
        chain.validate()?;
        let locator = match locator {
            PoolLocator::Address { address } => PoolLocator::Address {
                address: chain.address(&address)?,
            },
            PoolLocator::Opaque {
                protocol,
                manager,
                key,
            } => {
                if key.is_empty()
                    || key.len() > 256
                    || !key.is_ascii()
                    || key.bytes().any(|b| b.is_ascii_control())
                {
                    return Err(DomainError::Identity("opaque key".into()));
                }

                PoolLocator::Opaque {
                    protocol,
                    manager: chain.address(&manager)?,
                    key,
                }
            }
        };

        Ok(Self { chain, locator })
    }

    pub fn address(chain: ChainId, address: &str) -> Result<Self, DomainError> {
        Self::new(
            chain,
            PoolLocator::Address {
                address: address.into(),
            },
        )
    }

    pub fn chain(&self) -> &ChainId {
        &self.chain
    }

    pub fn locator(&self) -> &PoolLocator {
        &self.locator
    }

    pub fn as_address(&self) -> Option<&str> {
        match &self.locator {
            PoolLocator::Address { address } => Some(address),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct ContentHash(String);
impl ContentHash {
    pub fn of_bytes(bytes: &[u8]) -> Self {
        Self(format!("{:x}", Sha256::digest(bytes)))
    }

    pub fn of<T: Serialize>(value: &T) -> Result<Self, serde_json::Error> {
        let v = serde_json::to_value(value)?;
        let bytes = serde_json::to_vec(&(1u8, v))?;
        Ok(Self::of_bytes(&bytes))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl TryFrom<String> for ContentHash {
    type Error = DomainError;
    fn try_from(v: String) -> Result<Self, Self::Error> {
        if v.len() != 64
            || !v
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        {
            return Err(DomainError::Identity("SHA256".into()));
        }

        Ok(Self(v))
    }
}

impl From<ContentHash> for String {
    fn from(v: ContentHash) -> String {
        v.0
    }
}
