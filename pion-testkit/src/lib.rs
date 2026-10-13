//! Shared test/bench harness for Pion control-plane + Parton agent flows.
//!
//! [`BootstrapSession`] installs Valence (`SQLite` or Hybrid) and optionally an Axum
//! [`pion::runtime::parton_router`] on an ephemeral port for a [`MatrixSpec`] row.
//! [`ScenarioRunner`] drives multi-step correctness scenarios used by `pion-e2e` and
//! `pion-bench`.
//!
//! # Features
//!
//! - Matrix rows: [`MatrixSpec`] axes (topology, executor, backend, hardware).
//! - Session bootstrap: [`BootstrapSession`] provides Valence plus an optional
//!   Parton router for one matrix row ([Bootstrap a session](#bootstrap-a-session)).
//! - Scenarios: [`ScenarioSpec`] / [`ScenarioStep`] declarative flows.
//! - Runner: [`ScenarioRunner`] executes a scenario against a session
//!   ([Getting started](#getting-started)).
//! - Bench image: [`bench_image_ref`] / [`resolve_image_digest`] for deploy steps.
//! - Remote Parton client: [`RemoteParton`] / [`RemoteAgentConfig`] POST to
//!   `{base_url}/agent/execute` ([Remote Parton](#remote-parton)).
//!
//! # Getting started
//!
//! Prerequisites: depend on `pion-testkit` (pulls `pion` with `runtime`).
//!
//! 1. Build a [`MatrixSpec`] (or use [`MatrixSpec::isolated_harness_stub`]).
//! 2. [`BootstrapSession::new`] → [`BootstrapSession::install`].
//! 3. [`ScenarioRunner::run`] with [`ScenarioSpec::enroll_deploy_smoke`] and
//!    [`RunMode::Correctness`].
//! 4. Assert [`ScenarioResult::error`] is `None`.
//!
//! ```no_run
//! use pion_testkit::{
//!     BootstrapSession, MatrixSpec, RunMode, ScenarioRunner, ScenarioSpec,
//! };
//!
//! # async fn demo() -> anyhow::Result<()> {
//! let mut session = BootstrapSession::new(MatrixSpec::isolated_harness_stub())?;
//! session.install().await?;
//! let runner = ScenarioRunner::new(&session);
//! let result = runner
//!     .run(&ScenarioSpec::enroll_deploy_smoke(), RunMode::Correctness)
//!     .await?;
//! assert!(result.error.is_none());
//! # Ok(())
//! # }
//! ```
//!
//! # Bootstrap a session
//!
//! Session bootstrap provides a ready Valence handle (and optional ephemeral Parton
//! router) for one [`MatrixSpec`] row so e2e and bench scenarios can call control-plane APIs
//! without hand-wiring storage.
//!
//! Prerequisites: Tokio runtime; optional `db-hybrid` Cargo feature + `DATABASE_URL` for Hybrid.
//!
//! 1. Choose a [`MatrixSpec`] row.
//! 2. [`BootstrapSession::new`] then [`BootstrapSession::install`].
//! 3. Use [`BootstrapSession`] with [`ScenarioRunner`], or call control-plane APIs against the
//!    session Valence handle.
//!
//! Failures: Valence bootstrap errors, bind failures for the ephemeral router. Next:
//! [Getting started](#getting-started).
//!
//! ```no_run
//! use pion_testkit::{BootstrapSession, MatrixSpec};
//!
//! # async fn demo() -> anyhow::Result<()> {
//! let mut session = BootstrapSession::new(MatrixSpec::isolated_harness_stub())?;
//! session.install().await?;
//! assert!(session.is_ready());
//! assert!(session.valence("docs-bootstrap").is_ok());
//! # Ok(())
//! # }
//! ```
//!
//! # Remote Parton
//!
//! HTTP client for an out-of-process agent that exposes `POST /agent/execute` (client + contract
//! tests today; Parton agents are primarily pull-based on `/actions/claim`).
//!
//! Prerequisites: `PION_REMOTE_PARTON_URL` (optional token `PION_REMOTE_PARTON_TOKEN`).
//!
//! 1. [`RemoteParton::from_env`] — `Ok(None)` when URL unset.
//! 2. [`RemoteParton::execute_action`] with a `parton::ContainerActionRequest`.
//! 3. Read `response.success`.
//!
//! ```no_run
//! use pion_testkit::RemoteParton;
//!
//! # async fn demo(request: &parton::ContainerActionRequest) -> anyhow::Result<()> {
//! if let Some(client) = RemoteParton::from_env()? {
//!     let response = client.execute_action(request).await?;
//!     assert!(!client.base_url().is_empty());
//!     // Live-agent oracle: `response.success == true` when execute completed.
//!     assert!(response.success);
//! }
//! # Ok(())
//! # }
//! ```
//!
//! # Feature flags
//!
//! | Flag | Effect |
//! |------|--------|
//! | **`db-hybrid`** | Forwarded to `pion/db-hybrid` for Hybrid Valence bootstrap |

#![deny(missing_docs)]

pub mod bench_image;
pub mod bootstrap;
pub mod matrix;
pub mod remote_parton;
pub mod runner;
pub mod scenario;

pub use bench_image::{
    bench_image_ref, resolve_image_digest, BENCH_IMAGE_ENV, DEFAULT_BENCH_IMAGE,
};
pub use bootstrap::BootstrapSession;
pub use matrix::{Backend, Executor, Hardware, MatrixSpec, TelemetryAdapter, Topology};
pub use remote_parton::{
    RemoteAgentConfig, RemoteParton, EXECUTE_PATH, REMOTE_PARTON_TOKEN_ENV, REMOTE_PARTON_URL_ENV,
};
pub use runner::{RunMode, ScenarioResult, ScenarioRunner, StepTiming};
pub use scenario::{ScenarioSpec, ScenarioStep};
