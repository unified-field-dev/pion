//! Valence-backed tests for [`super::resolve_eligible_pool_nodes`].
//!
//! Node rows are upserted directly rather than driven through heartbeat ingest so each case can
//! state the exact hardware it wants to select over.

use chrono::Utc;
use valence::{Model, Valence};

use crate::generated::{
    PionControlPlaneNode, PionControlPlaneNodeConnectionMode, PionControlPlaneNodeStatus,
    PionControlPlanePoolCellMap, PionControlPlaneVirtualPool,
};

use crate::control_plane::deploy::{
    resolve_node_for_deploy_target, resolve_node_for_deploy_target_with_requirements,
};
use crate::control_plane::DeployTarget;

use super::{
    resolve_eligible_pool_nodes, CpuArchitecture, NodeHardwareRequirements, PoolResolutionError,
};

const POOL: &str = "edge-west";
const CELL: &str = "local-default";

async fn test_valence(operation: &str) -> Valence {
    crate::valence_bootstrap::bootstrap_sqlite_memory()
        .await
        .expect("sqlite memory bootstrap")
        .valence(operation)
        .expect("valence build")
}

/// Seeds a pool whose hardware rule lives in the `hardware_selector_json` column, which is where
/// operators and Gluon write it.
async fn seed_pool(valence: &Valence, hardware_selector: serde_json::Value) -> anyhow::Result<()> {
    seed_pool_columns(
        valence,
        hardware_selector,
        serde_json::json!({}),
        serde_json::json!({}),
    )
    .await
}

async fn seed_pool_with_policy(
    valence: &Valence,
    hardware_selector: serde_json::Value,
    placement_policy: serde_json::Value,
) -> anyhow::Result<()> {
    seed_pool_columns(
        valence,
        hardware_selector,
        serde_json::json!({}),
        placement_policy,
    )
    .await
}

/// Seeds a pool that carries its hardware rule the older way, nested in free-form `selector_json`
/// with `hardware_selector_json` left empty.
async fn seed_pool_legacy_selector(
    valence: &Valence,
    selector: serde_json::Value,
) -> anyhow::Result<()> {
    seed_pool_columns(
        valence,
        serde_json::json!({}),
        selector,
        serde_json::json!({}),
    )
    .await
}

async fn seed_pool_columns(
    valence: &Valence,
    hardware_selector: serde_json::Value,
    selector: serde_json::Value,
    placement_policy: serde_json::Value,
) -> anyhow::Result<()> {
    let now = Utc::now();
    let pool = PionControlPlaneVirtualPool::new(
        POOL.to_string(),
        "general".to_string(),
        "Edge west".to_string(),
        selector,
        hardware_selector,
        placement_policy,
        now,
        now,
    )?;
    PionControlPlaneVirtualPool::upsert_used(POOL, pool, valence, valence::use_!("When **Pion control plane** needs to persist work, we **save Pion Control Plane Virtual Pool** so the next step in that feature can continue with the latest values. People and services allowed for **Pion control plane** use this data for that workflow—not as a general export of unrelated personal fields.")).await?;
    let map =
        PionControlPlanePoolCellMap::new(POOL.to_string(), CELL.to_string(), 1, true, now, now)?;
    PionControlPlanePoolCellMap::upsert_used(&format!("{POOL}:{CELL}"), map, valence, valence::use_!("When **Pion control plane** needs to persist work, we **save Pion Control Plane Pool Cell Map** so the next step in that feature can continue with the latest values. People and services allowed for **Pion control plane** use this data for that workflow—not as a general export of unrelated personal fields.")).await?;
    Ok(())
}

struct NodeSpec {
    node_id: &'static str,
    arch: &'static str,
    cpu_logical: u32,
    memory_bytes: u64,
    running: u64,
}

async fn seed_node(valence: &Valence, spec: &NodeSpec) -> anyhow::Result<()> {
    seed_node_with_status(valence, spec, PionControlPlaneNodeStatus::Online).await
}

async fn seed_node_with_status(
    valence: &Valence,
    spec: &NodeSpec,
    status: PionControlPlaneNodeStatus,
) -> anyhow::Result<()> {
    let now = Utc::now();
    let node = PionControlPlaneNode::new(
        CELL.to_string(),
        spec.node_id.to_string(),
        PionControlPlaneNodeConnectionMode::Agent,
        status,
        String::new(),
        serde_json::json!({
            "arch": spec.arch,
            "cpu_logical": spec.cpu_logical,
            "memory_bytes": spec.memory_bytes,
            "containers": { "running": spec.running, "exited": 0, "unhealthy": 0 },
        }),
        serde_json::json!({}),
        now,
        now,
    )?;
    PionControlPlaneNode::upsert_used(spec.node_id, node, valence, valence::use_!("When **Pion control plane** needs to persist work, we **save Pion Control Plane Node** so the next step in that feature can continue with the latest values. People and services allowed for **Pion control plane** use this data for that workflow—not as a general export of unrelated personal fields.")).await?;
    Ok(())
}

