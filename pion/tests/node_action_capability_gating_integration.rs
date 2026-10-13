//! `health_check` / `handoff_bundle_import` node actions must be capability-gated like other
//! infra-provisioning actions: a caller must not be able to enqueue an agent-initiated
//! outbound-HTTP action for a node whose deploy capability has been disabled.
#![cfg(feature = "runtime")]
// Integration tests use `.expect()` / `.unwrap()` for brevity on fixtures.
#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;

use chrono::Utc;
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
    Ok(())
}

#[tokio::test]
async fn health_check_enqueue_succeeds_when_deploy_capability_enabled() -> anyhow::Result<()> {
    let v = common::test_valence("pion_health_check_cap_enabled").await;
    ready_node(&v, "node-hc-ok", "local-default").await?;
    // Capabilities default to disabled; the operator must opt this node into deploy.
    upsert_node_action_capability(
        "node-hc-ok",
        ContainerActionKind::Deploy,
        true,
        "test",
        Some("enable for capability-gating test"),
        None,
        &v,
    )
    .await?;
    let payload = serde_json::json!({"url": "https://example.com/health"});
    enqueue_node_action(
        "node-hc-ok",
        "local-default",
        "health_check",
        payload,
        3,
        Some("hc-ok:corr"),
        Some(1),
        &v,
    )
    .await?;
    Ok(())
}

#[tokio::test]
async fn health_check_enqueue_fails_when_deploy_capability_disabled() -> anyhow::Result<()> {
    let v = common::test_valence("pion_health_check_cap_disabled").await;
    ready_node(&v, "node-hc-denied", "local-default").await?;
    upsert_node_action_capability(
        "node-hc-denied",
        ContainerActionKind::Deploy,
        false,
        "test",
        Some("disable for SSRF capability-gating test"),
        None,
        &v,
    )
    .await?;

    let payload = serde_json::json!({"url": "https://example.com/health"});
    let err = enqueue_node_action(
        "node-hc-denied",
        "local-default",
        "health_check",
        payload,
        3,
        Some("hc-denied:corr"),
        Some(1),
        &v,
    )
    .await
    .expect_err("health_check must be denied when deploy capability is disabled");
    assert!(err.to_string().contains("disabled for node"));
    Ok(())
}

#[tokio::test]
async fn handoff_bundle_import_enqueue_fails_when_deploy_capability_disabled() -> anyhow::Result<()>
{
    let v = common::test_valence("pion_handoff_bundle_import_cap_disabled").await;
    ready_node(&v, "node-si-denied", "local-default").await?;
    upsert_node_action_capability(
        "node-si-denied",
        ContainerActionKind::Deploy,
        false,
        "test",
        Some("disable for SSRF capability-gating test"),
        None,
        &v,
    )
    .await?;

    let payload = serde_json::json!({
        "url": "https://example.com/import",
        "authorization_bearer": "Bearer abc",
        "bundle_base64": "",
    });
    let err = enqueue_node_action(
        "node-si-denied",
        "local-default",
        "handoff_bundle_import",
        payload,
        3,
        Some("si-denied:corr"),
        Some(1),
        &v,
    )
    .await
    .expect_err("handoff_bundle_import must be denied when deploy capability is disabled");
    assert!(err.to_string().contains("disabled for node"));
    Ok(())
}

#[tokio::test]
async fn grow_fs_enqueue_succeeds_when_deploy_capability_enabled() -> anyhow::Result<()> {
    let v = common::test_valence("pion_grow_fs_cap_enabled").await;
    ready_node(&v, "node-growfs-ok", "local-default").await?;
    upsert_node_action_capability(
        "node-growfs-ok",
        ContainerActionKind::Deploy,
        true,
        "test",
        Some("enable for grow_fs capability-gating test"),
        None,
        &v,
    )
    .await?;
    let payload = serde_json::json!({
        "node_id": "node-growfs-ok",
        "container_ref": "growfs-test",
        "action": "grow_fs",
        "grow_fs": { "mount_path": "/var/lib/registry" }
    });
    enqueue_node_action(
        "node-growfs-ok",
        "local-default",
        "grow_fs",
        payload,
        3,
        Some("growfs-ok:corr"),
        Some(1),
        &v,
    )
    .await?;
    Ok(())
}

#[tokio::test]
async fn grow_fs_enqueue_fails_when_deploy_capability_disabled() -> anyhow::Result<()> {
    let v = common::test_valence("pion_grow_fs_cap_disabled").await;
    ready_node(&v, "node-growfs-denied", "local-default").await?;
    upsert_node_action_capability(
        "node-growfs-denied",
        ContainerActionKind::Deploy,
        false,
        "test",
        Some("disable for grow_fs capability-gating test"),
        None,
        &v,
    )
    .await?;

    let payload = serde_json::json!({
        "node_id": "node-growfs-denied",
        "container_ref": "growfs-test",
        "action": "grow_fs",
        "grow_fs": { "mount_path": "/var/lib/registry" }
    });
    let err = enqueue_node_action(
        "node-growfs-denied",
        "local-default",
        "grow_fs",
        payload,
        3,
        Some("growfs-denied:corr"),
        Some(1),
        &v,
    )
    .await
    .expect_err("grow_fs must be denied when deploy capability is disabled");
    assert!(err.to_string().contains("disabled for node"));
    Ok(())
}
