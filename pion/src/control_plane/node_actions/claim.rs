//! Claim path: hands the oldest eligible pending command for a node to the agent, with
//! secret-ref resolution and defense-in-depth claim-race detection.

use std::collections::HashMap;

use anyhow::{anyhow, Context, Result};
use chrono::Utc;
use valence::Error as ValenceError;
use valence::{SortDirection, StringPredicate, Valence};

use crate::control_plane::secret_resolver::{
    default_secret_resolver, resolve_secrets_in_json, value_contains_secret_ref_placeholder,
};
use crate::generated::{
    PionNodeActionCommand, PionNodeActionCommandMutable, PionNodeActionCommandStatus,
};
use crate::logging::NodeActionContext;

use super::shared::{
    clip, duration_from_u64_secs, get_node_action_treating_deletion_as_missing, CLIP,
};
use super::types::ClaimedNodeAction;

async fn prior_sequences_terminal(
    correlation_key: &str,
    node_id: &str,
    sequence: i64,
    valence: &Valence,
) -> Result<bool> {
    if correlation_key.is_empty() || sequence <= 0 {
        return Ok(true);
    }
    let rows = PionNodeActionCommand::query_used(valence, valence::use_!("query PionNodeActionCommand in control_plane/node_actions/claim.rs; Valence persistence for this feature path; typed store; visible to session actor / service path."))
        .where_correlation_key(StringPredicate::Equals(correlation_key.to_string()))
        .where_node_id(StringPredicate::Equals(node_id.to_string()))
        .await
        .with_context(|| {
            format!("query node action commands for correlation_key={correlation_key} node_id={node_id}")
        })?;
    for r in rows {
        if *r.sequence() >= sequence {
            continue;
        }
        match r.status() {
            PionNodeActionCommandStatus::Succeeded | PionNodeActionCommandStatus::Cancelled => {}
            _ => return Ok(false),
        }
    }
    Ok(true)
}

async fn resolve_claim_payload_or_mark_failed(
    valence: &Valence,
    id: &str,
    row: &PionNodeActionCommand,
    out_payload: &mut serde_json::Value,
) -> Result<bool> {
    if !value_contains_secret_ref_placeholder(out_payload) {
        return Ok(true);
    }
    let Some(res) = default_secret_resolver() else {
        PionNodeActionCommandMutable::get_used(id, valence, valence::use_!("get PionNodeActionCommandMutable in control_plane/node_actions/claim.rs; Valence persistence for this feature path; typed store; visible to session actor / service path."))
            .await
            .with_context(|| format!("load command {id} to mark secret-resolver-missing failure"))?
            .set_status(PionNodeActionCommandStatus::Failed)?
            .set_last_error(clip(
                "claim-time: payload has $secret_ref but no pion::SecretResolver installed; configure set_default_secret_resolver in the process host",
                CLIP,
            ))?
            .set_lease_expires_at(super::shared::lease_sentinel())?
            .set_updated_at(Utc::now())?
            .commit()
            .await
            .with_context(|| format!("commit secret-resolver-missing failure for command {id}"))?;
        let fail_ck = row.correlation_key().clone();
        crate::maybe_publish_setup_wizard_tracked_photon(&fail_ck, "failed").await;
        return Ok(false);
    };
    if let Err(e) = resolve_secrets_in_json(valence, res.as_ref(), out_payload).await {
        PionNodeActionCommandMutable::get_used(id, valence, valence::use_!("get PionNodeActionCommandMutable in control_plane/node_actions/claim.rs; Valence persistence for this feature path; typed store; visible to session actor / service path."))
            .await
            .with_context(|| format!("load command {id} to mark secret resolution failure"))?
            .set_status(PionNodeActionCommandStatus::Failed)?
            .set_last_error(clip(&format!("claim-time secret resolution: {e}"), CLIP))?
            .set_lease_expires_at(super::shared::lease_sentinel())?
            .set_updated_at(Utc::now())?
            .commit()
            .await
            .with_context(|| format!("commit secret resolution failure for command {id}"))?;
        let fail_ck = row.correlation_key().clone();
        crate::maybe_publish_setup_wizard_tracked_photon(&fail_ck, "failed").await;
        return Ok(false);
    }
    Ok(true)
}