const SMALL_X86: NodeSpec = NodeSpec {
    node_id: "node-small-x86",
    arch: "x86_64",
    cpu_logical: 2,
    memory_bytes: 4 * 1024 * 1024 * 1024,
    running: 1,
};

const BIG_ARM: NodeSpec = NodeSpec {
    node_id: "node-big-arm",
    arch: "aarch64",
    cpu_logical: 16,
    memory_bytes: 64 * 1024 * 1024 * 1024,
    running: 7,
};

/// Pools written before the selector column existed carry `{"strategy":"manual-ui"}` in
/// `selector_json`, no hardware block, and a null `hardware_selector_json`. They must keep yielding
/// every reachable node, least-loaded first.
#[tokio::test]
async fn empty_selector_keeps_least_loaded_ordering() -> anyhow::Result<()> {
    let v = test_valence("pool_placement_empty_selector").await;
    seed_pool_legacy_selector(&v, serde_json::json!({ "strategy": "manual-ui" })).await?;
    seed_node(&v, &SMALL_X86).await?;
    seed_node(&v, &BIG_ARM).await?;

    let ranked =
        resolve_eligible_pool_nodes(POOL, &NodeHardwareRequirements::default(), &v).await?;
    let ids = ranked
        .iter()
        .map(|node| node.node_id.as_str())
        .collect::<Vec<_>>();
    assert_eq!(ids, vec![SMALL_X86.node_id, BIG_ARM.node_id]);
    assert_eq!(ranked[0].cell_id, CELL);
    assert_eq!(ranked[0].hardware.cpu_logical, Some(2));
    assert_eq!(ranked[1].running_containers, 7);
    Ok(())
}

#[tokio::test]
async fn pool_selector_filters_by_architecture() -> anyhow::Result<()> {
    let v = test_valence("pool_placement_selector_arch").await;
    seed_pool(&v, serde_json::json!({ "hardware": { "arch": "arm64" } })).await?;
    seed_node(&v, &SMALL_X86).await?;
    seed_node(&v, &BIG_ARM).await?;

    let ranked =
        resolve_eligible_pool_nodes(POOL, &NodeHardwareRequirements::default(), &v).await?;
    assert_eq!(ranked.len(), 1);
    assert_eq!(ranked[0].node_id, BIG_ARM.node_id);
    assert_eq!(ranked[0].hardware.arch, Some(CpuArchitecture::Aarch64));
    Ok(())
}

#[tokio::test]
async fn caller_requirements_narrow_an_unconstrained_pool() -> anyhow::Result<()> {
    let v = test_valence("pool_placement_caller_floor").await;
    seed_pool(&v, serde_json::json!({})).await?;
    seed_node(&v, &SMALL_X86).await?;
    seed_node(&v, &BIG_ARM).await?;

    let floor = NodeHardwareRequirements {
        min_cpu_logical: Some(8),
        min_memory_bytes: Some(32 * 1024 * 1024 * 1024),
        ..NodeHardwareRequirements::default()
    };
    let ranked = resolve_eligible_pool_nodes(POOL, &floor, &v).await?;
    assert_eq!(ranked.len(), 1);
    assert_eq!(ranked[0].node_id, BIG_ARM.node_id);
    Ok(())
}

#[tokio::test]
async fn no_matching_hardware_reports_the_unmet_requirement() -> anyhow::Result<()> {
    let v = test_valence("pool_placement_no_match").await;
    seed_pool(&v, serde_json::json!({})).await?;
    seed_node(&v, &SMALL_X86).await?;

    let floor = NodeHardwareRequirements {
        min_cpu_logical: Some(64),
        ..NodeHardwareRequirements::default()
    };
    let err = resolve_eligible_pool_nodes(POOL, &floor, &v)
        .await
        .expect_err("a 2-CPU pool cannot satisfy a 64-CPU floor");
    match &err {
        PoolResolutionError::NoMatchingHardware {
            pool_id,
            considered,
            unmet,
        } => {
            assert_eq!(pool_id, POOL);
            assert_eq!(*considered, 1);
            assert_eq!(unmet.len(), 1);
        }
        other => panic!("expected NoMatchingHardware, got {other:?}"),
    }
    assert!(err.to_string().contains("cpu_logical 2 < 64"));
    Ok(())
}

