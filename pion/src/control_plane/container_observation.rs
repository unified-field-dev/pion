//! Per-container observation rows upserted from agent heartbeats.

use anyhow::Result;
use chrono::{Duration, Utc};
use parton::{ContainerHealth, ContainerState, NodeHeartbeatReport};
use valence::{DateTimePredicate, Model, StringPredicate, Valence};

use crate::generated::{
    PionContainerObservation, PionContainerObservationHealth, PionContainerObservationState,
};

/// One observed container state transition (for Photon publish).
#[derive(Debug, Clone)]
pub struct ContainerStateTransition {
    /// Node that reported the container.
    pub node_id: String,
    /// Cell id on the heartbeat.
    pub cell_id: String,
    /// Docker container id.
    pub container_id: String,
    /// Docker container name.
    pub container_name: String,
    /// Optional Parton instance id (handoff cell).
    pub instance_id: Option<String>,
    /// Previous persisted state, if any (first observation is `None`).
    pub prev_state: Option<PionContainerObservationState>,
    /// New state after this heartbeat.
    pub next_state: PionContainerObservationState,
    /// Exit code when transitioning to exited/dead.
    pub exit_code: Option<i32>,
    /// When the container finished, if reported.
    pub finished_at: Option<chrono::DateTime<Utc>>,
    /// Docker restart count at observation time.
    pub restart_count: u64,
    /// Agent-reported observation timestamp.
    pub observed_at: chrono::DateTime<Utc>,
}

fn observation_row_id(node_id: &str, container_id: &str) -> String {
    format!("{node_id}::{container_id}")
}

fn map_state(state: ContainerState) -> PionContainerObservationState {
    match state {
        ContainerState::Created => PionContainerObservationState::Created,
        ContainerState::Running => PionContainerObservationState::Running,
        ContainerState::Restarting => PionContainerObservationState::Restarting,
        ContainerState::Paused => PionContainerObservationState::Paused,
        ContainerState::Exited => PionContainerObservationState::Exited,
        ContainerState::Dead => PionContainerObservationState::Dead,
        ContainerState::Removing => PionContainerObservationState::Removing,
    }
}

fn map_health(health: ContainerHealth) -> PionContainerObservationHealth {
    match health {
        ContainerHealth::Starting => PionContainerObservationHealth::Starting,
        ContainerHealth::Healthy => PionContainerObservationHealth::Healthy,
        ContainerHealth::Unhealthy => PionContainerObservationHealth::Unhealthy,
        ContainerHealth::NoCheck => PionContainerObservationHealth::NoCheck,
        ContainerHealth::Unknown => PionContainerObservationHealth::Unknown,
    }
}

fn heartbeat_stale_cutoff() -> chrono::DateTime<Utc> {
    let interval_secs = std::env::var("PARTON_HEARTBEAT_INTERVAL_SECS")
        .ok()
        .and_then(|s| s.parse::<i64>().ok())
        .filter(|&n| n > 0)
        .unwrap_or(10);
    Utc::now() - Duration::seconds(interval_secs * 6)
}

