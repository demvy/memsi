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
API key, and checks that one cycle finishes with zero HTTP attempts. Other CLI
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

The readability and testing pass was validated with Rust 1.98.1 and
`cargo-llvm-cov` 0.9.1:

| Check | Result |
| --- | --- |
| Workspace tests | 199 passed, up from 30 |
| LLVM function coverage | 1,035 / 1,183 (87.49%) |
| LLVM line coverage | 9,393 / 10,797 (87.00%) |
| Formatting | Passed |
| Clippy with warnings denied | Passed |
| Bootstrap without default features | Passed |

Inline test modules remain included in LLVM's totals. Function totals also
include closures, async wrappers, and compiler-generated entries. An audit of
zero-count locations, combining generic instantiations and excluding closures,
test helpers, and uninstantiated trait-default placeholders, found no uncovered
named production function locations. This is narrower than 100% LLVM function
coverage: error-path closures, transport-response paths, and some conditional
branches still have coverage gaps. Branch coverage was not measured.

Production readability edits preserve all non-whitespace source, including
string and character literals, before test modules in all 123 tracked crate
source files. Only test code and development dependencies introduce new logic.
