//! Control-plane **handoff directives**: mint signed [`parton::AgentDirective`] rows, deliver on
//! heartbeat, and acknowledge when the agent reports the wire token digest match.

use crate::logging::HandoffDirectiveContext;
use anyhow::Result;
use base64::Engine;
use chrono::{DateTime, Utc};
use rand::RngCore;
use valence::{Model, Valence};

use crate::generated::{
    PionAgentHandoffDirective, PionAgentHandoffDirectiveMutable, PionAgentHandoffDirectiveStatus,
};
use parton::{handoff_reenroll_directive_signature_message_v1, AgentDirective};

fn epoch() -> DateTime<Utc> {
    DateTime::<Utc>::UNIX_EPOCH
}

/// SHA-256 digest of `data`, lower-case hex (matches Parton agent + wizard issuance).
#[must_use]
pub fn sha256_hex_bytes(data: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    let b = Sha256::digest(data);
    b.iter()
        .fold(String::with_capacity(b.len() * 2), |mut s, x| {
            use std::fmt::Write;
            let _ = write!(s, "{x:02x}");
            s
        })
}

fn applied_token_sha256_hex(token_wire: &str) -> Option<String> {
    let t = token_wire.trim();
    if t.is_empty() {
        return None;
    }
    if let Ok(bytes) = base64::engine::general_purpose::STANDARD.decode(t) {
        return Some(sha256_hex_bytes(&bytes));
    }
    Some(sha256_hex_bytes(t.as_bytes()))
}

/// Sign the canonical v1 payload for a pending handoff directive row.
///
/// Returns a non-empty signature string for well-formed inputs. Crate-root guide:
/// [Handoff directives](crate#handoff-directives).
///
/// # Examples
///
/// ```
/// use pion::sign_re_enroll_directive;
/// use chrono::Utc;
///
/// let seed = [7u8; 32];
/// let now = Utc::now();
/// let signature = sign_re_enroll_directive(
///     &seed,
///     "directive-1",
///     "node-1",
///     "https://cp.example:3000",
///     "authority-1",
///     "base64-verify-key",
///     "base64-ciphertext",
///     "sha256-hex",
///     now,
///     now + chrono::Duration::hours(1),
///     "authority-1",
///     now,
/// );
/// assert!(!signature.is_empty());
/// ```
#[must_use]
#[allow(clippy::too_many_arguments)] // v1 handoff signature message binds all wire fields
pub fn sign_re_enroll_directive(
    signing_seed: &[u8; 32],
    directive_id: &str,
    target_node_id: &str,
    new_cp_url: &str,
    new_authority_id: &str,
    new_authority_verify_key: &str,
    new_token_ciphertext_b64: &str,
    new_token_sha256_hex: &str,
    not_before: DateTime<Utc>,
    not_after: DateTime<Utc>,
    issued_by_authority_id: &str,
    issued_at: DateTime<Utc>,
) -> String {
    let sk = parton::directive_signing_key_from_seed(signing_seed);
    let msg = handoff_reenroll_directive_signature_message_v1(
        directive_id,
        target_node_id,
        new_cp_url,
        new_authority_id,
        new_authority_verify_key,
        new_token_ciphertext_b64,
        new_token_sha256_hex,
        &not_before,
        &not_after,
        issued_by_authority_id,
        &issued_at,
    );
    parton::directive_sign(&sk, msg.as_bytes())
}

