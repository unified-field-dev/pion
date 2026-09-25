//! Chronon cron: sweep expired node-action leases (and stale pending) on a schedule.
//!
//! Maps [`crate::chronon_hooks::EXPIRED_LEASE_SWEEP`] onto a real
//! `#[chronon_coordinator_macros::script]` with `default_job` so hosts that call
//! [`super::register_default_jobs`] get the job upserted at boot.
//!
//! Body: [`crate::reconcile_node_action_commands`] — same reconcile used by Gluon
//! `process_bootstraps` and safe to keep as a hot-path safety net elsewhere.

use anyhow::Result;

/// Job name / cron match [`crate::chronon_hooks::EXPIRED_LEASE_SWEEP`].
///
/// # Errors
///
/// Propagates Valence context extraction failures and
/// [`crate::reconcile_node_action_commands`] errors.
#[chronon_coordinator_macros::script(
    name = "pion_node_actions_expired_lease_sweep",
    default_job(job = "pion.node_actions.expired_lease_sweep", cron = "*/2 * * * *")
)]
pub async fn pion_node_actions_expired_lease_sweep(
    ctx: Box<dyn chronon_core::ScriptContext>,
) -> Result<()> {
    let valence = chronon_valence_identity::valence_from_context(&*ctx)?;
    crate::reconcile_node_action_commands(&valence).await?;
    tracing::info!(
        target: "pion.node_actions",
        operation = "expired_lease_sweep",
        outcome = "ok",
        "expired lease / stale pending reconcile finished"
    );
    Ok(())
}
