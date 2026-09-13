//! Heartbeat ingest: upsert inventory, observed status, enrollment, and handoff directives.
//!
//! # Entry points
//!
//! - [`ingest_agent_heartbeat`] — HTTP / router-facing alias used by
//!   [`crate::runtime::parton_router`] and the composite server.
//! - [`ingest_node_heartbeat`] — core implementation (enrollment, cell seed, projections,
//!   handoff directive collection).
//!
//! New-node enrollment behavior is controlled by [`enrollment_strict_new_nodes`] and related env
//! flags documented on the enrollment helpers.

use anyhow::{Context, Result};
use chrono::Utc;
use valence::{Model, Valence};

use crate::generated::{
    PionControlPlaneCell, PionControlPlaneCellMode, PionControlPlaneCellStatus,
    PionControlPlaneNode, PionControlPlaneNodeConnectionMode, PionControlPlaneNodeMutable,
    PionControlPlaneObservedStatus, PionControlPlaneObservedStatusSource, PionNodeReachability,
    PionNodeReachabilityCpConnectSource, PionNodeReachabilityMutable,
    PionNodeReachabilityRuntimeConnectSource,
};

use crate::logging::{HandoffDirectiveContext, HeartbeatContext};

use super::agent_enrollment::{
    enrollment_strict_new_nodes, mark_enrollment_claimed, validate_enrollment_for_new_node,
};
use super::capabilities::ensure_default_node_action_capabilities;
use super::contracts::map_health_to_observed_enum;
use super::{derive_observed_health, map_health_to_node_status, NodeHeartbeatReport};

