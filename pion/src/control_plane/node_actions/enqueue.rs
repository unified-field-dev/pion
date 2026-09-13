//! Enqueue path: idempotent creation of pending command rows keyed by
//! `(correlation_key, sequence, node_id)`.

use anyhow::Context;

use super::error::NodeActionError;
use chrono::Utc;
use valence::{Model, Valence};

use crate::logging::{enqueue_noop_message, NodeActionContext};

use super::shared::lease_sentinel;
use super::types::action_kind_to_capability;
use crate::control_plane::capabilities::{is_action_enabled, list_node_action_capability_map};
use crate::control_plane::deploy::node_eligible_for_deploy;
use crate::generated::{PionControlPlaneNode, PionNodeActionCommand, PionNodeActionCommandStatus};

use super::shared::get_node_action_treating_deletion_as_missing;

fn deterministic_command_id(correlation_key: &str, sequence: i64, node_id: &str) -> String {
    let mut hasher = blake3::Hasher::new();
    hasher.update(b"pion_node_action_command/v1\0");
    hasher.update(correlation_key.as_bytes());
    hasher.update(b"\0");
    hasher.update(&sequence.to_be_bytes());
    hasher.update(b"\0");
    hasher.update(node_id.as_bytes());
    let hex = hasher.finalize().to_hex();
    format!("pna_{}", &hex[..32])
}

fn parse_enqueue_key(
    correlation_key: Option<&str>,
    sequence: Option<i64>,
) -> Result<(String, i64), NodeActionError> {
    let corr = correlation_key
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .ok_or(NodeActionError::InvalidEnqueueKey(
            "enqueue_node_action requires non-empty correlation_key",
        ))?
        .to_string();
    let seq = sequence.ok_or(NodeActionError::InvalidEnqueueKey(
        "enqueue_node_action requires sequence",
    ))?;
    if seq < 1 {
        return Err(NodeActionError::InvalidEnqueueKey(
            "enqueue_node_action requires sequence >= 1",
        ));
    }
    Ok((corr, seq))
}

/// Creates a pending command row targeting `node_id`.
///
/// # Contract
///
/// Delegates to [`enqueue_node_action_idempotent`] with `replan_token` unset. Requires a
/// non-empty `correlation_key` and `sequence >= 1` for deterministic command ids.
///
/// # Errors
///
/// Returns [`NodeActionError`] when the node is missing/ineligible, capability-disabled, the enqueue
/// key is invalid, or Valence upsert fails ([`NodeActionError::Internal`]).
///
/// # Examples
///
/// ```no_run
/// # async fn example(valence: &valence::Valence) -> anyhow::Result<()> {
/// use pion::enqueue_node_action;
/// let id = enqueue_node_action(
///     "node-1", "home", "deploy_handoff", serde_json::json!({}), 3,
///     Some("corr-1"), Some(1), valence,
/// ).await?;
/// assert!(id.starts_with("pna_"));
/// # Ok(())
/// # }
/// ```
#[allow(clippy::too_many_arguments)] // wire/API enqueue surface mirrors HTTP handler params
#[tracing::instrument(skip(payload, valence), fields(node_id = %node_id, action_kind = %action_kind))]
pub async fn enqueue_node_action(
    node_id: &str,
    cell_id: &str,
    action_kind: &str,
    payload: serde_json::Value,
    max_attempts: i64,
    correlation_key: Option<&str>,
    sequence: Option<i64>,
    valence: &Valence,
) -> Result<String, NodeActionError> {
    enqueue_node_action_idempotent(
        node_id,
        cell_id,
        action_kind,
        payload,
        max_attempts,
        correlation_key,
        sequence,
        None,
        valence,
    )
    .await
}

