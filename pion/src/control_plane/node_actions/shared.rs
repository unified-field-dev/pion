//! Shared helpers used across the node action submodules: string clipping, lease sentinels,
//! default-value constants, and soft-delete-tolerant reads.

use chrono::{Duration, Utc};
use valence::Error as ValenceError;
use valence::{Model, Valence};

use crate::generated::{PionControlPlaneNode, PionNodeActionCommand};

pub(super) const CLIP: usize = 16_384;
pub(super) const DEFAULT_MAX_ATTEMPTS: i64 = 3;
pub(super) const DEFAULT_PENDING_TIMEOUT_SECS: i64 = 300;
pub(super) const DEFAULT_LEASE_SECS: u64 = 120;

pub(super) fn clip(s: &str, max: usize) -> String {
    if s.len() <= max {
        s.to_string()
    } else {
        format!("{}…(truncated)", &s[..max.saturating_sub(16)])
    }
}

pub(super) fn lease_sentinel() -> chrono::DateTime<Utc> {
    chrono::DateTime::<Utc>::UNIX_EPOCH
}

pub(super) fn duration_from_u64_secs(secs: u64) -> Duration {
    Duration::seconds(i64::try_from(secs).unwrap_or(i64::MAX))
}

/// True when the node row was updated recently (heartbeat ingest bumps `updated_at`).
pub(crate) async fn is_node_recently_heartbeating(
    valence: &Valence,
    node_id: &str,
    within_secs: i64,
) -> bool {
    let Ok(Some(node)) = PionControlPlaneNode::get_used(node_id, valence, valence::use_!("get PionControlPlaneNode in control_plane/node_actions/shared.rs; Valence persistence for this feature path; typed store; visible to session actor / service path.")).await else {
        return false;
    };
    let cutoff = Utc::now() - Duration::seconds(within_secs);
    *node.updated_at() > cutoff
}

/// Like [`PionNodeActionCommand::get`], but if the row is in Valence’s queued-deletion state, treat
/// it as missing so idempotent enqueue and read paths can proceed while the async cascade runs.
pub(super) async fn get_node_action_treating_deletion_as_missing(
    command_id: &str,
    valence: &Valence,
) -> anyhow::Result<Option<PionNodeActionCommand>> {
    use anyhow::Context;
    match PionNodeActionCommand::get_used(command_id, valence, valence::use_!("get PionNodeActionCommand in control_plane/node_actions/shared.rs; Valence persistence for this feature path; typed store; visible to session actor / service path.")).await {
        Ok(m) => Ok(m),
        Err(ValenceError::PendingDeletion(_)) => Ok(None),
        Err(e) => Err(anyhow::Error::from(e))
            .with_context(|| format!("load node action command {command_id}")),
    }
}

#[cfg(test)]
mod pending_deletion_as_missing_tests {
    use valence::Error as ValenceError;

    /// Mirror of [`get_node_action_treating_deletion_as_missing`]: pending-deletion is treated as absent.
    fn map_get<T>(r: Result<Option<T>, ValenceError>) -> Result<Option<T>, ValenceError> {
        match r {
            Ok(m) => Ok(m),
            Err(ValenceError::PendingDeletion(_)) => Ok(None),
            Err(e) => Err(e),
        }
    }

    #[test]
    fn pending_deletion_becomes_none() {
        let r: Result<Option<i32>, _> = map_get(Err(ValenceError::PendingDeletion("x".into())));
        assert!(matches!(r, Ok(None)));
    }

    #[test]
    fn other_errors_passthrough() {
        let e = ValenceError::NotFound("n".into());
        let r: Result<Option<i32>, _> = map_get(Err(e));
        assert!(r.is_err());
    }
}
