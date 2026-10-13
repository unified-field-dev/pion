# Pion performance

These figures were measured on AWS same-region hosts (`c6i.large`, hybrid storage, Docker CLI
executor) and on multi-region (WAN) hosts. The `pion-bench` CLI in this repository runs the same
scenarios; see [`README.md`](README.md) for the commands.

## Latency and throughput

| Scenario | Experiment id | Result |
|----------|---------------|--------|
| Warm deploy to Ready (n=10) | `bm-d1` | p50 / p95 of 308 / 538 ms |
| Same warm deploy on a kind Kubernetes cluster | n/a | p50 / p95 of 949 / 1925 ms |
| Teardown to Gone | `bm-d2` | about 174 / 184 ms |
| Firehose of 50 starts at concurrency 4 | `bm-df0` | about 2 starts per second per agent host |

Same-region control-plane and agent layouts set the baseline for ingest and comparison runs.
WAN paths add RTT-dominated cost, so quote WAN numbers only for products that span regions.
Nomad warm time-to-ready rows from a single-node `-dev` cluster are not decision-grade.

## Guidance

Use the same-region AWS figures for default capacity planning. Add WAN deltas to them as a
latency budget. These runs did not validate projections at 5k vCPUs or 200 regions.

## How to read these results

AWS hardware labels are the capacity reference. Runs on the in-process harness check
correctness and serve as smoke tests; they say nothing about capacity. The figures above came
from external runs and are not reproduced by CI.
