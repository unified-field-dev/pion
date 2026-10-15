//! Enrollment ticket CRUD: create, pin agent pubkey, revoke, and list for a bootstrap session.

use anyhow::{anyhow, Context, Result};
use chrono::Utc;
use uuid::Uuid;
use valence::{Model, Valence};

use crate::generated::{PionAgentHostEnrollment, PionAgentHostEnrollmentStatus};

use super::config::{normalize_agent_host_hint, DEFAULT_SETUP_WIZARD_SESSION_RECORD_ID};
use super::error::EnrollmentError;
use super::token::{build_full_token, hex_sha256};

/// Placeholder datetime for "unset" optional fields in generated models.
fn unset_dt() -> chrono::DateTime<Utc> {
    chrono::DateTime::<Utc>::UNIX_EPOCH
}

/// Creates a **pending** host enrollment ticket for strict agent bootstrap flows.
///
/// Persists a [`crate::generated::PionAgentHostEnrollment`] row and returns `(enrollment_id, wire_token)`.
/// `bootstrap_session_id` groups the tickets of one bootstrap run so
/// [`list_enrollments_for_bootstrap_session`] can find them. It is stored in the row's
/// `setup_wizard_session_id` column.
/// The wire token (`ghe.<id>.<secret>`) is shown once to the operator; the agent presents it on
/// heartbeat until the row transitions to **claimed**.
///
/// # Side effects
///
/// Publishes [`crate::publish_setup_wizard_host_enrollment_updated`] (Photon) so enrollment UIs can
/// refresh.
///
/// # Errors
///
/// Returns `Err` when Valence upsert fails or the row fails validation at construction time.
/// A blank `bootstrap_session_id` is rejected before any write with an error that downcasts to
/// [`super::EnrollmentError::BootstrapSessionIdRequired`].
///
/// # Examples
///
/// ```no_run
/// # async fn demo(valence: &valence::Valence) -> anyhow::Result<()> {
/// use pion::create_host_enrollment;
/// let (enrollment_id, wire_token) = create_host_enrollment(
///     valence,
///     "10.0.0.1".into(),
///     "local-default".into(),
///     "default".into(),
///     3600,
/// ).await?;
/// assert!(!enrollment_id.is_empty());
/// assert!(wire_token.starts_with("ghe."));
/// # Ok(())
/// # }
/// ```
pub async fn create_host_enrollment(
    valence: &Valence,
    expected_agent_host: String,
    cell_id: String,
    bootstrap_session_id: String,
    ttl_secs: i64,
) -> Result<(String, String)> {
    let bootstrap_session_id = bootstrap_session_id.trim().to_string();
    if bootstrap_session_id.is_empty() {
        return Err(EnrollmentError::BootstrapSessionIdRequired.into());
    }
    let session_id_for_photon = bootstrap_session_id.clone();
    let enrollment_id = Uuid::new_v4().to_string();
    let secret: String = Uuid::new_v4().simple().to_string();
    let wire = build_full_token(&enrollment_id, &secret);
    let token_hash = hex_sha256(&wire);
    let now = Utc::now();
    let expires_at = now + chrono::Duration::seconds(ttl_secs.max(60));
    let unset = unset_dt();
    let row = PionAgentHostEnrollment::new(
        token_hash,
        normalize_agent_host_hint(&expected_agent_host),
        cell_id,
        PionAgentHostEnrollmentStatus::Pending,
        String::new(),
        bootstrap_session_id,
        now,
        expires_at,
        unset,
        unset,
        serde_json::json!({}),
        None,
    )
    .context("build host enrollment row")?;
    PionAgentHostEnrollment::upsert(&enrollment_id, row, valence, valence::use_!("When **Pion control plane** needs to persist work, we **save Pion Agent Host Enrollment** so the next step in that feature can continue with the latest values. People and services allowed for **Pion control plane** use this data for that workflow—not as a general export of unrelated personal fields."))
        .await
        .with_context(|| format!("upsert host enrollment {enrollment_id}"))?;
    tracing::info!(
        target: "security.enroll",
        enrollment_id = %enrollment_id,
        expected_agent_host = %normalize_agent_host_hint(&expected_agent_host),
        "host enrollment ticket created"
    );
    crate::publish_setup_wizard_host_enrollment_updated(
        session_id_for_photon,
        enrollment_id.clone(),
        "created",
    )
    .await;
    Ok((enrollment_id, wire))
}

