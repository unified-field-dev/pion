//! Selector-aware node resolution for [`crate::DeployTarget::Pool`] placement.
//!
//! [`resolve_eligible_pool_nodes`] is the single place that answers "which nodes may this pool
//! place onto right now". Deploy resolution takes the first entry; Gluon placement can walk the
//! ranked list to spread replicas.

use std::collections::HashSet;

use anyhow::Context;
use valence::{Model, Valence};

use crate::generated::{
    PionControlPlaneNode, PionControlPlanePoolCellMap, PionControlPlaneVirtualPool,
};

use super::error::PoolResolutionError;
use super::hardware::{NodeHardwareCapabilities, NodeHardwareRequirements, UnmetRequirement};
use super::selector::{PoolPlacementStrategy, VirtualPoolPlacementPolicy, VirtualPoolSelector};
use crate::control_plane::deploy::node_eligible_for_deploy;

/// A pool node that is reachable and clears both the pool selector and the workload floor.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EligiblePoolNode {
    /// Node id to deploy against.
    pub node_id: String,
    /// Cell the node belongs to.
    pub cell_id: String,
    /// Reported hostname.
    pub hostname: String,
    /// Hardware the node last reported.
    pub hardware: NodeHardwareCapabilities,
    /// Running container count from the node's most recent heartbeat.
    pub running_containers: u64,
}

/// Returns the nodes a virtual pool may place onto, best candidate first.
///
/// Eligibility is the conjunction of four things: the node sits in a cell the pool maps with
/// `enabled: true`, its inventory status accepts deploy work (`online`, `draining`, or `failed`,
/// so recovery deploys still reach unhealthy hosts), it clears the pool's stored hardware
/// selector, and it clears `requirements`.
///
/// Filtering uses hardware the agent already reports through heartbeat (`arch`, `cpu_logical`,
/// `memory_bytes`). Operator-managed labels are not consulted. Disk is out of scope here; volume
/// headroom belongs to Gluon capacity planning against per-mount heartbeat data.
///
/// Pass [`NodeHardwareRequirements::default`] for an unconstrained placement. A pool with no
/// stored selector (see [`VirtualPoolSelector::from_pool_columns`]) and an unconstrained
/// `requirements` yields every reachable node in the mapped cells, ordered exactly as pool
/// deploys ordered them before selectors existed.
///
/// # Errors
///
/// See [`PoolResolutionError`]. The returned vector is never empty: an empty candidate set is
/// reported as [`PoolResolutionError::NoEligibleNodes`] or
/// [`PoolResolutionError::NoMatchingHardware`] so the caller learns which one it was.
///
/// # Examples
///
/// ```no_run
/// # #[cfg(feature = "runtime")]
/// # {
/// use pion::{resolve_eligible_pool_nodes, CpuArchitecture, NodeHardwareRequirements};
///
/// # async fn demo(valence: &valence::Valence) -> anyhow::Result<()> {
/// let floor = NodeHardwareRequirements {
///     architectures: vec![CpuArchitecture::Aarch64],
///     min_cpu_logical: Some(4),
///     min_memory_bytes: Some(8 * 1024 * 1024 * 1024),
/// };
/// let ranked = resolve_eligible_pool_nodes("edge-west", &floor, valence).await?;
/// let best = &ranked[0];
/// println!("{} on {}", best.node_id, best.cell_id);
/// # Ok(())
/// # }
/// # }
/// ```
pub async fn resolve_eligible_pool_nodes(
    pool_id: &str,
    requirements: &NodeHardwareRequirements,
    valence: &Valence,
) -> Result<Vec<EligiblePoolNode>, PoolResolutionError> {
    let pool = PionControlPlaneVirtualPool::get_used(pool_id, valence, valence::use_!("get PionControlPlaneVirtualPool in control_plane/pool_placement/resolve.rs; Valence persistence for this feature path; typed store; visible to session actor / service path."))
        .await
        .with_context(|| format!("load pool {pool_id} for placement resolution"))?
        .ok_or_else(|| PoolResolutionError::PoolNotFound {
            pool_id: pool_id.to_string(),
        })?;
    let selector =
        VirtualPoolSelector::from_pool_columns(pool.hardware_selector_json(), pool.selector_json())
            .map_err(|source| PoolResolutionError::InvalidSelector {
                pool_id: pool_id.to_string(),
                source,
            })?;
    let policy =
        VirtualPoolPlacementPolicy::from_placement_policy_json(pool.placement_policy_json())
            .map_err(|source| PoolResolutionError::InvalidPlacementPolicy {
                pool_id: pool_id.to_string(),
                source,
            })?;

    let enabled_cells = enabled_cell_ids(pool_id, valence).await?;
    let reachable = reachable_nodes_in_cells(pool_id, &enabled_cells, valence).await?;

    let mut matched = Vec::with_capacity(reachable.len());
    let mut unmet = Vec::new();
    for (node_id, node) in &reachable {
        let hardware = NodeHardwareCapabilities::from_capabilities_json(node.capabilities_json());
        if let Some(reason) = first_unmet(&selector, requirements, &hardware) {
            unmet.push(reason);
            continue;
        }
        matched.push(EligiblePoolNode {
            node_id: node_id.clone(),
            cell_id: node.cell_id().clone(),
            hostname: node.hostname().clone(),
            running_containers: running_container_count(node),
            hardware,
        });
    }

    if matched.is_empty() {
        return Err(PoolResolutionError::NoMatchingHardware {
            pool_id: pool_id.to_string(),
            considered: reachable.len(),
            unmet,
        });
    }

    rank_candidates(&mut matched, policy.strategy);
    tracing::debug!(
        target: "pion.placement",
        pool_id = %pool_id,
        strategy = %policy.strategy.as_str(),
        considered = reachable.len(),
        eligible = matched.len(),
        "resolved eligible pool nodes"
    );
    Ok(matched)
}

