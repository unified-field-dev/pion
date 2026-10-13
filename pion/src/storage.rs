//! Valence storage for Pion control-plane schemas: an embedded `SQLite` control plane, or
//! Postgres with an in-memory cache (Hybrid feature).
//!
//! Control-plane tables use the `gluon` logical partition name, a historical key that is part of
//! the stored schema and must stay as is. Bootstraps register the same physical backend under
//! both `default` and `gluon` so `Valence::active_backend` and schema `database:` evaluators agree.
//!
//! # Profiles
//!
//! | Feature | Engine | Typical use |
//! |---------|--------|-------------|
//! | default (no `db-hybrid`) | [`valence::SQLITE_ENGINE_ID`] | Embedded control plane (SQLite file or memory) |
//! | `db-hybrid` ([`valence::HYBRID_ENGINE_ID`]) | Postgres + in-memory cache (IndraDB) | Distributed fleets |

/// Logical name for schemas in the primary control-plane partition (the `gluon` key).
pub const GLUON_LOGICAL_NAME: &str = "gluon";

/// Alias: control-plane schemas share the `gluon` logical partition.
pub const CONTROL_PLANE_LOGICAL_NAME: &str = "gluon";

/// Engine id selected at compile time (`sqlite` unless `db-hybrid` is enabled).
#[cfg(feature = "db-hybrid")]
pub const PION_ENGINE_ID: &str = valence::HYBRID_ENGINE_ID;

/// Engine id selected at compile time (`sqlite` unless `db-hybrid` is enabled).
#[cfg(not(feature = "db-hybrid"))]
pub const PION_ENGINE_ID: &str = valence::SQLITE_ENGINE_ID;

/// Database evaluator for schemas bound to [`GLUON_LOGICAL_NAME`].
pub const GLUON_DEFAULT_STORAGE: valence::DatabaseFromEngine =
    valence::Database::from_engine(GLUON_LOGICAL_NAME, PION_ENGINE_ID);

/// Database evaluator for control-plane–scoped schemas (same router key as [`GLUON_LOGICAL_NAME`]).
pub const CONTROL_PLANE_DEFAULT_STORAGE: valence::DatabaseFromEngine =
    valence::Database::from_engine(CONTROL_PLANE_LOGICAL_NAME, PION_ENGINE_ID);

/// Distinct logical names that must be registered on the process router for Pion schemas.
pub const PION_LOGICAL_NAMES: &[&str] = &[GLUON_LOGICAL_NAME];

/// Logical names registered for the process default key **and** Pion partitions.
///
/// Both names share one physical backend so `default_backend_key("default", …)` and schema
/// `gluon:…` evaluators hit the same store.
pub const BOOTSTRAP_LOGICAL_NAMES: &[&str] = &["default", GLUON_LOGICAL_NAME];

/// Default `SQLite` path when `VALENCE_SQLITE_PATH` is unset (file-backed smoke / server).
pub const DEFAULT_SQLITE_PATH: &str = "data/pion.sqlite3";

/// In-memory `SQLite` path for unit tests and the in-process test harness.
pub const MEMORY_SQLITE_PATH: &str = ":memory:";
