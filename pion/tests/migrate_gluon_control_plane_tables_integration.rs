//! Integration tests for [`pion::scripts::migrate_gluon_control_plane_tables`].
//!
//! The legacy table-copy path was removed; the entrypoint is a no-op on `SQLite`/Hybrid.

use pion::scripts::migrate_gluon_control_plane_tables::migrate_gluon_control_plane_tables_to_pion;

mod common;

#[tokio::test]
async fn migrate_is_noop_on_sqlite() -> anyhow::Result<()> {
    let v = common::test_valence("migrate_noop").await;
    migrate_gluon_control_plane_tables_to_pion(v).await?;
    Ok(())
}
