//! Lifecycle operations on the node action command queue: cancellation, timeout/lease
//! reconciliation, lease extension, single-command lookup, and env-tunable defaults.

use std::collections::HashSet;

use anyhow::{anyhow, Context, Result};

use super::error::NodeActionError;
use chrono::{Duration, Utc};
use valence::{Model, StringPredicate, Valence};

use crate::generated::{
    PionNodeActionCommand, PionNodeActionCommandMutable, PionNodeActionCommandStatus,
};
use crate::logging::NodeActionContext;

use super::shared::{
    duration_from_u64_secs, get_node_action_treating_deletion_as_missing,
    is_node_recently_heartbeating, lease_sentinel, DEFAULT_LEASE_SECS, DEFAULT_MAX_ATTEMPTS,
    DEFAULT_PENDING_TIMEOUT_SECS,
};

async fn reconcile_stale_pending_node_action(
    valence: &Valence,
    id: &str,
    cmd: &PionNodeActionCommand,
    ck: &str,
    now: chrono::DateTime<Utc>,
) -> Result<()> {
    if is_node_recently_heartbeating(valence, cmd.node_id(), 60).await {
        if PionNodeActionCommandMutable::get_used(id, valence, valence::use_!(r#"In **Pion control plane**, we **update Pion Node Action Command Mutable** in place so saved changes apply on the next read. The same actors who can run **Pion control plane** use the updated values; this step is not a silent copy to an external marketing system."#))
            .await
            .with_context(|| format!("load command {id} to extend pending window"))?
            .set_created_at(now)?
            .set_updated_at(now)?
            .commit()
            .await
            .is_ok()
        {
            let ctx = NodeActionContext::new(id, cmd.node_id(), cmd.action_kind())
                .with_correlation(cmd.correlation_key(), *cmd.sequence());
            ctx.log_recovery(
                "pending command is online again; extended pending window (unclaimed-timeout recovery)",
            );
        }
        return Ok(());
    }
    if PionNodeActionCommandMutable::get_used(id, valence, valence::use_!(r#"In **Pion control plane**, we **update Pion Node Action Command Mutable** in place so saved changes apply on the next read. The same actors who can run **Pion control plane** use the updated values; this step is not a silent copy to an external marketing system."#))
        .await
        .with_context(|| format!("load command {id} to mark stale-pending timeout"))?
        .set_status(PionNodeActionCommandStatus::Failed)?
        .set_last_error("No agent claimed this command within timeout".to_string())?
        .set_updated_at(now)?
        .commit()
        .await
        .is_ok()
    {
        crate::bootstrap_notify::notify_if_wizard_run(valence, ck).await;
        crate::maybe_publish_setup_wizard_tracked_photon(ck, "failed").await;
    }
    Ok(())
}

async fn reconcile_expired_lease_node_action(
    valence: &Valence,
    id: &str,
    cmd: &PionNodeActionCommand,
    ck: &str,
    now: chrono::DateTime<Utc>,
) -> Result<()> {
    if *cmd.attempt() >= *cmd.max_attempts() {
        if PionNodeActionCommandMutable::get_used(id, valence, valence::use_!(r#"In **Pion control plane**, we **update Pion Node Action Command Mutable** in place so saved changes apply on the next read. The same actors who can run **Pion control plane** use the updated values; this step is not a silent copy to an external marketing system."#))
            .await
            .with_context(|| format!("load command {id} to mark max-attempts lease expiry"))?
            .set_status(PionNodeActionCommandStatus::Failed)?
            .set_last_error("lease expired after max attempts".to_string())?
            .set_updated_at(now)?
            .commit()
            .await
            .is_ok()
        {
            crate::bootstrap_notify::notify_if_wizard_run(valence, ck).await;
            crate::maybe_publish_setup_wizard_tracked_photon(ck, "failed").await;
        }
    } else if PionNodeActionCommandMutable::get_used(id, valence, valence::use_!(r#"In **Pion control plane**, we **update Pion Node Action Command Mutable** in place so saved changes apply on the next read. The same actors who can run **Pion control plane** use the updated values; this step is not a silent copy to an external marketing system."#))
        .await
        .with_context(|| format!("load command {id} to retry after lease expiry"))?
        .set_status(PionNodeActionCommandStatus::Pending)?
        .set_lease_expires_at(lease_sentinel())?
        .set_updated_at(now)?
        .commit()
        .await
        .is_ok()
    {
        crate::bootstrap_notify::notify_if_wizard_run(valence, ck).await;
        crate::maybe_publish_setup_wizard_tracked_photon(ck, "lease_expired_retry").await;
    }
    Ok(())
}

/// Marks a non-terminal command as cancelled (operator / orchestrator).
///
/// # Errors
///
/// Returns `Err` when the command is unknown or Valence update fails.
pub async fn cancel_node_action(command_id: &str, valence: &Valence) -> Result<()> {
    let cmd = PionNodeActionCommand::get_used(command_id, valence, valence::use_!(r#"In **Pion control plane**, we **load Pion Node Action Command** so the application can decide what to do next in this workflow. The result is used by **Pion control plane** logic—not necessarily displayed on a page unless that feature’s UI shows it."#))
        .await
        .with_context(|| format!("load command {command_id} for cancel"))?
        .ok_or_else(|| anyhow!("unknown command {command_id}"))?;
    let ck = cmd.correlation_key().clone();
    let cancelled = cancel_node_action_if_non_terminal(command_id, valence).await?;
    if cancelled {
        crate::bootstrap_notify::notify_if_wizard_run(valence, &ck).await;
    }
    Ok(())
}

async fn cancel_node_action_if_non_terminal(command_id: &str, valence: &Valence) -> Result<bool> {
    let cmd = PionNodeActionCommand::get_used(command_id, valence, valence::use_!(r#"In **Pion control plane**, we **load Pion Node Action Command** so the application can decide what to do next in this workflow. The result is used by **Pion control plane** logic—not necessarily displayed on a page unless that feature’s UI shows it."#))
        .await
        .with_context(|| format!("load command {command_id} for cancel-if-non-terminal"))?
        .ok_or_else(|| anyhow!("unknown command {command_id}"))?;
    if matches!(
        cmd.status(),
        PionNodeActionCommandStatus::Succeeded
            | PionNodeActionCommandStatus::Failed
            | PionNodeActionCommandStatus::Cancelled
    ) {
        return Ok(false);
    }
    let now = Utc::now();
    PionNodeActionCommandMutable::get_used(command_id, valence, valence::use_!(r#"In **Pion control plane**, we **update Pion Node Action Command Mutable** in place so saved changes apply on the next read. The same actors who can run **Pion control plane** use the updated values; this step is not a silent copy to an external marketing system."#))
        .await
        .with_context(|| format!("load command {command_id} to cancel"))?
        .set_status(PionNodeActionCommandStatus::Cancelled)?
        .set_updated_at(now)?
        .commit()
        .await
        .with_context(|| format!("commit cancellation for command {command_id}"))?;
    Ok(true)
}

/// Expired leases and stale pending commands → retry or terminal failure.
///
/// # Errors
///
/// Propagates Valence query or update failures.
pub async fn reconcile_node_action_commands(valence: &Valence) -> Result<()> {
    let now = Utc::now();
    let rows = PionNodeActionCommand::query_used(valence, valence::use_!(r#"In **Pion control plane**, we **list Pion Node Action Command** so the product can show or process the matching set for this workflow. Callers allowed for **Pion control plane** use the list; it is not a public dump of every field to anonymous visitors."#))
        .await
        .context("query node action commands for reconcile")?;

    for cmd in rows {
        let Some(id) = cmd
            .id()
            .and_then(|r| valence::extract_id_from_record(r).ok())
        else {
            continue;
        };
        let ck = cmd.correlation_key().clone();

        match cmd.status() {
            PionNodeActionCommandStatus::Pending
                if *cmd.created_at() + pending_timeout_duration() < now =>
            {
                reconcile_stale_pending_node_action(valence, &id, &cmd, &ck, now).await?;
            }
            PionNodeActionCommandStatus::Running
                if *cmd.lease_expires_at() > lease_sentinel() && *cmd.lease_expires_at() < now =>
            {
                reconcile_expired_lease_node_action(valence, &id, &cmd, &ck, now).await?;
            }
            _ => {}
        }
    }

    Ok(())
}

/// Load one queued command by id for HTTP handlers and tests.
///
/// Returns `Ok(None)` when the id is missing **or** the row was soft-deleted
/// (deletion is treated as absent so handlers can respond 404 uniformly).
///
/// # Errors
///
/// Propagates Valence query failures.
pub async fn get_node_action_command(
    command_id: &str,
    valence: &Valence,
) -> Result<Option<PionNodeActionCommand>> {
    get_node_action_treating_deletion_as_missing(command_id, valence).await
}

/// Default retry budget for newly enqueued commands.
///
/// Reads `GLUON_NODE_ACTION_MAX_ATTEMPTS` (positive `i64`); defaults to **3** when unset
/// or invalid.
#[must_use]
pub fn default_max_attempts() -> i64 {
    std::env::var("GLUON_NODE_ACTION_MAX_ATTEMPTS")
        .ok()
        .and_then(|s| s.parse::<i64>().ok())
        .filter(|&n| n > 0)
        .unwrap_or(DEFAULT_MAX_ATTEMPTS)
}

/// Max time a command may stay `pending` before reconcile marks it failed (never claimed).
///
/// Reads `GLUON_NODE_ACTION_PENDING_TIMEOUT_SECS` (positive seconds); defaults to **300**.
#[must_use]
pub fn pending_timeout_duration() -> Duration {
    std::env::var("GLUON_NODE_ACTION_PENDING_TIMEOUT_SECS")
        .ok()
        .and_then(|s| s.parse::<i64>().ok())
        .filter(|&n| n > 0)
        .map_or_else(
            || Duration::seconds(DEFAULT_PENDING_TIMEOUT_SECS),
            Duration::seconds,
        )
}

/// Default claim lease duration when the agent omits `lease_duration_secs`.
///
/// Reads `GLUON_NODE_ACTION_LEASE_SECS` (positive seconds); defaults to **120**.
#[must_use]
pub fn default_lease_duration_secs() -> u64 {
    std::env::var("GLUON_NODE_ACTION_LEASE_SECS")
        .ok()
        .and_then(|s| s.parse::<u64>().ok())
        .filter(|&n| n > 0)
        .unwrap_or(DEFAULT_LEASE_SECS)
}

/// Cancels every non-terminal command sharing `correlation_key`.
///
/// # Errors
///
/// Propagates Valence query or update failures.
pub async fn cancel_non_terminal_node_actions_for_correlation(
    correlation_key: &str,
    valence: &Valence,
) -> Result<()> {
    if correlation_key.trim().is_empty() {
        return Ok(());
    }
    let rows = PionNodeActionCommand::query_used(valence, valence::use_!(r#"In **Pion control plane**, we **list Pion Node Action Command** so the product can show or process the matching set for this workflow. Callers allowed for **Pion control plane** use the list; it is not a public dump of every field to anonymous visitors."#))
        .where_correlation_key(StringPredicate::Equals(correlation_key.to_string()))
        .await
        .with_context(|| format!("query node action commands for correlation {correlation_key}"))?;
    let mut any = false;
    for cmd in rows {
        let Some(id) = cmd
            .id()
            .and_then(|r| valence::extract_id_from_record(r).ok())
        else {
            continue;
        };
        if cancel_node_action_if_non_terminal(&id, valence).await? {
            any = true;
        }
    }
    if any {
        crate::bootstrap_notify::notify_if_wizard_run(valence, correlation_key).await;
    }
    Ok(())
}

/// Resets **failed** commands at the lowest failed `sequence` back to `pending`, then cancels
/// stale `running` rows and extra `pending` rows (so we never cancel a row we just revived).
///
/// Returns `Some(min_failed_sequence)` when at least one failed row was reset; `None` when there
/// were no failed rows (caller may re-enqueue deploy, e.g. after global timeout left only pending).
///
/// # Errors
///
/// Propagates Valence query or update failures; bails when failed rows cannot be reset.
pub async fn reset_failed_node_actions_for_correlation(
    correlation_key: &str,
    valence: &Valence,
) -> Result<Option<i64>> {
    if correlation_key.trim().is_empty() {
        return Ok(None);
    }
    let rows = PionNodeActionCommand::query_used(valence, valence::use_!(r#"In **Pion control plane**, we **list Pion Node Action Command** so the product can show or process the matching set for this workflow. Callers allowed for **Pion control plane** use the list; it is not a public dump of every field to anonymous visitors."#))
        .where_correlation_key(StringPredicate::Equals(correlation_key.to_string()))
        .await
        .with_context(|| format!("query node action commands for correlation {correlation_key}"))?;

    let failed_min_seq = rows
        .iter()
        .filter(|c| *c.status() == PionNodeActionCommandStatus::Failed)
        .map(|c| *c.sequence())
        .min();

    let mut reset_ids: HashSet<String> = HashSet::new();
    let now = Utc::now();

    if let Some(m) = failed_min_seq {
        for cmd in rows
            .iter()
            .filter(|c| *c.status() == PionNodeActionCommandStatus::Failed && *c.sequence() == m)
        {
            let Some(id) = cmd
                .id()
                .and_then(|r| valence::extract_id_from_record(r).ok())
            else {
                continue;
            };
            PionNodeActionCommandMutable::get_used(&id, valence, valence::use_!(r#"In **Pion control plane**, we **update Pion Node Action Command Mutable** in place so saved changes apply on the next read. The same actors who can run **Pion control plane** use the updated values; this step is not a silent copy to an external marketing system."#))
                .await
                .with_context(|| format!("load command {id} to reset for replan"))?
                .set_status(PionNodeActionCommandStatus::Pending)?
                .set_attempt(0)?
                .set_lease_expires_at(lease_sentinel())?
                .set_last_error(String::new())?
                .set_updated_at(now)?
                .commit()
                .await
                .with_context(|| format!("commit replan reset for command {id}"))?;
            reset_ids.insert(id);
        }
        if reset_ids.is_empty() {
            anyhow::bail!(
                "failed node actions for correlation {correlation_key} could not be reset (missing ids)"
            );
        }
    }

    let min_cleanup = failed_min_seq.unwrap_or(1);

    let rows2 = PionNodeActionCommand::query_used(valence, valence::use_!(r#"In **Pion control plane**, we **list Pion Node Action Command** so the product can show or process the matching set for this workflow. Callers allowed for **Pion control plane** use the list; it is not a public dump of every field to anonymous visitors."#))
        .where_correlation_key(StringPredicate::Equals(correlation_key.to_string()))
        .await
        .with_context(|| {
            format!("re-query node action commands for correlation {correlation_key}")
        })?;

    let mut any_cancel = false;
    for cmd in rows2 {
        let Some(id) = cmd
            .id()
            .and_then(|r| valence::extract_id_from_record(r).ok())
        else {
            continue;
        };
        let seq = *cmd.sequence();
        match cmd.status() {
            PionNodeActionCommandStatus::Running
                if cancel_node_action_if_non_terminal(&id, valence).await? =>
            {
                any_cancel = true;
            }
            PionNodeActionCommandStatus::Pending => {
                let is_revived_frontier = seq == min_cleanup && reset_ids.contains(&id);
                if !is_revived_frontier && cancel_node_action_if_non_terminal(&id, valence).await? {
                    any_cancel = true;
                }
            }
            _ => {}
        }
    }

    if any_cancel || !reset_ids.is_empty() {
        crate::bootstrap_notify::notify_if_wizard_run(valence, correlation_key).await;
    }

    Ok(failed_min_seq)
}

/// Extends the lease for a `running` command owned by `node_id` (long-running Docker / HTTP).
///
/// # Errors
///
/// Returns [`NodeActionError`] when the command is unknown, targets another node, is not running,
/// or Valence update fails ([`NodeActionError::Internal`]).
pub async fn extend_node_action_lease(
    command_id: &str,
    node_id: &str,
    additional_secs: u64,
    valence: &Valence,
) -> Result<(), NodeActionError> {
    let add = additional_secs.max(1);
    let cmd = PionNodeActionCommand::get_used(command_id, valence, valence::use_!(r#"In **Pion control plane**, we **load Pion Node Action Command** so the application can decide what to do next in this workflow. The result is used by **Pion control plane** logic—not necessarily displayed on a page unless that feature’s UI shows it."#))
        .await
        .with_context(|| format!("load command {command_id} for lease extend"))?
        .ok_or_else(|| NodeActionError::CommandNotFound {
            command_id: command_id.to_string(),
        })?;
    let ck = cmd.correlation_key().clone();
    if cmd.node_id() != node_id {
        return Err(NodeActionError::NodeMismatch {
            command_id: command_id.to_string(),
        });
    }
    if *cmd.status() != PionNodeActionCommandStatus::Running {
        return Err(NodeActionError::NotRunning {
            command_id: command_id.to_string(),
            status: format!("{:?}", cmd.status()),
        });
    }
    let now = Utc::now();
    let current_end = *cmd.lease_expires_at();
    let base = if current_end > lease_sentinel() && current_end > now {
        current_end
    } else {
        now
    };
    let new_end = base + duration_from_u64_secs(add);
    PionNodeActionCommandMutable::get_used(command_id, valence, valence::use_!(r#"In **Pion control plane**, we **update Pion Node Action Command Mutable** in place so saved changes apply on the next read. The same actors who can run **Pion control plane** use the updated values; this step is not a silent copy to an external marketing system."#))
        .await
        .with_context(|| format!("load command {command_id} to extend lease"))?
        .set_lease_expires_at(new_end)?
        .set_updated_at(now)?
        .commit()
        .await
        .with_context(|| format!("commit lease extend for command {command_id}"))?;
    {
        let ctx = NodeActionContext::new(command_id, node_id, "").with_correlation(&ck, 0);
        ctx.log_lease_extend(&format!("lease_extended until={}", new_end.to_rfc3339()));
    }
    crate::bootstrap_notify::notify_if_wizard_run(valence, &ck).await;
    crate::maybe_publish_setup_wizard_tracked_photon(&ck, "lease_extended").await;
    Ok(())
}
