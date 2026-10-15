# Configuration

Run commands from the project root. `.env` is resolved from the working directory; process environment values take precedence. Keep CODEX_API_KEY, TELEGRAM_BOT_TOKEN, and TELEGRAM_CHAT_ID local. The .env.example contains placeholders only.

`memes-finder.toml` selects strategies.toml, watchlist retention, Telegram delivery, fallback Codex key references, and quota accounting. The default config disables Telegram. Enable it after supplying your bot token and chat ID.

The quota database is separate from signal state and stores hashed API key identities. Set reset_day to your subscription boundary. When starting with an already-used API key, configure initial_period and api_usage.initial_used as described in README.md; the application cannot infer usage recorded elsewhere.

`strategies.toml` contains a small-cap pump example. Adjust its metrics and signal settings before monitoring. Each strategy/profile evaluates independently, and logging inherits from the root unless explicitly overridden.

```bash
cargo build --release --locked --bin memes_finder
./target/release/memes_finder config-check
./target/release/memes_finder plugins
./target/release/memes_finder strategies
./target/release/memes_finder --network solana monitor
```

Configuration inspection commands are offline. Monitoring performs real read-only Codex requests and consumes subscription quota. When Telegram is enabled, matching signals are sent to the configured chat.

```bash
# Environment override; CLI --set takes precedence.
export MEME__MONITOR__CANDIDATE_WORKERS=1
./target/release/memes_finder monitor --set monitor.candidate_workers=2
./target/release/memes_finder config-check --config path/to/program.toml
```

The default network is Solana. The CLI also accepts the supported Robinhood, Base, Ethereum, and Arc identifiers described in README.md; actual data availability depends on Codex. The private strategy plugin, RPC adapters, HTTP server, replay, trading tools, and optional durable signal backend are not included.
