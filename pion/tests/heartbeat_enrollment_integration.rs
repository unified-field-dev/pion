//! End-to-end heartbeat ingest on in-memory Valence (enrollment + observed status).
//!
//! Complements the `parton` crate HTTP integration tests by exercising
//! [`pion::ingest_agent_heartbeat`] directly against the same logical-router shape the composite
//! server uses.

#![cfg(feature = "runtime")]
// Integration tests use `.expect()` / `.unwrap()` for brevity on fixtures.
#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;

use chrono::Utc;
use pion::generated::PionControlPlaneObservedStatus;
use pion::{
    create_host_enrollment, ingest_agent_heartbeat, ContainerStatusSummary, HostCapabilities,
    NodeHeartbeatReport,
};

use common::containers_from_summary;
#[tokio::test]
async fn ingest_heartbeat_creates_observed_status_snapshot() -> anyhow::Result<()> {
    let v = common::test_valence("pion_heartbeat_enrollment").await;

    let (_, token) = create_host_enrollment(
        &v,
        "127.0.0.1".to_string(),
        "local-default".to_string(),
        "default".to_string(),
        86_400,
    )
    .await?;

    let report = NodeHeartbeatReport {
        node_id: "agent-node-pion-it".to_string(),
        cell_id: "local-default".to_string(),
        capabilities: HostCapabilities {
            hostname: "agent-node-pion-it.local".to_string(),
            arch: "x86_64".to_string(),
            cpu_logical: 4,
            memory_bytes: 8 * 1024 * 1024 * 1024,
            mounts: vec![],
            labels: serde_json::json!({}),
        },
        containers: containers_from_summary(ContainerStatusSummary {
            running: 1,
            exited: 0,
            unhealthy: 0,
        }),
        observed_at: Utc::now(),
        enrollment_token: Some(token),
        applied_directive_token: None,
        apply_failed: None,
    };

    ingest_agent_heartbeat(&report, Some("127.0.0.1"), &v).await?;

    let observed = PionControlPlaneObservedStatus::query_used(&v, valence::use_!("query PionControlPlaneObservedStatus in pion/tests/heartbeat_enrollment_integration.rs; Valence persistence for this feature path; typed store; visible to test harness.")).await?;
    assert_eq!(observed.len(), 1, "expected one observed status snapshot");
    assert_eq!(observed[0].source().as_str(), "agent");
    Ok(())
}
