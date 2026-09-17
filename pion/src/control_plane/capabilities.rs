//! Per-node action capability toggles (which [`ContainerActionKind`] values a node may run).
//!
//! Capabilities are stored as Valence rows keyed by `node_id` + action suffix. Infra-style
//! kinds (`EnsureNetwork`, `ProbeHost`, `WireguardPeer`, `EnsureDockerImage`, `GrowFs`,
//! `TemplatedExec`) share the
//! **deploy** capability gate. Heartbeat ingest seeds defaults via
//! `ensure_default_node_action_capabilities`; enqueue / execute paths consult capability
//! maps before work proceeds.

use anyhow::{Context, Result};
use chrono::Utc;
use std::collections::HashMap;
use valence::{Model, Valence};

use crate::generated::{
    PionControlPlaneNodeActionCapability, PionControlPlaneNodeActionCapabilityCapability,
};

use super::ContainerActionKind;

fn all_container_action_kinds() -> [ContainerActionKind; 7] {
    [
        ContainerActionKind::Start,
        ContainerActionKind::Stop,
        ContainerActionKind::Restart,
        ContainerActionKind::Logs,
        ContainerActionKind::Inspect,
        ContainerActionKind::Deploy,
        ContainerActionKind::ProbeHost,
    ]
}

pub(crate) fn capability_row_id(node_id: &str, action: ContainerActionKind) -> String {
    let suffix = match action {
        // Shares the deploy capability gate (infra provisioning on the node).
        ContainerActionKind::EnsureNetwork
        | ContainerActionKind::ProbeHost
        | ContainerActionKind::WireguardPeer
        | ContainerActionKind::EnsureDockerImage
        | ContainerActionKind::GrowFs
        | ContainerActionKind::TemplatedExec => "deploy",
        _ => action.as_str(),
    };
    format!("{node_id}:{suffix}")
}

fn map_action_to_capability_enum(
    action: ContainerActionKind,
) -> PionControlPlaneNodeActionCapabilityCapability {
    match action {
        ContainerActionKind::Start => PionControlPlaneNodeActionCapabilityCapability::Start,
        ContainerActionKind::Stop => PionControlPlaneNodeActionCapabilityCapability::Stop,
        ContainerActionKind::Restart => PionControlPlaneNodeActionCapabilityCapability::Restart,
        ContainerActionKind::Logs => PionControlPlaneNodeActionCapabilityCapability::Logs,
        ContainerActionKind::Inspect => PionControlPlaneNodeActionCapabilityCapability::Inspect,
        ContainerActionKind::Deploy
        | ContainerActionKind::EnsureNetwork
        | ContainerActionKind::ProbeHost
        | ContainerActionKind::Diagnostic
        | ContainerActionKind::WireguardPeer
        | ContainerActionKind::EnsureDockerImage
        | ContainerActionKind::GrowFs
        | ContainerActionKind::TemplatedExec => {
            PionControlPlaneNodeActionCapabilityCapability::Deploy
        }
    }
}

/// Seeds a **disabled** capability row for every action kind on first heartbeat.
///
/// # Contract
///
/// New nodes start with every action capability `enabled: false` (secure default — an operator
/// must explicitly opt a node into deploy/exec-shaped actions via
/// [`upsert_node_action_capability`]). This only *creates* missing rows; it never overwrites an
/// existing row's `enabled` value, so an operator's prior toggle survives subsequent heartbeats.
pub(crate) async fn ensure_default_node_action_capabilities(
    node_id: &str,
    valence: &Valence,
) -> Result<()> {
    let now = Utc::now();
    for action in all_container_action_kinds() {
        let id = capability_row_id(node_id, action);
        if PionControlPlaneNodeActionCapability::get_used(&id, valence, valence::use_!("In **Pion control plane**, we **load Pion Control Plane Node Action Capability** so the application can decide what to do next in this workflow. The result is used by **Pion control plane** logic—not necessarily displayed on a page unless that feature’s UI shows it."))
            .await
            .with_context(|| format!("load default capability row {id}"))?
            .is_some()
        {
            continue;
        }
        let record = PionControlPlaneNodeActionCapability::new(
            node_id.to_string(),
            map_action_to_capability_enum(action),
            false,
            "heartbeat-default".to_string(),
            String::new(),
            "system".to_string(),
            now,
            now,
        )
        .context("build default capability row")?;
        PionControlPlaneNodeActionCapability::upsert_used(&id, record, valence, valence::use_!("When **Pion control plane** needs to persist work, we **save Pion Control Plane Node Action Capability** so the next step in that feature can continue with the latest values. People and services allowed for **Pion control plane** use this data for that workflow—not as a general export of unrelated personal fields."))
            .await
            .with_context(|| format!("upsert default capability row {id}"))?;
    }
    Ok(())
}

