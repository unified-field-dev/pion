//! Typed enrollment verify / claim failures for matchable library APIs.

/// Failure reasons for enrollment ticket creation, token verification and new-node validation.
///
/// Returned by enrollment validation / verify helpers used during heartbeat ingest, and (inside
/// [`anyhow::Error`], via `downcast_ref`) by [`super::create_host_enrollment`]. Callers that
/// need to distinguish "expired" vs "bad token" vs "source IP mismatch" can match on this enum.
/// Storage failures are wrapped in [`EnrollmentError::Internal`]. Converts into [`anyhow::Error`]
/// via `?`.
#[derive(Debug, thiserror::Error)]
pub enum EnrollmentError {
    /// New-node heartbeat omitted the enrollment wire token.
    #[error("enrollment token required for new agent node")]
    TokenRequired,
    /// Wire token did not parse as `enrollment_id.secret`.
    #[error("invalid enrollment token format")]
    InvalidFormat,
    /// No enrollment row for the parsed id.
    #[error("unknown enrollment id")]
    UnknownId,
    /// Enrollment row is not in `Pending` status.
    #[error("enrollment is not pending")]
    NotPending,
    /// Enrollment `expires_at` is in the past.
    #[error("enrollment expired")]
    Expired,
    /// Peer IP does not match `expected_agent_host` when source-IP verify is enabled.
    #[error("enrollment source ip does not match expected host")]
    SourceIpMismatch,
    /// Presented token hash does not match the stored hash.
    #[error("enrollment token mismatch")]
    TokenMismatch,
    /// Heartbeat `cell_id` does not match the enrollment row's cell.
    #[error("heartbeat cell_id does not match enrollment")]
    CellIdMismatch,
    /// [`super::create_host_enrollment`] got a blank `bootstrap_session_id`.
    #[error("bootstrap session id is required to create a host enrollment")]
    BootstrapSessionIdRequired,
    /// Valence / persistence failure while loading or updating enrollment.
    #[error(transparent)]
    Internal(#[from] anyhow::Error),
}

impl From<valence::Error> for EnrollmentError {
    fn from(value: valence::Error) -> Self {
        Self::Internal(anyhow::Error::from(value))
    }
}
