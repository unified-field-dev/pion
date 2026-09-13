//! Read-model projections for operator UIs (container / image / node snapshots).
//!
//! These helpers query Valence control-plane tables and reshape them into
//! [`RuntimeContainerSnapshot`] / related structs. Freshness labels use
//! [`crate::OBSERVED_STALE_AFTER_SECS`] relative to wall clock — they are not
//! written back to the database.

use anyhow::Result;
use chrono::Utc;
use std::collections::HashMap;
use valence::Valence;

use crate::generated::{
    PionControlPlaneNode, PionControlPlaneNodeConnectionMode, PionControlPlaneObservedStatus,
};

use super::ContainerStatusSummary;

use crate::OBSERVED_STALE_AFTER_SECS;

/// Runtime container summary projection used by control-plane UI/read APIs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimeContainerSnapshot {
    /// Control-plane node id.
    pub node_id: String,
    /// Cell (fleet partition) id.
    pub cell_id: String,
    /// Agent-reported hostname.
    pub hostname: String,
    /// Node inventory status wire value.
    pub node_status: String,
    /// Derived health class from latest observed status.
    pub observed_health: String,
    /// `fresh` or `stale` relative to [`crate::OBSERVED_STALE_AFTER_SECS`].
    pub freshness: String,
    /// Timestamp of the underlying observed-status row.
    pub observed_at: chrono::DateTime<chrono::Utc>,
    /// Running container count from heartbeat summary.
    pub running: u64,
    /// Exited container count from heartbeat summary.
    pub exited: u64,
    /// Unhealthy container count from heartbeat summary.
    pub unhealthy: u64,
}

/// Runtime image summary projection used by control-plane UI/read APIs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimeImageSnapshot {
    /// Gluon catalog image id (when joined).
    pub image_id: String,
    /// Repository name component.
    pub name: String,
    /// Tag component.
    pub tag: String,
    /// Catalog description text.
    pub description: String,
    /// When the image row was created in catalog.
    pub created_at: chrono::DateTime<chrono::Utc>,
    /// Count of nodes that reported this image on last heartbeat.
    pub observed_node_count: u64,
    /// Derived flavor label (for example `pion`, `monolithic`).
    pub flavor: String,
    /// Provenance hint (build pipeline / source).
    pub provenance: String,
}

/// Runtime health timeline projection used by control-plane UI/read APIs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimeHealthSnapshot {
    /// Observed-status row id.
    pub snapshot_id: String,
    /// Node that produced the heartbeat.
    pub node_id: String,
    /// Cell id on the observed-status row.
    pub cell_id: String,
    /// Ingest source wire value (for example agent heartbeat).
    pub source: String,
    /// Health class wire value.
    pub health: String,
    /// `fresh` or `stale` relative to [`crate::OBSERVED_STALE_AFTER_SECS`].
    pub freshness: String,
    /// When the observation was recorded.
    pub observed_at: chrono::DateTime<chrono::Utc>,
}

fn runtime_freshness_label(
    observed_at: chrono::DateTime<chrono::Utc>,
    now: chrono::DateTime<chrono::Utc>,
) -> &'static str {
    if now.signed_duration_since(observed_at)
        <= chrono::Duration::seconds(OBSERVED_STALE_AFTER_SECS)
    {
        "fresh"
    } else {
        "stale"
    }
}

fn summary_count(value: &serde_json::Value, key: &str, fallback: u64) -> u64 {
    value
        .get(key)
        .and_then(serde_json::Value::as_u64)
        .unwrap_or(fallback)
}

fn parse_container_summary(
    observed_json: &serde_json::Value,
    fallback: &ContainerStatusSummary,
) -> ContainerStatusSummary {
    let containers = observed_json.get("containers");
    // Canonical [`parton::NodeHeartbeatReport`] JSON: `containers.summary.{running,…}`.
    if let Some(summary) = containers.and_then(|v| v.get("summary")) {
        return ContainerStatusSummary {
            running: summary_count(summary, "running", fallback.running),
            exited: summary_count(summary, "exited", fallback.exited),
            unhealthy: summary_count(summary, "unhealthy", fallback.unhealthy),
        };
    }
    // Legacy flat shape: `containers.{running,…}` (pre–ContainerStatusReport wrapper).
    ContainerStatusSummary {
        running: containers.map_or(fallback.running, |v| {
            summary_count(v, "running", fallback.running)
        }),
        exited: containers.map_or(fallback.exited, |v| {
            summary_count(v, "exited", fallback.exited)
        }),
        unhealthy: containers.map_or(fallback.unhealthy, |v| {
            summary_count(v, "unhealthy", fallback.unhealthy)
        }),
    }
}

