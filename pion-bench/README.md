# pion-bench

Synthetic latency and throughput benchmarks for Pion control-plane + Parton agent flows.

| Document | Role |
|----------|------|
| [`PERFORMANCE.md`](PERFORMANCE.md) | Measured latency and throughput, capacity guidance, how to read the results |

Local JSON under `pion-bench/reports/` stays gitignored (see root `.gitignore`).

## Run (local)

From the pion repo root:

```bash
# List experiments
cargo run -p pion-bench -- experiments

# Heartbeat round-trip smoke (in-process stub)
cargo run -p pion-bench -- run --experiment bm-r0

# Warm stub deploy and teardown latency
cargo run -p pion-bench -- run --experiment bm-r1 --ops 5
cargo run -p pion-bench -- run --experiment bm-r2 --ops 5

# Firehose stub (N=10, c=2)
cargo run -p pion-bench -- run --experiment bm-rf0
```

Decision-grade numbers come from a dedicated `c6i.large`-class host with a warm process; see
the hardware notes in [`PERFORMANCE.md`](PERFORMANCE.md).

## Related

- Harness: [`pion-testkit`](../pion-testkit)
- Correctness: [`pion-e2e`](../pion-e2e/README.md)
