//! Mount `parton_router` and POST one heartbeat (`SQLite` `:memory:`, insecure local auth).
//!
//! ## Command
//! ```bash
//! PION_ALLOW_INSECURE=1 PARTON_ENROLLMENT_STRICT_NEW_NODES=0 \
//!   cargo run -p pion --example parton_router_smoke --features runtime
//! ```
//!
//! ## Success
//! Stdout prints `parton_router_smoke: OK`.

#![allow(clippy::print_stdout, clippy::unwrap_used, clippy::expect_used)]

use std::net::SocketAddr;
use std::sync::Arc;

use axum::body::Body;
use axum::extract::connect_info::ConnectInfo;
use axum::http::{header, Request, StatusCode};
use axum::Router;
use chrono::Utc;
use pion::runtime::{parton_router, HasValenceRouter};
use pion::{ContainerStatusReport, ContainerStatusSummary, HostCapabilities, NodeHeartbeatReport};
use tower::ServiceExt;
use valence::DatabaseRouter;

const NODE_ID: &str = "example-node";

#[derive(Clone)]
struct AppState {
    router: Arc<DatabaseRouter>,
    default_backend_key: String,
}

impl HasValenceRouter for AppState {
    fn valence_router(&self) -> Arc<DatabaseRouter> {
        Arc::clone(&self.router)
    }

    fn default_backend_key(&self) -> &str {
        &self.default_backend_key
    }
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Testing only: no PARTON_SHARED_TOKEN; allow first-seen nodes without enrollment ticket.
    std::env::set_var("PION_ALLOW_INSECURE", "1");
    std::env::set_var("PARTON_ENROLLMENT_STRICT_NEW_NODES", "0");
    // SQLite + ownership get path (same as pion-testkit labs).
    std::env::set_var("VALENCE_OWNERSHIP_UNIFIED_FETCH", "0");
    std::env::set_var("VALENCE_OWNERSHIP_COLOCATE", "0");

    let boot = pion::valence_bootstrap::bootstrap_sqlite_memory().await?;
    let state = AppState {
        router: Arc::clone(&boot.router),
        default_backend_key: boot.default_backend_key.clone(),
    };

    let app = Router::new()
        .merge(parton_router::<AppState>())
        .with_state(state);

    let report = NodeHeartbeatReport {
        node_id: NODE_ID.to_string(),
        cell_id: "local-default".to_string(),
        capabilities: HostCapabilities {
            hostname: "example-node.local".to_string(),
            arch: "x86_64".to_string(),
            cpu_logical: 2,
            memory_bytes: 4 * 1024 * 1024 * 1024,
            mounts: vec![],
            labels: serde_json::json!({}),
        },
        containers: ContainerStatusReport {
            containers: vec![],
            summary: ContainerStatusSummary {
                running: 0,
                exited: 0,
                unhealthy: 0,
            },
        },
        observed_at: Utc::now(),
        enrollment_token: None,
        applied_directive_token: None,
        apply_failed: None,
    };

    let body = serde_json::to_vec(&report)?;
    let mut req = Request::builder()
        .method("POST")
        .uri("/api/parton/heartbeat")
        .header(header::CONTENT_TYPE, "application/json")
        .header("x-parton-node-id", NODE_ID)
        .body(Body::from(body))
        .expect("heartbeat request");
    req.extensions_mut()
        .insert(ConnectInfo(SocketAddr::from(([127, 0, 0, 1], 40_000))));

    let res = app.oneshot(req).await?;
    anyhow::ensure!(
        res.status() == StatusCode::OK,
        "heartbeat expected 200, got {}",
        res.status()
    );

    println!("parton_router_smoke: OK");
    Ok(())
}
