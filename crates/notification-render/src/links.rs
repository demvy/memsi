//! Fixed destination hosts and explicit mainnet mappings; addresses are validated TokenIds.
use domain::ChainId;

pub fn gmgn_url(signal: &signal_api::Signal) -> Option<String> {
    let chain = match signal.token.chain() {
        ChainId::Solana { genesis_hash }
            if genesis_hash == "5eykt4UsFv8P8NJdTREpY1vzqKqZKvdpKuc147dw2N9d" =>
        {
            "sol"
        }

        ChainId::Evm { chain_id: 1 } => "eth",
        ChainId::Evm { chain_id: 8453 } => "base",
        _ => return None,
    };

    Some(format!(
        "https://gmgn.ai/{chain}/token/{}",
        signal.token.address()
    ))
}

pub fn defined_url(signal: &signal_api::Signal) -> Option<String> {
    let network = match signal.token.chain() {
        ChainId::Solana { genesis_hash }
            if genesis_hash == "5eykt4UsFv8P8NJdTREpY1vzqKqZKvdpKuc147dw2N9d" =>
        {
            "sol"
        }

        ChainId::Evm { chain_id: 1 } => "eth",
        ChainId::Evm { chain_id: 8453 } => "base",
        ChainId::Evm { chain_id: 4663 } => "robinhood",
        ChainId::Evm { chain_id: 5042 } => "arc",
        _ => return None,
    };

    Some(format!(
        "https://un.defined.fi/{network}/{}",
        signal.token.address()
    ))
}

pub(super) fn chain_name(signal: &signal_api::Signal) -> String {
    match signal.token.chain() {
        ChainId::Solana { .. } => "Solana".into(),
        ChainId::Evm { chain_id: 1 } => "Ethereum Mainnet".into(),
        ChainId::Evm { chain_id: 8453 } => "Base".into(),
        ChainId::Evm { chain_id: 4663 } => "Robinhood".into(),
        ChainId::Evm { chain_id: 5042 } => "Arc Mainnet".into(),
        ChainId::Evm { chain_id } => format!("EVM {chain_id}"),
    }
}