/// A constrained selector must not place onto a node that never reported its hardware.
#[tokio::test]
async fn nodes_without_reported_hardware_fail_a_constrained_selector() -> anyhow::Result<()> {
    let v = test_valence("pool_placement_unreported").await;
    seed_pool(
        &v,
        serde_json::json!({ "hardware": { "min_memory_bytes": 1024 } }),
    )
    .await?;
    let silent = NodeSpec {
        node_id: "node-silent",
        arch: "",
        cpu_logical: 0,
        memory_bytes: 0,
        running: 0,
    };
    seed_node(&v, &silent).await?;

    let err = resolve_eligible_pool_nodes(POOL, &NodeHardwareRequirements::default(), &v)
        .await
        .expect_err("unreported hardware must not satisfy a memory floor");
    assert!(matches!(
        err,
        PoolResolutionError::NoMatchingHardware { .. }
    ));

    // The same node is eligible once the pool stops constraining hardware.
    seed_pool(&v, serde_json::json!({})).await?;
    let ranked =
        resolve_eligible_pool_nodes(POOL, &NodeHardwareRequirements::default(), &v).await?;
    assert_eq!(ranked.len(), 1);
    assert!(ranked[0].hardware.is_unreported());
    Ok(())
}

#[tokio::test]
async fn placement_policy_can_prefer_memory_over_load() -> anyhow::Result<()> {
    let v = test_valence("pool_placement_policy_memory").await;
    seed_pool_with_policy(
        &v,
        serde_json::json!({}),
        serde_json::json!({ "strategy": "most-memory" }),
    )
    .await?;
    seed_node(&v, &SMALL_X86).await?;
    seed_node(&v, &BIG_ARM).await?;

    let ranked =
        resolve_eligible_pool_nodes(POOL, &NodeHardwareRequirements::default(), &v).await?;
    // BIG_ARM carries more running containers, so least-loaded would have put it second.
    assert_eq!(ranked[0].node_id, BIG_ARM.node_id);
    Ok(())
}

#[tokio::test]
async fn offline_nodes_are_never_eligible() -> anyhow::Result<()> {
    let v = test_valence("pool_placement_offline").await;
    seed_pool(&v, serde_json::json!({})).await?;
    seed_node_with_status(&v, &SMALL_X86, PionControlPlaneNodeStatus::Offline).await?;

    let err = resolve_eligible_pool_nodes(POOL, &NodeHardwareRequirements::default(), &v)
        .await
        .expect_err("offline nodes cannot accept deploys");
    assert!(matches!(err, PoolResolutionError::NoEligibleNodes { .. }));
    Ok(())
}

#[tokio::test]
async fn missing_pool_and_unmapped_cells_are_distinguished() -> anyhow::Result<()> {
    let v = test_valence("pool_placement_missing").await;
    let err = resolve_eligible_pool_nodes("nope", &NodeHardwareRequirements::default(), &v)
        .await
        .expect_err("unknown pool");
    assert!(matches!(err, PoolResolutionError::PoolNotFound { .. }));

    let now = Utc::now();
    let pool = PionControlPlaneVirtualPool::new(
        POOL.to_string(),
        "general".to_string(),
        String::new(),
        serde_json::json!({}),
        serde_json::json!({}),
        serde_json::json!({}),
        now,
        now,
    )?;
    PionControlPlaneVirtualPool::upsert_used(POOL, pool, &v, valence::use_!("When **Pion control plane** needs to persist work, we **save Pion Control Plane Virtual Pool** so the next step in that feature can continue with the latest values. People and services allowed for **Pion control plane** use this data for that workflow—not as a general export of unrelated personal fields.")).await?;
    let err = resolve_eligible_pool_nodes(POOL, &NodeHardwareRequirements::default(), &v)
        .await
        .expect_err("pool with no cell mappings");
    assert!(matches!(err, PoolResolutionError::NoEnabledCells { .. }));
    Ok(())
}

#[tokio::test]
async fn malformed_selector_is_refused_rather_than_ignored() -> anyhow::Result<()> {
    let v = test_valence("pool_placement_bad_selector").await;
    seed_pool(
        &v,
        serde_json::json!({ "hardware": { "min_cpu_logical": "four" } }),
    )
    .await?;
    seed_node(&v, &BIG_ARM).await?;

    let err = resolve_eligible_pool_nodes(POOL, &NodeHardwareRequirements::default(), &v)
        .await
        .expect_err("a typo in selector_json must not widen the pool");
    assert!(matches!(err, PoolResolutionError::InvalidSelector { .. }));
    Ok(())
}

