use domain::ChainId;
use signal_api::{Signal, SignalLink};
/// URL hosts and paths come from this allowlist, never from provider/token metadata.
pub fn for_signal(signal: &Signal) -> Vec<SignalLink> {
    match signal.token.chain() {
        ChainId::Solana { genesis_hash }
            if genesis_hash == "5eykt4UsFv8P8NJdTREpY1vzqKqZKvdpKuc147dw2N9d" =>
        {
            let mut links = vec![SignalLink {
                label: "Token".into(),
                url: format!(
                    "https://explorer.solana.com/address/{}",
                    signal.token.address()
                ),
            }];
            if let Some(address) = signal.pool.as_address() {
                links.push(SignalLink {
                    label: "Pool".into(),
                    url: format!("https://explorer.solana.com/address/{address}"),
                });
                links.push(SignalLink {
                    label: "Chart".into(),
                    url: format!("https://dexscreener.com/solana/{address}"),
                });
            }
            links
        }
        ChainId::Evm { chain_id } => {
            let host = match chain_id {
                1 => "etherscan.io",
                8453 => "basescan.org",
                _ => return vec![],
            };
            let mut links = vec![SignalLink {
                label: "Token".into(),
                url: format!("https://{host}/token/{}", signal.token.address()),
            }];
            if let Some(address) = signal.pool.as_address() {
                links.push(SignalLink {
                    label: "Pool".into(),
                    url: format!("https://{host}/address/{address}"),
                });
                let network = if *chain_id == 1 { "ethereum" } else { "base" };
                links.push(SignalLink {
                    label: "Chart".into(),
                    url: format!("https://dexscreener.com/{network}/{address}"),
                });
            }
            links
        }
        _ => vec![],
    }
}
