//! Report path: persists agent-reported outcomes for claimed commands and drives the
//! retry/terminal-failure transition.

use anyhow::Context;

use super::error::NodeActionError;
use chrono::Utc;
use valence::{Model, Valence};

use crate::generated::{
    PionNodeActionCommand, PionNodeActionCommandMutable, PionNodeActionCommandStatus,
    PionNodeActionResult,
};
use crate::logging::{terminal_failure_message, NodeActionContext};

use super::shared::{clip, lease_sentinel, CLIP};
use super::types::ReportNodeActionResult;

fn preferred_failure_message(stderr: &str, err_sum: &str) -> String {
    let st = stderr.trim();
    let es = err_sum.trim();
    if !st.is_empty() && !es.is_empty() {
        if st.len() >= es.len() {
            st.to_string()
        } else {
            es.to_string()
        }
    } else if !st.is_empty() {
        st.to_string()
    } else if !es.is_empty() {
        es.to_string()
    } else {
        "action failed".to_string()
    }
}

#[allow(clippy::too_many_arguments)] // failure-report helper mirrors report_node_action_result fields
async fn finish_failed_node_action_report(
    command_id: &str,
    node_id: &str,
    action_kind: &str,
    attempt: i64,
    cmd: &PionNodeActionCommand,
    correlation_key: &str,
    last: &str,
    stderr_len: usize,
    now: chrono::DateTime<Utc>,
    valence: &Valence,
) -> Result<(), NodeActionError> {
    if *cmd.attempt() < *cmd.max_attempts() {
        PionNodeActionCommandMutable::get(command_id, valence)
            .await
            .with_context(|| format!("load command {command_id} to requeue after failure"))?
            .set_status(PionNodeActionCommandStatus::Pending)?
            .set_last_error(clip(last, CLIP))?
            .set_lease_expires_at(lease_sentinel())?
            .set_updated_at(now)?
            .commit()
            .await
            .with_context(|| format!("commit retry-pending transition for command {command_id}"))?;
        {
            let ctx = NodeActionContext::new(command_id, node_id, action_kind)
                .with_correlation(correlation_key, *cmd.sequence())
                .with_attempt(attempt);
            ctx.log_result("retry", "result retry_pending");
        }
        crate::maybe_publish_setup_wizard_tracked_photon(correlation_key, "retry_pending").await;
    } else {
        PionNodeActionCommandMutable::get(command_id, valence)
            .await
            .with_context(|| format!("load command {command_id} to mark terminal failure"))?
            .set_status(PionNodeActionCommandStatus::Failed)?
            .set_last_error(clip(last, CLIP))?
            .set_updated_at(now)?
            .commit()
            .await
            .with_context(|| format!("commit terminal failure for command {command_id}"))?;
        {
            let ctx = NodeActionContext::new(command_id, node_id, action_kind)
                .with_correlation(correlation_key, *cmd.sequence())
                .with_attempt(attempt);
            ctx.log_result(
                "failed",
                &terminal_failure_message(&clip(last, 512), stderr_len),
            );
        }
        crate::maybe_publish_setup_wizard_tracked_photon(correlation_key, "failed").await;
    }

    crate::bootstrap_notify::notify_if_wizard_run(valence, correlation_key).await;
    Ok(())
}

/// Persist agent-reported outcome and update command status.
///
/// # Contract
///
/// Accepts only `running` commands whose `attempt` matches the body. Success marks `succeeded`;
/// failure requeues as `pending` until `max_attempts`, then `failed`.
///
/// # Errors
///
/// Returns [`NodeActionError`] for unknown commands, node mismatch, wrong status/attempt, or
/// Valence failures ([`NodeActionError::Internal`]).
///
/// # Examples
///
/// ```no_run
/// # async fn example(valence: &valence::Valence) -> anyhow::Result<()> {
/// use pion::{report_node_action_result, ReportNodeActionResult};
/// report_node_action_result(ReportNodeActionResult {
///     command_id: "pna_abc".into(), node_id: "node-1".into(), attempt: 1,
///     success: true, stdout: None, stderr: None, error_summary: None, payload_json: None,
/// }, valence).await?;
/// // Observable: `Ok(())` — mismatches yield `NodeActionError`.
/// # Ok(())
/// # }
/// ```
pub async fn report_node_action_result(
    body: ReportNodeActionResult,
    valence: &Valence,
) -> Result<(), NodeActionError> {
    let started = std::time::Instant::now();
    let reported_success = body.success;
    let result = report_node_action_result_inner(body, valence).await;
    let success = match &result {
        Ok(()) if reported_success => "true",
        Ok(()) => "false",
        Err(_) => "error",
    };
    crate::prom_metrics::record_node_action_report(success, started.elapsed());
    result
}