/// Oldest eligible pending command for `node_id`, transitioned to `running` with a new lease.
///
/// # Contract
///
/// Claims in sequence order; skips rows whose prior correlation sequences are non-terminal.
/// Resolves `$secret_ref` placeholders in `payload_json` at claim when a [`crate::SecretResolver`] is installed.
///
/// # Errors
///
/// Propagates Valence errors. Marks the command `failed` when secret resolution fails.
///
/// # Examples
///
/// ```no_run
/// # async fn example(valence: &valence::Valence) -> anyhow::Result<()> {
/// use pion::claim_pending_node_action;
/// let claimed = claim_pending_node_action("node-1", 120, valence).await?;
/// // Ok(None) when the queue is empty; Ok(Some) yields a leased command id.
/// match &claimed {
///     Some(c) => assert!(c.command_id.starts_with("pna_")),
///     None => assert!(claimed.is_none()),
/// }
/// # Ok(())
/// # }
/// ```
///
/// # F12: concurrency note
///
/// Serializes concurrent claim attempts *for the same `node_id` within this process* via an
/// in-process per-node mutex, then additionally verifies the commit via a per-attempt
/// [`PionNodeActionCommandMutable::set_claim_fence`] fencing token (see that function's callers
/// below) before returning a claimed command. The in-process lock makes the common case (a single
/// `pion-server` instance processing overlapping heartbeats/retries for one node) race-free; the
/// fencing token is defense-in-depth against Valence's lack of a true compare-and-swap primitive,
/// but — absent a real DB-level CAS — cannot fully protect against multiple `pion-server`
/// *replicas* claiming for the same node concurrently. See `node_action_atomic_claim_integration.rs`.
#[tracing::instrument(skip(valence), fields(node_id = %node_id))]
pub async fn claim_pending_node_action(
    node_id: &str,
    lease_duration_secs: u64,
    valence: &Valence,
) -> Result<Option<ClaimedNodeAction>> {
    let started = std::time::Instant::now();
    let lock = claim_lock_for_node(node_id);
    // Critical section: the guard is held for the entire read-check-commit sequence in
    // `claim_pending_node_action_locked` (query pending rows, verify, and transition one to
    // `running`), so no two claim attempts for the same `node_id` can interleave within this
    // process.
    let _claim_guard = lock.lock().await;
    let result = claim_pending_node_action_locked(node_id, lease_duration_secs, valence).await;
    let outcome = match &result {
        Ok(Some(_)) => "claimed",
        Ok(None) => "no_content",
        Err(_) => "error",
    };
    crate::prom_metrics::record_node_action_claim(outcome, started.elapsed());
    result
}

/// Per-node-id in-process claim lock registry backing [`claim_pending_node_action`]'s
/// intra-process serialization (F12).
fn claim_lock_for_node(node_id: &str) -> std::sync::Arc<tokio::sync::Mutex<()>> {
    static LOCKS: std::sync::OnceLock<
        std::sync::Mutex<HashMap<String, std::sync::Arc<tokio::sync::Mutex<()>>>>,
    > = std::sync::OnceLock::new();
    let registry = LOCKS.get_or_init(|| std::sync::Mutex::new(HashMap::new()));
    let mut guard = registry
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    guard
        .entry(node_id.to_string())
        .or_insert_with(|| std::sync::Arc::new(tokio::sync::Mutex::new(())))
        .clone()
}

async fn claim_pending_node_action_locked(
    node_id: &str,
    lease_duration_secs: u64,
    valence: &Valence,
) -> Result<Option<ClaimedNodeAction>> {
    let lease_secs = lease_duration_secs.max(1);
    let mut pending: Vec<_> = PionNodeActionCommand::query_used(valence, valence::use_!("query PionNodeActionCommand in control_plane/node_actions/claim.rs; Valence persistence for this feature path; typed store; visible to session actor / service path."))
        .where_node_id(StringPredicate::Equals(node_id.to_string()))
        .where_status(StringPredicate::Equals(
            PionNodeActionCommandStatus::Pending.as_str().to_string(),
        ))
        .order_by_sequence(SortDirection::Asc)
        .order_by_created_at(SortDirection::Asc)
        .await
        .with_context(|| format!("query pending node action commands for node {node_id}"))?;

    pending.sort_by(|a, b| {
        a.sequence()
            .cmp(b.sequence())
            .then_with(|| a.created_at().cmp(b.created_at()))
    });

    for cmd in pending {
        let corr = cmd.correlation_key();
        if !prior_sequences_terminal(corr, node_id, *cmd.sequence(), valence).await? {
            continue;
        }

        let id = cmd
            .id()
            .and_then(|r| valence::extract_id_from_record(r).ok())
            .ok_or_else(|| anyhow!("command missing id"))?;

        if let Some(claimed) = try_claim_candidate(&id, node_id, lease_secs, valence).await? {
            return Ok(Some(claimed));
        }
    }

    Ok(None)
}

