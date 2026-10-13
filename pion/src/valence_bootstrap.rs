//! Shared Valence bootstrap: **`SQLite` embedded CP** or **Postgres + in-memory cache** (Hybrid).
//!
//! [`StorageProfile::Hybrid`] selects the Postgres + in-process `IndraDB` cache path when the
//! `db-hybrid` feature is enabled.

use std::path::PathBuf;
use std::sync::Arc;

use anyhow::{bail, Context, Result};
use valence::{
    register_backend_logical_names, router_key, Actor, DatabaseBackend, DatabaseRouter,
    RegisterBackendLogicalNamesOptions, SqliteBackend, Valence,
};

use crate::storage::{
    BOOTSTRAP_LOGICAL_NAMES, DEFAULT_SQLITE_PATH, MEMORY_SQLITE_PATH, PION_ENGINE_ID,
};

/// Which durable Valence backend to boot.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StorageProfile {
    /// File or `:memory:` `SQLite` (embedded host pattern).
    Sqlite,
    /// Postgres primary + in-process `IndraDB` cache (shared fleet database pattern).
    Hybrid,
}

impl StorageProfile {
    /// Resolve from env: `PION_STORAGE=hybrid|sqlite`, else hybrid when `DATABASE_URL` is set.
    #[must_use]
    pub fn from_env() -> Self {
        match std::env::var("PION_STORAGE")
            .unwrap_or_default()
            .to_ascii_lowercase()
            .as_str()
        {
            "hybrid" | "postgres" | "pg" => Self::Hybrid,
            "sqlite" | "sql" => Self::Sqlite,
            _ if std::env::var("DATABASE_URL").is_ok() => Self::Hybrid,
            _ => Self::Sqlite,
        }
    }

    /// Compile-time profile matching Cargo features (`db-hybrid` → Hybrid, else Sqlite).
    #[must_use]
    pub const fn compile_default() -> Self {
        #[cfg(feature = "db-hybrid")]
        {
            Self::Hybrid
        }
        #[cfg(not(feature = "db-hybrid"))]
        {
            Self::Sqlite
        }
    }
}

/// Result of process Valence bootstrap.
pub struct BootstrappedValence {
    /// Process-wide database router.
    pub router: Arc<DatabaseRouter>,
    /// Default router key (`{engine}:default`).
    pub default_backend_key: String,
    /// Profile that was booted.
    pub profile: StorageProfile,
}

impl BootstrappedValence {
    /// Build a [`Valence`] handle for `operation`.
    ///
    /// # Errors
    ///
    /// Returns an error when the Valence builder fails.
    pub fn valence(&self, operation: &str) -> Result<Valence> {
        Valence::builder()
            .database_router(Arc::clone(&self.router))
            .default_backend_key(self.default_backend_key.clone())
            .with_actor(Actor::System {
                operation: operation.to_string(),
            })
            .build()
            .map_err(anyhow::Error::from)
    }
}

/// Connect storage and build the process [`DatabaseRouter`].
///
/// - **Sqlite**: `VALENCE_SQLITE_PATH` (default [`DEFAULT_SQLITE_PATH`]; use
///   [`MEMORY_SQLITE_PATH`] for tests).
/// - **Hybrid**: requires `DATABASE_URL` (Postgres). `IndraDB` is in-process. Needs
///   `--features db-hybrid`.
///
/// Always sets `VALENCE_OWNERSHIP_UNIFIED_FETCH=0` and `VALENCE_OWNERSHIP_COLOCATE=0` unless
/// already set (`SQLite` / hybrid do not use ownership JOINs).
///
/// # Errors
///
/// Returns an error when connect or router registration fails.
pub async fn bootstrap_valence(profile: StorageProfile) -> Result<BootstrappedValence> {
    if std::env::var_os("VALENCE_OWNERSHIP_UNIFIED_FETCH").is_none() {
        std::env::set_var("VALENCE_OWNERSHIP_UNIFIED_FETCH", "0");
    }
    if std::env::var_os("VALENCE_OWNERSHIP_COLOCATE").is_none() {
        std::env::set_var("VALENCE_OWNERSHIP_COLOCATE", "0");
    }

    let backend: Arc<dyn DatabaseBackend> = match profile {
        StorageProfile::Sqlite => {
            let path =
                std::env::var("VALENCE_SQLITE_PATH").unwrap_or_else(|_| DEFAULT_SQLITE_PATH.into());
            if path != MEMORY_SQLITE_PATH {
                ensure_parent_dir(&path)?;
            }
            tracing::info!(%path, "pion valence bootstrap: sqlite");
            Arc::new(
                SqliteBackend::connect(&path)
                    .await
                    .with_context(|| format!("SqliteBackend::connect({path})"))?,
            )
        }
        StorageProfile::Hybrid => {
            #[cfg(feature = "db-hybrid")]
            {
                bootstrap_hybrid_backend().await?
            }
            #[cfg(not(feature = "db-hybrid"))]
            {
                bootstrap_hybrid_backend_unavailable()?
            }
        }
    };

    match (profile, PION_ENGINE_ID) {
        (StorageProfile::Sqlite, id) if id == valence::SQLITE_ENGINE_ID => {}
        (StorageProfile::Hybrid, id) if id == valence::HYBRID_ENGINE_ID => {}
        (profile, id) => {
            bail!(
                "storage profile {profile:?} does not match compile-time PION_ENGINE_ID={id}; \
                 rebuild with the matching Cargo feature"
            );
        }
    }

    let mut router = DatabaseRouter::new();
    register_backend_logical_names(
        &mut router,
        backend,
        BOOTSTRAP_LOGICAL_NAMES,
        RegisterBackendLogicalNamesOptions::default(),
    );

    let default_backend_key = router_key("default", PION_ENGINE_ID);
    Ok(BootstrappedValence {
        router: Arc::new(router),
        default_backend_key,
        profile,
    })
}