/// Parses `observed_json.images` for runtime image rollups (joined with Gluon image catalog in `gluon::runtime_image_snapshots`).
pub fn parse_runtime_images(observed_json: &serde_json::Value) -> Vec<String> {
    let Some(images) = observed_json.get("images").and_then(|v| v.as_array()) else {
        return Vec::new();
    };

    images
        .iter()
        .filter_map(|image| {
            if let Some(raw) = image.as_str() {
                let trimmed = raw.trim();
                return if trimmed.is_empty() {
                    None
                } else {
                    Some(trimmed.to_string())
                };
            }
            let name = image
                .get("name")
                .and_then(|v| v.as_str())
                .unwrap_or_default();
            let tag = image
                .get("tag")
                .and_then(|v| v.as_str())
                .unwrap_or_default();
            let reference = image
                .get("reference")
                .and_then(|v| v.as_str())
                .unwrap_or_default();
            if !reference.trim().is_empty() {
                Some(reference.trim().to_string())
            } else if !name.trim().is_empty() && !tag.trim().is_empty() {
                Some(format!("{}:{}", name.trim(), tag.trim()))
            } else {
                None
            }
        })
        .collect()
}

/// Image flavor label for control-plane runtime views (aligned with Gluon catalog split-build tags).
pub fn derive_image_flavor(_name: &str, tag: &str) -> String {
    let tag_lower = tag.trim().to_ascii_lowercase();
    if tag_lower == "monolithic" || tag_lower.ends_with("-monolithic") {
        return "monolithic".to_string();
    }
    if (tag_lower == "apps" || tag_lower.ends_with("-apps")) && !tag_lower.contains("all_apps") {
        return "apps".to_string();
    }
    if tag_lower == "pion" || tag_lower.ends_with("-pion") {
        return "pion".to_string();
    }
    if tag_lower == "cp"
        || tag_lower.ends_with("-cp")
        || tag_lower == "control_plane"
        || tag_lower.ends_with("-control_plane")
    {
        return "pion".to_string();
    }
    if tag_lower == "worker"
        || tag_lower.ends_with("-worker")
        || tag_lower == "all_apps"
        || tag_lower.ends_with("-all_apps")
    {
        return "legacy-worker".to_string();
    }
    if tag_lower == "full"
        || tag_lower.ends_with("-full")
        || tag_lower == "all_apps_runtimes"
        || tag_lower.ends_with("-all_apps_runtimes")
    {
        return "monolithic".to_string();
    }
    if tag_lower == "chronon-runtime"
        || tag_lower.ends_with("-chronon-runtime")
        || tag_lower.ends_with("-chronon")
    {
        return "chronon-runtime".to_string();
    }
    if tag_lower == "photon-runtime"
        || tag_lower.ends_with("-photon-runtime")
        || tag_lower.ends_with("-photon")
    {
        return "photon-runtime".to_string();
    }
    if tag_lower == "boson-runtime"
        || tag_lower.ends_with("-boson-runtime")
        || tag_lower.ends_with("-boson")
    {
        return "boson-runtime".to_string();
    }

    "unknown".to_string()
}

