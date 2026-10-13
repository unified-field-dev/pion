//! Typed hardware selection for virtual pool placement.
//!
//! A [`crate::DeployTarget::Pool`] names a virtual pool, and the pool maps to cells. Which node
//! inside those cells actually receives the workload is decided here, from three inputs:
//!
//! | Input | Source | Meaning |
//! |-------|--------|---------|
//! | [`VirtualPoolSelector`] | pool `hardware_selector_json` | hardware floor for the whole pool |
//! | [`NodeHardwareRequirements`] | caller, per placement | hardware floor for one workload |
//! | [`VirtualPoolPlacementPolicy`] | pool `placement_policy_json` | how survivors are ranked |
//!
//! Node facts come from [`NodeHardwareCapabilities`], parsed out of the `capabilities_json` that
//! heartbeat ingest already writes. Operator-managed `labels_json` is not consulted: agents
//! report those labels and today they are empty, so a label-based rule would silently match
//! nothing.
//!
//! Disk is not a selector dimension. Volume headroom is a capacity-planning concern for the
//! orchestrator, which needs per-mount figures rather than pool membership.
//!
//! # Compatibility
//!
//! `hardware_selector_json` is an optional column, so existing pool rows read it back as JSON null
//! and stay unconstrained. An unconstrained selector plus an unconstrained caller floor reproduces
//! the previous behavior exactly: every reachable node in the mapped cells, least-loaded first.
//! Pools that stored a hardware rule under the older `selector_json.hardware` key still resolve
//! through [`VirtualPoolSelector::from_pool_columns`].
//!
//! # Entry point
//!
//! [`resolve_eligible_pool_nodes`] returns the ranked candidate list.
//! [`crate::resolve_node_for_deploy_target_with_requirements`] and
//! [`crate::preflight_deploy_target_with_requirements`] wrap it for the deploy path.

mod error;
mod hardware;
mod resolve;
mod selector;

#[cfg(test)]
mod tests;

pub use error::PoolResolutionError;
pub use hardware::{
    CpuArchitecture, NodeHardwareCapabilities, NodeHardwareRequirements, UnmetRequirement,
};
pub use resolve::{resolve_eligible_pool_nodes, EligiblePoolNode};
pub use selector::{PoolPlacementStrategy, VirtualPoolPlacementPolicy, VirtualPoolSelector};
