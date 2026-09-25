//! Valence mem bootstrap (+ optional Axum `parton_router`) for a [`crate::MatrixSpec`].

use std::net::SocketAddr;
use std::sync::Arc;

use anyhow::{bail, Result};
use axum::Router;
use tokio::task::JoinHandle;
use valence::{Actor, DatabaseRouter, Valence};

use crate::matrix::{Backend, TelemetryAdapter, Topology};
use crate::MatrixSpec;

/// Shared application state for the optional Parton HTTP surface.
#[derive(Clone)]
struct HttpAppState {
    router: Arc<DatabaseRouter>,
    default_backend_key: String,
}

impl pion::runtime::HasValenceRouter for HttpAppState {
    fn valence_router(&self) -> Arc<DatabaseRouter> {
        self.router.clone()
    }

    fn default_backend_key(&self) -> &str {
        &self.default_backend_key
    }
}

/// Holds bootstrap state for one matrix row.
///
/// Sets ownership env flags so `SQLite` / hybrid labs do not require Surreal-style ownership JOINs.
pub struct BootstrapSession {
    matrix: MatrixSpec,
    ready: bool,
    db_router: Option<Arc<DatabaseRouter>>,
    default_backend_key: Option<String>,
    /// Bound HTTP address when [`Self::start_http`] succeeded.
    http_addr: Option<SocketAddr>,
    http_task: Option<JoinHandle<()>>,
}

impl BootstrapSession {
    /// Start a session for the given matrix dimensions (applies ownership env + telemetry).
    ///
    /// # Errors
    ///
    /// Returns an error if a non-lab topology is requested without install support yet.
    pub fn new(matrix: MatrixSpec) -> Result<Self> {
        std::env::set_var("VALENCE_OWNERSHIP_UNIFIED_FETCH", "0");
        std::env::set_var("VALENCE_OWNERSHIP_COLOCATE", "0");
        apply_telemetry(matrix.telemetry);
        match matrix.topology {
            Topology::IsolatedHarness
            | Topology::LocalDocker
            | Topology::AwsSameRegion
            | Topology::AwsWan => {
                // In-process install boots Valence locally for all of these topologies today.
            }
        }
        Ok(Self {
            matrix,
            ready: false,
            db_router: None,
            default_backend_key: None,
            http_addr: None,
            http_task: None,
        })
    }

    /// Matrix dimensions for this session.
    #[must_use]
    pub const fn matrix(&self) -> &MatrixSpec {
        &self.matrix
    }

    /// Install Valence (`SQLite` `:memory:` or Hybrid when `DATABASE_URL` + `db-hybrid`).
    ///
    /// # Errors
    ///
    /// Returns an error if Valence connect or router registration fails.
    pub async fn install(&mut self) -> Result<()> {
        let boot = match self.matrix.backend {
            Backend::Sqlite => {
                std::env::set_var("VALENCE_SQLITE_PATH", pion::storage::MEMORY_SQLITE_PATH);
                pion::valence_bootstrap::bootstrap_valence(
                    pion::valence_bootstrap::StorageProfile::Sqlite,
                )
                .await?
            }
            Backend::Hybrid => {
                pion::valence_bootstrap::bootstrap_valence(
                    pion::valence_bootstrap::StorageProfile::Hybrid,
                )
                .await?
            }
        };
        self.db_router = Some(boot.router);
        self.default_backend_key = Some(boot.default_backend_key);
        self.ready = true;
        Ok(())
    }

    /// Optionally serve [`pion::runtime::parton_router`] on an ephemeral loopback port.
    ///
    /// # Errors
    ///
    /// Returns an error if install has not completed or the bind fails.
    pub async fn start_http(&mut self) -> Result<SocketAddr> {
        if !self.ready {
            bail!("BootstrapSession::install must succeed before start_http");
        }
        let router = self
            .db_router
            .clone()
            .ok_or_else(|| anyhow::anyhow!("database router missing after install"))?;
        let default_backend_key = self
            .default_backend_key
            .clone()
            .ok_or_else(|| anyhow::anyhow!("backend key missing after install"))?;

        let state = HttpAppState {
            router,
            default_backend_key,
        };
        let app = Router::new()
            .merge(pion::runtime::parton_router::<HttpAppState>())
            .with_state(state);

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
        let addr = listener.local_addr()?;
        let task = tokio::spawn(async move {
            let _ = axum::serve(
                listener,
                app.into_make_service_with_connect_info::<SocketAddr>(),
            )
            .await;
        });
        self.http_addr = Some(addr);
        self.http_task = Some(task);
        Ok(addr)
    }

    /// Bound HTTP address when [`Self::start_http`] was called.
    #[must_use]
    pub const fn http_addr(&self) -> Option<SocketAddr> {
        self.http_addr
    }

    /// Build a [`Valence`] handle for control-plane library calls.
    ///
    /// # Errors
    ///
    /// Returns an error if install has not completed or the builder fails.
    pub fn valence(&self, operation: &str) -> Result<Valence> {
        if !self.ready {
            bail!("BootstrapSession::install must succeed before valence()");
        }
        let router = self
            .db_router
            .clone()
            .ok_or_else(|| anyhow::anyhow!("database router missing after install"))?;
        let key = self
            .default_backend_key
            .clone()
            .ok_or_else(|| anyhow::anyhow!("backend key missing after install"))?;
        Valence::builder()
            .database_router(router)
            .default_backend_key(key)
            .with_actor(Actor::System {
                operation: operation.to_string(),
            })
            .build()
            .map_err(anyhow::Error::from)
    }

    /// Whether [`install`](Self::install) completed successfully.
    #[must_use]
    pub const fn is_ready(&self) -> bool {
        self.ready
    }
}

impl Drop for BootstrapSession {
    fn drop(&mut self) {
        if let Some(task) = self.http_task.take() {
            task.abort();
        }
    }
}

fn apply_telemetry(telemetry: TelemetryAdapter) {
    match telemetry {
        TelemetryAdapter::Off => {}
        TelemetryAdapter::Console => {
            use std::sync::Arc;

            use spectra_core::{set_sink, SpectraSink};

            struct TracingConsoleSink;

            impl SpectraSink for TracingConsoleSink {
                fn record_counter(&self, name: &str, labels: &[(&str, &str)], delta: i64) {
                    tracing::info!(target: "pion.testkit.telemetry", %name, ?labels, delta, "counter");
                }

                fn record_gauge(&self, name: &str, labels: &[(&str, &str)], value: f64) {
                    tracing::info!(target: "pion.testkit.telemetry", %name, ?labels, value, "gauge");
                }

                fn log_event(&self, table: &str, fields: &serde_json::Value) {
                    tracing::info!(target: "pion.testkit.telemetry", %table, %fields, "event");
                }
            }

            set_sink(Arc::new(TracingConsoleSink) as Arc<dyn SpectraSink>);
        }
    }
}
