//! Shared control-plane DTOs and read-model helpers.
//!
//! ## `PionNodeActionCommand.payload_json` (Parton)
//!
//! For `action_kind` values that deserialize to a Parton request (for example `deploy`), this
//! value is a [`parton::ContainerActionRequest`]. For `deploy_handoff` / `teardown_handoff`, use
//! [`DeployHandoffActionRequest`] / [`TeardownHandoffActionRequest`] (Parton maps them to deploy/stop).
//! Successful `deploy_handoff` results may include [`DeployHandoffActionResult`] fields merged into
//! `PionNodeActionResult.payload_json` alongside executor-specific keys.
//! [`DeployHandoffActionRequest::secret_env_vars`] may carry `$secret_ref` JSON in each `value`
//! (resolved at claim like `command` entries). The `command` field is
//! `Vec<serde_json::Value>`: normal arguments are JSON strings, or a **secret placeholder** for
//! claim-time Neutrino resolution:
//! `{"$secret_ref": {"id": "neutrino_secret:…", "version": 1}, "field": "password"}`.
//! The `field` selects a string key in the stored JSON (for example the `password` key next to
//! `user` in a stored root secret). See `secret_resolver` and `claim_pending_node_action`, which
//! replace placeholders with strings before the agent runs Docker.

use crate::generated::{PionControlPlaneNodeStatus, PionControlPlaneObservedStatusHealth};
use parton::{ContainerStatusSummary, VolumeMount};

/// Normalized workload health classes derived from heartbeat container counts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ObservedHealth {
    /// All reported containers are running with no exits or unhealthy counts.
    Healthy,
    /// Running containers exist but at least one has exited.
    Degraded,
    /// At least one container is unhealthy.
    Unhealthy,
    /// No containers or indeterminate workload state.
    Unknown,
}

/// Derives control-plane health class from runtime container summary.
pub fn derive_observed_health(summary: &ContainerStatusSummary) -> ObservedHealth {
    if summary.unhealthy > 0 {
        ObservedHealth::Unhealthy
    } else if summary.exited > 0 {
        ObservedHealth::Degraded
    } else if summary.running > 0 {
        ObservedHealth::Healthy
    } else {
        ObservedHealth::Unknown
    }
}

/// Maps derived health class to node status used in node inventory/read models.
pub fn map_health_to_node_status(health: ObservedHealth) -> PionControlPlaneNodeStatus {
    match health {
        ObservedHealth::Unhealthy => PionControlPlaneNodeStatus::Failed,
        ObservedHealth::Degraded => PionControlPlaneNodeStatus::Draining,
        // Unknown means heartbeat exists but workload health is indeterminate
        // (for example, idle node with no containers), not necessarily offline.
        ObservedHealth::Healthy | ObservedHealth::Unknown => PionControlPlaneNodeStatus::Online,
    }
}

