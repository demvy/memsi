//! Named mainnets. Provider IDs for EVM networks follow the chain ID;
//! Solana uses Codex's explicit network ID rather than an EVM identifier.
use crate::{AppConfig, ConfigError};
use domain::ChainId;
use std::{fmt, str::FromStr};
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Network {
    Solana,
    Robinhood,
    Base,
    EthereumMainnet,
    ArcMainnet,
}
impl Network {
    pub fn name(self) -> &'static str {
        match self {
            Self::Solana => "solana",
            Self::Robinhood => "robinhood",
            Self::Base => "base",
            Self::EthereumMainnet => "ethereum-mainnet",
            Self::ArcMainnet => "arc-mainnet",
        }
    }
    pub fn codex_id(self) -> i32 {
        match self {
            Self::Solana => 1399811149,
            Self::Robinhood => 4663,
            Self::Base => 8453,
            Self::EthereumMainnet => 1,
            Self::ArcMainnet => 5042,
        }
    }
    pub fn chain(self) -> ChainId {
        match self {
            Self::Solana => ChainId::Solana {
                genesis_hash: "5eykt4UsFv8P8NJdTREpY1vzqKqZKvdpKuc147dw2N9d".into(),
            },
            _ => ChainId::Evm {
                chain_id: self.codex_id() as u64,
            },
        }
    }
}
impl fmt::Display for Network {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}
impl FromStr for Network {
    type Err = String;
    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value
            .trim()
            .to_ascii_lowercase()
            .replace([' ', '_'], "-")
            .as_str()
        {
            "solana" | "sol" | "solana-mainnet" => Ok(Self::Solana),
            "robinhood" | "robinhood-mainnet" => Ok(Self::Robinhood),
            "base" | "base-mainnet" => Ok(Self::Base),
            "ethereum" | "ethereum-mainnet" | "eth" => Ok(Self::EthereumMainnet),
            "arc" | "arc-mainnet" => Ok(Self::ArcMainnet),
            _ => Err("choose solana, robinhood, base, ethereum-mainnet or arc-mainnet".into()),
        }
    }
}
impl AppConfig {
    /// Offline override. Only Codex providers can be reassigned to a selected network.
    pub fn select_network(&mut self, network: Network) -> Result<(), ConfigError> {
        let chain = network.chain();
        if chain != self.chain
            && self.plugins.iter().filter(|p| p.enabled).any(|p| {
                p.plugin.as_str().starts_with("provider.") && p.plugin.as_str() != "provider.codex"
            })
        {
            return Err(ConfigError::Validation(
                "--network requires a Codex-only configuration".into(),
            ));
        }
        if !self
            .plugins
            .iter()
            .any(|p| p.enabled && p.plugin.as_str() == "provider.codex")
        {
            return Err(ConfigError::Validation(
                "--network requires an enabled Codex provider".into(),
            ));
        }
        let mut selected = self.clone();
        selected.chain = chain;
        for provider in selected
            .plugins
            .iter_mut()
            .filter(|p| p.plugin.as_str() == "provider.codex")
        {
            provider.config.insert(
                "network_id".into(),
                toml::Value::Integer(network.codex_id() as i64),
            );
        }
        selected.validate()?;
        *self = selected;
        Ok(())
    }
}
