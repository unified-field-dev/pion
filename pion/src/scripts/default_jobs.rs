//! Pion Chronon default-job registration (feature `chronon`).
//!
//! Call [`register_default_jobs`] from the composite host at boot so attribute-declared
//! defaults (e.g. `pion.node_actions.expired_lease_sweep`) are upserted. Prefer calling
//! alongside Gluon's / Nucleus's registration when those crates are linked.

use std::sync::Arc;

use chronon_coordinator::ChrononCoordinatorBackend;
use valence::ValenceFactory;

/// Ensure link-time default jobs (including Pion `default_job` attributes).
///
/// # Examples
///
/// ```rust,ignore
/// use std::sync::Arc;
/// use pion::scripts::register_default_jobs;
///
/// # async fn boot(
/// #     backend: Arc<dyn chronon_coordinator::ChrononCoordinatorBackend>,
/// #     factory: Arc<dyn valence::ValenceFactory>,
/// # ) {
/// register_default_jobs(backend, factory).await;
/// # }
/// ```
pub async fn register_default_jobs(
    backend: Arc<dyn ChrononCoordinatorBackend>,
    factory: Arc<dyn ValenceFactory>,
) {
    register_embedded_default_jobs(backend, factory).await;
}

/// Ensure link-time default jobs discovered via Chronon inventory.
pub async fn register_embedded_default_jobs(
    backend: Arc<dyn ChrononCoordinatorBackend>,
    factory: Arc<dyn ValenceFactory>,
) {
    register_embedded_default_jobs_with_skip(backend, factory, &[]).await;
}

/// Like [`register_embedded_default_jobs`], but skips job names in `skip`.
pub async fn register_embedded_default_jobs_with_skip(
    backend: Arc<dyn ChrononCoordinatorBackend>,
    factory: Arc<dyn ValenceFactory>,
    skip: &[&str],
) {
    chronon_coordinator::register_default_jobs_embedded_with_skip(backend, factory, skip).await;
}
