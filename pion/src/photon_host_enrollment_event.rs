//! Photon topic for host enrollment changes (for a live enrollment UI).
//!
//! # Topic string stability
//!
//! The `#[photon::topic(name = "...")]` string remains `gluon.setup_wizard.host_enrollment.updated`
//! for subscriber compatibility. New bootstrap progress topics should use the
//! `fleet.bootstrap.*` prefix; this crate keeps the legacy name so existing subscribers keep
//! working. Valence model: [`crate::generated::PionAgentHostEnrollment`].

/// Payload published when a host enrollment row is created, claimed, or revoked.
#[cfg(feature = "runtime")]
#[photon::topic(name = "gluon.setup_wizard.host_enrollment.updated")]
pub struct PionSetupWizardHostEnrollmentUpdated {
    /// Bootstrap session record id (often [`crate::DEFAULT_SETUP_WIZARD_SESSION_RECORD_ID`]).
    pub setup_wizard_session_id: String,
    /// Enrollment row id.
    pub enrollment_id: String,
    /// Wire status string (`pending`, `claimed`, `revoked`, …).
    pub status: String,
}

/// Publishes the `gluon.setup_wizard.host_enrollment.updated` Photon payload (best-effort; errors
/// are discarded).
#[cfg(feature = "runtime")]
pub async fn publish_setup_wizard_host_enrollment_updated(
    setup_wizard_session_id: impl Into<String>,
    enrollment_id: impl Into<String>,
    status: impl Into<String>,
) {
    let setup_wizard_session_id = setup_wizard_session_id.into();
    let enrollment_id = enrollment_id.into();
    let status = status.into();
    let _ = PionSetupWizardHostEnrollmentUpdated {
        setup_wizard_session_id,
        enrollment_id,
        status,
    }
    .publish()
    .await;
}