#[tokio::test]
async fn malformed_placement_policy_is_refused() -> anyhow::Result<()> {
    let v = test_valence("pool_placement_bad_policy").await;
    seed_pool_with_policy(
        &v,
        serde_json::json!({}),
        serde_json::json!({ "strategy": "round-robin" }),
    )
    .await?;
    seed_node(&v, &BIG_ARM).await?;

    let err = resolve_eligible_pool_nodes(POOL, &NodeHardwareRequirements::default(), &v)
        .await
        .expect_err("an unknown strategy must not silently rank least-loaded");
    assert!(matches!(
        err,
        PoolResolutionError::InvalidPlacementPolicy { .. }
    ));
    Ok(())
}

/// A pool that names no hardware must mean "every node is eligible" rather than "no node reported
/// the hardware I require". Valence adds the column to existing tables as nullable, so JSON null is
/// the other empty spelling; [`super::VirtualPoolSelector`] unit tests cover it.
#[tokio::test]
async fn empty_hardware_selector_column_accepts_every_node() -> anyhow::Result<()> {
    let v = test_valence("pool_placement_empty_column").await;
    seed_pool(&v, serde_json::json!({})).await?;
    seed_node(&v, &SMALL_X86).await?;
    seed_node(&v, &BIG_ARM).await?;

    let ranked =
        resolve_eligible_pool_nodes(POOL, &NodeHardwareRequirements::default(), &v).await?;
    assert_eq!(ranked.len(), 2);
    Ok(())
}

#[tokio::test]
async fn legacy_selector_json_hardware_block_still_filters() -> anyhow::Result<()> {
    let v = test_valence("pool_placement_legacy_hardware").await;
    seed_pool_legacy_selector(
        &v,
        serde_json::json!({ "strategy": "manual-ui", "hardware": { "arch": "arm64" } }),
    )
    .await?;
    seed_node(&v, &SMALL_X86).await?;
    seed_node(&v, &BIG_ARM).await?;

    let ranked =
        resolve_eligible_pool_nodes(POOL, &NodeHardwareRequirements::default(), &v).await?;
    assert_eq!(ranked.len(), 1);
    assert_eq!(ranked[0].node_id, BIG_ARM.node_id);
    Ok(())
}

/// An operator migrating a pool onto the dedicated column should see the column take effect even
/// while a stale `hardware` block is still sitting in `selector_json`.
#[tokio::test]
async fn hardware_selector_column_wins_over_legacy_selector_json() -> anyhow::Result<()> {
    let v = test_valence("pool_placement_column_precedence").await;
    seed_pool_columns(
        &v,
        serde_json::json!({ "hardware": { "arch": "x86_64" } }),
        serde_json::json!({ "hardware": { "arch": "arm64" } }),
        serde_json::json!({}),
    )
    .await?;
    seed_node(&v, &SMALL_X86).await?;
    seed_node(&v, &BIG_ARM).await?;

    let ranked =
        resolve_eligible_pool_nodes(POOL, &NodeHardwareRequirements::default(), &v).await?;
    assert_eq!(ranked.len(), 1);
    assert_eq!(ranked[0].node_id, SMALL_X86.node_id);
    Ok(())
}

/// The deploy path must resolve through the same selector, otherwise a pool could pass preflight on
/// a node the pool does not actually accept.
#[tokio::test]
async fn deploy_target_resolution_honors_the_pool_selector() -> anyhow::Result<()> {
    let v = test_valence("pool_placement_deploy_target").await;
    seed_pool(
        &v,
        serde_json::json!({ "hardware": { "min_cpu_logical": 8 } }),
    )
    .await?;
    seed_node(&v, &SMALL_X86).await?;
    seed_node(&v, &BIG_ARM).await?;

    let resolved =
        resolve_node_for_deploy_target(&DeployTarget::Pool(POOL.to_string()), &v).await?;
    assert_eq!(resolved, BIG_ARM.node_id);
    Ok(())
}

/// A caller floor has to reach a pinned node too. Resolving a `Node` target against hardware the
/// node cannot provide should fail rather than hand back an unusable placement.
#[tokio::test]
async fn pinned_node_target_is_refused_when_it_misses_the_floor() -> anyhow::Result<()> {
    let v = test_valence("pool_placement_pinned_node").await;
    seed_pool(&v, serde_json::json!({})).await?;
    seed_node(&v, &SMALL_X86).await?;

    let floor = NodeHardwareRequirements {
        min_cpu_logical: Some(8),
        ..NodeHardwareRequirements::default()
    };
    let target = DeployTarget::Node(SMALL_X86.node_id.to_string());
    let err = resolve_node_for_deploy_target_with_requirements(&target, &floor, &v)
        .await
        .expect_err("a 2-CPU node cannot satisfy an 8-CPU floor");
    assert!(err.to_string().contains("cpu_logical 2 < 8"));

    // The same pin resolves once the workload stops asking for more than the node has.
    let resolved = resolve_node_for_deploy_target(&target, &v).await?;
    assert_eq!(resolved, SMALL_X86.node_id);
    Ok(())
}
