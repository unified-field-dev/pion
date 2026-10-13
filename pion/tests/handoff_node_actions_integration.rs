//! Handoff v2 node action kinds: enqueue, claim, and payload preservation.
#![cfg(feature = "runtime")]
// Integration tests use `.expect()` / `.unwrap()` for brevity on fixtures.
#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;

use chrono::Utc;
use pion::{
    claim_pending_node_action, create_host_enrollment, enqueue_node_action, ingest_agent_heartbeat,
    upsert_node_action_capability, ContainerActionKind, ContainerStatusSummary,
    DeployHandoffActionRequest, HostCapabilities, NodeHeartbeatReport,
    TeardownHandoffActionRequest, NODE_ACTION_KIND_DEPLOY_HANDOFF,
    NODE_ACTION_KIND_TEARDOWN_HANDOFF,
};

async fn ready_node(v: &valence::Valence, node_id: &str, cell_id: &str) -> anyhow::Result<()> {
    let (_, token) = create_host_enrollment(
        v,
        "127.0.0.1".to_string(),
        cell_id.to_string(),
        "default".to_string(),
        86_400,
    )
    .await?;
    let report = NodeHeartbeatReport {
        node_id: node_id.to_string(),
        cell_id: cell_id.to_string(),
        capabilities: HostCapabilities {
            hostname: format!("{node_id}.local"),
            arch: "x86_64".to_string(),
            cpu_logical: 4,
            memory_bytes: 8 * 1024 * 1024 * 1024,
            mounts: vec![],
            labels: serde_json::json!({}),
        },
        containers: common::containers_from_summary(ContainerStatusSummary {
            running: 1,
            exited: 0,
            unhealthy: 0,
        }),
        observed_at: Utc::now(),
        enrollment_token: Some(token),
        applied_directive_token: None,
        apply_failed: None,
    };
    ingest_agent_heartbeat(&report, Some("127.0.0.1"), v).await?;
    // Capabilities default to disabled; opt this node into deploy/stop for handoff tests.
    for action in [ContainerActionKind::Deploy, ContainerActionKind::Stop] {
        upsert_node_action_capability(node_id, action, true, "test", None, None, v).await?;
    }
    Ok(())
}

#[tokio::test]
async fn deploy_handoff_enqueue_claim_preserves_payload() -> anyhow::Result<()> {
    let v = common::test_valence("pion_handoff_actions").await;
    ready_node(&v, "node-handoff", "local-default").await?;

    let req = DeployHandoffActionRequest {
        node_id: "node-handoff".into(),
        container_ref: "handoff-svc".into(),
        image_ref: "ghcr.io/example/handoff:1".into(),
        env_vars: vec!["FOO=bar".into()],
        secret_env_vars: vec![],
        port_mappings: vec!["8080:8080".into()],
        extra_hosts: vec![],
        volume_mounts: vec![],
        network: Some("bridge".into()),
        health_url: Some("http://127.0.0.1:8080/health".into()),
        import_base_url: None,
        transfer_id: Some("xfer-9".into()),
    };
    let payload = serde_json::to_value(&req)?;

    let _cmd = enqueue_node_action(
        "node-handoff",
        "local-default",
        NODE_ACTION_KIND_DEPLOY_HANDOFF,
        payload.clone(),
        3,
        Some("handoff:corr"),
        Some(1),
        &v,
    )
    .await?;

    let claimed = claim_pending_node_action("node-handoff", 120, &v)
        .await?
        .expect("claimed command");
    assert_eq!(claimed.action_kind, NODE_ACTION_KIND_DEPLOY_HANDOFF);
    // Assert semantic fields rather than raw stored-string formatting.
    assert_eq!(
        claimed
            .payload_json
            .get("transfer_id")
            .and_then(|v| v.as_str()),
        Some("xfer-9")
    );
    assert_eq!(
        claimed
            .payload_json
            .get("container_ref")
            .and_then(|v| v.as_str()),
        Some("handoff-svc")
    );
    assert_eq!(
        claimed
            .payload_json
            .get("image_ref")
            .and_then(|v| v.as_str()),
        Some("ghcr.io/example/handoff:1")
    );
    Ok(())
}

#[tokio::test]
async fn teardown_handoff_idempotent_enqueue() -> anyhow::Result<()> {
    let v = common::test_valence("pion_handoff_teardown").await;
    ready_node(&v, "node-teardown", "local-default").await?;

    let req = TeardownHandoffActionRequest {
        node_id: "node-teardown".into(),
        container_ref: "handoff-svc".into(),
        remove_volumes: false,
        expected_container_id: None,
    };
    let payload = serde_json::to_value(&req)?;
    let id1 = enqueue_node_action(
        "node-teardown",
        "local-default",
        NODE_ACTION_KIND_TEARDOWN_HANDOFF,
        payload.clone(),
        3,
        Some("teardown:corr"),
        Some(1),
        &v,
    )
    .await?;
    let id2 = enqueue_node_action(
        "node-teardown",
        "local-default",
        NODE_ACTION_KIND_TEARDOWN_HANDOFF,
        payload,
        3,
        Some("teardown:corr"),
        Some(1),
        &v,
    )
    .await?;
    assert_eq!(id1, id2);
    Ok(())
}
