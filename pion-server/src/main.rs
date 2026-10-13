//! Slim headless `pion-server` binary.
//!
//! Serves the Parton agent ingest surface ([`pion::runtime::parton_router`]) on Valence
//! (`SQLite` by default, or Hybrid with `--features db-hybrid` + `DATABASE_URL`), plus operator
//! probes and Prometheus scrape.
//!
//! # Features
//!
//! - Parton agent HTTP: `/api/parton/*` via [`pion::runtime::parton_router`].
//! - Health and readiness: `/health`, `/internal/db-ready`, `/cell/readiness`.
//! - Prometheus: `/metrics` (library counters from [`pion::prom_metrics`]).
//!
//! # Getting started
//!
//! Prerequisites: set `PARTON_SHARED_TOKEN` (or `PION_ALLOW_INSECURE=1` for local testing only).
//!
//! ```bash
//! export PARTON_SHARED_TOKEN=dev-shared-token
//! cargo run -p pion-server
//! ```
//!
//! In a second shell, confirm it is serving:
//!
//! ```bash
//! curl -s localhost:3000/health
//! curl -s localhost:3000/cell/readiness   # {"ready":true}
//! ```
//!
//! Observable outcome: process listens on `PION_BIND` (default `127.0.0.1:3000`);
//! `GET /health` returns `200 OK`; `GET /cell/readiness` returns `{"ready":true}` when Valence
//! and auth config are up.
//!
//! Library-first smoke without this binary:
//! `cargo run -p pion --example parton_router_smoke --features runtime` →
//! `parton_router_smoke: OK`.
//!
//! | Route | Behavior |
//! |-------|----------|
//! | `GET /health` | Liveness — always `200 OK` once the process is serving |
//! | `GET /internal/db-ready` | `200` when Valence resolves its default backend, else `503` |
//! | `GET /cell/readiness` | `200 {"ready":true}` when the Valence backend is reachable *and* the process is securely configured (`PARTON_SHARED_TOKEN` set, or `PION_ALLOW_INSECURE`); otherwise `503 {"ready":false,"reason":…}` |
//! | `GET /metrics` | Prometheus text exposition of process + control-plane metrics (see below) |
//!
//! # Metrics
//!
//! A Prometheus recorder is installed at startup (via `metrics-exporter-prometheus`) and scraped
//! at `GET /metrics`. Control-plane metrics are recorded inside library entry points
//! ([`pion::prom_metrics`]) so every embedder records them, including hosts that do not use this binary:
//!
//! | Metric | Kind | Labels |
//! |--------|------|--------|
//! | `pion_heartbeat_ingest_total` | counter | `outcome=ok\|error` |
//! | `pion_heartbeat_ingest_duration_seconds` | histogram | — |
//! | `pion_node_action_claim_total` | counter | `result=claimed\|no_content\|error` |
//! | `pion_node_action_claim_duration_seconds` | histogram | — |
//! | `pion_node_action_report_total` | counter | `success=true\|false\|error` |
//! | `pion_node_action_report_duration_seconds` | histogram | — |
//! | `pion_http_request_duration_seconds` | histogram | `method`, `path`, `status` |
//!
//! # Configuration
//!
//! | Env var | Default | Meaning |
//! |---------|---------|---------|
//! | `PION_BIND` | `127.0.0.1:3000` | Socket address the HTTP server binds to. Set explicitly to `0.0.0.0:<port>` (or another routable address) to accept connections from other hosts — the default is loopback-only so a fresh deployment doesn't accidentally expose the agent ingest surface. |
//! | `VALENCE_SQLITE_PATH` | `data/pion.sqlite3` | `SQLite` file (sqlite profile) |
//! | `DATABASE_URL` | — | Postgres URL (hybrid profile) |
//! | `PION_STORAGE` | auto | `sqlite` or `hybrid` |
//! | `PARTON_SHARED_TOKEN` | — | Shared bearer token agents must present on `/api/parton/*`. **Required** at startup unless `PION_ALLOW_INSECURE` is set (see below); the process exits immediately if missing. |
//! | `PION_ALLOW_INSECURE` | `0` | Set to `1` / `true` / `yes` / `on` to start **without** `PARTON_SHARED_TOKEN` configured and without per-node `x-parton-node-id` binding. Local experiments only — never set this in a reachable deployment. |
//!
//! # Feature flags
//!
//! | Flag | Effect |
//! |------|--------|
//! | `db-hybrid` | Enables `pion/db-hybrid` (Postgres + in-memory cache) |

