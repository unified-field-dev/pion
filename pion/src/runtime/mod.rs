//! Parton agent HTTP surface (`/api/parton/*`) for headless control-plane binaries.
//!
//! Mount [`parton_router`] after Valence bootstrap. Routes:
//! `/api/parton/heartbeat`, `/api/parton/actions/claim`, `/api/parton/actions/result`,
//! `/api/parton/actions/extend-lease`. Auth, SPIFFE, and rate limits live in the submodules
//! below. Crate-root guide: [HTTP Parton surface](crate#http-parton-surface).
//!
//! Runnable smoke: `cargo run -p pion --example parton_router_smoke --features runtime`
//! → stdout `parton_router_smoke: OK`.

use std::sync::Arc;

use axum::extract::DefaultBodyLimit;
use axum::routing::post;
use axum::Router;

mod auth;
mod handlers;
mod rate_limit;
mod spiffe;

pub use auth::insecure_mode_allowed;

use handlers::{
    ingest_agent_heartbeat_handler, parton_claim_action_handler,
    parton_extend_action_lease_handler, parton_report_action_handler,
};

/// State surface required by [`parton_router`].
pub trait HasValenceRouter: Clone + Send + Sync + 'static {
    /// Returns the process-wide Valence [`DatabaseRouter`](valence::DatabaseRouter) for agent routes.
    ///
    /// # Contract
    ///
    /// The router must already have Pion logical names registered
    /// ([`crate::storage::BOOTSTRAP_LOGICAL_NAMES`]).
    fn valence_router(&self) -> Arc<valence::DatabaseRouter>;

    /// Default Valence backend key (`{engine}:default`) matching the booted storage profile.
    fn default_backend_key(&self) -> &str;
}

/// Max accepted request body size on the agent ingest surface (1 MiB).
///
/// Heartbeat / claim / result / extend-lease bodies are small JSON documents; capping body size
/// bounds memory used decoding an inbound request before any auth or rate-limit check runs.
pub const AGENT_REQUEST_BODY_LIMIT_BYTES: usize = 1_048_576;

/// Mount Parton agent ingest routes (heartbeat, action claim/result/extend-lease).
///
/// Prerequisites: `S: `[`HasValenceRouter`] with Pion logical names registered; set
/// `PARTON_SHARED_TOKEN` or `PION_ALLOW_INSECURE=1` (local testing only).
///
/// # Examples
///
/// ```ignore
/// use axum::Router;
/// use pion::runtime::{parton_router, HasValenceRouter};
///
/// fn mount<S: HasValenceRouter>(state: S) -> Router<S> {
///     Router::new().merge(parton_router::<S>()).with_state(state)
/// }
/// ```
///
/// Observable outcome in the workspace smoke: HTTP 200 and
/// `parton_router_smoke: OK` from `examples/parton_router_smoke.rs`.
pub fn parton_router<S>() -> Router<S>
where
    S: HasValenceRouter,
{
    Router::new()
        .route(
            "/api/parton/heartbeat",
            post(ingest_agent_heartbeat_handler::<S>),
        )
        .route(
            "/api/parton/actions/claim",
            post(parton_claim_action_handler::<S>),
        )
        .route(
            "/api/parton/actions/result",
            post(parton_report_action_handler::<S>),
        )
        .route(
            "/api/parton/actions/extend-lease",
            post(parton_extend_action_lease_handler::<S>),
        )
        .layer(DefaultBodyLimit::max(AGENT_REQUEST_BODY_LIMIT_BYTES))
}

#[cfg(test)]
mod tests {
    use super::AGENT_REQUEST_BODY_LIMIT_BYTES;

    #[test]
    fn agent_request_body_limit_is_one_mebibyte() {
        assert_eq!(AGENT_REQUEST_BODY_LIMIT_BYTES, 1_048_576);
    }
}
