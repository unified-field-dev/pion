//! `claim_pending_node_action` must never hand the same command to two concurrent
//! claimers. Valence has no compare-and-swap primitive, so this documents the mitigation
//! (post-commit re-read; abandon on lost race) rather than a true DB-level CAS guarantee.
#![cfg(feature = "runtime")]
// Integration tests use `.expect()` / `.unwrap()` for brevity on fixtures.
#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;

use chrono::Utc;
use pion::{
    claim_pending_node_action, create_host_enrollment, enqueue_node_action, ingest_agent_heartbeat,
    upsert_node_action_capability, ContainerActionKind, ContainerStatusSummary, HostCapabilities,
    NodeHeartbeatReport,
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

/// Two concurrent claims against the same single pending command must not both succeed: at most
/// one caller may receive `Some(..)`. This exercises the post-commit verification that guards against
/// double claims (Valence has no compare-and-swap, so this is a best-effort mitigation, not a hard guarantee
/// under every possible interleaving/backend).
#[tokio::test]
async fn concurrent_claims_never_both_succeed() -> anyhow::Result<()> {
    let v = common::test_valence("pion_atomic_claim").await;
    ready_node(&v, "node-race", "local-default").await?;

    let payload = serde_json::json!({
        "node_id": "node-race",
        "container_ref": "svc-race",
        "action": "deploy",
    });
    enqueue_node_action(
        "node-race",
        "local-default",
        "deploy",
        payload,
        3,
        Some("race:corr"),
        Some(1),
        &v,
    )
    .await?;

    let (a, b) = tokio::join!(
        claim_pending_node_action("node-race", 120, &v),
        claim_pending_node_action("node-race", 120, &v),
    );
    let a = a?;
    let b = b?;

    let successes = usize::from(a.is_some()) + usize::from(b.is_some());
    assert!(
        successes <= 1,
        "expected at most one concurrent claim to succeed, got a={a:?} b={b:?}"
    );
    Ok(())
}

/// Sequential claims against the same command (the common, non-racy path) must still work: the
/// first claim succeeds and a second claim against the now-`running` command finds nothing else
/// pending.
#[tokio::test]
async fn sequential_claim_then_reclaim_finds_nothing_pending() -> anyhow::Result<()> {
    let v = common::test_valence("pion_atomic_claim_sequential").await;
    ready_node(&v, "node-seq", "local-default").await?;

    let payload = serde_json::json!({
        "node_id": "node-seq",
        "container_ref": "svc-seq",
        "action": "deploy",
    });
    enqueue_node_action(
        "node-seq",
        "local-default",
        "deploy",
        payload,
        3,
        Some("seq:corr"),
        Some(1),
        &v,
    )
    .await?;

    let first = claim_pending_node_action("node-seq", 120, &v).await?;
    assert!(first.is_some(), "expected first claim to succeed");

    let second = claim_pending_node_action("node-seq", 120, &v).await?;
    assert!(
        second.is_none(),
        "expected no additional pending command after the first claim"
    );
    Ok(())
}
