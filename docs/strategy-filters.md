# Token filter reference

MEMSI supports the public `pump_high` handler with independently evaluated profiles. Common filters are mandatory AND conditions; profiles can add filters and signal overrides. The included strategies.toml is a neutral example. Required unknown or stale metrics do not pass screening. Use README.md for configuration and all signal options.

## Metric semantics and current Codex adapter

| Filter | Meaning | Live adapter mapping |
|---|---|---|
| `market_cap_usd` | Circulating MCap, USD | `circulatingMarketCap` |
| `fdv_usd` | Fully diluted valuation, USD | `marketCap`; never used as a circulating MCap fallback |
| `total_liquidity_usd` | Provider-reported combined pool liquidity | `totalLiquidityUsd`; Codex coverage is its 15 deepest pools with recent activity, not exhaustive verified TVL |
| `pool_liquidity_usd` | Selected pool liquidity, USD | Existing pool snapshot |
| `token_age_secs` | Exact age since token creation | Currently unknown; indexing time is not mint time |
| `trading_age_secs` | Exact age since first pair | Currently unknown; ranked top pool is not necessarily first pool |
| `provider_seen_age_secs` | Age since Codex first indexed the token | `token.createdAt` |
| `top_pool_age_secs` | Age of current top pool | Filter-result `createdAt` |
| `volume_5m_usd`, `volume_1h_usd`, `volume_24h_usd` | Token trading volume, USD | `volume5m`, `volume1`, `volume24` |
| `volume_1m_usd`, `volume_6h_usd` | Exact token volume windows, USD | Currently unknown; no extrapolation from another window or selected-pool volume |
| `volume_1h_to_liquidity` | Token 1h volume / combined liquidity | Ratio of the corresponding fresh token metrics; unknown if liquidity is zero |
| `volume_5m_to_liquidity` | Token 5m volume / combined liquidity | `volume5m / totalLiquidityUsd`; unknown if liquidity is zero |
| `total_fees_native` | Lifetime fees in native chain coins, e.g. SOL on Solana | Currently unknown; neither USD fees nor 24h fees are a substitute |
| `total_fees_usd` | Lifetime trading fees, USD | Currently unknown; never substituted by 24h fees |
| `total_fees_24h_usd` | Trading fees over 24h, USD | `totalFees24`; includes pool, gas and tips under Codex's definition |
| `insider_supply_pct` | Percent supply held by provider-classified insiders | `insiderHeldPercentage` |
| `dev_supply_pct` | Percent supply held by developer | `devHeldPercentage` |
| `top10_holders_supply_pct` | Percent supply in 10 largest holder addresses | `top10HoldersPercent` |
| `top10_percent_holders_supply_pct` | Percent supply in largest 10% of holder addresses | Currently unknown; distinct from 10 addresses |
| `sniper_count`, `sniper_supply_pct` | Sniper wallets count / percent supply held | `sniperCount`, `sniperHeldPercentage` |
| `bundler_count`, `bundler_supply_pct` | Bundler wallets count / percent supply held | `bundlerCount`, `bundlerHeldPercentage` |
| `phishing_wallet_pct` | Percent holder wallets classified as phishing | Currently unknown; suspicious-wallet supply is not a substitute |

Source contract: [Codex TokenFilterResult](https://docs.codex.io/api-reference/types/tokenfilterresult).
Availability remains network/token/plan-dependent. Null values are unknown, not zero.
The extra token snapshot is fetched once per evaluation when any strategy needs token
facts, shared across its profiles, and counted by the existing HTTP quota ledger.
No live API acceptance test was performed for these newly requested fields.

Missing, stale, future or invalid required metrics produce `Unavailable`; a known
failed bound produces `Reject`. Provider-reported values use fetch time when source time
is unavailable. Observations and source timestamps are retained in decision evidence
and included in input hashes. All providers keep default unknown support for metrics
that they do not implement; old fixtures/replays remain compatible.

## Pump screening

See the root strategies.toml for the small-cap `pump-high` example. Required token filters run before candle acquisition. The signal then needs a close-to-close gain above 10% over five completed one-minute intervals, a green latest completed candle with a higher close than its predecessor, and known positive five-minute candle volume. A positive five-minute return with a currently falling close does not match. Volume/liquidity remains an independent mandatory metric filter.