/// Agent-connected control-plane nodes in `cell_id` (from heartbeats).
///
/// This list is independent of `PionAgentHostEnrollment` wizard tickets. When strict enrollment is
/// disabled via `PARTON_ENROLLMENT_STRICT_NEW_NODES=0`, [`super::heartbeat::ingest_node_heartbeat`]
/// can create a new [`PionControlPlaneNode`] on the first heartbeat **without** claiming an enrollment
/// row — so the setup wizard may show no enrollments while nodes still appear here.
///
/// Returns `(node_id, hostname)` sorted by node id.
///
/// # Errors
///
/// Propagates Valence query failures.
pub async fn list_agent_nodes_for_cell(
    valence: &Valence,
    cell_id: &str,
) -> Result<Vec<(String, String)>> {
    let target = cell_id.trim();
    let nodes = PionControlPlaneNode::query_used(valence, valence::use_!("query PionControlPlaneNode in src/control_plane/projections.rs; Valence persistence for this feature path; typed store; visible to session actor / service path.")).await?;
    let mut out: Vec<(String, String)> = nodes
        .into_iter()
        .filter(|n| {
            n.cell_id().trim() == target
                && n.connection_mode() == &PionControlPlaneNodeConnectionMode::Agent
        })
        .filter_map(|n| {
            let id = n
                .id()
                .and_then(|t| valence::extract_id_from_record(t).ok())
                .filter(|s| !s.is_empty())?;
            Some((id, n.hostname().trim().to_string()))
        })
        .collect();
    out.sort_by(|a, b| a.0.cmp(&b.0));
    Ok(out)
}

/// Lists latest runtime container snapshots grouped per node.
///
/// # Errors
///
/// Propagates Valence query failures.
pub async fn list_runtime_container_snapshots(
    valence: &Valence,
) -> Result<Vec<RuntimeContainerSnapshot>> {
    let nodes = PionControlPlaneNode::query_used(valence, valence::use_!("query PionControlPlaneNode in src/control_plane/projections.rs; Valence persistence for this feature path; typed store; visible to session actor / service path.")).await?;
    let observed = PionControlPlaneObservedStatus::query_used(valence, valence::use_!("query PionControlPlaneObservedStatus in src/control_plane/projections.rs; Valence persistence for this feature path; typed store; visible to session actor / service path.")).await?;
    let now = Utc::now();

    let mut newest_by_node: HashMap<String, PionControlPlaneObservedStatus> = HashMap::new();
    for snapshot in observed {
        let node_id = snapshot.node_id().clone();
        let should_replace = newest_by_node
            .get(&node_id)
            .is_none_or(|existing| snapshot.observed_at() > existing.observed_at());
        if should_replace {
            newest_by_node.insert(node_id, snapshot);
        }
    }

    let mut rows = Vec::new();
    for node in nodes {
        let node_id = node
            .id()
            .map(|t| valence::extract_id_from_record(t).unwrap_or_default())
            .unwrap_or_default();
        let Some(snapshot) = newest_by_node.get(&node_id) else {
            continue;
        };
        let fallback = ContainerStatusSummary {
            running: 0,
            exited: 0,
            unhealthy: 0,
        };
        let counts = parse_container_summary(snapshot.observed_json(), &fallback);
        rows.push(RuntimeContainerSnapshot {
            node_id,
            cell_id: node.cell_id().clone(),
            hostname: node.hostname().clone(),
            node_status: node.status().as_str().to_string(),
            observed_health: snapshot.health().as_str().to_string(),
            freshness: runtime_freshness_label(*snapshot.observed_at(), now).to_string(),
            observed_at: *snapshot.observed_at(),
            running: counts.running,
            exited: counts.exited,
            unhealthy: counts.unhealthy,
        });
    }

    rows.sort_by_key(|b| std::cmp::Reverse(b.observed_at));
    Ok(rows)
}

