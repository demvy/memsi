# Workspace tests and coverage

Run the entire workspace, including CLI integration tests and optional plugins:

```sh
cargo fmt --all -- --check
cargo test --workspace --all-targets --all-features
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test -p app-bootstrap --no-default-features
```

Tests use deterministic market, provider, signal, and strategy fixtures in
`test-support/`. These modules are compiled only by tests; they add no public
runtime API. Coverage includes validation boundaries, serialization, arithmetic
overflow, dependency graphs, analysis, filtering, caching, admission, quotas,
checkpoint restoration, delivery settlement, and executable command handling.

The CLI monitor integration test clears its child environment, provides an empty
API key, and checks that monitoring exits with a redacted JSON error before
market-data requests. Other CLI
configuration tests explicitly disable Telegram. No real provider credentials or
remote notification destinations are needed.

## Coverage measurement

Install `cargo-llvm-cov` and the matching Rust LLVM component if needed:

```sh
rustup component add llvm-tools-preview
cargo install cargo-llvm-cov --locked
```

Generate a report without shared fixtures and standalone test files:

```sh
cargo llvm-cov --workspace --all-targets --all-features \
  --ignore-filename-regex 'test-support/|/tests\.rs|/tests/|_tests\.rs' \
  --json --output-path target/workspace-coverage.json
```

Coverage numbers must be regenerated for the current source tree. Removing unused
code changes the denominator; a historical percentage does not describe the
current build.

Inline test modules remain included in LLVM's totals. Function totals also
include closures, async wrappers, and compiler-generated entries. Tests of
removed modules are removed with those modules; regression tests for active
monitoring, cancellation, filtering, caching, quotas, and delivery remain.