/// Upsert per-container observations for this heartbeat; returns transitions only when `state` changes.
///
/// # Errors
///
/// Propagates Valence query or upsert failures.
pub async fn upsert_observations(
    report: &NodeHeartbeatReport,
    valence: &Valence,
) -> Result<Vec<ContainerStateTransition>> {
    let ingested_at = Utc::now();
    let mut transitions = Vec::new();

    for c in &report.containers.containers {
        let row_id = observation_row_id(&report.node_id, &c.container_id);
        let next_state = map_state(c.state);
        let existing = PionContainerObservation::get_used(&row_id, valence, valence::use_!(r#"In **Pion control plane**, we **load Pion Container Observation** so the application can decide what to do next in this workflow. The result is used by **Pion control plane** logic—not necessarily displayed on a page unless that feature’s UI shows it."#)).await?;
        let prev_state = existing.as_ref().map(|row| row.state().clone());
        let first_observed_at = if prev_state.as_ref() == Some(&next_state) {
            existing
                .as_ref()
                .map_or(c.last_observed_at, |row| *row.first_observed_at())
        } else {
            c.last_observed_at
        };

        if prev_state.as_ref() != Some(&next_state) {
            transitions.push(ContainerStateTransition {
                node_id: report.node_id.clone(),
                cell_id: report.cell_id.clone(),
                container_id: c.container_id.clone(),
                container_name: c.container_name.clone(),
                instance_id: c.instance_id.clone(),
                prev_state: prev_state.clone(),
                next_state: next_state.clone(),
                exit_code: c.exit_code,
                finished_at: c.finished_at,
                restart_count: c.restart_count,
                observed_at: c.last_observed_at,
            });
        }

        let row = PionContainerObservation::new(
            report.node_id.clone(),
            report.cell_id.clone(),
            c.container_id.clone(),
            c.container_name.clone(),
            c.instance_id.clone(),
            next_state,
            c.exit_code.map(i64::from),
            c.started_at,
            c.finished_at,
            map_health(c.health),
            i64::try_from(c.restart_count).unwrap_or(i64::MAX),
            first_observed_at,
            c.last_observed_at,
            ingested_at,
            c.probe_error.clone(),
        )?;
        PionContainerObservation::upsert_used(&row_id, row, valence, valence::use_!(r#"When **Pion control plane** needs to persist work, we **save Pion Container Observation** so the next step in that feature can continue with the latest values. People and services allowed for **Pion control plane** use this data for that workflow—not as a general export of unrelated personal fields."#)).await?;
    }

    prune_stale(&report.node_id, heartbeat_stale_cutoff(), valence).await?;
    Ok(transitions)
}

/// Remove observation rows for this node that have not been seen recently (container removed).
pub async fn prune_stale(
    node_id: &str,
    stale_before: chrono::DateTime<Utc>,
    valence: &Valence,
) -> Result<()> {
    let stale = PionContainerObservation::query_used(valence, valence::use_!(r#"In **Pion control plane**, we **list Pion Container Observation** so the product can show or process the matching set for this workflow. Callers allowed for **Pion control plane** use the list; it is not a public dump of every field to anonymous visitors."#))
        .where_node_id(StringPredicate::Equals(node_id.to_string()))
        .where_last_observed_at(DateTimePredicate::Before(stale_before))
        .await?;
    for row in stale {
        if let Some(id) = row.id() {
            let key = id.id().to_string();
            let _ = PionContainerObservation::delete_used(&key, valence, valence::use_!(r#"When **Pion control plane** finishes cleanup, we **remove Pion Container Observation** so leftover rows do not remain after the operation. Only the cleanup path for **Pion control plane** uses this step; it is not shown as a standalone end-user page by itself."#)).await;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use parton::{
        ContainerHealth, ContainerStatus, ContainerStatusReport, ContainerStatusSummary,
        HostCapabilities,
    };

    pub(super) async fn test_valence() -> Valence {
        let boot = crate::valence_bootstrap::bootstrap_sqlite_memory()
            .await
            .expect("sqlite memory bootstrap");
        boot.valence("container_observation_test")
            .expect("valence build")
    }

    fn sample_container(state: ContainerState, id: &str, name: &str) -> ContainerStatus {
        ContainerStatus {
            container_id: id.to_string(),
            container_name: name.to_string(),
            instance_id: Some("inst-abc".to_string()),
            state,
            exit_code: None,
            started_at: None,
            finished_at: None,
            health: ContainerHealth::NoCheck,
            restart_count: 0,
            last_observed_at: Utc::now(),
            probe_error: None,
        }
    }

    #[tokio::test]
    async fn upsert_emits_transition_on_state_change() -> anyhow::Result<()> {
        let v = test_valence().await;
        let report = NodeHeartbeatReport {
            node_id: "node-1".into(),
            cell_id: "home".into(),
            capabilities: HostCapabilities {
                hostname: "h".into(),
                arch: "x86_64".into(),
                cpu_logical: 1,
                memory_bytes: 0,
                mounts: vec![],
                labels: serde_json::json!({}),
            },
            containers: ContainerStatusReport {
                containers: vec![sample_container(
                    ContainerState::Running,
                    "cid1",
                    "apps-home-0",
                )],
                summary: ContainerStatusSummary {
                    running: 1,
                    exited: 0,
                    unhealthy: 0,
                },
            },
            observed_at: Utc::now(),
            enrollment_token: None,
            applied_directive_token: None,
            apply_failed: None,
        };
        let t1 = upsert_observations(&report, &v).await?;
        assert_eq!(t1.len(), 1);
        assert!(t1[0].prev_state.is_none());
        assert_eq!(t1[0].next_state, PionContainerObservationState::Running);

        let mut report2 = report.clone();
        report2.containers.containers[0].state = ContainerState::Exited;
        report2.containers.containers[0].exit_code = Some(137);
        let t2 = upsert_observations(&report2, &v).await?;
        assert_eq!(t2.len(), 1);
        assert_eq!(
            t2[0].prev_state,
            Some(PionContainerObservationState::Running)
        );
        assert_eq!(t2[0].next_state, PionContainerObservationState::Exited);
        Ok(())
    }
}