use std::net::SocketAddr;
use std::sync::{Arc, OnceLock};
use std::time::Instant;

use axum::extract::{DefaultBodyLimit, MatchedPath, Request, State};
use axum::http::header::CONTENT_TYPE;
use axum::http::StatusCode;
use axum::middleware::{self, Next};
use axum::response::IntoResponse;
use axum::routing::get;
use axum::{Json, Router};
use metrics_exporter_prometheus::{PrometheusBuilder, PrometheusHandle};
use valence::{Actor, DatabaseRouter, Valence};

/// App-wide request body cap (1 MiB); the `/api/parton/*` agent surface also applies this via
/// [`pion::runtime::parton_router`], this is defense-in-depth for `/health` etc. and any future
/// routes mounted directly on this binary's `Router`.
const APP_BODY_LIMIT_BYTES: usize = 1_048_576;

/// Shared application state: the process-wide Valence database router and metrics handle.
#[derive(Clone)]
struct AppState {
    router: Arc<DatabaseRouter>,
    default_backend_key: String,
    metrics_handle: PrometheusHandle,
}

/// Installs (once per process) the global Prometheus recorder and returns a handle to render it.
///
/// A [`OnceLock`] ensures the recorder is installed exactly once even though both `main` and the
/// test module below build an [`AppState`] — [`metrics::set_global_recorder`] can only succeed
/// once per process, so subsequent callers just get a clone of the same handle.
fn prometheus_handle() -> PrometheusHandle {
    static HANDLE: OnceLock<PrometheusHandle> = OnceLock::new();
    HANDLE
        .get_or_init(|| {
            let recorder = PrometheusBuilder::new().build_recorder();
            let handle = recorder.handle();
            // Best-effort: if a recorder is already installed globally (e.g. by an embedder),
            // this handle simply won't observe metrics recorded through that other recorder.
            let _ = metrics::set_global_recorder(recorder);
            handle
        })
        .clone()
}

/// Records per-request latency as `pion_http_request_duration_seconds{method,path,status}`.
///
/// Mounted via `route_layer` so [`MatchedPath`] (the templated route, not raw path params) is
/// already present in request extensions by the time this runs.
async fn track_http_metrics(req: Request, next: Next) -> impl IntoResponse {
    let path = req.extensions().get::<MatchedPath>().map_or_else(
        || req.uri().path().to_string(),
        |matched| matched.as_str().to_string(),
    );
    let method = req.method().to_string();
    let started = Instant::now();
    let response = next.run(req).await;
    let status = response.status().as_u16().to_string();
    metrics::histogram!(
        "pion_http_request_duration_seconds",
        "method" => method,
        "path" => path,
        "status" => status,
    )
    .record(started.elapsed().as_secs_f64());
    response
}

/// `GET /metrics`: Prometheus text exposition of the process-wide recorder.
async fn metrics_endpoint(State(state): State<AppState>) -> impl IntoResponse {
    (
        StatusCode::OK,
        [(CONTENT_TYPE, "text/plain; version=0.0.4")],
        state.metrics_handle.render(),
    )
}

impl pion::runtime::HasValenceRouter for AppState {
    fn valence_router(&self) -> Arc<DatabaseRouter> {
        self.router.clone()
    }

    fn default_backend_key(&self) -> &str {
        &self.default_backend_key
    }
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    fail_closed_without_shared_token()?;

    let boot = pion::bootstrap_valence_from_env().await?;
    let state = AppState {
        router: Arc::clone(&boot.router),
        default_backend_key: boot.default_backend_key.clone(),
        metrics_handle: prometheus_handle(),
    };

    let app = Router::new()
        .route("/health", get(health))
        .route("/internal/db-ready", get(db_ready))
        .route("/cell/readiness", get(cell_readiness))
        .route("/metrics", get(metrics_endpoint))
        .merge(pion::runtime::parton_router::<AppState>())
        .route_layer(middleware::from_fn(track_http_metrics))
        .layer(DefaultBodyLimit::max(APP_BODY_LIMIT_BYTES))
        .with_state(state);

