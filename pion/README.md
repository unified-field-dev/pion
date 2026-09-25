# Pion

**Pion (`pion`) — cell control plane for Parton agents**

Pion owns Valence models and logic for heartbeats, node/cell state, the node-action queue,
agent host enrollment, and operator projections. HTTP paths for agents stay on the Parton
protocol surface (`/api/parton/*`); this crate supplies the handlers and types hosts mount.

Optional orchestration peers (image catalog, builds, HAProxy authority handoff) integrate via
process-local hooks — see [`bootstrap_notify`](src/bootstrap_notify.rs) and
[`registry_storage_notify`](src/registry_storage_notify.rs).

Prefer `pion-server` for the slim headless ingest binary in this workspace.

## `pion` binary

The `pion` binary (`cargo run -p pion --features runtime`) is an **intentional stub**: it prints a
one-shot message and exits with status 1 until a thin split-control-plane process wrapper is
wired. Prefer `pion-server` for the supported headless ingest process, or embed the `pion`
library from a composite Leptos `server` binary.

## Documentation

- **Rust API docs:** `cargo doc -p pion --features runtime --no-deps --open` (crate root summarizes wiring and links to this README).
- **Examples:** [`examples/README.md`](examples/README.md).
- **Security:** [`../SECURITY.md`](../SECURITY.md) — vulnerability reporting. SPIFFE: [`../docs/spiffe.md`](../docs/spiffe.md).
- **SPIFFE rollout:** [`../docs/spiffe.md`](../docs/spiffe.md).
- **Supply chain:** [`../docs/supply-chain.md`](../docs/supply-chain.md).

## Testing

- **Unit + integration:** `cargo test -p pion --features runtime`
- **Spectra instrumentation:** `pion/tests/instrumentation_operations.rs` (metrics/events emit via `RecordingSink`).
- **Migration script:** `pion/tests/migrate_gluon_control_plane_tables_integration.rs` (Chronon no-op entrypoint for retired Surreal copy).
- **Chronon expired-lease sweep (feature `chronon`):** script
  `pion_node_actions_expired_lease_sweep` / job `pion.node_actions.expired_lease_sweep`; hosts call
  `pion::scripts::register_default_jobs` at boot.
- **Heartbeat ingest:** `pion/tests/heartbeat_enrollment_integration.rs` (enrollment token → observed status).

## Spectra logging

Product-owned metrics and event logs live under `pion/schemas/spectra/` and emit via `pion::logging`.
Query in Spectra with filters such as `pion_node_action_log`, `pion_heartbeat_log`, and
`pion_handoff_log`. Headless `pion-server` installs an NDJSON sink at boot (`pion::logging::sink`).
