# Verification

Re-run after code or doc changes. See [CONTRIBUTING.md](../CONTRIBUTING.md#rust-standards--lint-policy)
for the lint policy these commands enforce.

## Environment

Match the `env` block of [`.github/workflows/ci.yml`](../.github/workflows/ci.yml) so warnings
fail locally as they do in CI:

```bash
export RUSTFLAGS="-D warnings"
export PION_ALLOW_INSECURE=1
```

Toolchain: stable (same as CI).

## PR CI parity

Required PR jobs in `ci.yml` and the local commands that match them:

| CI job | Local command |
|--------|----------------|
| `fmt` | `cargo fmt --all --check` |
| `check` | `cargo check -p pion --features runtime` and `cargo check -p pion-server` |
| `clippy` | `cargo clippy -p pion --all-targets --features runtime -- -D warnings` and `cargo clippy -p pion-server --all-targets -- -D warnings` |
| `test` | `cargo test -p pion --features runtime`, `cargo test -p pion-server`, `cargo test -p pion-e2e`, `cargo test -p pion-spectra-topics` |
| `docs` | `RUSTDOCFLAGS="-D warnings" cargo doc -p pion --features runtime --no-deps` and `RUSTDOCFLAGS="-D warnings" cargo doc -p pion-spectra-topics --no-deps`, then `cargo test -p pion --doc --features runtime` |
| `deny` | `cargo deny check` |
| `coverage` | `cargo llvm-cov -p pion --features runtime --fail-under-lines 50 --summary-only` |
| `bench-smoke` | `cargo check -p pion-bench` |

## Commands

Run these in order; each must pass with no warnings (same block as PR CI):

```bash
# Formatting
cargo fmt --all --check

# Type/borrow check (workspace missing_docs = deny)
cargo check -p pion --features runtime
cargo check -p pion-server

# Full lint gate (lib + bins + tests), warnings are errors
cargo clippy -p pion --all-targets --features runtime -- -D warnings
cargo clippy -p pion-server --all-targets -- -D warnings

# Unit + integration tests
cargo test -p pion --features runtime
cargo test -p pion-server
cargo test -p pion-e2e
cargo test -p pion-spectra-topics

# API docs render cleanly (fail on rustdoc warnings)
RUSTDOCFLAGS="-D warnings" cargo doc -p pion --features runtime --no-deps
RUSTDOCFLAGS="-D warnings" cargo doc -p pion-spectra-topics --no-deps

# Doctests (rust,no_run compile-checked; runnable ones executed)
cargo test -p pion --doc --features runtime

# Dependency policy (advisories, licenses, sources) — see docs/supply-chain.md
cargo deny check
```

## Coverage & benches

```bash
# Coverage floor (ratchet toward 90%)
cargo llvm-cov -p pion --features runtime --fail-under-lines 50 --summary-only

# Criterion micro-benches
# cargo bench -p pion --features runtime --bench micro

# System bench CLI smoke (CI job bench-smoke)
cargo check -p pion-bench
```

## Notes

- Workspace `[workspace.lints.rust] missing_docs = "deny"` and `unsafe_code = "forbid"`;
  `[workspace.lints.rustdoc]` denies broken/private intra-doc links, invalid HTML/codeblock
  attributes, and missing crate-level docs; restriction lints (`unwrap_used`, `expect_used`,
  `print_stdout`, `print_stderr`, `dbg_macro`, `todo`, `unimplemented`) are enforced in
  `[workspace.lints.clippy]`.
- [`clippy.toml`](../clippy.toml) allows `unwrap` in tests; integration test files also
  carry a file-level allow where the config does not reach helper functions.
- Trait `# Contract` section: [`SecretResolver`](../pion/src/control_plane/secret_resolver.rs).
- Library logging goes through `tracing`; `pion-server` initializes a subscriber in
  `main.rs` (filter via `RUST_LOG`). The stub `pion` binary may print a one-shot message
  via a documented `#![allow(clippy::print_stderr)]`.
- Generated Valence models are excluded from docs/clippy via
  [`pion/src/generated.rs`](../pion/src/generated.rs).