/// Convenience: bootstrap using the compile-time storage profile.
///
/// # Errors
///
/// See [`bootstrap_valence`].
pub async fn bootstrap_valence_from_env() -> Result<BootstrappedValence> {
    let wanted = StorageProfile::from_env();
    let compile = StorageProfile::compile_default();
    if wanted != compile {
        tracing::warn!(
            ?wanted,
            ?compile,
            "PION_STORAGE/DATABASE_URL profile differs from compile-time feature; using compile-time profile"
        );
    }
    bootstrap_valence(compile).await
}

/// In-memory `SQLite` bootstrap for unit tests and the in-process test harness.
///
/// # Errors
///
/// See [`bootstrap_valence`].
pub async fn bootstrap_sqlite_memory() -> Result<BootstrappedValence> {
    std::env::set_var("VALENCE_SQLITE_PATH", MEMORY_SQLITE_PATH);
    bootstrap_valence(StorageProfile::Sqlite).await
}

fn ensure_parent_dir(path: &str) -> Result<()> {
    let p = PathBuf::from(path);
    if let Some(parent) = p.parent().filter(|p| !p.as_os_str().is_empty()) {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("create SQLite parent directory {}", parent.display()))?;
    }
    Ok(())
}

#[cfg(feature = "db-hybrid")]
async fn bootstrap_hybrid_backend() -> Result<Arc<dyn DatabaseBackend>> {
    let url = std::env::var("DATABASE_URL")
        .context("DATABASE_URL is required for hybrid Valence bootstrap")?;
    tracing::info!("pion valence bootstrap: hybrid (postgres + indradb)");
    hybrid_from_url(&url).await
}

#[cfg(not(feature = "db-hybrid"))]
fn bootstrap_hybrid_backend_unavailable() -> Result<Arc<dyn DatabaseBackend>> {
    bail!(
        "hybrid storage requested but crate built without feature `db-hybrid` \
         (rebuild with --features db-hybrid)"
    );
}

#[cfg(feature = "db-hybrid")]
async fn hybrid_from_url(url: &str) -> Result<Arc<dyn DatabaseBackend>> {
    use valence::{HybridBackend, PostgresBackend};

    const ATTEMPTS: u32 = 8;
    let mut last_err = None;
    for attempt in 1..=ATTEMPTS {
        match PostgresBackend::connect(url).await {
            Ok(primary) => {
                let hybrid = HybridBackend::builder()
                    .primary(Arc::new(primary))
                    .warm_edges(true)
                    .build()
                    .await
                    .context("HybridBackend::build failed")?;
                return Ok(Arc::new(hybrid));
            }
            Err(e) => {
                let msg = e.to_string();
                let transient = msg.contains("pg_type_typname_nsp_index")
                    || msg.contains("duplicate key")
                    || msg.contains("already exists");
                last_err = Some(e);
                if !transient || attempt == ATTEMPTS {
                    break;
                }
                let backoff_ms = 100u64 * u64::from(attempt);
                tracing::warn!(
                    %url,
                    attempt,
                    backoff_ms,
                    "pion valence bootstrap: Postgres connect race; retrying"
                );
                tokio::time::sleep(std::time::Duration::from_millis(backoff_ms)).await;
            }
        }
    }
    Err(last_err
        .map(anyhow::Error::from)
        .unwrap_or_else(|| anyhow::anyhow!("PostgresBackend::connect failed")))
    .with_context(|| format!("PostgresBackend::connect failed for {url}"))
}