/// Maps derived health class to observed-status enum persisted in telemetry rows.
pub(crate) fn map_health_to_observed_enum(
    health: ObservedHealth,
) -> PionControlPlaneObservedStatusHealth {
    match health {
        ObservedHealth::Healthy => PionControlPlaneObservedStatusHealth::Healthy,
        ObservedHealth::Degraded => PionControlPlaneObservedStatusHealth::Degraded,
        ObservedHealth::Unhealthy => PionControlPlaneObservedStatusHealth::Unhealthy,
        ObservedHealth::Unknown => PionControlPlaneObservedStatusHealth::Unknown,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
/// Outcome of a runtime container action execution request.
pub enum RuntimeContainerActionStatus {
    /// Action completed successfully.
    Success,
    /// Node capability disabled or policy denied the action.
    Denied,
    /// Backend or executor reported failure.
    Failed,
}

impl RuntimeContainerActionStatus {
    /// Stable wire-format value used in persisted action events.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Success => "success",
            Self::Denied => "denied",
            Self::Failed => "failed",
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
/// Persisted action event projection returned to callers.
pub struct RuntimeContainerActionResult {
    /// Persisted action event row id.
    pub event_id: String,
    /// Target node id.
    pub node_id: String,
    /// Logical container reference (name or id).
    pub container_ref: String,
    /// Action kind wire value (for example `deploy`).
    pub action: String,
    /// Outcome wire value from [`RuntimeContainerActionStatus::as_str`].
    pub status: String,
    /// Human-readable summary or error text.
    pub message: String,
    /// Structured executor payload (Docker ids, URLs, etc.).
    pub payload_json: serde_json::Value,
    /// When the action was observed or persisted.
    pub observed_at: chrono::DateTime<chrono::Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
/// Deploy target selection for runtime deployment actions.
pub enum DeployTarget {
    /// Deploy to a specific node id.
    Node(String),
    /// Deploy to any eligible node in a pool id.
    Pool(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
/// Preflight verdict for a deploy target.
pub enum DeployPreflightStatus {
    /// Target passes eligibility checks.
    Pass,
    /// Target fails one or more checks.
    Fail,
}

impl DeployPreflightStatus {
    /// Stable wire-format value used by UI/server projections.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Pass => "pass",
            Self::Fail => "fail",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
/// Preflight result for a resolved deploy target.
pub struct DeployTargetPreflight {
    /// Node id selected from the deploy target.
    pub resolved_node_id: String,
    /// Pass/fail verdict.
    pub status: DeployPreflightStatus,
    /// Operator-facing explanation when deploy is blocked.
    pub message: String,
    /// True when deploy may proceed.
    pub can_deploy: bool,
}

/// One secret-backed container env var for [`DeployHandoffActionRequest`].
///
/// `value` is normally a JSON string after claim-time resolution. While the action is pending,
/// `value` may embed a Neutrino placeholder object:
/// `{"$secret_ref":{"id":"…","version":1},"field":"…"}` (resolved recursively in
/// [`crate::resolve_secrets_in_json`]).
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq)]
pub struct DeploySecretEnvVar {
    /// Container env var name (`NAME` in `--env-file`).
    pub name: String,
    /// Plain string after claim, or a `$secret_ref` placeholder while pending.
    pub value: serde_json::Value,
}

/// Payload for queued `deploy_handoff` node actions (Parton maps to [`parton::ContainerActionKind::Deploy`]).
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq)]
pub struct DeployHandoffActionRequest {
    /// Target node id (must match the queued command row).
    pub node_id: String,
    /// Logical container name or reference passed to the agent.
    pub container_ref: String,
    /// Docker image reference to run.
    pub image_ref: String,
    /// Plain `-e KEY=value` env vars (non-secret).
    #[serde(default)]
    pub env_vars: Vec<String>,
    /// Env vars passed to the container via a short-lived `--env-file` on the agent (never `-e NAME=value` for these).
    #[serde(default)]
    pub secret_env_vars: Vec<DeploySecretEnvVar>,
    /// Host:container port mappings (for example `8080:80`).
    #[serde(default)]
    pub port_mappings: Vec<String>,
    /// Docker `--add-host` entries.
    #[serde(default)]
    pub extra_hosts: Vec<String>,
    /// Bind mounts and named volumes.
    #[serde(default)]
    pub volume_mounts: Vec<VolumeMount>,
    /// Docker network name (optional).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub network: Option<String>,
    /// Reserved for future health-check wiring (HTTP check URL).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub health_url: Option<String>,
    /// Reserved for future import-endpoint wiring.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub import_base_url: Option<String>,
    /// Optional handoff transfer id for orchestrator correlation.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub transfer_id: Option<String>,
}

/// Structured fields Parton may merge into `PionNodeActionResult.payload_json` after a successful
/// `deploy_handoff` run (CN-4). Extra Docker/runtime keys may still be present on the same JSON
/// object; unknown fields are ignored when deserializing this struct.
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct DeployHandoffActionResult {
    /// Docker container id after a successful deploy.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub container_id: Option<String>,
    /// Canonical URL (no trailing slash) for `GET/POST /__handoff_internal/import` on the cell.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub import_base_url: Option<String>,
    /// Canonical health-check URL for the deployed cell (when wired).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub health_url: Option<String>,
    /// Diagnostic provenance for [`Self::import_base_url`] (e.g. `parton_env`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
}

/// Payload for queued `teardown_handoff` node actions (Parton maps to [`parton::ContainerActionKind::Stop`]).
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct TeardownHandoffActionRequest {
    /// Target node id (must match the queued command row).
    pub node_id: String,
    /// Logical container name or reference to stop/remove.
    pub container_ref: String,
    /// When true, Parton runs `docker rm -v <container_ref>` after the Stop succeeds, reclaiming
    /// anonymous/named volumes (see `parton::command_queue::run_teardown_handoff`).
    #[serde(default)]
    pub remove_volumes: bool,
    /// Optional Docker id guard — teardown skipped when the running id differs.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expected_container_id: Option<String>,
}
