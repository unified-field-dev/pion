# Contributing to pion

`pion` is the cell control-plane library for Unified Field fleets (heartbeat ingest, node-action
queue, host enrollment, handoff directives). It ships the `pion` library, a stub `pion` binary, the
headless `pion-server` binary, and `pion-spectra-topics` (stable Spectra topic-name constants,
kept in sync with `pion`'s registered schemas by a dedicated test).

## Documentation

When you change public API behavior, configuration, or wiring steps:

1. Update rustdoc on the affected symbols (the workspace enforces `missing_docs = "deny"`).
2. Public functions returning `Result` need a `# Errors` section describing failure modes.
3. Trait methods with meaningful semantics carry a `# Contract` subsection (invariants, caller
   obligations, no-op behavior) — see [`SecretResolver`](pion/src/control_plane/secret_resolver.rs)
   and [`LocalBootstrapNotify`](pion/src/bootstrap_notify.rs).
4. Prefer `# Examples` on key entry points
   ([`ingest_agent_heartbeat`](pion/src/control_plane/heartbeat.rs),
   [`claim_pending_node_action`](pion/src/control_plane/node_actions/claim.rs),
   [`enqueue_node_action`](pion/src/control_plane/node_actions/enqueue.rs)).
5. Run the verification block in [`docs/VERIFICATION.md`](docs/VERIFICATION.md) before opening a PR.

### Style

- Organize crate-root docs by task (heartbeat ingest, node actions, enrollment, runtime mount).
- Put full code snippets on the item that owns the API; the crate root links without duplicating.
- Generated Valence models under [`pion/src/generated.rs`](pion/src/generated.rs) are excluded from
  missing-docs via an explicit allow block — do not edit generated output by hand.

## Rust standards & lint policy

Workspace lints live in the root [`Cargo.toml`](Cargo.toml) (`[workspace.lints.*]`).
Library and binary crates inherit them via `[lints] workspace = true`. CI-equivalent gate:

```bash
cargo clippy -p pion --all-targets --features runtime -- -D warnings
cargo clippy -p pion-server --all-targets -- -D warnings
```

### Enforced (restriction / correctness)

| Lint | Level | Intent |
|------|-------|--------|
| `clippy::unwrap_used` / `expect_used` | deny | No silent panics in library/runtime paths |
| `clippy::dbg_macro` | deny | No leftover debug macros |
| `clippy::print_stdout` / `print_stderr` | deny | Prefer `tracing` in library/binary paths |
| `clippy::todo` / `unimplemented` | deny | No placeholders in shipped code |
| `clippy::too_many_arguments` | deny | Group related args (canonical wire structs may `#[allow]` with a reason) |
| `clippy::unnested_or_patterns` | deny | Collapse `A \| B` patterns |
| `rust.missing_docs` | deny | Public API docs |
| `rust.unsafe_code` | forbid | No `unsafe` |
| `rustdoc::broken_intra_doc_links` | deny | No broken `[…]` links in rustdoc |
| `rustdoc::private_intra_doc_links` | deny | No links to private items from public docs |
| `rustdoc::invalid_html_tags` | deny | Valid HTML in rustdoc |
| `rustdoc::invalid_codeblock_attributes` | deny | Valid codeblock attributes |
| `rustdoc::missing_crate_level_docs` | deny | Crate-level `//!` required |

Pedantic is enabled at warn (CI denies via `-D warnings`); nursery is allowed.
`too_many_lines` and `cognitive_complexity` are deny. Prefer extracting helpers, and use
`#[allow(...)]` with a one-line reason only when a refactor would risk behavior change.

### Logging

Replace `println!` / `eprintln!` in library and long-running binary paths with `tracing`
(`tracing::info!` / `warn!` / `error!`). `pion-server` initializes a `tracing-subscriber` in
`main.rs` (filter via `RUST_LOG`). The stub `pion` binary is allowed to print a one-shot operator
message until process wiring lands.

### Tests

Unit tests (`#[cfg(test)]`) and integration tests under `pion/tests/` may use `unwrap` / `expect`;
[`clippy.toml`](clippy.toml) allows this, and integration test files carry a file-level
`#![allow(clippy::unwrap_used, clippy::expect_used)]` where the config does not reach helper fns.
Do not re-add broad workspace `allow`s for restriction lints without discussion.

### Error handling

Prefer typed errors (`thiserror`) for public matchable APIs, such as
[`EnrollmentError`](pion/src/control_plane/agent_enrollment/error.rs) and
[`NodeActionError`](pion/src/control_plane/node_actions/error.rs), and `anyhow::Result` with
`.context()` at Valence/HTTP glue boundaries. Document failure modes in `# Errors`.

### Structure budgets

Production module files: soft max 500 LOC, hard max 800 LOC (excluding `#[cfg(test)]`
and generated code). Prefer domain `mod` directories; no kitchen-sink `utils`. Clippy denies
`too_many_lines` (≤120), `too_many_arguments` (≤7), and `cognitive_complexity`.

### Telemetry

- Logging: `tracing` + `#[instrument]` on handlers / claim / ingest (filter via `RUST_LOG`).
- Spectra metrics and events NDJSON remains the product telemetry path.
- Prometheus: `pion-server` exposes `GET /metrics` (`pion_heartbeat_ingest_total`,
  `pion_node_action_claim_total`, `pion_node_action_report_total`, plus duration histograms).

### Supply chain

Use **cargo-deny** (`deny.toml` + CI `deny` job) for advisories/licenses/sources — this satisfies
the RustSec advisory gate (no separate `cargo-audit` job). See
[`docs/supply-chain.md`](docs/supply-chain.md).

### Coverage & benches

- CI `coverage` job: `cargo llvm-cov -p pion --features runtime --fail-under-lines 50` (baseline ~53%; ratchet toward 90%).
- Criterion micro-benches: `cargo bench -p pion --features runtime --bench micro`.
- System latency benchmarks: [`pion-bench`](pion-bench/) (manual/release; CI only `cargo check -p pion-bench`).
  SLOs / methodology: [`pion-bench/PERFORMANCE.md`](pion-bench/PERFORMANCE.md).

## CI

CI runs on every push and PR ([`.github/workflows/ci.yml`](.github/workflows/ci.yml)):

```bash
cargo fmt --all --check
cargo check -p pion --features runtime
cargo check -p pion-server
cargo clippy -p pion --all-targets --features runtime -- -D warnings
cargo clippy -p pion-server --all-targets -- -D warnings
cargo test -p pion --features runtime
cargo deny check
cargo llvm-cov -p pion --features runtime --fail-under-lines 50 --summary-only
cargo check -p pion-bench
```

## Verification

See [`docs/VERIFICATION.md`](docs/VERIFICATION.md) for the full command block.

## Security (contributors)

Follow [`SECURITY.md`](SECURITY.md) for vulnerability reporting when changing auth,
enrollment, enqueue, or agent HTTP surfaces. SPIFFE guide: [`docs/spiffe.md`](docs/spiffe.md).
