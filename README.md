# MEMSI

**MEMSI tracks and filters memecoins using independently configured strategies.** It helps liquidity providers and LP farmers find tokens worth inspecting, compare market activity, and receive research alerts before deciding what to do manually.

MEMSI is a Rust CLI application. The current monitoring path uses **Codex.io** for discovery, token metrics, pools, and OHLCV data. It can evaluate multiple strategies and profiles against shared inputs and deliver signals to console and Telegram. It does not automatically trade or open LP positions.

[Quick start](#quick-start) · [Configuration](#configuration) · [Strategy reference](#strategy-reference) · [Custom strategies](#custom-strategies) · [Architecture](docs/arch.md)

## What it does

- Discovers up to 100 candidates ranked by hourly trading volume.
- Applies market-cap, liquidity, volume, holder-risk, and other supported filters.
- Checks cheap token metrics before requesting pools and candles.
- Runs independent strategy instances; one rejection does not cancel another match.
- Shares candle requests and analysis where inputs and configuration permit it.
- Sends structured alerts with strategy, chain, contract address, chart/pool links, and a human-readable token age when the provider supplies it.
- Tracks API consumption per key across restarts and warns near the configured quota.

These are research signals, not proof that a token is safe or that an LP position will be profitable. Availability of individual metrics depends on the data source.

## Quick start

Use Rust 1.94 or newer, Cargo, and a local Codex API key. Build from the repository root:

```bash
cargo build --release --locked --bin memes_finder
./target/release/memes_finder --help
```

Set credentials in your process environment or a local `.env`:

```dotenv
CODEX_API_KEY=your_local_api_key
# Only needed if you enable Telegram:
# TELEGRAM_BOT_TOKEN=your_local_bot_token
# TELEGRAM_CHAT_ID=your_local_chat_id
```

Do not commit actual credentials. The application reads `.env` from the **working directory**, not the executable directory. Process environment values take precedence.

Create or review `memes-finder.toml` and `strategies.toml` using the examples below. The neutral strategy example is an options reference, not the contents of your existing local strategy file. Check your sink settings before starting monitoring.

```bash
# Offline: validates configuration without market reads or sending alerts.
./target/release/memes_finder config-check
./target/release/memes_finder plugins

# Uses API quota and delivers to the configured sinks.
./target/release/memes_finder monitor
```

After changing configuration, run config-check and restart the monitor. If Cargo dependencies are already cached, builds can also use `--offline`.

## Configuration

There are two configuration files and local credentials:

| Location | Purpose |
|---|---|
| `memes-finder.toml` | Application settings: strategy file, watchlist TTL, Telegram, API quota |
| `strategies.toml` | Strategy definitions, profiles, filters, signal parameters, logging |
| Environment / `.env` | Credentials referenced by name; not strategy rules |

### Application configuration

```toml
# memes-finder.toml — a neutral starting point
strategies_file = "strategies.toml"
watchlist_retention_minutes = 30
telegram = false
fallback_api_keys = [] # e.g. ["env:CODEX_BACKUP_API_KEY"]

[api_usage]
monthly_limit = 10000
warn_at = 9000
reset_day = 1
 database = "api-usage.sqlite"
```

| Field | Default | Meaning |
|---|---|---|
| `strategies_file` | Required | Strategy file; relative to the application config |
| `watchlist_retention_minutes` | 30 | Time since last discovery appearance before removing a watch record; integer 1…10080 |
| `telegram` | `false` | Enable Telegram delivery; requires local bot token and chat ID |
| `fallback_api_keys` | `[]` | Additional Codex key references, not provider URLs or notification channels |
| `api_usage.monthly_limit` | 10000 | Local request budget per key and configured period |
| `api_usage.warn_at` | 9000 | Warning threshold; strictly between zero and the monthly limit |
| `api_usage.reset_day` | 1 | UTC reset day, 1…31; clamped to the last day of shorter months |
| `api_usage.database` | `api-usage.sqlite` | Persistent per-key accounting, not a candle database |
| `api_usage.initial_period` | Omitted | Optional period start, `YYYY-MM-DD`, for seeding dashboard usage |
| `api_usage.initial_used` | `{}` | Optional map from environment variable names to initial counts; requires `initial_period` |

Set the quota and reset schedule to match your own subscription. The async SQLx ledger stores accounting in SQLite. Existing ledger files remain compatible. The local ledger counts this application's requests plus any configured initial baseline; it does not discover usage by other applications sharing the key. A warning is claimed once per key/period; exhausting all configured keys stops data parsing.

A token last seen 30 minutes ago is removed at the next monitor cycle. Discovery refreshes timestamps before cleanup, so a token present in the current fetch is kept. With five-minute discovery this corresponds to roughly six missed fetches. TTL also elapses during API outages. Reappearing tokens can be screened and admitted again. Candle/analyzer caches have separate bounded lifecycles.

Advanced monitor overrides remain available; for example: `MEME__MONITOR__WATCHLIST_RETENTION_MINUTES=15`. See [configuration and overrides](docs/configuration.md) for the broader CLI surface.

### Networks and commands

Solana is the default. Network selection updates both chain identity and Codex network ID; selecting a network does not guarantee coverage for every metric.

```bash
./target/release/memes_finder monitor --network solana
./target/release/memes_finder monitor --network base
./target/release/memes_finder monitor --network ethereum
./target/release/memes_finder monitor --network robinhood
./target/release/memes_finder monitor --network arc
./target/release/memes_finder monitor --debug
./target/release/memes_finder config-check --config /path/to/memes-finder.toml
```

`meme-monitor`, built from `app-cli`, is an alternative executable for the same CLI. Tools such as `range-check` run separate checks rather than another monitoring loop.

## Strategy reference

The compact multi-strategy format names **definitions** and **profiles** freely. `id` identifies an instance namespace; `kind` selects an implemented handler. Adding a new name does not install a new algorithm. Unknown fields are rejected.

### Small-cap pump example

The `pump_high` handler signals when the close has gained more than `min_gain_pct` over five completed one-minute intervals. The latest completed candle must be green and close above the previous close. All token filters also need to pass. This is an example for testing alerts; tune the limits before monitoring.

```toml
# Example pump screening for small-cap tokens. Telegram is configured separately.
logging = true
filter_interval_minutes = 3
timeframe_secs = 60
metrics_max_age_secs = 60

[filters]
market_cap_usd = { gt = 200000, lt = 3000000 }
total_liquidity_usd = { gt = 20000 }

[profiles.high.filters]
volume_5m_to_liquidity = { gt = 0.45 }

[[strategies]]
id = "pump"
kind = "pump_high"
[strategies.signal]
context_bars = 20
min_gain_pct = 10.0
```

This configuration creates the `pump-high` instance. Market-cap bounds are strict: $200,000 < MCap < $3,000,000. Total liquidity must exceed $20,000 and five-minute trading volume / liquidity must exceed 0.45. The ratio uses total traded volume; candle colour does not identify actual buyer-initiated volume.

### Common options and inheritance

| Scope / field | Default or bounds | Meaning |
|---|---|---|
| Root `logging` | `true` | Default diagnostic policy |
| Root `filter_interval_minutes` | Required; integer 1…1440 | Discovery cadence in minutes |
| Root `timeframe_secs` | Required | Candle resolution, independent of discovery cadence |
| Root `metrics_max_age_secs` | 60; 1…86400 | Maximum age of mandatory token metrics |
| Root `filters` | `{}` | Required AND bounds shared by all profiles |
| Root `profiles` | `{}` | Shared profile definitions |
| Root `strategies` | 1…32 unique definitions | At least one must be enabled |
| Definition `id` | Required | Unique namespace |
| Definition `kind` | Required | Existing compact handler selector |
| Definition `enabled` | `true` | Disabled definitions create no strategy/analyzer instances |
| Definition `logging` | Inherited | Overrides root logging |
| Definition `timeframe_secs` | Root value | Per-definition resolution, subject to handler restrictions |
| Definition `signal` | Required table | Handler-specific fields; omitted fields use handler defaults |
| Definition `profiles` | Shared profiles | Selects local profiles, inheriting missing fields from same-name shared profiles |
| Profile `logging` | Definition/root | Explicit profile setting has priority |
| Profile `filters` | Required after inheritance | AND bounds added to common filters |
| Profile `signal` | Omitted | Shallow signal override where the handler supports it |

Important rules:

- Local `profiles` selects **only** the listed profiles, not every shared profile.
- Missing profile fields inherit. An explicitly supplied local `filters` table replaces the shared profile's entire filters table; this is not recursive merge.
- A metric cannot appear in both common and profile filters.
- Each enabled definition needs 1…64 profiles after expansion. The monitor also limits total strategy instances to 64; definition/profile limits are not multiplicative capacity.
- IDs contain up to 96 ASCII characters from `a-z`, `0-9`, `.`, `_`, `-`, starting with a letter or digit. Generated namespaced IDs must fit the same limit.
- Cadence and metric freshness are not per-profile or per-definition fields.
- TOML has no `null` here: omit optional fields instead.

Domain resolution accepts 1…604800 seconds. The current Codex adapter explicitly maps `1`, `5`, `15`, `30`, `60`, `300`, `900`, `1800`, `3600`, `14400`, `43200`, `86400`, `604800`; another value may validate structurally but fail as unsupported when read. Individual handlers can impose tighter restrictions.

### Logging

Precedence is **explicit profile → definition → root → default `true`**. A shared profile's setting applies wherever it is used; a definition-local profile can override that setting without repeating its shared filters.

`logging=false` suppresses that instance's verdicts, debug/trace output, and unavailable/acceptance error text, including cycle JSON errors. When all instances are muted, discovery listings and candidate failure/deadline diagnostics are muted as well. It does not disable evaluation, signals, delivery, quarantine, or internal error counts. `--debug` does not override this setting.

Startup/shutdown, aggregate counters, Codex transport/quota, and discovery API failures are shared system diagnostics and remain visible. Console **signal delivery** is separate from diagnostic logging.

### Metric bounds

Each metric uses at least one of `gt`, `gte`, `lt`, `lte`. `gt` and `gte` cannot coexist, nor can `lt` and `lte`. Lower/upper bounds must form a nonempty interval.

```toml
# Independent syntax examples, not actual metric names:
strict_lower = { gt = 100 }
inclusive_lower = { gte = 100 }
strict_upper = { lt = 100 }
inclusive_upper = { lte = 100 }
interval = { gt = 100, lte = 200 }
exact = { gte = 0, lte = 0 }
```

In a real filters table, replace those illustrative names with supported keys:

| Metric key | Unit / meaning | Current Codex adapter |
|---|---|---|
| `market_cap_usd` | Circulating market cap, USD | `circulatingMarketCap` |
| `fdv_usd` | Fully diluted value, USD | `marketCap` |
| `total_liquidity_usd` | Provider-reported combined liquidity, USD | `totalLiquidityUsd`; not proof of exhaustive pool TVL |
| `pool_liquidity_usd` | Selected pool liquidity, USD | Pool snapshot, unavailable during token prefetch |
| `token_age_secs` | Exact token creation age | Unknown |
| `trading_age_secs` | Exact age since first pair | Unknown |
| `provider_seen_age_secs` | Age from provider token timestamp | `token.createdAt`, not an independent on-chain mint check |
| `top_pool_age_secs` | Provider top-pool age | Filter-result `createdAt` |
| `volume_1m_usd` | Token 1m volume, USD | Unknown |
| `volume_5m_usd` | Token 5m volume, USD | `volume5m` |
| `volume_1h_usd` | Token 1h volume, USD | `volume1` |
| `volume_6h_usd` | Token 6h volume, USD | Unknown |
| `volume_24h_usd` | Token 24h volume, USD | `volume24` |
| `volume_1h_to_liquidity` | 1h volume / total liquidity | Derived; unknown for zero liquidity |
| `volume_5m_to_liquidity` | 5m volume / total liquidity | Derived; unknown for zero liquidity |
| `total_fees_usd` | Lifetime fees, USD | Unknown |
| `total_fees_native` | Lifetime fees, whole native coins | Unknown |
| `total_fees_24h_usd` | 24h fees, USD | `totalFees24` |
| `dev_supply_pct` | Developer supply share, % | Provider field |
| `insider_supply_pct` | Insider supply share, % | Provider field |
| `top10_holders_supply_pct` | Supply share of the 10 largest addresses, % | Provider field |
| `top10_percent_holders_supply_pct` | Supply share of the largest 10% of all holder addresses, % | Unknown; different from 10 addresses |
| `sniper_count` | Wallet count | Provider field |
| `sniper_supply_pct` | Sniper supply share, % | Provider field |
| `phishing_wallet_pct` | Phishing-wallet share, % | Unknown |
| `bundler_supply_pct` | Bundler supply share, % | Provider field |
| `bundler_count` | Wallet count | Provider field |

Bounds must be finite and nonnegative; percentages are 0…100 and counts are integers. Missing, future, stale, or unsupported mandatory metrics produce `Unavailable`, not a pass or an assumed zero. Token volumes are not substituted with one pool's volumes. See [metric availability](docs/strategy-filters.md) for provider details.

### Pump signal options

| Field | Default | Meaning |
|---|---|---|
| `context_bars` | 20 | Requested context, integer 20…120 |
| `min_gain_pct` | 10.0 | Strict minimum close-to-close percentage gain over five minutes; finite and positive |

`timeframe_secs` must be 60 for `pump_high`. Six completed closes are needed to measure five full minutes. The handler examines the latest six candles even when more history is available. Incomplete, stale, gapped or missing-volume inputs cannot match. The underlying range and volume adapters share the existing planner; range confirmation is not a pump signal gate and RangeDetector is unchanged.

A successful decision emits `PUMP_HIGH`. Re-evaluation of the same completed candle is deduplicated per strategy/config/token/pool; a later qualifying candle can produce another alert. This state is in memory and is cleared on restart.

## Alerts, state, and polling

Alerts identify the strategy and chain, link the contract address to GMGN where supported, and include a token-specific `un.defined.fi` link. Defined paths are `sol`, `robinhood`, `base`, `arc`, `eth`; URLs are constructed without fetching pages. Created age uses provider token metadata and is shown as hours/days/months/years, or unknown if absent. It is not the alert timestamp or a pool-age substitute.

| State | Default storage |
|---|---|
| Candles and request cache | RAM; reused during the process, not persisted to SQLite |
| Computed analysis | Evaluation-local RAM cache |
| Watchlist | RAM; retained for the current process |
| Accepted signals, dedupe, delivery outbox | RAM |
| API consumption and quota-warning claims | `api-usage.sqlite`, survives restarts |

Stable signal IDs suppress duplicate acceptance for the same completed candle during the process lifetime. Restart clears deduplication state; global exactly-once remote delivery is not guaranteed.

Every successful discovery refresh screens returned tokens again. Only tokens with at least one `Watch`/`Match` remain admitted; rejected/unavailable tokens are dropped when no other profile qualifies. The compact watchlist capacity is 512. A full watchlist does not prevent fresh candidates from being evaluated or signaling. Tokens absent from the new top-100 page are not independently re-polled; their watch records expire by the application TTL. The monitor does not store a journal of every rejected candidate.

## Custom strategies

### Reuse an existing algorithm

Add another `[[strategies]]` with a unique `id`, a supported `kind`, independent `signal`, and the profiles you want. No rebuild is needed: config-check and restart are enough. Use `enabled=false` to disable computation or `logging=false` to mute diagnostics. A new algorithm outside the handler's schema needs a Rust plugin.

### Add a compile-time Rust plugin

1. Create `crates/strategy-custom/`; the workspace already includes `crates/*`. Depend on contracts such as `domain`, `strategy-api`, `plugin-api`, and `serde`. Add `analysis-api`/`analysis-model` when using analyzer bindings/results. Keep configuration/validation in `config.rs`, rules in `rules.rs`, and the adapter/public entry points in `lib.rs`.
2. Implement `Strategy` with a descriptor such as `strategy.custom`, instance ID, active/shadow mode, config digest, input requirements, screening, and evaluation. Use typed configuration, explicit defaults, and `deny_unknown_fields`.
3. Add an optional dependency and Cargo feature in `app-bootstrap/Cargo.toml`. Register the descriptor in `catalog()` and a typed factory arm in `build_strategy()` in `app-bootstrap/src/lib.rs`, under the matching feature.
4. Forward the feature through `app-cli`; enable it for the root binary's dependency or add root feature forwarding. A bootstrap-only feature does not enable itself in the root executable.
5. Start with an advanced configuration instance. To support a new compact `kind`, add expansion to `app-config/src/screener.rs`; extend namespacing in `strategy_set.rs` if introducing new binding names.
6. Ensure the engine validates your evidence and SignalService can build a valid signal draft for the decision. A novel signal path needs builder/dispatch changes, possibly new kinds/facts and rendering. Never call a sink directly from strategy code.
7. Test boundaries, missing/stale data, dedupe, failed acceptance, coexistence with other instances, disabled/logging policies, and engine-to-fake-sink delivery.

| Strategy methods | Responsibility |
|---|---|
| `descriptor`, `instance`, `mode`, `config_digest` | Implementation/instance/configuration identity |
| `prefilter` | Cheap candidate-only gate |
| `requirements` | Pool policy, candles, units, history, analyzer bindings |
| `discovery_token_filters` | Broad discovery filter pushdown |
| `uses_token_metrics`, `required_token_metrics` | Whether metrics are needed and the exact projection |
| `screen_token_metrics` | Screening before pool/candle I/O |
| `evaluate` | Decision with reasons and evidence, without delivery side effects |
| `token_filter_recheck_secs`, `next_market_check` | Scheduling hints; do not bypass discovery-driven scheduling |

The planner currently accepts required range and volume bindings with optional trend. An empty `analyses` list is not supported, even for a candle-only strategy. New schemas/DAGs require changes to `analysis-model`, `AnalysisValue`, planner/compute, `StrategyContext`, and evidence validation. Reusing supported inputs is simpler than extending these contracts.

This is an **advanced-config fragment**, not a standalone compact strategy file. It requires the custom plugin to be registered/compiled and referenced analyzers to exist in the advanced `[[plugins]]` configuration:

```toml
[[strategies]]
id = "custom-a"
plugin = "strategy.custom"
enabled = true
logging = false
mode = "shadow" # or "active"
[strategies.config]
range_analyzer = "range-input"
volume_analyzer = "volume-input"
# Additional fields belong to your own typed config.
```

```bash
# After implementing and registering your custom crate:
cargo test --offline -p strategy-custom
cargo test --offline -p strategy-engine -p monitor-runtime
cargo build --release --locked --bin memes_finder
```

There is no runtime library loading. Keep private thresholds in local configuration; a private algorithm in a separate crate still requires a build including that crate.

## Runtime state

Signals, duplicate suppression, the delivery outbox, candles and watchlist records live in memory. Restarting clears this state. API usage is persisted in SQLite through SQLx so per-key quota accounting survives restarts.

The workspace builds one binary, `memes_finder`. See [arch.md](arch.md) for module boundaries.