#[tracing::instrument(
    skip(body, valence),
    fields(
        command_id = %body.command_id,
        node_id = %body.node_id,
        attempt = body.attempt,
        success = body.success,
    )
)]
async fn report_node_action_result_inner(
    body: ReportNodeActionResult,
    valence: &Valence,
) -> Result<(), NodeActionError> {
    let cmd = PionNodeActionCommand::get_used(&body.command_id, valence, valence::use_!("In **Pion control plane**, we **load Pion Node Action Command** so the application can decide what to do next in this workflow. The result is used by **Pion control plane** logic—not necessarily displayed on a page unless that feature’s UI shows it."))
        .await
        .with_context(|| format!("load command {} for report", body.command_id))?
        .ok_or_else(|| NodeActionError::CommandNotFound {
            command_id: body.command_id.clone(),
        })?;

    if cmd.node_id() != &body.node_id {
        return Err(NodeActionError::NodeMismatch {
            command_id: body.command_id.clone(),
        });
    }
    match cmd.status() {
        PionNodeActionCommandStatus::Running => {}
        PionNodeActionCommandStatus::Succeeded | PionNodeActionCommandStatus::Failed => {
            return Err(NodeActionError::AlreadyTerminal {
                command_id: body.command_id.clone(),
                status: format!("{:?}", cmd.status()),
            });
        }
        other => {
            return Err(NodeActionError::NotRunning {
                command_id: body.command_id.clone(),
                status: format!("{other:?}"),
            });
        }
    }
    if *cmd.attempt() != body.attempt {
        return Err(NodeActionError::AttemptMismatch {
            command_id: body.command_id.clone(),
            expected: *cmd.attempt(),
            got: body.attempt,
        });
    }

    let correlation_key = cmd.correlation_key().clone();

    let now = Utc::now();
    let result_id = uuid::Uuid::new_v4().to_string();
    let stdout = body.stdout.as_deref().unwrap_or("");
    let stderr = body.stderr.as_deref().unwrap_or("");
    let err_sum = body.error_summary.as_deref().unwrap_or("");
    let payload = body.payload_json.unwrap_or_else(|| serde_json::json!({}));

    let res = PionNodeActionResult::new(
        body.command_id.clone(),
        body.node_id.clone(),
        body.attempt,
        body.success,
        clip(stdout, CLIP),
        clip(stderr, CLIP),
        clip(err_sum, CLIP),
        payload,
        now,
    )
    .context("build node action result row")?;
    PionNodeActionResult::upsert_used(&result_id, res, valence, valence::use_!("When **Pion control plane** needs to persist work, we **save Pion Node Action Result** so the next step in that feature can continue with the latest values. People and services allowed for **Pion control plane** use this data for that workflow—not as a general export of unrelated personal fields."))
        .await
        .with_context(|| format!("upsert node action result {result_id}"))?;

    if body.success {
        PionNodeActionCommandMutable::get(&body.command_id, valence)
            .await
            .with_context(|| format!("load command {} to mark success", body.command_id))?
            .set_status(PionNodeActionCommandStatus::Succeeded)?
            .set_updated_at(now)?
            .commit()
            .await
            .with_context(|| {
                format!("commit success transition for command {}", body.command_id)
            })?;
        {
            let ctx = NodeActionContext::new(&body.command_id, &body.node_id, cmd.action_kind())
                .with_correlation(&correlation_key, *cmd.sequence())
                .with_attempt(body.attempt);
            ctx.log_result("success", "result success");
        }
        crate::bootstrap_notify::notify_if_wizard_run(valence, &correlation_key).await;
        crate::maybe_publish_setup_wizard_tracked_photon(&correlation_key, "succeeded").await;
        return Ok(());
    }

    let last = preferred_failure_message(stderr, err_sum);
    finish_failed_node_action_report(
        &body.command_id,
        &body.node_id,
        cmd.action_kind(),
        body.attempt,
        &cmd,
        &correlation_key,
        &last,
        stderr.len(),
        now,
        valence,
    )
    .await
}
