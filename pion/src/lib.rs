//! Cell control plane for fleet agents: heartbeat ingest, leased node-action queue,
//! host enrollment, and signed handoff directives on a stable `/api/parton/*` surface.
//!
//! Enable the `runtime` Cargo feature for Valence persistence, Parton wire types, and the
//! Axum [`runtime::parton_router`]. Headless binaries use that router; host applications call
//! the same library APIs in-process. Orchestration peers register optional process-local hooks
//! ([`bootstrap_notify`], [`registry_storage_notify`]) without a compile-time dependency from
//! this crate.
//!
//! # Features
//!
//! - **Heartbeat ingest** — Turns each agent heartbeat into up-to-date node and cell inventory
//!   with [`ingest_agent_heartbeat`] / [`ingest_node_heartbeat`], so the control plane always
//!   knows which hosts are alive. [Heartbeat ingest](#heartbeat-ingest)
//! - **Node-action queue** — Lets the control plane enqueue work for one agent, which claims it
//!   under a lease and reports the result: [`enqueue_node_action`],
//!   [`claim_pending_node_action`], [`report_node_action_result`] and
//!   [`list_node_action_capability_map`]. [Node-action queue](#node-action-queue)
//! - **Host enrollment** — Issues one-time `ghe.<id>.<secret>` tickets with
//!   [`create_host_enrollment`] so a new host can join under strict enrollment; tickets are filed
//!   under a bootstrap session and listed with [`list_enrollments_for_bootstrap_session`].
//!   [Host enrollment](#host-enrollment)
//! - **Handoff directives** — Mints signed Parton directives with [`sign_re_enroll_directive`]
//!   and delivers them on the next heartbeat with [`collect_handoff_directives_for_heartbeat`]
//!   when a host must re-enroll elsewhere. [Handoff directives](#handoff-directives)
//! - **System-owned handoff schema** — Keeps authority handoff and agent directive rows
//!   `SYSTEM_ONLY` for create, update and delete, so interactive session actors cannot change
//!   them directly. [System-owned handoff rows](#system-owned-handoff-rows)
//! - **Secret-ref at claim** — Resolves `$secret_ref` placeholders only when an agent claims an
//!   action, through [`try_resolve_one_secret_ref`], [`resolve_secrets_in_json`] and
//!   [`SecretResolver`], so queued payloads never hold secret values.
//!   [Secret-ref at claim](#secret-ref-at-claim)
//! - **HTTP Parton surface** — Serves the agent API when you mount [`runtime::parton_router`]
//!   (see [`runtime::HasValenceRouter`]) on a host's router. [HTTP Parton surface](#http-parton-surface)
//! - **Auth, SPIFFE and rate limits** — Fails closed on agent routes in [`runtime`], accepting a
//!   token or a SPIFFE identity and rate-limiting callers.
//!   [Auth, SPIFFE, and rate limits](#auth-spiffe-and-rate-limits)
//! - **Spectra and Prometheus** — Emits product telemetry through [`logging`] and exposes scrape
//!   metrics through [`prom_metrics`]. [Spectra and Prometheus](#spectra-and-prometheus)
//! - **Orchestrator hooks** — Notifies an orchestrator in the same process about bootstrap and
//!   mount-stats events via [`bootstrap_notify`] and [`registry_storage_notify`], with no
//!   compile-time dependency on it. [Orchestrator hooks](#orchestrator-hooks)
//!
//! # Getting started
//!
//! Prerequisites: `pion` with `features = ["runtime"]`, and a Valence handle whose router already
//! registered Pion logical names (`default` + `gluon`) via
//! [`valence_bootstrap::bootstrap_valence`] / [`storage::BOOTSTRAP_LOGICAL_NAMES`].
//!
//! 1. Bootstrap Valence (`SQLite` memory for tests and local runs, or file / Hybrid for durable stores).
//! 2. Call [`ingest_agent_heartbeat`] with a [`parton::NodeHeartbeatReport`].
//! 3. Read [`parton::HeartbeatResponse::acknowledged_at`] — ingest succeeded when the call returns `Ok`.
//!
//! ```no_run
//! # #[cfg(feature = "runtime")]
//! # {
//! use pion::ingest_agent_heartbeat;
//! use parton::NodeHeartbeatReport;
//! # async fn demo(report: NodeHeartbeatReport, valence: &valence::Valence) -> anyhow::Result<()> {
//! let response = ingest_agent_heartbeat(&report, None, valence).await?;
//! assert!(response.acknowledged_at.timestamp() > 0);
//! # Ok(())
//! # }
//! # }
//! ```
//!
//! Runnable HTTP smoke (in-memory `SQLite` + insecure local auth):
//! `cargo run -p pion --example parton_router_smoke --features runtime`
//! (set `PION_ALLOW_INSECURE=1` and `PARTON_ENROLLMENT_STRICT_NEW_NODES=0`). Success stdout:
//! `parton_router_smoke: OK`.
//!
//! Operator binary: `cargo run -p pion-server` with `PARTON_SHARED_TOKEN` set (or
//! `PION_ALLOW_INSECURE=1` for local testing only). Vulnerability reporting:
//! [`SECURITY.md`](https://github.com/unified-field-dev/pion/blob/main/SECURITY.md). SPIFFE: [`docs/spiffe.md`](https://github.com/unified-field-dev/pion/blob/main/docs/spiffe.md).
//!
//! Next: pick a branching path from [Features](#features), or continue with the sections below.
//!
//! # Heartbeat ingest
//!
//! Heartbeat ingest provides acknowledged inventory updates from a Parton agent report:
//! node/cell upserts, observed status, enrollment side effects, and handoff delivery on the
//! response.
//!
//! Prerequisites: `runtime` feature; Valence bootstrapped with Pion logical names; for first-seen
//! nodes under strict enrollment, a valid `ghe.<id>.<secret>` token on the report (local testing:
//! `PION_ALLOW_INSECURE=1` + `PARTON_ENROLLMENT_STRICT_NEW_NODES=0`).
//!
//! 1. Build or receive a [`parton::NodeHeartbeatReport`].
//! 2. Call [`ingest_agent_heartbeat`] (HTTP / router path) or [`ingest_node_heartbeat`] (same core).
//! 3. On success, use [`parton::HeartbeatResponse`] (`acknowledged_at`, optional `directives`).
//!
//! Failures: Valence / serialization errors, or enrollment rejection when strict mode rejects a
//! first-seen node. Next: [Node-action queue](#node-action-queue) or [HTTP Parton surface](#http-parton-surface).
//!
//! ```no_run
//! # #[cfg(feature = "runtime")]
//! # {
//! use pion::ingest_agent_heartbeat;
//! use parton::NodeHeartbeatReport;
//! # async fn demo(report: NodeHeartbeatReport, valence: &valence::Valence) -> anyhow::Result<()> {
//! let response = ingest_agent_heartbeat(&report, None, valence).await?;
//! assert!(response.acknowledged_at.timestamp() > 0);
//! # Ok(())
//! # }
//! # }
//! ```
//!
//! # Host enrollment
//!
//! Host enrollment gives a new host a one-time ticket so its Parton agent can join the control
//! plane under strict enrollment, without a shared secret baked into the image. Each ticket is
//! filed under a bootstrap session id, so the installer or bootstrap run that minted the tickets
//! can later list exactly the hosts it brought up. An installer or bootstrap job creates one
//! ticket per host when it reserves that host, before the host first boots; the agent presents
//! the token on its first heartbeat.
//!
//! Prerequisites: `runtime` feature; Valence control-plane DB. Wire form is
//! `ghe.<enrollment_id>.<secret>` (shown once).
//!
//! 1. Call [`create_host_enrollment`] with expected agent host, cell, bootstrap session id, and
//!    TTL seconds.
//! 2. Hand the returned `wire_token` to the host (cloud-init or the operator); the agent presents
//!    it on heartbeat, which claims the ticket.
//! 3. List the session's tickets with [`list_enrollments_for_bootstrap_session`] to see which
//!    hosts have claimed theirs.
//!
//! ```no_run
//! # #[cfg(feature = "runtime")]
//! # {
//! use pion::{create_host_enrollment, list_enrollments_for_bootstrap_session};
//! use pion::generated::PionAgentHostEnrollmentStatus;
//! # async fn demo(valence: &valence::Valence) -> anyhow::Result<()> {
//! let (enrollment_id, wire_token) = create_host_enrollment(
//!     valence,
//!     "10.0.0.1".into(),
//!     "local-default".into(),
//!     "install-7".into(),
//!     3600,
//! ).await?;
//! assert!(!enrollment_id.is_empty());
//! assert!(wire_token.starts_with("ghe."));
//!
//! let tickets = list_enrollments_for_bootstrap_session(valence, "install-7").await?;
//! assert_eq!(tickets.len(), 1);
//! assert_eq!(*tickets[0].status(), PionAgentHostEnrollmentStatus::Pending);
//! # Ok(())
//! # }
//! # }
//! ```
//!
//! Failures: a blank bootstrap session id is rejected before anything is written, with an error
//! that downcasts to [`EnrollmentError::BootstrapSessionIdRequired`]; Valence upsert and
//! validation errors pass through. Listing a blank or unknown session id returns an empty list,
//! and tickets stored with no session id are listed under
//! [`DEFAULT_BOOTSTRAP_SESSION_RECORD_ID`]. Source-IP checks apply at verify time when
//! `PARTON_ENROLLMENT_VERIFY_SOURCE_IP` is on (default).
//!
//! ```no_run
//! # #[cfg(feature = "runtime")]
//! # {
//! use pion::{create_host_enrollment, EnrollmentError};
//! # async fn demo(valence: &valence::Valence) {
//! let err = create_host_enrollment(valence, String::new(), "local-default".into(), " ".into(), 3600)
//!     .await
//!     .unwrap_err();
//! assert!(matches!(
//!     err.downcast_ref::<EnrollmentError>(),
//!     Some(EnrollmentError::BootstrapSessionIdRequired)
//! ));
//! # }
//! # }
//! ```
//!
//! Next: [Heartbeat ingest](#heartbeat-ingest), where the agent's first heartbeat claims the
//! ticket.
//!
//! # Node-action queue
//!
//! Enqueue work for a Parton agent; the agent claims, executes, and reports.
//!
//! Prerequisites: `runtime` feature; target node already in inventory and deploy-eligible with the
//! action capability enabled ([`list_node_action_capability_map`]). Prefer
//! [`enqueue_node_action`] with non-empty `correlation_key` and `sequence >= 1`.
//!
//! 1. [`enqueue_node_action`] → deterministic `pna_…` command id.
//! 2. Agent path: [`claim_pending_node_action`] → execute → [`report_node_action_result`].
//! 3. Wire counterpart: `parton::post_claim_action` on `/api/parton/actions/claim`.
//!
//! Failures: [`NodeActionError`] (missing/ineligible node, capability denied, bad enqueue key,
//! Valence). Next: [Secret-ref at claim](#secret-ref-at-claim) when payloads carry `$secret_ref`.
//!
//! ```no_run
//! # #[cfg(feature = "runtime")]
//! # {
//! use pion::enqueue_node_action;
//! # async fn demo(valence: &valence::Valence) -> anyhow::Result<()> {
//! let command_id = enqueue_node_action(
//!     "node-1",
//!     "local-default",
//!     "health_check",
//!     serde_json::json!({ "url": "http://127.0.0.1:8080/health" }),
//!     3,
//!     Some("corr-health"),
//!     Some(1),
//!     valence,
//! ).await?;
//! assert!(command_id.starts_with("pna_"));
//! # Ok(())
//! # }
//! # }
//! ```
//!
//! Claim when the agent polls (empty queue → `Ok(None)`):
//!
//! ```no_run
//! # #[cfg(feature = "runtime")]
//! # {
//! use pion::claim_pending_node_action;
//! # async fn demo(valence: &valence::Valence) -> anyhow::Result<()> {
//! let claimed = claim_pending_node_action("node-1", 120, valence).await?;
//! match &claimed {
//!     Some(c) => assert!(c.command_id.starts_with("pna_")),
//!     None => assert!(claimed.is_none()),
//! }
//! # Ok(())
//! # }
//! # }
//! ```
//!
//! Report after the agent finishes:
//!
//! ```no_run
//! # #[cfg(feature = "runtime")]
//! # {
//! use pion::{report_node_action_result, ReportNodeActionResult};
//! # async fn demo(valence: &valence::Valence) -> anyhow::Result<()> {
//! report_node_action_result(
//!     ReportNodeActionResult {
//!         command_id: "pna_abc".into(),
//!         node_id: "node-1".into(),
//!         attempt: 1,
//!         success: true,
//!         stdout: None,
//!         stderr: None,
//!         error_summary: None,
//!         payload_json: None,
//!     },
//!     valence,
//! )
//! .await?;
//! // Observable: `Ok(())` — mismatches yield `NodeActionError`.
//! # Ok(())
//! # }
//! # }
//! ```
//!
//! # Secret-ref at claim
//!
//! Queued payloads store `$secret_ref` placeholders, not secret values. At claim, placeholders become
//! strings when a [`SecretResolver`] is installed.
//!
//! Wire shape (for example in a `command` array entry):
//! `{"$secret_ref": {"id": "neutrino_secret:…", "version": 1}, "field": "password"}`.
//!
//! Prerequisites: `runtime` feature; implement [`SecretResolver`] and optionally
//! [`set_default_secret_resolver`] so [`claim_pending_node_action`] can resolve nested markers.
//!
//! 1. Enqueue payload containing `$secret_ref` objects.
//! 2. Install resolver (process-wide or pass explicitly to [`try_resolve_one_secret_ref`]).
//! 3. Claim — or call [`try_resolve_one_secret_ref`] / [`resolve_secrets_in_json`] directly.
//!
//! Failures: missing resolver when placeholders are present (command marked `failed`), missing
//! secret/field, decryption errors. Next: [Node-action queue](#node-action-queue).
//!
//! ```no_run
//! # #[cfg(feature = "runtime")]
//! # {
//! use pion::{try_resolve_one_secret_ref, SecretResolver};
//! use async_trait::async_trait;
//! use serde_json::json;
//! use valence::Valence;
//!
//! struct StubResolver;
//! #[async_trait]
//! impl SecretResolver for StubResolver {
//!     async fn resolve_secret_root_field(
//!         &self,
//!         _valence: &Valence,
//!         _secret_id: &str,
//!         _version: i64,
//!         field: &str,
//!     ) -> Result<String, anyhow::Error> {
//!         Ok(format!("stub-{field}"))
//!     }
//! }
//!
//! # async fn demo(valence: &Valence) -> anyhow::Result<()> {
//! let marker = json!({
//!     "$secret_ref": { "id": "neutrino_secret:demo", "version": 1 },
//!     "field": "password",
//! });
//! let resolved = try_resolve_one_secret_ref(valence, &StubResolver, &marker).await?;
//! assert_eq!(resolved.as_deref(), Some("stub-password"));
//! # Ok(())
//! # }
//! # }
//! ```
//!
//! # Handoff directives
//!
//! Handoff directives provide signed re-enroll rows that heartbeat delivery returns on
//! [`parton::HeartbeatResponse::directives`].
//!
//! Prerequisites: signing seed and directive fields; Valence for mint/collect/ack paths.
//!
//! 1. [`sign_re_enroll_directive`] (or mint helpers that upsert a `Pending` row).
//! 2. Delivery: [`collect_handoff_directives_for_heartbeat`] during ingest.
//! 3. Ack: [`acknowledge_handoff_directives_for_heartbeat`] when the agent reports
//!    `applied_directive_token`. Agent-side apply: `parton::apply_directives`.
//!
//! Failures: Valence query/update errors; empty signature indicates misuse of inputs. Next:
//! [Heartbeat ingest](#heartbeat-ingest).
//!
//! ```no_run
//! use pion::sign_re_enroll_directive;
//! use chrono::Utc;
//!
//! # fn demo() {
//! let seed = [7u8; 32];
//! let now = Utc::now();
//! let signature = sign_re_enroll_directive(
//!     &seed,
//!     "directive-1",
//!     "node-1",
//!     "https://cp.example:3000",
//!     "authority-1",
//!     "base64-verify-key",
//!     "base64-ciphertext",
//!     "sha256-hex",
//!     now,
//!     now + chrono::Duration::hours(1),
//!     "authority-1",
//!     now,
//! );
//! assert!(!signature.is_empty());
//! # }
//! ```
//!
//! # System-owned handoff rows
//!
//! Authority handoff and agent re-enroll directive tables reject session-actor
//! writes. Valence schemas for `pion_control_plane_handoff`,
//! `pion_control_plane_handoff_migration`, and `pion_agent_handoff_directive`
//! declare `SYSTEM_ONLY` on create/update/delete (agent directives also on read).
//! Orchestrator / fleet-composer privileged-action workers start as System and write
//! these rows; product UI must queue a command rather than calling Valence upsert
//! under the user.
//!
//! **Prerequisites:** `runtime` feature; understand Valence privacy
//! `SYSTEM_ONLY` evaluators. Schema sources live under `pion/schemas/valence/`.
//!
//! ```rust,ignore
//! // Contract check: handoff schemas keep SYSTEM_ONLY mutation policies.
//! const HANDOFF_TABLES: &[&str] = &[
//!     "pion_control_plane_handoff",
//!     "pion_agent_handoff_directive",
//! ];
//! assert!(HANDOFF_TABLES.iter().all(|t| t.contains("handoff")));
//! // Policy shape (illustrative): create/update/delete allow: [SYSTEM_ONLY]
//! let policy = "SYSTEM_ONLY";
//! assert_eq!(policy, "SYSTEM_ONLY");
//! ```
//!
//! Interactive prepare/overlap/finalize under a user Valence fails closed at
//! persistence. Next: [Handoff directives](#handoff-directives) for signing, or the
//! host orchestrator's `privileged_action` System worker path.
//!
//! # HTTP Parton surface
//!
//! Mount agent ingest routes on an Axum `Router`: `/api/parton/heartbeat`,
//! `/api/parton/actions/claim`, `/api/parton/actions/result`,
//! `/api/parton/actions/extend-lease`.
//!
//! Prerequisites: `runtime` feature; implement [`runtime::HasValenceRouter`] on app state; set
//! `PARTON_SHARED_TOKEN` (or `PION_ALLOW_INSECURE=1` for local testing).
//!
//! 1. Bootstrap Valence ([`valence_bootstrap::bootstrap_sqlite_memory`] for tests).
//! 2. Implement [`runtime::HasValenceRouter`].
//! 3. `Router::new().merge(parton_router::<AppState>()).with_state(state)`.
//! 4. Agents POST JSON with `x-parton-node-id` (and token headers when secure).
//!
//! Runnable: `cargo run -p pion --example parton_router_smoke --features runtime` →
//! `parton_router_smoke: OK`. Long-running binary: `pion-server`. Failures: auth reject,
//! rate limit, body limit ([`runtime::AGENT_REQUEST_BODY_LIMIT_BYTES`]). Next:
//! [Auth, SPIFFE, and rate limits](#auth-spiffe-and-rate-limits).
//!
//! ```ignore
//! use axum::Router;
//! use pion::runtime::{parton_router, HasValenceRouter};
//!
//! fn mount<S: HasValenceRouter>(state: S) -> Router<S> {
//!     Router::new().merge(parton_router::<S>()).with_state(state)
//! }
//! ```
//!
//! # Auth, SPIFFE, and rate limits
//!
//! Agent routes fail closed without `PARTON_SHARED_TOKEN` unless `PION_ALLOW_INSECURE=1`.
//! Optional SPIFFE JWT-SVID via `PION_AUTH_MODE` (`shared_token` / `dual` / `spiffe`) and
//! `PION_SPIFFE_*`; see [`docs/spiffe.md`](https://github.com/unified-field-dev/pion/blob/main/docs/spiffe.md). Per-node binding uses `x-parton-node-id`. Rate limit
//! default: `PION_AGENT_RATE_LIMIT_PER_MIN` (120).
//!
//! Prerequisites: env configured before serving. Implementation lives under [`runtime`]
//! (`auth`, `spiffe`, `rate_limit` modules).
//!
//! 1. Set `PARTON_SHARED_TOKEN` (or `PION_ALLOW_INSECURE=1` for local testing only).
//! 2. Optionally set `PION_AUTH_MODE` / `PION_SPIFFE_*` for dual or SPIFFE-only.
//! 3. Serve [`runtime::parton_router`] (or `cargo run -p pion-server`).
//! 4. Probe `GET /cell/readiness` → `{"ready":true}` when Valence and auth config are up.
//!
//! Failures: missing token at startup (`pion-server` exits); agent requests rejected without
//! token / node binding; SPIFFE misconfig fails closed. Next:
//! [HTTP Parton surface](#http-parton-surface).
//!
//! ```text
//! PARTON_SHARED_TOKEN=dev-shared-token
//! # optional: PION_AUTH_MODE=dual  + PION_SPIFFE_* (see docs/spiffe.md in the repository)
//! cargo run -p pion-server
//! # then: curl -s localhost:3000/cell/readiness   → {"ready":true}
//! ```
//!
//! # Spectra and Prometheus
//!
//! Product Spectra emit helpers live under [`logging`]. Operational Prometheus counters/histograms
//! are recorded in library entry points via [`prom_metrics`] (for example
//! `pion_heartbeat_ingest_total`, `pion_node_action_claim_total`). Topic-name constants without
//! the full runtime stack: crate `pion-spectra-topics`.
//!
//! Prerequisites: `runtime` feature for emit helpers; a Prometheus recorder installed by the host
//! (`pion-server` installs one and serves `GET /metrics`).
//!
//! 1. Depend on `pion-spectra-topics` for wire names, or call [`logging`] emit helpers from `pion`.
//! 2. Use event/metric constants such as `pion_heartbeat_log` /
//!    `spectra.event.pion_heartbeat_log` and `spectra.metric.pion_heartbeats_ingested`.
//! 3. Scrape `GET /metrics` on `pion-server` for `pion_heartbeat_ingest_*` families.
//!
//! Failures: missing Prometheus recorder drops scrape series; wrong topic string breaks
//! subscribers. Next: crate `pion-spectra-topics` (runnable doctest).
//!
//! ```text
//! # wire names (also asserted in pion-spectra-topics doctests):
//! #   spectra.event.pion_heartbeat_log
//! #   spectra.metric.pion_heartbeats_ingested
//! # after cargo run -p pion-server:
//! curl -s localhost:3000/metrics | grep pion_heartbeat_ingest
//! ```
//!
//! # Orchestrator hooks
//!
//! Process-local hooks so hosts notify bootstrap pipelines and project mount stats without a
//! `pion → orchestrator` Cargo dependency.
//!
//! Prerequisites: Valence available; call installers once at startup.
//!
//! 1. [`bootstrap_notify::set_local_bootstrap_notifier`]: re-evaluate bootstrap correlations
//!    after node-action mutations ([`bootstrap_notify::LocalBootstrapNotify`]).
//! 2. [`registry_storage_notify::set_registry_storage_projector`]: project mount stats onto
//!    host storage rows ([`registry_storage_notify::RegistryStorageProjector`]).
//!
//! Default notifiers are no-ops. Failures stay inside the host notifier (must not fail the
//! node-action path). Next: [Node-action queue](#node-action-queue).
//!
//! ```ignore
//! use std::sync::Arc;
//! use pion::bootstrap_notify::{set_local_bootstrap_notifier, LocalBootstrapNotify};
//!
//! // After Valence bootstrap — install a real `LocalBootstrapNotify` impl:
//! set_local_bootstrap_notifier(Arc::new(MyNotify) as Arc<dyn LocalBootstrapNotify>);
//! ```
//!
//! # Binary runtimes
//!
//! - `pion-server`: slim headless ingest process (Parton routes + health/readiness/metrics).
//! - `pion` binary: stub entry for split deployments; prefer `pion-server` or embedding this
//!   library.
//!
//! # Integration checklist (host binary)
//!
//! 1. Register Valence logical names (`default` + `gluon`) via
//!    [`valence_bootstrap::bootstrap_valence`] / [`storage::BOOTSTRAP_LOGICAL_NAMES`].
//! 2. Register optional hooks ([`bootstrap_notify`], [`registry_storage_notify`]) once at startup.
//! 3. Mount [`runtime::parton_router`] or call [`ingest_agent_heartbeat`] /
//!    [`claim_pending_node_action`] from your own handlers.
//! 4. Set `PARTON_SHARED_TOKEN` (never ship `PION_ALLOW_INSECURE=1` on a reachable host).
//!
//! # Feature flags
//!
//! | Flag | Effect |
//! |------|--------|
//! | `runtime` | Full stack: Valence, Parton types, [`runtime`] Axum router, control-plane APIs |
//! | `db-hybrid` | Compile against distributed Postgres + in-memory cache (default storage is `SQLite`) |
//! | `chronon` | Implies `runtime`. Chronon script inventory and `scripts::register_default_jobs` for the expired-lease sweep |
//! | *(default, no `runtime`)* | Exports [`OBSERVED_STALE_AFTER_SECS`] only (WASM-friendly optional dep) |
//!
//! Re-exported Parton wire types (`NodeHeartbeatReport`, `AgentDirective`,
//! `ContainerActionKind`, …) inherit rustdoc from the `parton` crate.
//!
//! # Further reading
//!
//! - [`README.md`](https://github.com/unified-field-dev/pion/blob/main/pion/README.md): crate overview
//! - [`CONTRIBUTING.md`](https://github.com/unified-field-dev/pion/blob/main/CONTRIBUTING.md): lint policy and doc conventions
//! - [`docs/VERIFICATION.md`](https://github.com/unified-field-dev/pion/blob/main/docs/VERIFICATION.md): fmt, clippy, test and rustdoc gate
//! - [`examples/README.md`](https://github.com/unified-field-dev/pion/blob/main/pion/examples/README.md): `parton_router_smoke` teaching path