/// Pool selector first, then the workload floor, so an operator-facing pool misconfiguration is
/// reported ahead of a workload that simply asked for too much.
fn first_unmet(
    selector: &VirtualPoolSelector,
    requirements: &NodeHardwareRequirements,
    hardware: &NodeHardwareCapabilities,
) -> Option<UnmetRequirement> {
    selector
        .hardware
        .unmet(hardware)
        .or_else(|| requirements.unmet(hardware))
}

async fn enabled_cell_ids(
    pool_id: &str,
    valence: &Valence,
) -> Result<HashSet<String>, PoolResolutionError> {
    let cells = PionControlPlanePoolCellMap::query_used(valence, valence::use_!("query PionControlPlanePoolCellMap in control_plane/pool_placement/resolve.rs; Valence persistence for this feature path; typed store; visible to session actor / service path."))
        .await
        .with_context(|| format!("query pool-cell map for pool {pool_id}"))?
        .into_iter()
        .filter(|row| row.virtual_pool_id() == pool_id && *row.enabled())
        .map(|row| row.cell_id().clone())
        .collect::<HashSet<_>>();
    if cells.is_empty() {
        return Err(PoolResolutionError::NoEnabledCells {
            pool_id: pool_id.to_string(),
        });
    }
    Ok(cells)
}

/// Nodes in `cells` whose status accepts deploy work, paired with their stable node ids.
///
/// Rows without a stable primary key cannot be targeted, so they are dropped. When dropping them
/// is what emptied the set, the caller hears [`PoolResolutionError::UnstableNodeId`] instead of a
/// misleading "no reachable nodes".
async fn reachable_nodes_in_cells(
    pool_id: &str,
    cells: &HashSet<String>,
    valence: &Valence,
) -> Result<Vec<(String, PionControlPlaneNode)>, PoolResolutionError> {
    let in_cells = PionControlPlaneNode::query_used(valence, valence::use_!("query PionControlPlaneNode in control_plane/pool_placement/resolve.rs; Valence persistence for this feature path; typed store; visible to session actor / service path."))
        .await
        .context("query nodes for pool placement resolution")?
        .into_iter()
        .filter(|node| node_eligible_for_deploy(node.status()) && cells.contains(node.cell_id()))
        .collect::<Vec<_>>();
    if in_cells.is_empty() {
        return Err(PoolResolutionError::NoEligibleNodes {
            pool_id: pool_id.to_string(),
        });
    }
    let identified = in_cells
        .into_iter()
        .filter_map(|node| stable_node_id(&node).map(|id| (id, node)))
        .collect::<Vec<_>>();
    if identified.is_empty() {
        return Err(PoolResolutionError::UnstableNodeId {
            pool_id: pool_id.to_string(),
        });
    }
    Ok(identified)
}

fn stable_node_id(node: &PionControlPlaneNode) -> Option<String> {
    node.id()
        .and_then(|record| valence::extract_id_from_record(record).ok())
        .filter(|id| !id.is_empty())
}

fn running_container_count(node: &PionControlPlaneNode) -> u64 {
    node.capabilities_json()
        .get("containers")
        .and_then(|containers| containers.get("running"))
        .and_then(|running| {
            running
                .as_u64()
                .or_else(|| running.as_i64().and_then(|n| u64::try_from(n).ok()))
        })
        .unwrap_or_default()
}

/// Orders candidates best-first.
///
/// Every strategy falls through to fewest running containers, then hostname, then node id, so the
/// order is total and stable across calls. [`PoolPlacementStrategy::LeastLoaded`] is exactly that
/// fallback, which is the ordering pool deploys have always used.
fn rank_candidates(candidates: &mut [EligiblePoolNode], strategy: PoolPlacementStrategy) {
    candidates.sort_by(|a, b| {
        strategy_order(a, b, strategy)
            .then_with(|| a.running_containers.cmp(&b.running_containers))
            .then_with(|| a.hostname.cmp(&b.hostname))
            .then_with(|| a.node_id.cmp(&b.node_id))
    });
}

fn strategy_order(
    a: &EligiblePoolNode,
    b: &EligiblePoolNode,
    strategy: PoolPlacementStrategy,
) -> std::cmp::Ordering {
    match strategy {
        PoolPlacementStrategy::LeastLoaded => std::cmp::Ordering::Equal,
        PoolPlacementStrategy::MostCpu => b
            .hardware
            .cpu_logical
            .unwrap_or_default()
            .cmp(&a.hardware.cpu_logical.unwrap_or_default()),
        PoolPlacementStrategy::MostMemory => b
            .hardware
            .memory_bytes
            .unwrap_or_default()
            .cmp(&a.hardware.memory_bytes.unwrap_or_default()),
    }
}
