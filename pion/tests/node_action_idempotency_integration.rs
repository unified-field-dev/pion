//! Idempotency for enqueue by `(correlation_key, sequence, node_id)`.
#![cfg(feature = "runtime")]
// Integration tests use `.expect()` / `.unwrap()` for brevity on fixtures.
#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;

use chrono::Utc;
use pion::generated::PionNodeActionCommand;
use pion::{
    create_host_enrollment, enqueue_node_action, ingest_agent_heartbeat,
    upsert_node_action_capability, ContainerActionKind, ContainerStatusSummary, HostCapabilities,
    NodeHeartbeatReport,
};

use common::containers_from_summary;

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
    ingest_agent_heartbeat(&report, Some("127.0.0.1"), v).await?;
    // Capabilities default to disabled (F7); opt this node into deploy for the idempotency test.
    upsert_node_action_capability(
        node_id,
        ContainerActionKind::Deploy,
        true,
        "test",
        None,
        None,
        v,
    )
    .await?;
    Ok(())
}

#[tokio::test]
async fn enqueue_same_key_is_noop() -> anyhow::Result<()> {
    let v = common::test_valence("pion_enqueue_idempotency").await;
    ready_node(&v, "node-idem", "local-default").await?;
    let payload = serde_json::json!({"action":"deploy","container_ref":"idem"});
    let id1 = enqueue_node_action(
        "node-idem",
        "local-default",
        "deploy",
        payload.clone(),
        3,
        Some("idem:corr"),
        Some(1),
        &v,
    )
    .await?;
    let id2 = enqueue_node_action(
        "node-idem",
        "local-default",
        "deploy",
        payload,
        3,
        Some("idem:corr"),
        Some(1),
        &v,
    )
    .await?;
    assert_eq!(id1, id2);
    let rows = PionNodeActionCommand::query_used(&v, valence::use_!(r#"**Test:** Fixture **Pion Node Action Command** list for `tests` so the suite can arrange and assert persistence behavior. CI and developers running the suite only."#)).await?;
    assert_eq!(rows.len(), 1);
    Ok(())
}