/// Attempts to claim a single pending candidate command already known to belong to `node_id`.
///
/// Returns `Ok(None)` for any reason the caller should skip to the next candidate (row no longer
/// pending, deleted mid-flight, or a lost race against a concurrent claimer — see
/// [`claim_pending_node_action`]'s F12 doc note); `Ok(Some(_))` once fully claimed.
async fn try_claim_candidate(
    id: &str,
    node_id: &str,
    lease_secs: u64,
    valence: &Valence,
) -> Result<Option<ClaimedNodeAction>> {
    let Some(fresh) = get_node_action_treating_deletion_as_missing(id, valence).await? else {
        return Ok(None);
    };
    if *fresh.status() != PionNodeActionCommandStatus::Pending {
        return Ok(None);
    }

    let now = Utc::now();
    let lease_end = now + duration_from_u64_secs(lease_secs);
    let next_attempt = *fresh.attempt() + 1;

    let Some(again) = get_node_action_treating_deletion_as_missing(id, valence).await? else {
        return Ok(None);
    };
    if *again.status() != PionNodeActionCommandStatus::Pending {
        return Ok(None);
    }

    let mut_row = match PionNodeActionCommandMutable::get_used(id, valence, valence::use_!("get PionNodeActionCommandMutable in control_plane/node_actions/claim.rs; Valence persistence for this feature path; typed store; visible to session actor / service path.")).await {
        Ok(r) => r,
        Err(ValenceError::PendingDeletion(_)) => return Ok(None),
        Err(e) => {
            return Err(anyhow::Error::from(e))
                .with_context(|| format!("load command {id} for claim"))
        }
    };
    // Valence has no compare-and-swap / conditional-update primitive (`commit()` is an
    // unconditional last-write-wins overwrite), so two concurrent claimers can both read the
    // same `pending` row and both `commit()` a transition to `running`. `lease_expires_at` can't
    // be used to detect this after the fact: some backends round-trip `DateTime<Utc>` through
    // storage at whole-second precision, so two commits within the same second are
    // indistinguishable by timestamp. Instead, stamp the row with a fresh random `claim_fence`
    // token on every claim attempt and re-read it immediately after commit — if a concurrent
    // claimer's write landed after ours, the freshly-read token will be theirs, not ours, and we
    // abandon the claim rather than hand the same command to two agents. See
    // `node_action_atomic_claim_integration.rs`.
    let fence = uuid::Uuid::new_v4().to_string();
    let row = mut_row
        .set_status(PionNodeActionCommandStatus::Running)?
        .set_attempt(next_attempt)?
        .set_lease_expires_at(lease_end)?
        .set_claim_fence(fence.clone())?
        .set_updated_at(now)?
        .commit()
        .await
        .with_context(|| format!("commit claim transition for command {id}"))?;

    if !won_claim_race(id, &fence, valence).await? {
        let ctx = NodeActionContext::new(id, node_id, row.action_kind())
            .with_correlation(row.correlation_key(), *row.sequence());
        ctx.log_claim(
            "lost claim race to a concurrent claimer (post-commit verify mismatch); abandoning",
        );
        return Ok(None);
    }

    let corr_opt = {
        let c = row.correlation_key();
        if c.is_empty() {
            None
        } else {
            Some(c.clone())
        }
    };
    let seq_opt = if *row.sequence() == 0 {
        None
    } else {
        Some(*row.sequence())
    };
    let mut out_payload = row.payload_json().clone();
    if !resolve_claim_payload_or_mark_failed(valence, id, &row, &mut out_payload).await? {
        tracing::warn!(
            target: "security.claim",
            command_id = %id,
            node_id = %node_id,
            action_kind = %row.action_kind(),
            "claim failed: secret resolution error"
        );
        return Ok(None);
    }
    {
        let ctx = NodeActionContext::new(id, node_id, row.action_kind())
            .with_correlation(corr_opt.as_deref().unwrap_or(""), seq_opt.unwrap_or(0))
            .with_attempt(*row.attempt() as i64);
        ctx.log_claim(&format!(
            "claimed correlation_key={corr_opt:?} sequence={seq_opt:?}"
        ));
    }
    tracing::info!(
        target: "security.claim",
        command_id = %id,
        node_id = %node_id,
        action_kind = %row.action_kind(),
        attempt = *row.attempt(),
        "node action claimed"
    );
    let ck = row.correlation_key().clone();
    crate::bootstrap_notify::notify_if_wizard_run(valence, &ck).await;
    crate::maybe_publish_setup_wizard_tracked_photon(&ck, "running").await;
    Ok(Some(ClaimedNodeAction {
        command_id: id.to_string(),
        action_kind: row.action_kind().clone(),
        payload_json: out_payload,
        attempt: *row.attempt(),
        correlation_key: corr_opt,
        sequence: seq_opt,
    }))
}

/// Re-reads the command row after `commit()` and checks that our `fence` value is still present
/// (see [`try_claim_candidate`]'s F12 doc note on why a random per-attempt token, not a
/// timestamp, is used to detect an overwrite by a concurrent claimer).
async fn won_claim_race(id: &str, fence: &str, valence: &Valence) -> Result<bool> {
    let Some(verify) = get_node_action_treating_deletion_as_missing(id, valence).await? else {
        return Ok(false);
    };
    Ok(*verify.status() == PionNodeActionCommandStatus::Running && verify.claim_fence() == fence)
}
