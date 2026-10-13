//! Claim-time resolution of `{"$secret_ref":...}` in Parton `command` JSON.
#![cfg(feature = "runtime")]
// Integration tests use `.expect()` / `.unwrap()` for brevity on fixtures.
#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;

use std::sync::Arc;

use async_trait::async_trait;
use pion::{
    claim_pending_node_action, enqueue_node_action, SecretResolver, NODE_ACTION_KIND_DEPLOY_HANDOFF,
};
use valence::Valence;

struct MockOk;
#[async_trait]
impl SecretResolver for MockOk {
    async fn resolve_secret_root_field(
        &self,
        _v: &Valence,
        _id: &str,
        _version: i64,
        field: &str,
    ) -> Result<String, anyhow::Error> {
        Ok(format!("mock-{field}"))
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
    // Capabilities default to disabled; opt this node into deploy for the claim tests.
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
async fn claim_replaces_secret_ref_in_command() -> anyhow::Result<()> {
    let _ = pion::set_default_secret_resolver(Arc::new(MockOk));
    let v = common::test_valence("pion_secret_ref_ok").await;
    ready_node(&v, "node-sr-ok", "local-default").await?;
    let payload = serde_json::json!({
        "node_id": "node-sr-ok",
        "container_ref": "c1",
        "action": "deploy",
        "image_ref": "img:1",
        "env_vars": [],
        "port_mappings": ["3000:3000"],
        "command": [
            "--pass",
            {"$secret_ref": {"id": "neutrino_secret:abc", "version": 1}, "field": "password"}
        ],
        "restart_policy": "unless-stopped"
    });
    let id = enqueue_node_action(
        "node-sr-ok",
        "local-default",
        "deploy",
        payload,
        3,
        Some("sec-ref-ok"),
        Some(1),
        &v,
    )
    .await?;
    let claimed = claim_pending_node_action("node-sr-ok", 120, &v)
        .await?
        .expect("claimed");
    assert_eq!(claimed.command_id, id);
    let cmd = &claimed.payload_json;
    let pass = cmd
        .get("command")
        .and_then(|c| c.as_array())
        .and_then(|a| a.get(1))
        .and_then(|v| v.as_str());
    assert_eq!(
        pass,
        Some("mock-password"),
        "expected resolved password string, got: {cmd:#}"
    );
    Ok(())
}

#[tokio::test]
async fn claim_replaces_secret_ref_in_deploy_handoff_secret_env_vars() -> anyhow::Result<()> {
    let _ = pion::set_default_secret_resolver(Arc::new(MockOk));
    let v = common::test_valence("pion_secret_ref_deploy_handoff").await;
    ready_node(&v, "node-sr-dh", "local-default").await?;
    let payload = serde_json::json!({
        "node_id": "node-sr-dh",
        "container_ref": "handoff-c",
        "image_ref": "img:handoff",
        "env_vars": ["PUBLIC=1"],
        "secret_env_vars": [
            {
                "name": "HANDOFF_BUNDLE_ROOT_SECRET",
                "value": {"$secret_ref": {"id": "neutrino_secret:handoff", "version": 1}, "field": "root_secret"}
            }
        ],
        "port_mappings": ["8080:8080"],
        "extra_hosts": [],
        "volume_mounts": [],
        "network": null,
        "health_url": null,
        "import_base_url": null,
        "transfer_id": "tid-1"
    });
    let id = enqueue_node_action(
        "node-sr-dh",
        "local-default",
        NODE_ACTION_KIND_DEPLOY_HANDOFF,
        payload,
        3,
        Some("sec-ref-dh"),
        Some(1),
        &v,
    )
    .await?;
    let claimed = claim_pending_node_action("node-sr-dh", 120, &v)
        .await?
        .expect("claimed");
    assert_eq!(claimed.command_id, id);
    let cmd = &claimed.payload_json;
    let val = cmd
        .get("secret_env_vars")
        .and_then(|a| a.as_array())
        .and_then(|a| a.first())
        .and_then(|o| o.get("value"))
        .and_then(|v| v.as_str());
    assert_eq!(
        val,
        Some("mock-root_secret"),
        "expected resolved secret env value, got: {cmd:#}"
    );
    Ok(())
}
