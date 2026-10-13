//! Matrix-driven **correctness** integration tests for Pion + Parton.
//!
//! The library crate has no public API. Scenarios live under `tests/` and exercise
//! `pion_testkit::ScenarioRunner` against an in-process control plane.
//!
//! # Getting started
//!
//! Prerequisites: a checkout of the workspace (see the verification guide at
//! [`docs/VERIFICATION.md`](https://github.com/unified-field-dev/pion/blob/main/docs/VERIFICATION.md)).
//!
//! ```bash
//! cargo test -p pion-e2e
//! ```
//!
//! Observable outcome: tests pass (exit 0). Optional Docker-gated scenarios require
//! `PION_E2E_DOCKER=1` (see `tests/local_docker.rs`).
//!
//! Harness types: crate `pion-testkit` (`BootstrapSession`, `ScenarioSpec`).

#![deny(missing_docs)]
