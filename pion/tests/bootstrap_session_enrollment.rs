//! Host enrollments keyed by a neutral bootstrap session id (create, then list by that id).

#![cfg(feature = "runtime")]
// Integration tests use `.expect()` / `.unwrap()` for brevity on fixtures.
#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;

use pion::generated::PionAgentHostEnrollment;
use pion::{create_host_enrollment, list_enrollments_for_bootstrap_session, EnrollmentError};

#[tokio::test]
async fn enrollment_created_for_session_is_listed_for_that_session() -> anyhow::Result<()> {
    let v = common::test_valence("pion_bootstrap_session_enrollment").await;

    let (enrollment_id, _token) = create_host_enrollment(
        &v,
        String::new(),
        "cell-a".into(),
        "boot-session-1".into(),
        600,
    )
    .await?;
    let (_other_id, _) = create_host_enrollment(
        &v,
        String::new(),
        "cell-a".into(),
        "boot-session-2".into(),
        600,
    )
    .await?;

    let rows = list_enrollments_for_bootstrap_session(&v, "boot-session-1").await?;
    assert_eq!(rows.len(), 1, "only the session-1 ticket is listed");
    let row_id = valence::extract_id_from_record(rows[0].id().expect("row id"))?;
    assert_eq!(row_id, enrollment_id);
    assert_eq!(rows[0].setup_wizard_session_id(), "boot-session-1");

    // Surrounding whitespace on the query is ignored.
    let padded = list_enrollments_for_bootstrap_session(&v, "  boot-session-1 ").await?;
    assert_eq!(padded.len(), 1);
    Ok(())
}

#[tokio::test]
async fn unknown_or_blank_session_lists_nothing() -> anyhow::Result<()> {
    let v = common::test_valence("pion_bootstrap_session_enrollment_sad").await;
    create_host_enrollment(
        &v,
        String::new(),
        "cell-a".into(),
        "boot-session-1".into(),
        600,
    )
    .await?;

    assert!(
        list_enrollments_for_bootstrap_session(&v, "no-such-session")
            .await?
            .is_empty()
    );
    assert!(list_enrollments_for_bootstrap_session(&v, "")
        .await?
        .is_empty());
    assert!(list_enrollments_for_bootstrap_session(&v, "   ")
        .await?
        .is_empty());
    Ok(())
}

#[tokio::test]
async fn blank_session_id_is_rejected_before_any_write() -> anyhow::Result<()> {
    let v = common::test_valence("pion_bootstrap_session_enrollment_blank").await;

    for blank in ["", "  \t"] {
        let err = create_host_enrollment(&v, String::new(), "cell-a".into(), blank.into(), 600)
            .await
            .expect_err("blank bootstrap session id must be rejected");
        assert!(
            matches!(
                err.downcast_ref::<EnrollmentError>(),
                Some(EnrollmentError::BootstrapSessionIdRequired)
            ),
            "unexpected error: {err:#}"
        );
    }

    let rows = PionAgentHostEnrollment::query(
        &v,
        valence::use_!(
            r"**Test:** the Pion bootstrap-session enrollment suite **counts every host enrollment ticket** to prove a rejected create wrote nothing."
        ),
    )
    .await?;
    assert!(rows.is_empty(), "no enrollment row may be written");
    Ok(())
}