/// Ensure the target node exists, is deploy-eligible, and has the action capability enabled.
async fn ensure_node_enqueueable(
    node_id: &str,
    action_kind: &str,
    valence: &Valence,
) -> Result<(), NodeActionError> {
    let node = PionControlPlaneNode::get_used(node_id, valence, valence::use_!(r#"In **Pion control plane**, we **load Pion Control Plane Node** so the application can decide what to do next in this workflow. The result is used by **Pion control plane** logic—not necessarily displayed on a page unless that feature’s UI shows it."#))
        .await
        .with_context(|| format!("load node {node_id} for enqueue"))
        .map_err(NodeActionError::Internal)?
        .ok_or_else(|| NodeActionError::NodeNotFound {
            node_id: node_id.to_string(),
        })?;
    if !node_eligible_for_deploy(node.status()) {
        tracing::warn!(
            target: "security.deploy",
            node_id = %node_id,
            action_kind = %action_kind,
            status = %node.status().as_str(),
            "enqueue rejected: node ineligible for deploy"
        );
        return Err(NodeActionError::NodeIneligible {
            node_id: node_id.to_string(),
            status: node.status().as_str().to_string(),
        });
    }
    if let Some(cap) = action_kind_to_capability(action_kind) {
        let map = list_node_action_capability_map(valence)
            .await
            .context("list node action capability map for enqueue")
            .map_err(NodeActionError::Internal)?;
        if !is_action_enabled(&map, node_id, cap) {
            tracing::warn!(
                target: "security.deploy",
                node_id = %node_id,
                action_kind = %action_kind,
                "enqueue rejected: capability disabled"
            );
            return Err(NodeActionError::CapabilityDenied {
                action_kind: action_kind.to_string(),
                node_id: node_id.to_string(),
            });
        }
    }
    Ok(())
}

/// Idempotent enqueue keyed by `(correlation_key, sequence, node_id)`.
///
/// # Contract
///
/// Returns the existing command id when a row is already `pending`/`running`, or terminal without
/// `replan_token`. A non-empty `replan_token` allows re-enqueue after terminal states.
///
/// # Errors
///
/// Same as [`enqueue_node_action`] ([`NodeActionError`]), including invalid correlation/sequence.
///
/// # Examples
///
/// ```no_run
/// # async fn example(valence: &valence::Valence) -> anyhow::Result<()> {
/// use pion::enqueue_node_action_idempotent;
/// let id = enqueue_node_action_idempotent(
///     "node-1", "home", "deploy_handoff", serde_json::json!({}), 3,
///     Some("corr-1"), Some(1), None, valence,
/// ).await?;
/// assert!(id.starts_with("pna_"));
/// # Ok(())
/// # }
/// ```
#[allow(clippy::too_many_arguments)] // idempotent enqueue carries correlation + replan fields
pub async fn enqueue_node_action_idempotent(
    node_id: &str,
    cell_id: &str,
    action_kind: &str,
    payload: serde_json::Value,
    max_attempts: i64,
    correlation_key: Option<&str>,
    sequence: Option<i64>,
    replan_token: Option<&str>,
    valence: &Valence,
) -> Result<String, NodeActionError> {
    ensure_node_enqueueable(node_id, action_kind, valence).await?;

    let now = Utc::now();
    let (corr, seq) = parse_enqueue_key(correlation_key, sequence)?;
    let attempts = if max_attempts < 1 { 1 } else { max_attempts };
    let cmd_id = deterministic_command_id(&corr, seq, node_id);
    if let Some(existing) = get_node_action_treating_deletion_as_missing(&cmd_id, valence).await? {
        match existing.status() {
            PionNodeActionCommandStatus::Pending | PionNodeActionCommandStatus::Running => {
                let ctx = NodeActionContext::new(&cmd_id, node_id, action_kind)
                    .with_correlation(&corr, seq);
                ctx.log_enqueue(
                    "noop",
                    &enqueue_noop_message(existing.status().as_str(), false),
                );
                return Ok(cmd_id);
            }
            PionNodeActionCommandStatus::Succeeded
            | PionNodeActionCommandStatus::Failed
            | PionNodeActionCommandStatus::Cancelled => {
                let has_replan_token = replan_token
                    .map(str::trim)
                    .is_some_and(|token| !token.is_empty());
                if !has_replan_token {
                    let ctx = NodeActionContext::new(&cmd_id, node_id, action_kind)
                        .with_correlation(&corr, seq);
                    ctx.log_enqueue(
                        "noop",
                        &enqueue_noop_message(existing.status().as_str(), true),
                    );
                    return Ok(cmd_id);
                }
            }
        }
    }
    {
        let ctx =
            NodeActionContext::new(&cmd_id, node_id, action_kind).with_correlation(&corr, seq);
        ctx.log_enqueue(
            "enqueued",
            &format!("enqueued max_attempts={attempts} correlation_key={corr} sequence={seq}"),
        );
    }
    let row = PionNodeActionCommand::new(
        node_id.to_string(),
        cell_id.to_string(),
        action_kind.to_string(),
        payload,
        PionNodeActionCommandStatus::Pending,
        0,
        attempts,
        lease_sentinel(),
        corr.clone(),
        seq,
        String::new(),
        String::new(),
        now,
        now,
    )
    .context("build node action command row for enqueue")?;
    PionNodeActionCommand::upsert_used(&cmd_id, row, valence, valence::use_!(r#"When **Pion control plane** needs to persist work, we **save Pion Node Action Command** so the next step in that feature can continue with the latest values. People and services allowed for **Pion control plane** use this data for that workflow—not as a general export of unrelated personal fields."#))
        .await
        .with_context(|| format!("upsert node action command {cmd_id}"))?;
    tracing::info!(
        target: "security.deploy",
        node_id = %node_id,
        action_kind = %action_kind,
        command_id = %cmd_id,
        correlation_key = %corr,
        sequence = seq,
        "node action enqueued"
    );
    crate::bootstrap_notify::notify_if_wizard_run(valence, &corr).await;
    crate::maybe_publish_setup_wizard_tracked_photon(&corr, "pending").await;
    Ok(cmd_id)
}