/// Mint wire token bytes, sealed-box ciphertext, digest, signature, and upsert a `Pending` row.
///
/// # Errors
///
/// Propagates sealing, model construction, and Valence upsert failures.
#[allow(clippy::too_many_arguments)] // handoff mint binds authority, target, and overlap params
pub async fn upsert_pending_handoff_directive(
    valence: &Valence,
    directive_id: &str,
    target_node_id: &str,
    new_cp_url: &str,
    new_authority_id: &str,
    new_authority_verify_key: &str,
    enrollment_pubkey_b64: &str,
    overlap_hours: u32,
    issued_by_authority_id: &str,
    signing_seed: &[u8; 32],
) -> Result<()> {
    let mut wire = Vec::with_capacity(16 + 32);
    wire.extend_from_slice(uuid::Uuid::new_v4().as_bytes());
    let mut extra = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut extra);
    wire.extend_from_slice(&extra);

    let digest_hex = sha256_hex_bytes(&wire);
    let ciphertext = parton::seal_to_recipient(enrollment_pubkey_b64.trim(), &wire)?;
    let ct_b64 = base64::engine::general_purpose::STANDARD.encode(&ciphertext);

    let now = Utc::now();
    let not_after = now + chrono::Duration::hours(i64::from(overlap_hours.max(1)));
    let not_before = now - chrono::Duration::minutes(5);

    let sig = sign_re_enroll_directive(
        signing_seed,
        directive_id,
        target_node_id,
        new_cp_url,
        new_authority_id,
        new_authority_verify_key,
        &ct_b64,
        &digest_hex,
        not_before,
        not_after,
        issued_by_authority_id,
        now,
    );

    let row = PionAgentHandoffDirective::new(
        target_node_id.to_string(),
        new_cp_url.to_string(),
        new_authority_id.to_string(),
        new_authority_verify_key.to_string(),
        ct_b64,
        digest_hex,
        not_before,
        not_after,
        issued_by_authority_id.to_string(),
        now,
        sig,
        PionAgentHandoffDirectiveStatus::Pending,
        epoch(),
        epoch(),
        String::new(),
        epoch(),
    )?;

    PionAgentHandoffDirective::upsert_used(directive_id, row, valence, valence::use_!(r#"When **Pion control plane** needs to persist work, we **save Pion Agent Handoff Directive** so the next step in that feature can continue with the latest values. People and services allowed for **Pion control plane** use this data for that workflow—not as a general export of unrelated personal fields."#)).await?;
    Ok(())
}

/// When the agent reports `applied_directive_token`, mark matching `Delivered` directives acknowledged.
///
/// # Errors
///
/// Propagates Valence query or update failures.
pub async fn acknowledge_handoff_directives_for_heartbeat(
    report: &parton::NodeHeartbeatReport,
    valence: &Valence,
) -> Result<()> {
    let Some(tok) = report.applied_directive_token.as_deref() else {
        return Ok(());
    };
    let Some(digest) = applied_token_sha256_hex(tok) else {
        return Ok(());
    };

    let rows = PionAgentHandoffDirective::query_used(valence, valence::use_!(r#"In **Pion control plane**, we **list Pion Agent Handoff Directive** so the product can show or process the matching set for this workflow. Callers allowed for **Pion control plane** use the list; it is not a public dump of every field to anonymous visitors."#)).await?;
    let now = Utc::now();
    for row in rows {
        if row.target_node_id().trim() != report.node_id.trim() {
            continue;
        }
        if *row.status() != PionAgentHandoffDirectiveStatus::Delivered {
            continue;
        }
        if row.new_token_sha256_hex().trim() != digest {
            continue;
        }
        let Some(rid) = row.id() else {
            continue;
        };
        let id = valence::extract_id_from_record(rid).map_err(|e| anyhow::anyhow!("{e}"))?;
        PionAgentHandoffDirectiveMutable::get_used(id.as_str(), valence, valence::use_!(r#"In **Pion control plane**, we **update Pion Agent Handoff Directive Mutable** in place so saved changes apply on the next read. The same actors who can run **Pion control plane** use the updated values; this step is not a silent copy to an external marketing system."#))
            .await?
            .set_status(PionAgentHandoffDirectiveStatus::Acknowledged)?
            .set_acknowledged_at(now)?
            .commit()
            .await?;
    }
    Ok(())
}

fn row_to_agent_directive(row: &PionAgentHandoffDirective) -> Option<AgentDirective> {
    let id = row
        .id()
        .and_then(|r| valence::extract_id_from_record(r).ok())
        .filter(|s| !s.is_empty())?;
    Some(AgentDirective::ReEnroll {
        directive_id: id,
        new_cp_url: row.new_cp_url().clone(),
        new_authority_id: row.new_authority_id().clone(),
        new_authority_verify_key: row.new_authority_verify_key().clone(),
        new_token_ciphertext: row.new_token_ciphertext().clone(),
        not_before: *row.not_before(),
        not_after: *row.not_after(),
        issued_by_authority_id: row.issued_by_authority_id().clone(),
        issued_at: *row.issued_at(),
        signature: row.signature().clone(),
    })
}

async fn transition_pending_to_delivered(
    valence: &Valence,
    directive_table_id: &str,
    now: DateTime<Utc>,
) -> valence::Result<()> {
    PionAgentHandoffDirectiveMutable::get_used(directive_table_id, valence, valence::use_!(r#"In **Pion control plane**, we **update Pion Agent Handoff Directive Mutable** in place so saved changes apply on the next read. The same actors who can run **Pion control plane** use the updated values; this step is not a silent copy to an external marketing system."#))
        .await?
        .set_status(PionAgentHandoffDirectiveStatus::Delivered)?
        .set_delivered_at(now)?
        .commit()
        .await?;
    Ok(())
}

/// Query pending/delivered directives for `report.node_id`, transition `Pending → Delivered`, and
/// return the wire payload for the heartbeat JSON body.
///
/// # Errors
///
/// Propagates Valence query or update failures.
pub async fn collect_handoff_directives_for_heartbeat(
    report: &parton::NodeHeartbeatReport,
    valence: &Valence,
) -> Result<Vec<AgentDirective>> {
    let now = Utc::now();
    let mut rows: Vec<PionAgentHandoffDirective> = PionAgentHandoffDirective::query_used(valence, valence::use_!(r#"In **Pion control plane**, we **list Pion Agent Handoff Directive** so the product can show or process the matching set for this workflow. Callers allowed for **Pion control plane** use the list; it is not a public dump of every field to anonymous visitors."#))
        .await?
        .into_iter()
        .filter(|r| r.target_node_id().trim() == report.node_id.trim())
        .filter(|r| {
            matches!(
                *r.status(),
                PionAgentHandoffDirectiveStatus::Pending
                    | PionAgentHandoffDirectiveStatus::Delivered
            )
        })
        .filter(|r| *r.not_after() > now && *r.not_before() <= now)
        .collect();
    rows.sort_by_key(|r| *r.issued_at());

    let mut out = Vec::new();
    for row in rows {
        let id = row
            .id()
            .and_then(|r| valence::extract_id_from_record(r).ok())
            .unwrap_or_default();
        if id.is_empty() {
            HandoffDirectiveContext::from_node(&report.node_id).warn_missing_id();
            continue;
        }

        let current = if *row.status() == PionAgentHandoffDirectiveStatus::Pending {
            if let Err(e) = transition_pending_to_delivered(valence, id.as_str(), now).await {
                HandoffDirectiveContext::from_node(&report.node_id)
                    .warn_deliver_failed(&id, &e.to_string());
            }
            PionAgentHandoffDirective::get_used(id.as_str(), valence, valence::use_!(r#"In **Pion control plane**, we **load Pion Agent Handoff Directive** so the application can decide what to do next in this workflow. The result is used by **Pion control plane** logic—not necessarily displayed on a page unless that feature’s UI shows it."#))
                .await
                .map_err(|e| anyhow::anyhow!(e))?
                .unwrap_or(row)
        } else {
            row
        };

        if let Some(d) = row_to_agent_directive(&current) {
            out.push(d);
        }
    }

    Ok(out)
}