#![cfg_attr(all(feature = "runtime", doc), deny(rustdoc::broken_intra_doc_links))]

/// Stale threshold for observed-status freshness labels (seconds), aligned with cell reconcile.
///
/// Used by projection helpers and operator UIs to label snapshots as stale relative to wall clock.
pub const OBSERVED_STALE_AFTER_SECS: i64 = 120;

pub mod chronon_hooks;

#[cfg(feature = "runtime")]
pub mod generated;
#[cfg(feature = "runtime")]
pub mod logging;
#[cfg(feature = "runtime")]
pub mod prom_metrics;
#[cfg(feature = "runtime")]
mod schemas;
#[cfg(feature = "runtime")]
mod spectra_schemas;
#[cfg(feature = "runtime")]
pub mod storage;
#[cfg(feature = "runtime")]
pub mod valence_bootstrap;

#[cfg(feature = "runtime")]
pub mod bootstrap_notify;
#[cfg(feature = "runtime")]
mod control_plane;
#[cfg(feature = "runtime")]
mod photon_container_observation_event;
#[cfg(feature = "runtime")]
mod photon_host_enrollment_event;
#[cfg(feature = "runtime")]
pub mod registry_storage_notify;
#[cfg(feature = "runtime")]
pub use photon_container_observation_event::PionContainerStateChanged;
mod photon_setup_wizard_node_action_events;

#[cfg(feature = "runtime")]
pub mod runtime;
#[cfg(feature = "runtime")]
pub mod scripts;

#[cfg(feature = "runtime")]
pub use control_plane::*;
#[cfg(feature = "runtime")]
pub use photon_host_enrollment_event::publish_setup_wizard_host_enrollment_updated;
#[cfg(feature = "runtime")]
pub use photon_setup_wizard_node_action_events::maybe_publish_setup_wizard_tracked_photon;
#[cfg(feature = "runtime")]
pub use valence_bootstrap::{bootstrap_valence, bootstrap_valence_from_env, StorageProfile};
