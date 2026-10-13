# Pion examples

Teaching path for mounting the Parton agent HTTP surface. These examples live on
the `pion` package.

| Example | Role |
|---------|------|
| `parton_router_smoke` | `bootstrap_sqlite_memory` + `parton_router` + one heartbeat |

## 1. Parton router — `parton_router_smoke`

Mount Parton agent routes in-process before you run `pion-server`.

```bash
PION_ALLOW_INSECURE=1 PARTON_ENROLLMENT_STRICT_NEW_NODES=0 \
  cargo run -p pion --example parton_router_smoke --features runtime
```

Success: stdout prints `parton_router_smoke: OK` (exit code 0).

API path exercised (in order):

1. `valence_bootstrap::bootstrap_sqlite_memory` — in-process SQLite router
2. `HasValenceRouter` — app state for the agent routes
3. `runtime::parton_router` — `/api/parton/heartbeat` (and claim/result/lease)
4. `POST /api/parton/heartbeat` — ingest one `NodeHeartbeatReport` via oneshot

Testing flags: `PION_ALLOW_INSECURE=1` skips `PARTON_SHARED_TOKEN`;
`PARTON_ENROLLMENT_STRICT_NEW_NODES=0` allows a first-seen node without an
enrollment ticket. Production hosts set a shared token and keep enrollment strict.

Look next at `parton_router_smoke.rs`, then `pion-server` for a long-running binary
with health/readiness.
