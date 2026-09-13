//! Deploy target resolution, preflight, and container deploy helpers.
//!
//! # Eligibility
//!
//! A node may receive deploys when its inventory status is `online`, `draining`, or
//! `failed` (recovery work must still reach unhealthy hosts). Fully offline /
//! unknown statuses are rejected. Pool targets additionally apply the pool's hardware
//! selector and then pick the best-ranked survivor; see
//! [`crate::resolve_eligible_pool_nodes`].
//!
//! # Flow
//!
//! 1. [`resolve_node_for_deploy_target`] — map [`DeployTarget`] → node id.
//! 2. [`preflight_deploy_target`] — same resolution plus the deploy capability toggle.
//! 3. [`deploy_container_to_target`] — execute deploy after a passing preflight.
//!
//! # Hardware requirements
//!
//! The `_with_requirements` variants take a [`NodeHardwareRequirements`] floor for the workload
//! being placed. The plain entry points pass an unconstrained floor, so they behave as they did
//! before selectors existed.

use anyhow::{Context, Result};
use valence::{Model, Valence};

use crate::generated::{PionControlPlaneNode, PionControlPlaneNodeStatus};

use super::actions::execute_node_container_action_with_executor_and_request;
use super::capabilities::{is_action_enabled, list_node_action_capability_map};
use super::pool_placement::{
    resolve_eligible_pool_nodes, NodeHardwareCapabilities, NodeHardwareRequirements,
};
use super::{
    ContainerActionKind, DeployPreflightStatus, DeployTarget, DeployTargetPreflight,
    RuntimeContainerActionResult,
};

/// Node is reachable and may run deploy actions.
///
/// `draining` is derived from workload signals (e.g. exited containers on the host) but the agent
/// is still heartbeating; blocking deploy would prevent fixing that state (e.g. starting a registry).
///
/// `failed` is set when the agent reports unhealthy running containers. Blocking the queue in that
/// state prevents **recovery** deploys (Nucleus / Parton) from being enqueued for the very workload
/// that is unhealthy, so it is still eligible for the action queue (recovery deploys). The agent
/// must be heartbeating; fully offline nodes remain ineligible.
pub(crate) fn node_eligible_for_deploy(status: &PionControlPlaneNodeStatus) -> bool {
    matches!(
        status,
        PionControlPlaneNodeStatus::Online
            | PionControlPlaneNodeStatus::Draining
            | PionControlPlaneNodeStatus::Failed
    )
}

/// Resolves a deploy target to a node id eligible for deploy work.
///
/// Eligible inventory statuses: `online`, `draining`, and `failed` (so recovery
/// deploys can still reach unhealthy hosts). Offline / other statuses are rejected.
/// For [`DeployTarget::Pool`], returns the best-ranked node that passes the pool's hardware
/// selector, which for a pool with no selector is the eligible node in enabled cells with the
/// lowest reported running-container count (stable tie-break on hostname then node id).
///
/// Placements with a hardware floor should call
/// [`resolve_node_for_deploy_target_with_requirements`] instead.
///
/// # Errors
///
/// Returns `Err` when the target node or pool is missing, ineligible, or has no candidates.
pub async fn resolve_node_for_deploy_target(
    target: &DeployTarget,
    valence: &Valence,
) -> Result<String> {
    resolve_node_for_deploy_target_with_requirements(
        target,
        &NodeHardwareRequirements::default(),
        valence,
    )
    .await
}