/// Lists observed health snapshots ordered by newest first.
///
/// # Errors
///
/// Propagates Valence query failures.
pub async fn list_runtime_health_snapshots(
    valence: &Valence,
) -> Result<Vec<RuntimeHealthSnapshot>> {
    let observed = PionControlPlaneObservedStatus::query_used(valence, valence::use_!("query PionControlPlaneObservedStatus in src/control_plane/projections.rs; Valence persistence for this feature path; typed store; visible to session actor / service path.")).await?;
    let now = Utc::now();
    let mut rows = observed
        .into_iter()
        .map(|snapshot| RuntimeHealthSnapshot {
            snapshot_id: snapshot
                .id()
                .map(|t| valence::extract_id_from_record(t).unwrap_or_default())
                .unwrap_or_default(),
            node_id: snapshot.node_id().clone(),
            cell_id: snapshot.cell_id().clone(),
            source: snapshot.source().as_str().to_string(),
            health: snapshot.health().as_str().to_string(),
            freshness: runtime_freshness_label(*snapshot.observed_at(), now).to_string(),
            observed_at: *snapshot.observed_at(),
        })
        .collect::<Vec<_>>();
    rows.sort_by_key(|b| std::cmp::Reverse(b.observed_at));
    Ok(rows)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn runtime_freshness_label_matches_stale_threshold() {
        let now = Utc::now();
        assert_eq!(runtime_freshness_label(now, now), "fresh");
        assert_eq!(
            runtime_freshness_label(
                now - chrono::Duration::seconds(OBSERVED_STALE_AFTER_SECS + 1),
                now
            ),
            "stale"
        );
    }

    #[test]
    fn parse_container_summary_uses_fallback_when_json_missing() {
        let fallback = ContainerStatusSummary {
            running: 4,
            exited: 1,
            unhealthy: 2,
        };
        let parsed = parse_container_summary(&serde_json::json!({}), &fallback);
        assert_eq!(parsed, fallback);
    }

    #[test]
    fn parse_container_summary_reads_nested_summary_from_heartbeat_report() {
        let fallback = ContainerStatusSummary {
            running: 0,
            exited: 0,
            unhealthy: 0,
        };
        let parsed = parse_container_summary(
            &serde_json::json!({
                "containers": {
                    "containers": [],
                    "summary": { "running": 2, "exited": 1, "unhealthy": 0 }
                }
            }),
            &fallback,
        );
        assert_eq!(
            parsed,
            ContainerStatusSummary {
                running: 2,
                exited: 1,
                unhealthy: 0,
            }
        );
    }

    #[test]
    fn parse_container_summary_reads_legacy_flat_containers_object() {
        let fallback = ContainerStatusSummary {
            running: 0,
            exited: 0,
            unhealthy: 0,
        };
        let parsed = parse_container_summary(
            &serde_json::json!({
                "containers": { "running": 3, "exited": 2, "unhealthy": 1 }
            }),
            &fallback,
        );
        assert_eq!(
            parsed,
            ContainerStatusSummary {
                running: 3,
                exited: 2,
                unhealthy: 1,
            }
        );
    }

    #[test]
    fn parse_runtime_images_supports_string_and_object_shapes() {
        let parsed = parse_runtime_images(&serde_json::json!({
            "images": [
                "ghcr.io/example/a:latest",
                {"name":"ghcr.io/example/b","tag":"v1"},
                {"reference":"ghcr.io/example/c:v2"}
            ]
        }));
        assert_eq!(
            parsed,
            vec![
                "ghcr.io/example/a:latest".to_string(),
                "ghcr.io/example/b:v1".to_string(),
                "ghcr.io/example/c:v2".to_string()
            ]
        );
    }

    #[test]
    fn derive_image_flavor_supports_split_suffixes() {
        assert_eq!(derive_image_flavor("unified-field", "20260312-cp"), "pion");
        assert_eq!(
            derive_image_flavor("unified-field", "sha-worker"),
            "legacy-worker"
        );
        assert_eq!(
            derive_image_flavor("unified-field", "sha-full"),
            "monolithic"
        );
        assert_eq!(
            derive_image_flavor("unified-field", "abc-all_apps_runtimes"),
            "monolithic"
        );
        assert_eq!(
            derive_image_flavor("unified-field", "abc-all_apps"),
            "legacy-worker"
        );
        assert_eq!(
            derive_image_flavor("unified-field", "abc-control_plane"),
            "pion"
        );
        assert_eq!(
            derive_image_flavor("unified-field", "sha-chronon-runtime"),
            "chronon-runtime"
        );
        assert_eq!(
            derive_image_flavor("unified-field", "sha-photon-runtime"),
            "photon-runtime"
        );
        assert_eq!(
            derive_image_flavor("unified-field", "sha-boson-runtime"),
            "boson-runtime"
        );
        assert_eq!(derive_image_flavor("parton", "sha"), "unknown");
    }
}
