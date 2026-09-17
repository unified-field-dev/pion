//! Enrollment ticket CRUD: create, pin agent pubkey, revoke, and list for a setup-wizard session.

use anyhow::{anyhow, Context, Result};
use chrono::Utc;
use uuid::Uuid;
use valence::{Model, Valence};

use crate::generated::{
    PionAgentHostEnrollment, PionAgentHostEnrollmentMutable, PionAgentHostEnrollmentStatus,
};

use super::config::{normalize_agent_host_hint, DEFAULT_SETUP_WIZARD_SESSION_RECORD_ID};
use super::token::{build_full_token, hex_sha256};

/// Placeholder datetime for "unset" optional fields in generated models.
fn unset_dt() -> chrono::DateTime<Utc> {
    chrono::DateTime::<Utc>::UNIX_EPOCH
}

/// Creates a **pending** host enrollment ticket for strict agent bootstrap flows.
///
/// Persists a [`crate::generated::PionAgentHostEnrollment`] row and returns `(enrollment_id, wire_token)`.
/// The wire token (`ghe.<id>.<secret>`) is shown once to the operator; the agent presents it on
/// heartbeat until the row transitions to **claimed**.
///
/// # Side effects
///
/// Publishes [`crate::publish_setup_wizard_host_enrollment_updated`] (Photon) so Host Setup UIs can
/// refresh.
///
/// # Errors
///
/// Returns `Err` when Valence upsert fails or the row fails validation at construction time.
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
    setup_wizard_session_id: String,
    ttl_secs: i64,
) -> Result<(String, String)> {
    let session_id_for_photon = setup_wizard_session_id.clone();
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
        setup_wizard_session_id,
        now,
        expires_at,
        unset,
        unset,
        serde_json::json!({}),
        None,
    )
    .context("build host enrollment row")?;
    PionAgentHostEnrollment::upsert_used(&enrollment_id, row, valence, valence::use_!("When **Pion control plane** needs to persist work, we **save Pion Agent Host Enrollment** so the next step in that feature can continue with the latest values. People and services allowed for **Pion control plane** use this data for that workflow—not as a general export of unrelated personal fields."))
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
    let row = PionAgentHostEnrollment::get_used(enrollment_id, valence, valence::use_!("In **Pion control plane**, we **load Pion Agent Host Enrollment** so the application can decide what to do next in this workflow. The result is used by **Pion control plane** logic—not necessarily displayed on a page unless that feature’s UI shows it."))
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
    let m = PionAgentHostEnrollmentMutable::get(enrollment_id, valence)
        .await
        .with_context(|| format!("load mutable enrollment {enrollment_id} to set pubkey"))?
        .set_enrollment_pubkey(pk.to_string())
        .with_context(|| format!("set enrollment_pubkey on enrollment {enrollment_id}"))?;
    m.commit()
        .await
        .with_context(|| format!("commit pubkey update for enrollment {enrollment_id}"))?;
    Ok(())
}

/// Revoke a pending enrollment (wizard "remove" accidental host ticket).
///
/// # Errors
///
/// Returns `Err` when the enrollment is missing or Valence update fails.
pub async fn revoke_host_enrollment(valence: &Valence, enrollment_id: &str) -> Result<()> {
    let row = PionAgentHostEnrollment::get_used(enrollment_id, valence, valence::use_!("In **Pion control plane**, we **load Pion Agent Host Enrollment** so the application can decide what to do next in this workflow. The result is used by **Pion control plane** logic—not necessarily displayed on a page unless that feature’s UI shows it."))
        .await
        .with_context(|| format!("load enrollment {enrollment_id} to revoke"))?
        .ok_or_else(|| anyhow!("enrollment not found"))?;
    if *row.status() != PionAgentHostEnrollmentStatus::Pending {
        return Ok(());
    }
    let session_id_for_photon = row.setup_wizard_session_id().clone();
    let enrollment_id_owned = enrollment_id.to_string();
    let now = Utc::now();
    let m = PionAgentHostEnrollmentMutable::get(enrollment_id, valence)
        .await
        .with_context(|| format!("load mutable enrollment {enrollment_id} to revoke"))?;
    m.set_status(PionAgentHostEnrollmentStatus::Revoked)?
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

/// List enrollments created for a bootstrap / Host Setup session (pending + claimed + revoked).
///
/// Prefer [`list_enrollments_for_bootstrap_session`] in new code. The Valence field remains
/// `setup_wizard_session_id` until a schema migration wave.
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

/// Neutral alias for [`list_enrollments_for_session`].
///
/// # Errors
///
/// Propagates Valence query failures (same as [`list_enrollments_for_session`]).
pub async fn list_enrollments_for_bootstrap_session(
    valence: &Valence,
    bootstrap_session_id: &str,
) -> Result<Vec<PionAgentHostEnrollment>> {
    let mut rows = PionAgentHostEnrollment::query_used(valence, valence::use_!("In **Pion control plane**, we **list Pion Agent Host Enrollment** so the product can show or process the matching set for this workflow. Callers allowed for **Pion control plane** use the list; it is not a public dump of every field to anonymous visitors."))
        .await
        .context("query host enrollments for session list")?;
    let q = bootstrap_session_id.trim();
    rows.retain(|r| {
        let row_sid = r.setup_wizard_session_id().trim();
        row_sid == q || (q == DEFAULT_SETUP_WIZARD_SESSION_RECORD_ID && row_sid.is_empty())
    });
    rows.sort_by_key(|r| *r.created_at());
    Ok(rows)
}

#[cfg(test)]
mod tests {
    use super::DEFAULT_SETUP_WIZARD_SESSION_RECORD_ID;

    #[test]
    fn list_enrollments_for_session_filter_includes_blank_session_for_default_query() {
        let keep = |row_sid: &str, query: &str| {
            let row_sid = row_sid.trim();
            let query = query.trim();
            row_sid == query
                || (query == DEFAULT_SETUP_WIZARD_SESSION_RECORD_ID && row_sid.is_empty())
        };
        assert!(keep("default", "default"));
        assert!(keep("", "default"));
        assert!(!keep("", "other"));
        assert!(keep("other", "other"));
    }
}
