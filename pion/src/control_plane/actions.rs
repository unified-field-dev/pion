//! Synchronous container-action execution and audit event persistence.
//!
//! These helpers run an action **in-process** via a [`parton::ContainerActionExecutor`]
//! (capability-gated), then write a `PionControlPlaneActionEvent` row. Prefer the
//! async node-action queue ([`crate::enqueue_node_action`] / claim / report) for
//! agent-delivered work; use this module for control-plane-local execution and
//! operator tools that need an immediate result.

use anyhow::Result;
use chrono::Utc;
use valence::{Model, Valence};

use crate::generated::{
    PionControlPlaneActionEvent, PionControlPlaneActionEventAction,
    PionControlPlaneActionEventStatus,
};

use super::capabilities::{is_action_enabled, list_node_action_capability_map};
use super::{
    ContainerActionKind, ContainerActionRequest, RuntimeContainerActionResult,
    RuntimeContainerActionStatus,
};

fn map_action_to_event_action_enum(
    action: ContainerActionKind,
) -> PionControlPlaneActionEventAction {
    match action {
        ContainerActionKind::Start => PionControlPlaneActionEventAction::Start,
        ContainerActionKind::Stop => PionControlPlaneActionEventAction::Stop,
        ContainerActionKind::Restart => PionControlPlaneActionEventAction::Restart,
        ContainerActionKind::Logs => PionControlPlaneActionEventAction::Logs,
        ContainerActionKind::Inspect | ContainerActionKind::Diagnostic => {
            PionControlPlaneActionEventAction::Inspect
        }
        // Audit/logging: infra provisioning actions share the deploy event bucket.
        ContainerActionKind::Deploy
        | ContainerActionKind::EnsureNetwork
        | ContainerActionKind::ProbeHost
        | ContainerActionKind::WireguardPeer
        | ContainerActionKind::EnsureDockerImage
        | ContainerActionKind::GrowFs
        | ContainerActionKind::TemplatedExec => PionControlPlaneActionEventAction::Deploy,
    }
}

fn map_status_to_event_enum(
    status: RuntimeContainerActionStatus,
) -> PionControlPlaneActionEventStatus {
    match status {
        RuntimeContainerActionStatus::Success => PionControlPlaneActionEventStatus::Success,
        RuntimeContainerActionStatus::Denied => PionControlPlaneActionEventStatus::Denied,
        RuntimeContainerActionStatus::Failed => PionControlPlaneActionEventStatus::Failed,
    }
}

