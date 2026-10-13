//! Control-plane **runtime**: heartbeat ingest, node actions, enrollment, deploy helpers, and
//! read-model projections.
//!
//! # Public API surface
//!
//! Most hosts should use the crate-root re-exports from the
//! parent crate (for example [`crate::ingest_agent_heartbeat`] and [`crate::claim_pending_node_action`])
//! rather than reaching into submodules here. This module exists to group implementation files for
//! maintainers.
//!
//! | Submodule | Role |
//! |-----------|------|
//! | `heartbeat` | [`crate::ingest_node_heartbeat`], [`crate::ingest_agent_heartbeat`] |
//! | `agent_enrollment` | [`crate::create_host_enrollment`], session-scoped listing |
//! | `node_actions` | enqueue / claim / report / lease extension |
//! | `projections` | Operator snapshots derived from telemetry |
//! | `deploy` / `actions` | Container actions against a resolved node |
//! | `pool_placement` | Typed hardware selection for [`crate::DeployTarget::Pool`] |
//! | `contracts` / `capabilities` | Shared DTOs and per-node capability maps |

mod actions;
mod agent_enrollment;
mod agent_handoff_directive;
mod capabilities;
mod container_observation;
mod contracts;
mod deploy;
mod heartbeat;
mod node_actions;
mod pool_placement;
mod projections;
mod secret_resolver;

pub use agent_enrollment::{
    create_host_enrollment, enrollment_strict_new_nodes, list_enrollments_for_bootstrap_session,
    list_enrollments_for_session, normalize_agent_host_hint, revoke_host_enrollment,
    set_host_enrollment_pubkey, EnrollmentError, DEFAULT_BOOTSTRAP_SESSION_RECORD_ID,
    DEFAULT_SETUP_WIZARD_SESSION_RECORD_ID,
};

pub use parton::{
    AgentDirective, ContainerActionKind, ContainerActionRequest, ContainerStatus,
    ContainerStatusReport, ContainerStatusSummary, HeartbeatResponse, HostCapabilities,
    NodeHeartbeatReport,
};

pub use actions::{
    execute_node_container_action, execute_node_container_action_with_executor,
    execute_node_container_action_with_executor_and_request, parse_action_kind,
};
pub use agent_handoff_directive::{
    acknowledge_handoff_directives_for_heartbeat, collect_handoff_directives_for_heartbeat,
    sha256_hex_bytes, sign_re_enroll_directive, upsert_pending_handoff_directive,
};
pub use capabilities::{list_node_action_capability_map, upsert_node_action_capability};
pub use container_observation::{upsert_observations, ContainerStateTransition};
pub use contracts::{
    derive_observed_health, map_health_to_node_status, DeployHandoffActionRequest,
    DeployHandoffActionResult, DeployPreflightStatus, DeploySecretEnvVar, DeployTarget,
    DeployTargetPreflight, ObservedHealth, RuntimeContainerActionResult,
    RuntimeContainerActionStatus, TeardownHandoffActionRequest,
};
pub use deploy::{
    deploy_container_to_target, deploy_container_to_target_with_command,
    ensure_docker_image_on_deploy_target, preflight_deploy_target,
    preflight_deploy_target_with_requirements, resolve_node_for_deploy_target,
    resolve_node_for_deploy_target_with_requirements,
};
pub use heartbeat::{ingest_agent_heartbeat, ingest_node_heartbeat};
pub use pool_placement::{
    resolve_eligible_pool_nodes, CpuArchitecture, EligiblePoolNode, NodeHardwareCapabilities,
    NodeHardwareRequirements, PoolPlacementStrategy, PoolResolutionError, UnmetRequirement,
    VirtualPoolPlacementPolicy, VirtualPoolSelector,
};

pub use node_actions::{
    cancel_node_action, cancel_non_terminal_node_actions_for_correlation,
    claim_pending_node_action, default_lease_duration_secs, default_max_attempts,
    enqueue_node_action, enqueue_node_action_idempotent, extend_node_action_lease,
    get_node_action_command, pending_timeout_duration, reconcile_node_action_commands,
    report_node_action_result, reset_failed_node_actions_for_correlation, ClaimedNodeAction,
    NodeActionError, ReportNodeActionResult, NODE_ACTION_KIND_DEPLOY_HANDOFF,
    NODE_ACTION_KIND_TEARDOWN_HANDOFF,
};
pub use projections::{
    derive_image_flavor, list_agent_nodes_for_cell, list_runtime_container_snapshots,
    list_runtime_health_snapshots, parse_runtime_images, RuntimeContainerSnapshot,
    RuntimeHealthSnapshot, RuntimeImageSnapshot,
};
pub use secret_resolver::{
    default_secret_resolver, resolve_secrets_in_json, set_default_secret_resolver,
    try_resolve_one_secret_ref, value_contains_secret_ref_placeholder, SecretResolver,
};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn derive_health_prefers_unhealthy() {
        let summary = ContainerStatusSummary {
            running: 10,
            exited: 3,
            unhealthy: 1,
        };
        assert_eq!(derive_observed_health(&summary), ObservedHealth::Unhealthy);
    }

    #[test]
    fn derive_health_degraded_when_exited_without_unhealthy() {
        let summary = ContainerStatusSummary {
            running: 5,
            exited: 1,
            unhealthy: 0,
        };
        assert_eq!(derive_observed_health(&summary), ObservedHealth::Degraded);
    }

    #[test]
    fn derive_health_healthy_when_only_running() {
        let summary = ContainerStatusSummary {
            running: 4,
            exited: 0,
            unhealthy: 0,
        };
        assert_eq!(derive_observed_health(&summary), ObservedHealth::Healthy);
    }

    #[test]
    fn derive_health_unknown_when_empty_snapshot() {
        let summary = ContainerStatusSummary {
            running: 0,
            exited: 0,
            unhealthy: 0,
        };
        assert_eq!(derive_observed_health(&summary), ObservedHealth::Unknown);
    }

    #[test]
    fn health_to_node_status_mapping_is_deterministic() {
        assert_eq!(
            map_health_to_node_status(ObservedHealth::Healthy).as_str(),
            "online"
        );
        assert_eq!(
            map_health_to_node_status(ObservedHealth::Degraded).as_str(),
            "draining"
        );
        assert_eq!(
            map_health_to_node_status(ObservedHealth::Unhealthy).as_str(),
            "failed"
        );
        assert_eq!(
            map_health_to_node_status(ObservedHealth::Unknown).as_str(),
            "online"
        );
    }

    #[test]
    fn deploy_preflight_status_as_str_is_stable() {
        assert_eq!(DeployPreflightStatus::Pass.as_str(), "pass");
        assert_eq!(DeployPreflightStatus::Fail.as_str(), "fail");
    }
}
