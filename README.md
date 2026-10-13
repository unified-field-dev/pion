# Pion

[![CI](https://github.com/unified-field-dev/pion/actions/workflows/ci.yml/badge.svg)](https://github.com/unified-field-dev/pion/actions/workflows/ci.yml)
[![License: MIT](https://img.shields.io/badge/License-MIT-blue.svg)](LICENSE)

[GitHub](https://github.com/unified-field-dev/pion) · `cargo doc -p pion --open`

## About

Pion is a cell control-plane library and slim server for fleet agents. It
persists node/cell inventory from heartbeats, runs a leased node-action queue,
enrolls hosts with one-time tickets, and delivers signed agent handoff directives
over a stable `/api/parton/*` HTTP surface. Storage is Valence: SQLite for an
embedded control plane in your host application, or distributed Postgres with an
in-memory cache.

- Heartbeat ingest: node/cell inventory, observed status, container observations.
- Node-action queue: enqueue, claim, lease, report and reconcile, with default-deny capabilities.
- Host enrollment: one-time tickets, with source-IP verification by default.
- Handoff directives: mint signed Parton directives and deliver them on heartbeat.
- Auth: fail-closed shared token or SPIFFE, rate and body limits, per-node binding.
- Secrets: `$secret_ref` placeholders in the queue, resolved only on claim through a host-supplied `SecretResolver`.
- Deploy helpers and projections: operator read models and deploy preflight checks.
- Metrics: Spectra helpers and Prometheus, plus an embeddable `parton_router` and the `pion-server` binary.

An orchestrator enqueues actions in-process. Optional peers plug in through
process-local hooks (`bootstrap_notify`, `registry_storage_notify`), so Pion
has no compile-time dependency on them. The Valence logical partition is named
`gluon` for historical reasons; treat it as an opaque storage key.

See [`pion/README.md`](pion/README.md) for crate-level API details.

## Examples

See [`pion/examples/README.md`](pion/examples/README.md) for the teaching path
(`parton_router_smoke` and `pion-server`).

## Getting started

You need a stable Rust toolchain and a network connection to fetch the git
dependencies.

1. Build and run the library tests (optional, but confirms the toolchain works):

   ```bash
   cargo test -p pion --features runtime
   ```

2. Start the server with a shared token that agents must present:

   ```bash
   PARTON_SHARED_TOKEN=dev-shared-token cargo run -p pion-server
   ```

   It listens on `127.0.0.1:3000` (override with `PION_BIND`) and creates its
   SQLite file at `data/pion.sqlite3` on first start (override with
   `VALENCE_SQLITE_PATH`).

3. In a second shell, confirm it is up:

   ```bash
   curl -s localhost:3000/health
   curl -s localhost:3000/cell/readiness
   ```

   `/health` returns `200 OK`. `/cell/readiness` returns `{"ready":true}` once
   the database is reachable and auth is configured. `curl -s localhost:3000/metrics`
   shows the Prometheus counters.

## Security

Vulnerability reporting: [`SECURITY.md`](SECURITY.md). Dependency policy:
[`docs/supply-chain.md`](docs/supply-chain.md). SPIFFE auth modes:
[`docs/spiffe.md`](docs/spiffe.md).

## Verify

CI runs on every push and PR ([`.github/workflows/ci.yml`](.github/workflows/ci.yml)):

```bash
cargo fmt --all --check
cargo check -p pion --features runtime
cargo check -p pion-server
cargo clippy -p pion --all-targets --features runtime -- -D warnings
cargo clippy -p pion-server --all-targets -- -D warnings
cargo test -p pion --features runtime
cargo deny check
```

The full command block is in [`docs/VERIFICATION.md`](docs/VERIFICATION.md).
To contribute, see [`CONTRIBUTING.md`](CONTRIBUTING.md).

## FAQ

**Is it production-ready?** Pion is v0.1.0 and experimental. Heartbeat ingest,
the node-action queue, enrollment, fail-closed agent auth, rate limits,
secret-ref indirection and optional SPIFFE verification are implemented and tested.

**Do I need Postgres?** No. SQLite (`VALENCE_SQLITE_PATH`) embeds the control plane
in your host process. For a shared fleet database, set `PION_STORAGE=hybrid` and
`DATABASE_URL` for distributed Postgres with an in-memory cache (`db-hybrid`).

**How do agents authenticate to Pion?** By default a shared token (`PARTON_SHARED_TOKEN`)
on every `/api/parton/*` route, plus `x-parton-node-id` binding. `pion-server` refuses
to start without the token unless `PION_ALLOW_INSECURE=1` (testing only). For production
fleets, enable SPIFFE (`PION_AUTH_MODE=dual`, then `spiffe`); see
[`docs/spiffe.md`](docs/spiffe.md).

**Where do secrets in queued actions come from?** Queued actions store `$secret_ref`
placeholders, not secret values. Values resolve only when an authenticated agent
claims the action, and are not written back into the queue.

**Where does Parton fit in?** Parton is the node agent on the other side of the HTTP
surface Pion serves. See [github.com/unified-field-dev/parton](https://github.com/unified-field-dev/parton).

**Where do orchestrators fit in?** An orchestrator enqueues node actions and may
register process-local hooks.

## License

MIT. See [LICENSE](LICENSE).
