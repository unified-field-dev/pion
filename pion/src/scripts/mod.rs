//! Maintenance scripts for the Pion control-plane domain.
//!
//! Each script is a plain `async fn` (see [`migrate_gluon_control_plane_tables`]) callable directly
//! by operators or wrapped by a downstream scheduler. With feature `chronon`, Chronon inventory
//! scripts (module `expired_lease_sweep`) register via `register_default_jobs` at host boot.
//!
//! # Available scripts
//!
//! | Script | Purpose |
//! |--------|---------|
//! | [`migrate_gluon_control_plane_tables`] | **No-op** Chronon-stable job name (legacy Surreal table copy removed) |
//! | `expired_lease_sweep` (feature `chronon`) | Cron `*/2 * * * *` → [`crate::reconcile_node_action_commands`] |
//!
//! # Testing
//!
//! Integration tests live under `pion/tests/` (for example
//! `migrate_gluon_control_plane_tables_integration.rs`) and call the plain `async fn` entrypoints
//! directly.

pub mod migrate_gluon_control_plane_tables;

#[cfg(feature = "chronon")]
pub mod default_jobs;
#[cfg(feature = "chronon")]
pub mod expired_lease_sweep;

#[cfg(feature = "chronon")]
#[doc(inline)]
pub use default_jobs::{
    register_default_jobs, register_embedded_default_jobs, register_embedded_default_jobs_with_skip,
};
#[cfg(feature = "chronon")]
#[doc(inline)]
pub use expired_lease_sweep::pion_node_actions_expired_lease_sweep;