/// Upserts a node action capability toggle row.
///
/// # Errors
///
/// Propagates Valence read or upsert failures.
pub async fn upsert_node_action_capability(
    node_id: &str,
    action: ContainerActionKind,
    enabled: bool,
    source: &str,
    reason: Option<&str>,
    updated_by: Option<&str>,
    valence: &Valence,
) -> Result<()> {
    let now = Utc::now();
    let id = capability_row_id(node_id, action);
    let created_at = if let Some(existing) = PionControlPlaneNodeActionCapability::get_used(&id, valence, valence::use_!("In **Pion control plane**, we **load Pion Control Plane Node Action Capability** so the application can decide what to do next in this workflow. The result is used by **Pion control plane** logic—not necessarily displayed on a page unless that feature’s UI shows it."))
        .await
        .with_context(|| format!("load capability row {id} for upsert"))?
    {
        *existing.created_at()
    } else {
        now
    };
    let record = PionControlPlaneNodeActionCapability::new(
        node_id.to_string(),
        map_action_to_capability_enum(action),
        enabled,
        source.to_string(),
        reason.unwrap_or_default().to_string(),
        updated_by.unwrap_or("system").to_string(),
        created_at,
        now,
    )
    .context("build capability row")?;
    PionControlPlaneNodeActionCapability::upsert_used(&id, record, valence, valence::use_!("When **Pion control plane** needs to persist work, we **save Pion Control Plane Node Action Capability** so the next step in that feature can continue with the latest values. People and services allowed for **Pion control plane** use this data for that workflow—not as a general export of unrelated personal fields."))
        .await
        .with_context(|| format!("upsert capability row {id}"))?;
    tracing::info!(
        target: "security.capability",
        node_id = %node_id,
        action = %action.as_str(),
        enabled = %enabled,
        source = %source,
        updated_by = %updated_by.unwrap_or("system"),
        "node action capability upserted"
    );
    Ok(())
}

/// Returns capability map keyed by node id then action name.
///
/// # Errors
///
/// Propagates Valence query failures.
pub async fn list_node_action_capability_map(
    valence: &Valence,
) -> Result<HashMap<String, HashMap<String, bool>>> {
    let rows = PionControlPlaneNodeActionCapability::query_used(valence, valence::use_!("In **Pion control plane**, we **list Pion Control Plane Node Action Capability** so the product can show or process the matching set for this workflow. Callers allowed for **Pion control plane** use the list; it is not a public dump of every field to anonymous visitors."))
        .await
        .context("query node action capability rows")?;
    let mut map: HashMap<String, HashMap<String, bool>> = HashMap::new();
    for row in rows {
        map.entry(row.node_id().clone())
            .or_default()
            .insert(row.capability().as_str().to_string(), *row.enabled());
    }
    Ok(map)
}

