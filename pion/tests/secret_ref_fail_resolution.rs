//! Fail path: claim-time resolution error marks the command `failed` (separate test binary so
//! `set_default_secret_resolver` is not contended with other tests in the same process).
#![cfg(feature = "runtime")]
// Integration tests use `.expect()` / `.unwrap()` for brevity on fixtures.
#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;

use std::sync::Arc;

use async_trait::async_trait;
use pion::generated::PionNodeActionCommandStatus;
use pion::{
    claim_pending_node_action, enqueue_node_action, get_node_action_command, SecretResolver,
};
use valence::Valence;

struct MockFail;
#[async_trait]
impl SecretResolver for MockFail {
    async fn resolve_secret_root_field(
        &self,
        _v: &Valence,
        _id: &str,
        _version: i64,
        _field: &str,
    ) -> Result<String, anyhow::Error> {
        anyhow::bail!("intentional resolve failure for test")
    }
}

async fn ready_node(v: &valence::Valence, node_id: &str, cell_id: &str) -> anyhow::Result<()> {
    use chrono::Utc;
    use pion::{
        create_host_enrollment, ingest_agent_heartbeat, upsert_node_action_capability,
        ContainerActionKind, ContainerStatusSummary, HostCapabilities, NodeHeartbeatReport,
    };
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
    // Capabilities default to disabled; opt this node into deploy for the claim test.
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
async fn claim_marks_failed_when_secret_resolve_errors() -> anyhow::Result<()> {
    let _ = pion::set_default_secret_resolver(Arc::new(MockFail));
    let v = common::test_valence("pion_secret_ref_fail").await;
    ready_node(&v, "node-sr-fail", "local-default").await?;
    let payload = serde_json::json!({
        "node_id": "node-sr-fail",
        "container_ref": "c1",
        "action": "deploy",
        "image_ref": "img:1",
        "env_vars": [],
        "port_mappings": ["3000:3000"],
        "command": [
            "--pass",
            {"$secret_ref": {"id": "x", "version": 1}, "field": "password"}
        ],
        "restart_policy": "unless-stopped"
    });
    let id = enqueue_node_action(
        "node-sr-fail",
        "local-default",
        "deploy",
        payload,
        3,
        Some("sec-ref-fail"),
        Some(1),
        &v,
    )
    .await?;
    let last = claim_pending_node_action("node-sr-fail", 120, &v).await?;
    assert!(
        last.is_none(),
        "expected no claimed payload to agent when resolution fails"
    );
    let row = get_node_action_command(&id, &v).await?.expect("row");
    assert_eq!(*row.status(), PionNodeActionCommandStatus::Failed);
    let err = row.last_error();
    assert!(
        err.contains("claim-time secret resolution"),
        "unexpected: {err}"
    );
    Ok(())
}