async fn persist_action_event(
    node_id: &str,
    container_ref: &str,
    action: ContainerActionKind,
    status: RuntimeContainerActionStatus,
    message: &str,
    payload_json: serde_json::Value,
    valence: &Valence,
) -> Result<RuntimeContainerActionResult> {
    let observed_at = Utc::now();
    let event_id = uuid::Uuid::new_v4().to_string();
    let record = PionControlPlaneActionEvent::new(
        node_id.to_string(),
        container_ref.to_string(),
        map_action_to_event_action_enum(action),
        map_status_to_event_enum(status),
        message.to_string(),
        payload_json.clone(),
        observed_at,
        observed_at,
    )?;
    PionControlPlaneActionEvent::upsert_used(&event_id, record, valence, valence::use_!(r#"When **Pion control plane** needs to persist work, we **save Pion Control Plane Action Event** so the next step in that feature can continue with the latest values. People and services allowed for **Pion control plane** use this data for that workflow—not as a general export of unrelated personal fields."#)).await?;
    Ok(RuntimeContainerActionResult {
        event_id,
        node_id: node_id.to_string(),
        container_ref: container_ref.to_string(),
        action: action.as_str().to_string(),
        status: status.as_str().to_string(),
        message: message.to_string(),
        payload_json,
        observed_at,
    })
}

/// Parse a wire / CLI action string into a [`ContainerActionKind`].
///
/// Matching is case-insensitive after trim. Accepted values mirror Parton's
/// `ContainerActionKind::as_str()` (`start`, `stop`, `restart`, `logs`, `inspect`,
/// `deploy`, `ensure_network`, `probe_host`, `diagnostic`, `wireguard_peer`,
/// `ensure_docker_image`, `grow_fs`, `templated_exec`). Returns `None` for unknown strings (callers should
/// surface a validation error rather than defaulting).
pub fn parse_action_kind(raw: &str) -> Option<ContainerActionKind> {
    match raw.trim().to_ascii_lowercase().as_str() {
        "start" => Some(ContainerActionKind::Start),
        "stop" => Some(ContainerActionKind::Stop),
        "restart" => Some(ContainerActionKind::Restart),
        "logs" => Some(ContainerActionKind::Logs),
        "inspect" => Some(ContainerActionKind::Inspect),
        "deploy" => Some(ContainerActionKind::Deploy),
        "ensure_network" => Some(ContainerActionKind::EnsureNetwork),
        "probe_host" => Some(ContainerActionKind::ProbeHost),
        "diagnostic" => Some(ContainerActionKind::Diagnostic),
        "wireguard_peer" => Some(ContainerActionKind::WireguardPeer),
        "ensure_docker_image" => Some(ContainerActionKind::EnsureDockerImage),
        "grow_fs" => Some(ContainerActionKind::GrowFs),
        "templated_exec" => Some(ContainerActionKind::TemplatedExec),
        _ => None,
    }
}

/// Executes a runtime action on a node using the default executor.
///
/// # Errors
///
/// Propagates capability lookup, executor, and Valence persist failures.
pub async fn execute_node_container_action(
    node_id: &str,
    container_ref: &str,
    action: ContainerActionKind,
    tail_lines: Option<u32>,
    valence: &Valence,
) -> Result<RuntimeContainerActionResult> {
    execute_node_container_action_with_executor(
        &parton::DockerCliActionExecutor,
        node_id,
        container_ref,
        action,
        tail_lines,
        valence,
    )
    .await
}

/// Executes a runtime action with a custom executor implementation.
///
/// # Errors
///
/// Propagates capability lookup, executor, and Valence persist failures.
pub async fn execute_node_container_action_with_executor<E: parton::ContainerActionExecutor>(
    executor: &E,
    node_id: &str,
    container_ref: &str,
    action: ContainerActionKind,
    tail_lines: Option<u32>,
    valence: &Valence,
) -> Result<RuntimeContainerActionResult> {
    execute_node_container_action_with_executor_and_request(
        executor,
        node_id,
        container_ref,
        action,
        tail_lines,
        None,
        Vec::new(),
        Vec::new(),
        None,
        Vec::new(),
        valence,
    )
    .await
}

/// Executes a runtime action with full request fields.
///
/// # Errors
///
/// Propagates capability lookup, executor, and Valence persist failures.
#[allow(clippy::too_many_arguments)] // executor dispatch mirrors Parton `ContainerActionRequest` fields
pub async fn execute_node_container_action_with_executor_and_request<
    E: parton::ContainerActionExecutor,
>(
    executor: &E,
    node_id: &str,
    container_ref: &str,
    action: ContainerActionKind,
    tail_lines: Option<u32>,
    image_ref: Option<String>,
    env_vars: Vec<String>,
    port_mappings: Vec<String>,
    deploy_entrypoint: Option<String>,
    deploy_command: Vec<String>,
    valence: &Valence,
) -> Result<RuntimeContainerActionResult> {
    let capability_map = list_node_action_capability_map(valence).await?;
    if !is_action_enabled(&capability_map, node_id, action) {
        return persist_action_event(
            node_id,
            container_ref,
            action,
            RuntimeContainerActionStatus::Denied,
            "action denied: capability disabled for node",
            serde_json::json!({}),
            valence,
        )
        .await;
    }

    let request = ContainerActionRequest {
        node_id: node_id.to_string(),
        container_ref: container_ref.to_string(),
        action,
        tail_lines,
        image_ref,
        env_vars,
        secret_env_vars: vec![],
        port_mappings,
        entrypoint: deploy_entrypoint,
        command: deploy_command
            .into_iter()
            .map(serde_json::Value::String)
            .collect(),
        restart_policy: None,
        volume_mounts: vec![],
        resource_limits: None,
        health_check: None,
        labels: std::collections::HashMap::new(),
        extra_hosts: vec![],
        network: None,
        diagnostic: None,
        wireguard_peer: None,
        grow_fs: None,
        templated_exec: None,
        expected_container_id: None,
    };
    match parton::execute_container_action(executor, &request) {
        Ok(response) => {
            persist_action_event(
                node_id,
                container_ref,
                action,
                RuntimeContainerActionStatus::Success,
                &response.message,
                response.payload,
                valence,
            )
            .await
        }
        Err(error) => {
            let message = format!("action failed: {error}");
            persist_action_event(
                node_id,
                container_ref,
                action,
                RuntimeContainerActionStatus::Failed,
                &message,
                serde_json::json!({ "error": error.to_string() }),
                valence,
            )
            .await
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_action_kind_is_case_insensitive() {
        assert_eq!(parse_action_kind("START"), Some(ContainerActionKind::Start));
        assert_eq!(
            parse_action_kind("restart"),
            Some(ContainerActionKind::Restart)
        );
        assert_eq!(
            parse_action_kind("deploy"),
            Some(ContainerActionKind::Deploy)
        );
        assert_eq!(parse_action_kind("bogus"), None);
        assert_eq!(
            parse_action_kind("ensure_docker_image"),
            Some(ContainerActionKind::EnsureDockerImage)
        );
        assert_eq!(
            parse_action_kind("grow_fs"),
            Some(ContainerActionKind::GrowFs)
        );
        assert_eq!(
            parse_action_kind("GROW_FS"),
            Some(ContainerActionKind::GrowFs)
        );
        assert_eq!(
            parse_action_kind("templated_exec"),
            Some(ContainerActionKind::TemplatedExec)
        );
    }

    #[test]
    fn parse_action_kind_engine_promote_is_none() {
        assert_eq!(parse_action_kind("engine_promote"), None);
        assert_eq!(parse_action_kind("ENGINE_PROMOTE"), None);
    }
}