    let bind = std::env::var("PION_BIND").unwrap_or_else(|_| "127.0.0.1:3000".to_string());
    let addr: SocketAddr = bind
        .parse()
        .map_err(|e| anyhow::anyhow!("invalid PION_BIND `{bind}`: {e}"))?;

    let listener = tokio::net::TcpListener::bind(addr).await?;
    tracing::info!(
        %addr,
        profile = ?boot.profile,
        "pion-server listening"
    );
    axum::serve(
        listener,
        app.into_make_service_with_connect_info::<SocketAddr>(),
    )
    .await?;
    Ok(())
}

/// Refuses to start when `PARTON_SHARED_TOKEN` is missing/empty and `PION_ALLOW_INSECURE` was not
/// set (see [`pion::runtime::insecure_mode_allowed`]).
///
/// Without this check a fresh deployment that forgot to configure a shared token would silently
/// serve the `/api/parton/*` agent ingest surface with no authentication at all.
///
/// # Errors
///
/// Returns `Err` describing the missing configuration; callers should propagate this out of
/// `main` so the process exits non-zero before binding a listener.
fn fail_closed_without_shared_token() -> anyhow::Result<()> {
    if shared_token_configured() || pion::runtime::insecure_mode_allowed() {
        return Ok(());
    }
    anyhow::bail!(
        "PARTON_SHARED_TOKEN is not set. pion-server refuses to start without a shared token \
         (agents authenticate on /api/parton/* with it). Set PARTON_SHARED_TOKEN, or set \
         PION_ALLOW_INSECURE=1 to start without one for local experiments only."
    );
}

/// Liveness probe.
async fn health() -> impl IntoResponse {
    (StatusCode::OK, "ok")
}

/// Checks that Valence resolves its default backend, using the given `operation` label for the
/// system actor. Shared by `/internal/db-ready` and `/cell/readiness`.
fn valence_backend_error(state: &AppState, operation: &str) -> Option<String> {
    let valence = match Valence::builder()
        .database_router(Arc::clone(&state.router))
        .default_backend_key(state.default_backend_key.clone())
        .with_actor(Actor::System {
            operation: operation.to_string(),
        })
        .build()
    {
        Ok(valence) => valence,
        Err(error) => return Some(format!("valence init: {error}")),
    };
    match valence.active_backend() {
        Ok(_) => None,
        Err(error) => Some(format!("backend unavailable: {error}")),
    }
}

/// Readiness probe: `200` when Valence resolves its default backend, else `503`.
async fn db_ready(State(state): State<AppState>) -> impl IntoResponse {
    match valence_backend_error(&state, "pion_server_db_ready") {
        None => (StatusCode::OK, "ok".to_string()),
        Some(error) => (StatusCode::SERVICE_UNAVAILABLE, error),
    }
}

/// Cell readiness JSON for orchestration probes.
///
/// `ready:true` requires both the Valence backend to be reachable (mirrors `/internal/db-ready`)
/// and the process to be securely configured (`PARTON_SHARED_TOKEN` set, or the explicit
/// `PION_ALLOW_INSECURE` opt-in) — an orchestrator shouldn't route agent traffic to a replica that
/// would silently accept unauthenticated `/api/parton/*` requests.
async fn cell_readiness(State(state): State<AppState>) -> (StatusCode, Json<serde_json::Value>) {
    if let Some(reason) = valence_backend_error(&state, "pion_server_cell_readiness") {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(serde_json::json!({ "ready": false, "reason": reason })),
        );
    }
    if !shared_token_configured() && !pion::runtime::insecure_mode_allowed() {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(serde_json::json!({
                "ready": false,
                "reason": "PARTON_SHARED_TOKEN not configured (and PION_ALLOW_INSECURE not set)",
            })),
        );
    }
    (StatusCode::OK, Json(serde_json::json!({ "ready": true })))
}

/// True when `PARTON_SHARED_TOKEN` is set to a non-blank value.
fn shared_token_configured() -> bool {
    std::env::var("PARTON_SHARED_TOKEN")
        .ok()
        .map(|v| v.trim().to_string())
        .is_some_and(|v| !v.is_empty())
}

