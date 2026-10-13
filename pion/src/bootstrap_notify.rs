//! Process-wide hook for local bootstrap notifications from node-action paths.
//!
//! Node-action reconciliation can advance bootstrap pipelines that a host started elsewhere, so a
//! host can register a notifier to re-evaluate them when a node action changes state.
//! Hosts register an implementation at process startup via [`set_local_bootstrap_notifier`]
//! (keeps orchestration peers out of this crate's dependency graph).
//!
//! # Integration
//!
//! Call your host's hook installer once after Valence is available. The default notifier is a
//! no-op, which keeps unit tests and minimal binaries quiet.
//!
//! ```ignore
//! // In server startup, after Valence bootstrap:
//! my_host::install_pion_hooks();
//! ```
//!
//! [`LocalBootstrapNotify`] is implemented by the host/orchestrator wiring that registers the hook.

use std::sync::{Arc, OnceLock};

use async_trait::async_trait;
use valence::Valence;

/// Notifies local bootstrap flows when a bootstrap-scoped correlation key should be re-evaluated.
#[async_trait]
pub trait LocalBootstrapNotify: Send + Sync {
    /// Invoked after node-action mutations tied to `correlation_key` (best-effort).
    ///
    /// # Contract
    ///
    /// Callers may invoke this on every relevant queue mutation. Implementations must tolerate
    /// duplicate keys, ignore unknown correlations, and must not fail the node-action path
    /// (errors stay inside the host notifier). The default install is a no-op.
    async fn notify_if_wizard_run(&self, valence: &Valence, correlation_key: &str);
}

struct Noop;
#[async_trait]
impl LocalBootstrapNotify for Noop {
    async fn notify_if_wizard_run(&self, _valence: &Valence, _correlation_key: &str) {}
}

static NOTIFIER: OnceLock<Arc<dyn LocalBootstrapNotify>> = OnceLock::new();

/// Install the global notifier (idempotent after first successful `set`; subsequent calls are ignored
/// by [`OnceLock`], so keep this at a single well-defined startup site).
pub fn set_local_bootstrap_notifier(n: Arc<dyn LocalBootstrapNotify>) {
    let _ = NOTIFIER.set(n);
}

fn notifier() -> Arc<dyn LocalBootstrapNotify> {
    NOTIFIER.get().cloned().unwrap_or_else(|| Arc::new(Noop))
}

pub(crate) async fn notify_if_wizard_run(valence: &Valence, correlation_key: &str) {
    notifier()
        .notify_if_wizard_run(valence, correlation_key)
        .await;
}