/// Resolves a deploy target to a node id that also clears a workload hardware floor.
///
/// For [`DeployTarget::Pool`], `requirements` stacks on top of the pool's own selector: a node
/// must satisfy both. For [`DeployTarget::Node`], the pinned node is checked against
/// `requirements` and refused when it falls short, so a hardware-sensitive workload cannot be
/// pinned onto a host that cannot run it.
///
/// An unconstrained `requirements` is equivalent to [`resolve_node_for_deploy_target`].
///
/// # Errors
///
/// Returns `Err` when the target is missing or ineligible, when no pool node clears both floors
/// (see [`crate::PoolResolutionError`]), or when a pinned node falls short of `requirements`.
pub async fn resolve_node_for_deploy_target_with_requirements(
    target: &DeployTarget,
    requirements: &NodeHardwareRequirements,
    valence: &Valence,
) -> Result<String> {
    match target {
        DeployTarget::Node(node_id) => {
            let node = PionControlPlaneNode::get_used(node_id, valence, valence::use_!(r#"In **Pion control plane**, we **load Pion Control Plane Node** so the application can decide what to do next in this workflow. The result is used by **Pion control plane** logic—not necessarily displayed on a page unless that feature’s UI shows it."#))
                .await
                .with_context(|| format!("load node {node_id} for deploy target resolution"))?
                .ok_or_else(|| anyhow::anyhow!("node '{node_id}' not found"))?;
            if !node_eligible_for_deploy(node.status()) {
                anyhow::bail!(
                    "node '{}' cannot accept deploys while status is '{}' (expected online, draining, or failed for recovery work)",
                    node_id,
                    node.status().as_str()
                );
            }
            if let Some(unmet) = requirements.unmet(
                &NodeHardwareCapabilities::from_capabilities_json(node.capabilities_json()),
            ) {
                anyhow::bail!(
                    "node '{node_id}' does not meet the deploy hardware requirements ({unmet})"
                );
            }
            Ok(node_id.clone())
        }
        DeployTarget::Pool(pool_id) => {
            let ranked = resolve_eligible_pool_nodes(pool_id, requirements, valence).await?;
            ranked
                .into_iter()
                .next()
                .map(|node| node.node_id)
                .ok_or_else(|| {
                    anyhow::anyhow!("pool '{pool_id}' produced an empty eligible node list")
                })
        }
    }
}

/// Evaluates deploy target readiness: resolve a node, then check the deploy capability toggle.
///
/// Does not execute a deploy. `can_deploy` / [`DeployPreflightStatus::Pass`] require both
/// eligibility (see [`resolve_node_for_deploy_target`]) and an enabled deploy capability
/// row for that node.
///
/// # Errors
///
/// Propagates node resolution and capability lookup failures.
pub async fn preflight_deploy_target(
    target: &DeployTarget,
    valence: &Valence,
) -> Result<DeployTargetPreflight> {
    preflight_deploy_target_with_requirements(target, &NodeHardwareRequirements::default(), valence)
        .await
}

/// Like [`preflight_deploy_target`], but resolves against a workload hardware floor.
///
/// Use this from managed reconcile so the preflight verdict reflects the same hardware
/// constraints the eventual deploy will use, rather than passing preflight on a node the deploy
/// would then refuse.
///
/// # Errors
///
/// Propagates node resolution and capability lookup failures.
pub async fn preflight_deploy_target_with_requirements(
    target: &DeployTarget,
    requirements: &NodeHardwareRequirements,
    valence: &Valence,
) -> Result<DeployTargetPreflight> {
    let resolved_node_id =
        resolve_node_for_deploy_target_with_requirements(target, requirements, valence).await?;
    let capability_map = list_node_action_capability_map(valence).await?;
    let capability_enabled = is_action_enabled(
        &capability_map,
        &resolved_node_id,
        ContainerActionKind::Deploy,
    );
    let status = if capability_enabled {
        DeployPreflightStatus::Pass
    } else {
        DeployPreflightStatus::Fail
    };
    let message = if capability_enabled {
        format!("Deploy capability is enabled for resolved node '{resolved_node_id}'.")
    } else {
        format!("Deploy capability is disabled for resolved node '{resolved_node_id}'.")
    };
    Ok(DeployTargetPreflight {
        resolved_node_id,
        status,
        message,
        can_deploy: capability_enabled,
    })
}

/// Deploys a container image to a resolved node/pool target.
///
/// # Errors
///
/// Propagates preflight or executor failures.
pub async fn deploy_container_to_target(
    target: DeployTarget,
    container_ref: &str,
    image_ref: &str,
    env_vars: Vec<String>,
    port_mappings: Vec<String>,
    valence: &Valence,
) -> Result<RuntimeContainerActionResult> {
    deploy_container_to_target_with_command(
        target,
        container_ref,
        image_ref,
        env_vars,
        port_mappings,
        None,
        Vec::new(),
        valence,
    )
    .await
}

/// Like [`deploy_container_to_target`], but allows overriding entrypoint and image command (e.g. bootstrap scripts).
///
/// # Errors
///
/// Propagates preflight or executor failures.
#[allow(clippy::too_many_arguments)] // deploy wire surface carries entrypoint/command overrides
pub async fn deploy_container_to_target_with_command(
    target: DeployTarget,
    container_ref: &str,
    image_ref: &str,
    env_vars: Vec<String>,
    port_mappings: Vec<String>,
    deploy_entrypoint: Option<String>,
    deploy_command: Vec<String>,
    valence: &Valence,
) -> Result<RuntimeContainerActionResult> {
    let preflight = preflight_deploy_target(&target, valence).await?;
    if !preflight.can_deploy {
        anyhow::bail!("{}", preflight.message);
    }
    let node_id = preflight.resolved_node_id;
    execute_node_container_action_with_executor_and_request(
        &parton::DockerCliActionExecutor,
        &node_id,
        container_ref,
        ContainerActionKind::Deploy,
        None,
        Some(image_ref.to_string()),
        env_vars,
        port_mappings,
        deploy_entrypoint,
        deploy_command,
        valence,
    )
    .await
}

/// Ensures `image_ref` exists on the resolved deploy target (`docker image inspect` or `docker pull`).
///
/// Uses the same deploy capability preflight as [`deploy_container_to_target_with_command`].
///
/// # Errors
///
/// Propagates preflight or executor failures.
pub async fn ensure_docker_image_on_deploy_target(
    target: DeployTarget,
    image_ref: &str,
    valence: &Valence,
) -> Result<RuntimeContainerActionResult> {
    let preflight = preflight_deploy_target(&target, valence).await?;
    if !preflight.can_deploy {
        anyhow::bail!("{}", preflight.message);
    }
    let node_id = preflight.resolved_node_id;
    let image = image_ref.trim();
    if image.is_empty() {
        anyhow::bail!("image_ref must be non-empty for ensure_docker_image_on_deploy_target");
    }
    execute_node_container_action_with_executor_and_request(
        &parton::DockerCliActionExecutor,
        &node_id,
        "gluon-registry-runtime-image",
        ContainerActionKind::EnsureDockerImage,
        None,
        Some(image.to_string()),
        vec![],
        vec![],
        None,
        vec![],
        valence,
    )
    .await
}

#[cfg(test)]
mod node_eligible_tests {
    use super::node_eligible_for_deploy;
    use crate::generated::PionControlPlaneNodeStatus;

    #[test]
    fn online_draining_and_failed_may_queue_actions() {
        assert!(node_eligible_for_deploy(
            &PionControlPlaneNodeStatus::Online
        ));
        assert!(node_eligible_for_deploy(
            &PionControlPlaneNodeStatus::Draining
        ));
        assert!(node_eligible_for_deploy(
            &PionControlPlaneNodeStatus::Failed
        ));
    }

    #[test]
    fn offline_may_not_queue_actions() {
        assert!(!node_eligible_for_deploy(
            &PionControlPlaneNodeStatus::Offline
        ));
    }
}
