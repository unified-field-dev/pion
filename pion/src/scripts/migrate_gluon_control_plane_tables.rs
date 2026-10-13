//! Chronon-stable legacy table-rename no-op (former `gluon_*` → `pion_*` table copy).
//!
//! New fleets use `SQLite` or Hybrid Valence and do not need a table migration. The
//! entrypoint name stays so registered Chronon job names remain stable during upgrades.

use valence::Valence;

/// Legacy table-rename no-op; function name kept for Chronon job stability.
///
/// # Errors
///
/// Never fails.
#[allow(clippy::unused_async)] // stable async entrypoint for Chronon/tests
pub async fn migrate_gluon_control_plane_tables_to_pion(_valence: Valence) -> anyhow::Result<()> {
    tracing::info!(
        "[migrate_gluon_control_plane_tables_to_pion] no-op: legacy table migration removed; \
         use SQLite or Hybrid Valence for new deployments"
    );
    Ok(())
}