pub(crate) fn is_action_enabled(
    map: &HashMap<String, HashMap<String, bool>>,
    node_id: &str,
    action: ContainerActionKind,
) -> bool {
    let cap_key = match action {
        ContainerActionKind::EnsureNetwork
        | ContainerActionKind::ProbeHost
        | ContainerActionKind::Diagnostic
        | ContainerActionKind::WireguardPeer
        | ContainerActionKind::EnsureDockerImage
        | ContainerActionKind::GrowFs
        | ContainerActionKind::TemplatedExec => "deploy",
        _ => action.as_str(),
    };
    map.get(node_id)
        .and_then(|node_caps| node_caps.get(cap_key))
        .copied()
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn capability_row_id_is_deterministic() {
        assert_eq!(
            capability_row_id("node-a", ContainerActionKind::Logs),
            "node-a:logs".to_string()
        );
        assert_eq!(
            capability_row_id("node-a", ContainerActionKind::EnsureNetwork),
            "node-a:deploy".to_string()
        );
    }

    #[test]
    fn is_action_enabled_defaults_to_false() {
        let map: HashMap<String, HashMap<String, bool>> = HashMap::new();
        assert!(!is_action_enabled(
            &map,
            "node-a",
            ContainerActionKind::Inspect
        ));
    }

    #[test]
    fn is_action_enabled_reads_capability_map() {
        let mut node_caps = HashMap::new();
        node_caps.insert("inspect".to_string(), true);
        let mut map = HashMap::new();
        map.insert("node-a".to_string(), node_caps);
        assert!(is_action_enabled(
            &map,
            "node-a",
            ContainerActionKind::Inspect
        ));
        assert!(!is_action_enabled(
            &map,
            "node-a",
            ContainerActionKind::Stop
        ));
    }

    #[test]
    fn is_action_enabled_treats_ensure_docker_image_as_deploy_capability() {
        let mut node_caps = HashMap::new();
        node_caps.insert("deploy".to_string(), true);
        let mut map = HashMap::new();
        map.insert("node-a".to_string(), node_caps);
        assert!(is_action_enabled(
            &map,
            "node-a",
            ContainerActionKind::EnsureDockerImage
        ));
    }

    #[test]
    fn is_action_enabled_treats_grow_fs_as_deploy_capability() {
        let mut node_caps = HashMap::new();
        node_caps.insert("deploy".to_string(), true);
        let mut map = HashMap::new();
        map.insert("node-a".to_string(), node_caps);
        assert!(is_action_enabled(
            &map,
            "node-a",
            ContainerActionKind::GrowFs
        ));
        assert_eq!(
            capability_row_id("node-a", ContainerActionKind::GrowFs),
            "node-a:deploy"
        );

        let mut denied = HashMap::new();
        denied.insert("deploy".to_string(), false);
        map.insert("node-a".to_string(), denied);
        assert!(!is_action_enabled(
            &map,
            "node-a",
            ContainerActionKind::GrowFs
        ));
    }

    #[test]
    fn is_action_enabled_treats_templated_exec_as_deploy_capability() {
        let mut node_caps = HashMap::new();
        node_caps.insert("deploy".to_string(), true);
        let mut map = HashMap::new();
        map.insert("node-a".to_string(), node_caps);
        assert!(is_action_enabled(
            &map,
            "node-a",
            ContainerActionKind::TemplatedExec
        ));
        assert_eq!(
            capability_row_id("node-a", ContainerActionKind::TemplatedExec),
            "node-a:deploy"
        );

        let mut denied = HashMap::new();
        denied.insert("deploy".to_string(), false);
        map.insert("node-a".to_string(), denied);
        assert!(!is_action_enabled(
            &map,
            "node-a",
            ContainerActionKind::TemplatedExec
        ));
    }

    async fn test_valence() -> Valence {
        let boot = crate::valence_bootstrap::bootstrap_sqlite_memory()
            .await
            .expect("sqlite memory bootstrap");
        boot.valence("capabilities_test").expect("valence build")
    }

    /// F7: fresh capability rows must default to `enabled: false` — operators opt nodes into
    /// deploy/exec-shaped actions explicitly via [`upsert_node_action_capability`].
    #[tokio::test]
    async fn ensure_default_node_action_capabilities_seeds_disabled_rows() -> anyhow::Result<()> {
        let v = test_valence().await;
        ensure_default_node_action_capabilities("node-fresh", &v).await?;
        let map = list_node_action_capability_map(&v).await?;
        for action in all_container_action_kinds() {
            assert!(
                !is_action_enabled(&map, "node-fresh", action),
                "expected {action:?} to default to disabled"
            );
        }
        Ok(())
    }

    /// A second call (subsequent heartbeat) must not clobber an operator's prior opt-in.
    #[tokio::test]
    async fn ensure_default_node_action_capabilities_does_not_overwrite_existing_toggle(
    ) -> anyhow::Result<()> {
        let v = test_valence().await;
        ensure_default_node_action_capabilities("node-existing", &v).await?;
        upsert_node_action_capability(
            "node-existing",
            ContainerActionKind::Deploy,
            true,
            "operator",
            None,
            None,
            &v,
        )
        .await?;
        // Simulate a subsequent heartbeat re-running the default-seed step.
        ensure_default_node_action_capabilities("node-existing", &v).await?;
        let map = list_node_action_capability_map(&v).await?;
        assert!(is_action_enabled(
            &map,
            "node-existing",
            ContainerActionKind::Deploy
        ));
        Ok(())
    }
}