async fn ensure_cell_exists(cell_id: &str, valence: &Valence) -> Result<()> {
    if PionControlPlaneCell::get_used(cell_id, valence, valence::use_!(r#"In **Pion control plane**, we **load Pion Control Plane Cell** so the application can decide what to do next in this workflow. The result is used by **Pion control plane** logic—not necessarily displayed on a page unless that feature’s UI shows it."#))
        .await
        .with_context(|| format!("load cell {cell_id} for heartbeat ingest"))?
        .is_some()
    {
        return Ok(());
    }

    let now = Utc::now();
    let record = PionControlPlaneCell::new(
        cell_id.to_string(),
        "local".to_string(),
        "local".to_string(),
        PionControlPlaneCellMode::Local,
        PionControlPlaneCellStatus::Active,
        serde_json::json!({
            "source": "heartbeat-ingest",
            "seeded_from_report": true
        }),
        serde_json::json!({}),
        now,
        now,
    )
    .context("build control plane cell row")?;
    PionControlPlaneCell::upsert_used(cell_id, record, valence, valence::use_!(r#"When **Pion control plane** needs to persist work, we **save Pion Control Plane Cell** so the next step in that feature can continue with the latest values. People and services allowed for **Pion control plane** use this data for that workflow—not as a general export of unrelated personal fields."#))
        .await
        .with_context(|| format!("upsert control plane cell {cell_id}"))?;
    Ok(())
}

fn report_capabilities_json(report: &NodeHeartbeatReport) -> serde_json::Value {
    serde_json::json!({
        "arch": report.capabilities.arch,
        "cpu_logical": report.capabilities.cpu_logical,
        "memory_bytes": report.capabilities.memory_bytes,
        "containers": report.containers.summary(),
        "container_observations": report.containers.containers,
    })
}

async fn upsert_node_record(
    report: &NodeHeartbeatReport,
    node_status: crate::generated::PionControlPlaneNodeStatus,
    now: chrono::DateTime<Utc>,
    valence: &Valence,
) -> Result<()> {
    if let Some(existing) = PionControlPlaneNode::get_used(&report.node_id, valence, valence::use_!(r#"In **Pion control plane**, we **load Pion Control Plane Node** so the application can decide what to do next in this workflow. The result is used by **Pion control plane** logic—not necessarily displayed on a page unless that feature’s UI shows it."#))
        .await
        .with_context(|| format!("load node {} for heartbeat upsert", report.node_id))?
    {
        let mutable = PionControlPlaneNodeMutable::get_used(&report.node_id, valence, valence::use_!(r#"In **Pion control plane**, we **update Pion Control Plane Node Mutable** in place so saved changes apply on the next read. The same actors who can run **Pion control plane** use the updated values; this step is not a silent copy to an external marketing system."#))
            .await
            .with_context(|| {
                format!("load mutable node {} for heartbeat upsert", report.node_id)
            })?;
        let failure_domain = existing.failure_domain().clone();
        mutable
            .set_cell_id(report.cell_id.clone())?
            .set_hostname(report.capabilities.hostname.clone())?
            .set_status(node_status)?
            .set_failure_domain(failure_domain)?
            .set_capabilities_json(report_capabilities_json(report))?
            .set_labels_json(report.capabilities.labels.clone())?
            .set_updated_at(now)?
            .commit()
            .await
            .with_context(|| format!("commit node {} heartbeat update", report.node_id))?;
        return Ok(());
    }

    let node = PionControlPlaneNode::new(
        report.cell_id.clone(),
        report.capabilities.hostname.clone(),
        PionControlPlaneNodeConnectionMode::Agent,
        node_status,
        String::new(),
        report_capabilities_json(report),
        report.capabilities.labels.clone(),
        now,
        now,
    )
    .context("build control plane node row")?;
    PionControlPlaneNode::upsert_used(&report.node_id, node, valence, valence::use_!(r#"When **Pion control plane** needs to persist work, we **save Pion Control Plane Node** so the next step in that feature can continue with the latest values. People and services allowed for **Pion control plane** use this data for that workflow—not as a general export of unrelated personal fields."#))
        .await
        .with_context(|| format!("upsert control plane node {}", report.node_id))?;
    Ok(())
}

/// Maintains [`PionNodeReachability`] from TCP `peer_ip` (CP-observed agent address).
async fn upsert_node_reachability(
    node_id: &str,
    peer_ip: Option<&str>,
    valence: &Valence,
) -> Result<()> {
    let now = Utc::now();
    let peer_trim = peer_ip.map(str::trim).filter(|s| !s.is_empty());
    let node_rid = valence::RecordId::new("pion_control_plane_node", node_id);

    match PionNodeReachability::get_used(node_id, valence, valence::use_!(r#"In **Pion control plane**, we **load Pion Node Reachability** so the application can decide what to do next in this workflow. The result is used by **Pion control plane** logic—not necessarily displayed on a page unless that feature’s UI shows it."#))
        .await
        .with_context(|| format!("load reachability row for node {node_id}"))?
    {
        Some(existing) => {
            let mut m = PionNodeReachabilityMutable::get_used(node_id, valence, valence::use_!(r#"In **Pion control plane**, we **update Pion Node Reachability Mutable** in place so saved changes apply on the next read. The same actors who can run **Pion control plane** use the updated values; this step is not a silent copy to an external marketing system."#))
                .await
                .with_context(|| format!("load mutable reachability row for node {node_id}"))?;
            if let Some(p) = peer_trim {
                m = m.set_peer_ip(p.to_string())?;
                if matches!(
                    existing.cp_connect_source(),
                    PionNodeReachabilityCpConnectSource::PeerIp
                ) {
                    m = m.set_cp_connect_host(p.to_string())?;
                }
            }
            m.set_updated_at(now)?
                .commit()
                .await
                .with_context(|| format!("commit reachability update for node {node_id}"))?;
        }
        None => {
            if let Some(p) = peer_trim {
                let row = PionNodeReachability::new(
                    node_rid,
                    Some(p.to_string()),
                    Some(p.to_string()),
                    PionNodeReachabilityCpConnectSource::PeerIp,
                    None,
                    PionNodeReachabilityRuntimeConnectSource::None,
                    now,
                    now,
                )
                .context("build node reachability row")?;
                PionNodeReachability::upsert_used(node_id, row, valence, valence::use_!(r#"When **Pion control plane** needs to persist work, we **save Pion Node Reachability** so the next step in that feature can continue with the latest values. People and services allowed for **Pion control plane** use this data for that workflow—not as a general export of unrelated personal fields."#))
                    .await
                    .with_context(|| format!("upsert reachability row for node {node_id}"))?;
            }
        }
    }
    Ok(())
}

async fn upsert_observed_status(report: &NodeHeartbeatReport, valence: &Valence) -> Result<()> {
    let observed_health = derive_observed_health(report.containers.summary());
    let observed_id = format!(
        "{}:{}:{}",
        report.cell_id,
        report.node_id,
        report.observed_at.timestamp_millis()
    );
    let observed = PionControlPlaneObservedStatus::new(
        report.cell_id.clone(),
        report.node_id.clone(),
        PionControlPlaneObservedStatusSource::Agent,
        map_health_to_observed_enum(observed_health),
        serde_json::to_value(report).context("serialize heartbeat report for observed status")?,
        report.observed_at,
    )
    .context("build observed status row")?;
    PionControlPlaneObservedStatus::upsert_used(&observed_id, observed, valence, valence::use_!(r#"When **Pion control plane** needs to persist work, we **save Pion Control Plane Observed Status** so the next step in that feature can continue with the latest values. People and services allowed for **Pion control plane** use this data for that workflow—not as a general export of unrelated personal fields."#))
        .await
        .with_context(|| format!("upsert observed status {observed_id}"))?;
    Ok(())
}

/// Ingests heartbeat telemetry and upserts node, reachability, and observed-status records.
///
/// # Enrollment (first-seen nodes)
///
/// When `report.node_id` is not yet in inventory:
/// - If `enrollment_strict_new_nodes()` is on, a valid enrollment token is **required**
///   (and optional source-IP checks apply when configured).
/// - Otherwise a present token is validated when possible; a missing/invalid token falls
///   back to trusting `report.cell_id` (open enrollment).
///
/// Existing nodes skip enrollment and keep updating inventory from the report.
///
/// # Parameters
///
/// - `peer_ip`: immediate TCP client address when the caller is an edge HTTP handler; used for
///   optional enrollment source checks (`enrollment_verify_source_ip` in `agent_enrollment`).
///
/// # Errors
///
/// Propagates Valence and serialization failures (for example invalid container payloads),
/// and enrollment validation failures when strict mode rejects a first-seen node.
pub async fn ingest_node_heartbeat(
    report: &NodeHeartbeatReport,
    peer_ip: Option<&str>,
    valence: &Valence,
) -> Result<parton::HeartbeatResponse> {
    let node_exists = PionControlPlaneNode::get_used(&report.node_id, valence, valence::use_!(r#"In **Pion control plane**, we **load Pion Control Plane Node** so the application can decide what to do next in this workflow. The result is used by **Pion control plane** logic—not necessarily displayed on a page unless that feature’s UI shows it."#))
        .await
        .with_context(|| {
            format!(
                "check node {} existence for heartbeat ingest",
                report.node_id
            )
        })?
        .is_some();

    let enrollment_token = report
        .enrollment_token
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty());

    let (effective_cell_id, claim_wire_token): (String, Option<String>) = if node_exists {
        (report.cell_id.clone(), None)
    } else if enrollment_strict_new_nodes() {
        let cell =
            validate_enrollment_for_new_node(valence, enrollment_token, &report.cell_id, peer_ip)
                .await?;
        let wire = enrollment_token
            .map(str::to_string)
            .ok_or_else(|| anyhow::anyhow!("enrollment token required"))?;
        (cell, Some(wire))
    } else if let Some(tok) = enrollment_token {
        match validate_enrollment_for_new_node(valence, Some(tok), &report.cell_id, peer_ip).await {
            Ok(cell) => (cell, Some(tok.to_string())),
            Err(_) => (report.cell_id.clone(), None),
        }
    } else {
        (report.cell_id.clone(), None)
    };

    ensure_cell_exists(&effective_cell_id, valence).await?;

    let mut report_for_ingest = report.clone();
    report_for_ingest.cell_id = effective_cell_id;

    let now = Utc::now();
    let observed_health = derive_observed_health(report_for_ingest.containers.summary());
    let node_status = map_health_to_node_status(observed_health);
    upsert_node_record(&report_for_ingest, node_status, now, valence).await?;
    upsert_node_reachability(&report_for_ingest.node_id, peer_ip, valence).await?;
    ensure_default_node_action_capabilities(&report_for_ingest.node_id, valence).await?;
    upsert_observed_status(&report_for_ingest, valence).await?;

    let transitions =
        super::container_observation::upsert_observations(&report_for_ingest, valence).await?;

    let telemetry = HeartbeatContext::new(
        report_for_ingest.node_id.clone(),
        report_for_ingest.cell_id.clone(),
    );
    apply_heartbeat_side_effects(
        &telemetry,
        &report_for_ingest,
        report,
        node_exists,
        enrollment_token,
        claim_wire_token.as_deref(),
        peer_ip,
        valence,
        &transitions,
    )
    .await?;

    telemetry.record_ingested_ok();

    Ok(parton::HeartbeatResponse {
        acknowledged_at: Utc::now(),
        directives: collect_handoff_directives_with_telemetry(report, valence).await?,
    })
}

#[allow(clippy::too_many_arguments)] // heartbeat side effects fan out to enrollment, photon, and observation
async fn apply_heartbeat_side_effects(
    telemetry: &HeartbeatContext,
    report_for_ingest: &NodeHeartbeatReport,
    _report: &NodeHeartbeatReport,
    node_exists: bool,
    enrollment_token: Option<&str>,
    claim_wire_token: Option<&str>,
    peer_ip: Option<&str>,
    valence: &Valence,
    transitions: &[super::container_observation::ContainerStateTransition],
) -> Result<()> {
    for transition in transitions {
        if let Err(e) =
            crate::photon_container_observation_event::publish_container_state_changed(transition)
                .await
        {
            telemetry.record_publish(
                "error",
                &format!("pion.container.state_changed publish skipped: {e}"),
            );
        } else {
            telemetry.record_publish("success", "pion.container.state_changed published");
        }
    }

    if let Some(wire) = claim_wire_token {
        let _ = mark_enrollment_claimed(
            valence,
            wire,
            &report_for_ingest.node_id,
            &report_for_ingest.cell_id,
            peer_ip,
        )
        .await;
    }

    if node_exists {
        if let Some(tok) = enrollment_token {
            let _ = mark_enrollment_claimed(
                valence,
                tok,
                &report_for_ingest.node_id,
                &report_for_ingest.cell_id,
                peer_ip,
            )
            .await;
        }
    }

    if let Err(e) = super::agent_handoff_directive::acknowledge_handoff_directives_for_heartbeat(
        report_for_ingest,
        valence,
    )
    .await
    {
        HandoffDirectiveContext::from_node(&report_for_ingest.node_id)
            .warn_ack_skipped(&e.to_string());
    }

    let mounts: Vec<(String, u64, u64)> = report_for_ingest
        .capabilities
        .mounts
        .iter()
        .map(|m| (m.mount_point.clone(), m.total_bytes, m.available_bytes))
        .collect();
    crate::registry_storage_notify::project_registry_storage_mounts(
        valence,
        &report_for_ingest.node_id,
        &mounts,
    )
    .await;

    Ok(())
}

async fn collect_handoff_directives_with_telemetry(
    report: &NodeHeartbeatReport,
    valence: &Valence,
) -> Result<Vec<parton::AgentDirective>> {
    match super::agent_handoff_directive::collect_handoff_directives_for_heartbeat(report, valence)
        .await
    {
        Ok(d) => Ok(d),
        Err(e) => {
            HandoffDirectiveContext::from_node(&report.node_id)
                .warn_collect_skipped(&e.to_string());
            Ok(Vec::new())
        }
    }
}

/// Ingests a [`parton::NodeHeartbeatReport`] (same shape as the Parton agent JSON payload).
///
/// This is the entrypoint the composite `server` and [`crate::runtime::parton_router`] use for
/// `/api/parton/heartbeat`. It delegates to [`ingest_node_heartbeat`].
///
/// # Errors
///
/// Propagates Valence and enrollment validation failures from [`ingest_node_heartbeat`].
///
/// # Examples
///
/// ```no_run
/// # #[cfg(feature = "runtime")]
/// # {
/// use pion::ingest_agent_heartbeat;
/// use parton::NodeHeartbeatReport;
/// # async fn demo(report: NodeHeartbeatReport, valence: &valence::Valence) -> anyhow::Result<()> {
/// let response = ingest_agent_heartbeat(&report, None, valence).await?;
/// assert!(response.acknowledged_at.timestamp() > 0);
/// # Ok(())
/// # }
/// # }
/// ```
#[tracing::instrument(
    skip(report, valence),
    fields(node_id = %report.node_id, cell_id = %report.cell_id, peer_ip = ?peer_ip)
)]
pub async fn ingest_agent_heartbeat(
    report: &parton::NodeHeartbeatReport,
    peer_ip: Option<&str>,
    valence: &Valence,
) -> Result<parton::HeartbeatResponse> {
    let started = std::time::Instant::now();
    let result = ingest_node_heartbeat(report, peer_ip, valence).await;
    crate::prom_metrics::record_heartbeat_ingest(result.is_ok(), started.elapsed());
    result
}
