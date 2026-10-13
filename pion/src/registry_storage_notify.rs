//! Process-wide hook for projecting Parton mount stats onto host storage rows.
//!
//! Heartbeat ingest stays free of orchestrator compile-time deps. Hosts register one or more
//! implementations via [`register_registry_storage_projector`], for example one per
//! storage subsystem the host links.
//!
//! Failures in a projector must not fail the heartbeat; implementations should log and
//! return quietly. All registered projectors run (fan-out).

use std::sync::{Arc, Mutex, OnceLock};

use async_trait::async_trait;
use valence::Valence;

/// Projects `(mount_point, total_bytes, available_bytes)` onto storage fields in Valence.
#[async_trait]
pub trait RegistryStorageProjector: Send + Sync {
    /// Best-effort merge of mount stats into Valence (registry and/or stack volumes).
    async fn project_mounts(&self, valence: &Valence, node_id: &str, mounts: &[(String, u64, u64)]);
}

struct Noop;
#[async_trait]
impl RegistryStorageProjector for Noop {
    async fn project_mounts(
        &self,
        _valence: &Valence,
        _node_id: &str,
        _mounts: &[(String, u64, u64)],
    ) {
    }
}

static PROJECTORS: OnceLock<Mutex<Vec<Arc<dyn RegistryStorageProjector>>>> = OnceLock::new();

fn projectors() -> &'static Mutex<Vec<Arc<dyn RegistryStorageProjector>>> {
    PROJECTORS.get_or_init(|| Mutex::new(Vec::new()))
}

/// Register a projector (fan-out; multiple registrants are all invoked).
pub fn register_registry_storage_projector(p: Arc<dyn RegistryStorageProjector>) {
    if let Ok(mut guard) = projectors().lock() {
        guard.push(p);
    }
}

/// Install a projector. Prefer [`register_registry_storage_projector`] when composing
/// several projectors; this alias remains for existing call sites.
pub fn set_registry_storage_projector(p: Arc<dyn RegistryStorageProjector>) {
    register_registry_storage_projector(p);
}

/// Invoke all registered projectors (no-op when none registered).
pub(crate) async fn project_registry_storage_mounts(
    valence: &Valence,
    node_id: &str,
    mounts: &[(String, u64, u64)],
) {
    if mounts.is_empty() {
        return;
    }
    let list = match projectors().lock() {
        Ok(guard) => guard.clone(),
        Err(_) => return,
    };
    if list.is_empty() {
        let noop: Arc<dyn RegistryStorageProjector> = Arc::new(Noop);
        noop.project_mounts(valence, node_id, mounts).await;
        return;
    }
    for p in list {
        p.project_mounts(valence, node_id, mounts).await;
    }
}
