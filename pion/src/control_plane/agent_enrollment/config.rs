//! Enrollment defaults, feature flags, and host-hint normalization.

/// Default bootstrap session record id (`"default"`).
///
/// Prefer [`DEFAULT_BOOTSTRAP_SESSION_RECORD_ID`]. Empty `setup_wizard_session_id` rows (a stored
/// field name kept for compatibility) still match this id so enrollment lists keep working after
/// upgrades.
pub const DEFAULT_SETUP_WIZARD_SESSION_RECORD_ID: &str = "default";

/// Neutral alias for [`DEFAULT_SETUP_WIZARD_SESSION_RECORD_ID`] (bootstrap session id).
pub const DEFAULT_BOOTSTRAP_SESSION_RECORD_ID: &str = DEFAULT_SETUP_WIZARD_SESSION_RECORD_ID;

/// True when the operator has explicitly opted into running without a configured shared token.
///
/// Duplicated from [`crate::runtime::insecure_mode_allowed`] (gated behind the `runtime` feature;
/// see that function for the canonical doc) so enrollment config helpers stay available whenever
/// `control_plane` is compiled under `runtime`.
fn insecure_mode_allowed() -> bool {
    std::env::var("PION_ALLOW_INSECURE").is_ok_and(|v| {
        matches!(
            v.trim().to_ascii_lowercase().as_str(),
            "1" | "true" | "yes" | "on"
        )
    })
}

/// When true, heartbeats that would create a **new** node must present a valid enrollment token.
///
/// **Default is `true` (secure).** `PARTON_ENROLLMENT_STRICT_NEW_NODES=0` (or `false` / `no` /
/// `off`) only relaxes this **when `PION_ALLOW_INSECURE` is also set** (`insecure_mode_allowed`)
/// — a lone `PARTON_ENROLLMENT_STRICT_NEW_NODES=0` with no insecure opt-in is ignored and
/// enrollment stays strict. This prevents a deployment from accidentally disabling new-node
/// enrollment checks via a single misconfigured/leaked env var without also explicitly opting
/// into the broader "local experiment" insecure mode.
pub fn enrollment_strict_new_nodes() -> bool {
    let lax_requested = match std::env::var("PARTON_ENROLLMENT_STRICT_NEW_NODES") {
        Ok(v) => {
            let v = v.trim().to_ascii_lowercase();
            matches!(v.as_str(), "0" | "false" | "no" | "off")
        }
        Err(_) => false,
    };
    !(lax_requested && insecure_mode_allowed())
}

/// Require connecting peer IP to match enrollment `expected_agent_host` (host part only).
///
/// **Default is `true` (secure).** Set `PARTON_ENROLLMENT_VERIFY_SOURCE_IP=0` (or `false` / `no` /
/// `off`) only for NAT / port-forwarded local setups where the enrolling agent's observed peer IP
/// legitimately does not match the host hint recorded on the enrollment ticket (see
/// `SECURITY.md` for the opt-out tradeoffs). Note the verify path already treats an empty
/// `expected_agent_host` as "no host recorded, skip the check" regardless of this flag.
pub(super) fn enrollment_verify_source_ip() -> bool {
    match std::env::var("PARTON_ENROLLMENT_VERIFY_SOURCE_IP") {
        Ok(v) => {
            let v = v.trim().to_ascii_lowercase();
            !matches!(v.as_str(), "0" | "false" | "no" | "off")
        }
        Err(_) => true,
    }
}

/// Normalize host input: trim, lowercase host part, default port 3000 for display hints only.
pub fn normalize_agent_host_hint(raw: &str) -> String {
    raw.trim().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serial_test::serial;

    #[test]
    #[serial]
    fn enrollment_verify_source_ip_defaults_to_true() {
        std::env::remove_var("PARTON_ENROLLMENT_VERIFY_SOURCE_IP");
        assert!(enrollment_verify_source_ip());
    }

    #[test]
    #[serial]
    fn enrollment_verify_source_ip_accepts_common_falsy_opt_out_spellings() {
        for v in ["0", "false", "FALSE", "no", "off", " Off "] {
            std::env::set_var("PARTON_ENROLLMENT_VERIFY_SOURCE_IP", v);
            assert!(
                !enrollment_verify_source_ip(),
                "expected {v:?} to opt out of source-ip verification"
            );
        }
        std::env::remove_var("PARTON_ENROLLMENT_VERIFY_SOURCE_IP");
    }

    #[test]
    #[serial]
    fn enrollment_verify_source_ip_stays_enabled_for_truthy_or_unrecognized_values() {
        for v in ["1", "true", "yes", "on", "garbage"] {
            std::env::set_var("PARTON_ENROLLMENT_VERIFY_SOURCE_IP", v);
            assert!(
                enrollment_verify_source_ip(),
                "expected {v:?} to keep source-ip verification enabled"
            );
        }
        std::env::remove_var("PARTON_ENROLLMENT_VERIFY_SOURCE_IP");
    }

    fn clear_strict_enrollment_env() {
        std::env::remove_var("PARTON_ENROLLMENT_STRICT_NEW_NODES");
        std::env::remove_var("PION_ALLOW_INSECURE");
    }

    #[test]
    #[serial]
    fn enrollment_strict_new_nodes_defaults_to_strict() {
        clear_strict_enrollment_env();
        assert!(enrollment_strict_new_nodes());
        clear_strict_enrollment_env();
    }

    /// `PARTON_ENROLLMENT_STRICT_NEW_NODES=0` alone (no `PION_ALLOW_INSECURE`) must be
    /// ignored — enrollment stays strict.
    #[test]
    #[serial]
    fn enrollment_strict_new_nodes_ignores_lax_opt_out_without_insecure_mode() {
        clear_strict_enrollment_env();
        std::env::set_var("PARTON_ENROLLMENT_STRICT_NEW_NODES", "0");
        assert!(
            enrollment_strict_new_nodes(),
            "lax enrollment must require PION_ALLOW_INSECURE=1"
        );
        clear_strict_enrollment_env();
    }

    /// `PARTON_ENROLLMENT_STRICT_NEW_NODES=0` is only honored once `PION_ALLOW_INSECURE=1`
    /// is also set.
    #[test]
    #[serial]
    fn enrollment_strict_new_nodes_relaxes_when_insecure_mode_also_set() {
        clear_strict_enrollment_env();
        std::env::set_var("PARTON_ENROLLMENT_STRICT_NEW_NODES", "0");
        std::env::set_var("PION_ALLOW_INSECURE", "1");
        assert!(!enrollment_strict_new_nodes());
        clear_strict_enrollment_env();
    }

    #[test]
    #[serial]
    fn enrollment_strict_new_nodes_stays_strict_when_only_insecure_mode_set() {
        clear_strict_enrollment_env();
        std::env::set_var("PION_ALLOW_INSECURE", "1");
        assert!(
            enrollment_strict_new_nodes(),
            "PION_ALLOW_INSECURE alone must not relax enrollment strictness"
        );
        clear_strict_enrollment_env();
    }
}
