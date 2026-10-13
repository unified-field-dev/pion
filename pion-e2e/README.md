# pion-e2e

Matrix-driven correctness integration tests for Pion control-plane + Parton agent flows.

| | `pion/tests` | `pion-e2e` |
|---|---|---|
| Scope | Fast, crate-scoped | Cross-cutting matrix via `pion-testkit` |
| Topology | In-process Valence mem | `isolated-harness` (default) + gated `local-docker` |
| Assertions | Direct library calls | Declarative [`ScenarioRunner`](../pion-testkit) |

Performance budgets belong in [`pion-bench`](../pion-bench/README.md).

## Run

From the pion repo root:

```bash
# Default: in-process harness + stub executor (must pass)
cargo test -p pion-e2e

# LocalDocker (ignored unless Docker is available)
PION_E2E_DOCKER=1 cargo test -p pion-e2e -- --ignored
```

## Coverage (skeleton)

| Scenario | isolated-harness / stub | local-docker |
|----------|---------------------|--------------|
| `enroll_deploy_smoke` | ✓ | ignored gate |
| matrix / scenario JSON roundtrip | ✓ | — |

## Related

- Harness: [`pion-testkit`](../pion-testkit)
- Benchmarks: [`pion-bench`](../pion-bench/README.md)