/// Persists the agent host's sealed-box public key on a pending enrollment row.
///
/// # Errors
///
/// Returns `Err` when the enrollment is missing, not pending, or Valence update fails.
pub async fn set_host_enrollment_pubkey(
    valence: &Valence,
    enrollment_id: &str,
    enrollment_pubkey_b64: &str,
) -> Result<()> {
    let row = PionAgentHostEnrollment::get(enrollment_id, valence, valence::use_!(r"During **host enrollment** in the Pion control plane, we **load the pending enrollment ticket** so we can attach the agent's sealed-box public key. Operators and control-plane services use this ticket; the agent does not read the Valence row itself."))
        .await
        .with_context(|| format!("load enrollment {enrollment_id} to set pubkey"))?
        .ok_or_else(|| anyhow!("enrollment {enrollment_id} not found"))?;
    if *row.status() != PionAgentHostEnrollmentStatus::Pending {
        anyhow::bail!("enrollment {enrollment_id} is not pending; cannot set enrollment_pubkey");
    }
    let pk = enrollment_pubkey_b64.trim();
    if pk.is_empty() {
        anyhow::bail!("enrollment_pubkey is empty");
    }
    let m = row
        .get_mutable(valence, valence::use_!(r"During **host enrollment** in the Pion control plane, we **write the agent's sealed-box public key** onto the pending enrollment ticket so later handoff can encrypt material to that host. Operators and control-plane services use the updated ticket."))
        .set_enrollment_pubkey(pk.to_string())
        .with_context(|| format!("set enrollment_pubkey on enrollment {enrollment_id}"))?;
    m.commit()
        .await
        .with_context(|| format!("commit pubkey update for enrollment {enrollment_id}"))?;
    Ok(())
}

/// Revoke a pending enrollment, for example a host ticket created by mistake.
///
/// # Errors
///
/// Returns `Err` when the enrollment is missing or Valence update fails.
pub async fn revoke_host_enrollment(valence: &Valence, enrollment_id: &str) -> Result<()> {
    let row = PionAgentHostEnrollment::get(enrollment_id, valence, valence::use_!(r"When an operator **revokes** a pending host enrollment ticket, we **load that enrollment** to confirm it is still pending before changing its status. Operators and enrollment consoles use this ticket state."))
        .await
        .with_context(|| format!("load enrollment {enrollment_id} to revoke"))?
        .ok_or_else(|| anyhow!("enrollment not found"))?;
    if *row.status() != PionAgentHostEnrollmentStatus::Pending {
        return Ok(());
    }
    let session_id_for_photon = row.setup_wizard_session_id().clone();
    let enrollment_id_owned = enrollment_id.to_string();
    let now = Utc::now();
    row.get_mutable(valence, valence::use_!(r"When an operator **revokes** a pending host enrollment ticket, we **mark that enrollment revoked** so its wire token can no longer be claimed. Operators and enrollment consoles see the revoked status on refresh."))
        .set_status(PionAgentHostEnrollmentStatus::Revoked)?
        .set_revoked_at(now)?
        .commit()
        .await
        .with_context(|| format!("commit revoke for enrollment {enrollment_id}"))?;
    tracing::info!(
        target: "security.enroll",
        enrollment_id = %enrollment_id_owned,
        "enrollment ticket revoked"
    );
    crate::publish_setup_wizard_host_enrollment_updated(
        session_id_for_photon,
        enrollment_id_owned,
        "revoked",
    )
    .await;
    Ok(())
}

/// List enrollments created for a bootstrap session (pending, claimed and revoked).
///
/// Prefer [`list_enrollments_for_bootstrap_session`] in new code. The Valence field remains
/// `setup_wizard_session_id` until a schema migration renames it.
///
/// # Errors
///
/// Propagates Valence query failures.
pub async fn list_enrollments_for_session(
    valence: &Valence,
    setup_wizard_session_id: &str,
) -> Result<Vec<PionAgentHostEnrollment>> {
    list_enrollments_for_bootstrap_session(valence, setup_wizard_session_id).await
}

/// List enrollments created for a bootstrap session (pending, claimed and revoked), oldest first.
///
/// A blank or unknown `bootstrap_session_id` returns an empty list. Rows stored with a blank
/// session id are listed under [`super::DEFAULT_BOOTSTRAP_SESSION_RECORD_ID`].
///
/// # Errors
///
/// Propagates Valence query failures (same as [`list_enrollments_for_session`]).
pub async fn list_enrollments_for_bootstrap_session(
    valence: &Valence,
    bootstrap_session_id: &str,
) -> Result<Vec<PionAgentHostEnrollment>> {
    let q = bootstrap_session_id.trim();
    if q.is_empty() {
        return Ok(Vec::new());
    }
    let mut rows = PionAgentHostEnrollment::query(valence, valence::use_!("In **Pion control plane**, we **list Pion Agent Host Enrollment** so the product can show or process the matching set for this workflow. Callers allowed for **Pion control plane** use the list; it is not a public dump of every field to anonymous visitors."))
        .await
        .context("query host enrollments for session list")?;
    rows.retain(|r| session_matches(r.setup_wizard_session_id(), q));
    rows.sort_by_key(|r| *r.created_at());
    Ok(rows)
}

/// Whether a row's stored session id belongs to the (trimmed, non-blank) query.
fn session_matches(row_sid: &str, query: &str) -> bool {
    let row_sid = row_sid.trim();
    row_sid == query || (query == DEFAULT_SETUP_WIZARD_SESSION_RECORD_ID && row_sid.is_empty())
}

#[cfg(test)]
mod tests {
    use super::session_matches;

    #[test]
    fn list_enrollments_for_session_filter_includes_blank_session_for_default_query() {
        assert!(session_matches("default", "default"));
        assert!(session_matches("", "default"));
        assert!(!session_matches("", "other"));
        assert!(session_matches("other", "other"));
        assert!(!session_matches("other", "default"));
    }
}