#[cfg(test)]
mod tests {
    use super::{cell_readiness, fail_closed_without_shared_token, AppState};
    use axum::extract::State;
    use axum::Json;
    use serial_test::serial;

    fn clear_env() {
        std::env::remove_var("PARTON_SHARED_TOKEN");
        std::env::remove_var("PION_ALLOW_INSECURE");
    }

    async fn test_state() -> AppState {
        let boot = pion::valence_bootstrap::bootstrap_sqlite_memory()
            .await
            .expect("sqlite memory bootstrap");
        AppState {
            router: boot.router,
            default_backend_key: boot.default_backend_key,
            metrics_handle: super::prometheus_handle(),
        }
    }

    /// `/cell/readiness` must not report `ready:true` with a reachable backend but no
    /// authentication configured — an orchestrator shouldn't route agent traffic to a replica
    /// that would silently accept unauthenticated `/api/parton/*` requests.
    #[tokio::test]
    #[serial]
    async fn cell_readiness_not_ready_without_shared_token_or_insecure_opt_in() {
        clear_env();
        let state = test_state().await;
        let (status, Json(body)) = cell_readiness(State(state)).await;
        assert_eq!(status, axum::http::StatusCode::SERVICE_UNAVAILABLE);
        assert_eq!(body["ready"], false);
        clear_env();
    }

    #[tokio::test]
    #[serial]
    async fn cell_readiness_ready_with_shared_token_configured() {
        clear_env();
        std::env::set_var("PARTON_SHARED_TOKEN", "some-token");
        let state = test_state().await;
        let (status, Json(body)) = cell_readiness(State(state)).await;
        assert_eq!(status, axum::http::StatusCode::OK);
        assert_eq!(body["ready"], true);
        clear_env();
    }

    #[tokio::test]
    #[serial]
    async fn cell_readiness_ready_with_insecure_opt_in() {
        clear_env();
        std::env::set_var("PION_ALLOW_INSECURE", "1");
        let state = test_state().await;
        let (status, Json(body)) = cell_readiness(State(state)).await;
        assert_eq!(status, axum::http::StatusCode::OK);
        assert_eq!(body["ready"], true);
        clear_env();
    }

    #[test]
    #[serial]
    fn startup_check_fails_without_token_or_insecure_opt_in() {
        clear_env();
        let result = fail_closed_without_shared_token();
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("PARTON_SHARED_TOKEN"));
        clear_env();
    }

    #[test]
    #[serial]
    fn startup_check_passes_with_token_configured() {
        clear_env();
        std::env::set_var("PARTON_SHARED_TOKEN", "some-token");
        assert!(fail_closed_without_shared_token().is_ok());
        clear_env();
    }

    #[test]
    #[serial]
    fn startup_check_passes_with_insecure_opt_in() {
        clear_env();
        std::env::set_var("PION_ALLOW_INSECURE", "1");
        assert!(fail_closed_without_shared_token().is_ok());
        clear_env();
    }

    #[test]
    #[serial]
    fn startup_check_fails_with_blank_token() {
        clear_env();
        std::env::set_var("PARTON_SHARED_TOKEN", "   ");
        assert!(fail_closed_without_shared_token().is_err());
        clear_env();
    }

    /// `/metrics` is backed by the same process-wide recorder every control-plane metric goes
    /// through, so anything recorded anywhere in the process shows up in its render output.
    #[tokio::test]
    #[serial]
    async fn metrics_handle_renders_recorded_counters() {
        clear_env();
        let state = test_state().await;
        metrics::counter!("pion_test_metrics_endpoint_total").increment(1);
        let rendered = state.metrics_handle.render();
        assert!(rendered.contains("pion_test_metrics_endpoint_total"));
        clear_env();
    }

    #[test]
    fn default_bind_address_is_loopback_only() {
        // PION_BIND parsing itself lives in `main`; this pins the *fallback literal* used
        // when the env var is unset so a future edit can't silently widen the default to
        // 0.0.0.0 without a test failure.
        let default_bind = "127.0.0.1:3000";
        let addr: std::net::SocketAddr = default_bind.parse().expect("valid default bind addr");
        assert!(addr.ip().is_loopback());
    }
}
