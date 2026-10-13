//! Authorization, node-binding, and rate-limit gate helpers for the Parton agent HTTP surface.
//!
//! # Security (implemented)
//!
//! - **Shared token** (`PARTON_SHARED_TOKEN` / `x-parton-token`): fail closed unless
//!   [`insecure_mode_allowed`] (testing only).
//! - **SPIFFE JWT-SVID** (`PION_AUTH_MODE=dual|spiffe`): verify `Authorization: Bearer`
//!   against trust-domain + `spiffe://{trust}/parton/node/{node_id}` (see [`super::spiffe`]).
//! - **Node binding**: `x-parton-node-id` must match the request `node_id`.
//! - **Rate limit**: per-key token bucket (`PION_AGENT_RATE_LIMIT_PER_MIN`, default 120).
//!
//! Vulnerability reporting: repository root `SECURITY.md`.

use axum::http::{HeaderMap, StatusCode};

use super::rate_limit;
use super::spiffe::{self, AuthMode};

/// True when the operator has explicitly opted into running without a configured shared token.
///
/// Set `PION_ALLOW_INSECURE=1` (or `true` / `yes` / `on`) only for local experiments; production
/// deployments must configure `PARTON_SHARED_TOKEN` so agent requests fail closed.
pub fn insecure_mode_allowed() -> bool {
    std::env::var("PION_ALLOW_INSECURE").is_ok_and(|v| {
        matches!(
            v.trim().to_ascii_lowercase().as_str(),
            "1" | "true" | "yes" | "on"
        )
    })
}

/// Constant-time string comparison (same approach as enrollment-token comparison during
/// heartbeat ingest) so shared-token comparisons here don't leak timing information.
fn constant_time_eq(a: &str, b: &str) -> bool {
    use subtle::ConstantTimeEq;
    let a = a.as_bytes();
    let b = b.as_bytes();
    if a.len() != b.len() {
        return false;
    }
    a.ct_eq(b).into()
}

fn shared_token_ok(headers: &HeaderMap) -> Result<(), (StatusCode, String)> {
    let configured = std::env::var("PARTON_SHARED_TOKEN")
        .ok()
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty());
    let Some(expected) = configured else {
        if insecure_mode_allowed() {
            return Ok(());
        }
        tracing::error!(
            target: "security.authz",
            check = "shared_token",
            "agent request rejected: PARTON_SHARED_TOKEN is not configured"
        );
        return Err((
            StatusCode::INTERNAL_SERVER_ERROR,
            "PARTON_SHARED_TOKEN is not configured; refusing agent requests (set \
             PION_ALLOW_INSECURE=1 to run without a shared token, local dev only)"
                .to_string(),
        ));
    };

    let provided = headers
        .get("x-parton-token")
        .and_then(|value| value.to_str().ok())
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .map(ToOwned::to_owned);

    match provided {
        Some(token) if constant_time_eq(&token, &expected) => Ok(()),
        _ => {
            tracing::warn!(
                target: "security.authz",
                check = "shared_token",
                "agent request rejected: missing or mismatched x-parton-token"
            );
            Err((
                StatusCode::UNAUTHORIZED,
                "Unauthorized agent request".to_string(),
            ))
        }
    }
}

fn spiffe_ok(headers: &HeaderMap, node_id: &str) -> Result<(), (StatusCode, String)> {
    let Some(jwt) = spiffe::bearer_jwt_from_headers(headers) else {
        tracing::warn!(
            target: "security.authz",
            check = "spiffe_jwt",
            "agent request rejected: missing Authorization Bearer JWT-SVID"
        );
        return Err((
            StatusCode::UNAUTHORIZED,
            "Unauthorized agent request (missing SPIFFE JWT-SVID)".to_string(),
        ));
    };
    spiffe::verify_jwt_svid(&jwt, node_id)
}

/// Authorizes an inbound Parton agent request per `PION_AUTH_MODE`.
///
/// - `shared_token` (default): `PARTON_SHARED_TOKEN` / `x-parton-token` (fails closed unless
///   [`insecure_mode_allowed`]).
/// - `dual`: accept either a valid shared token **or** a valid JWT-SVID.
/// - `spiffe`: require a valid JWT-SVID (shared token ignored); the insecure local bypass still applies
///   when no SPIFFE material is presented and `PION_ALLOW_INSECURE=1`.
pub(super) fn authorize_agent_request(
    headers: &HeaderMap,
    node_id: &str,
) -> Result<(), (StatusCode, String)> {
    match AuthMode::from_env() {
        AuthMode::SharedToken => shared_token_ok(headers),
        AuthMode::Dual => {
            if shared_token_ok(headers).is_ok() {
                return Ok(());
            }
            spiffe_ok(headers, node_id)
        }
        AuthMode::Spiffe => {
            if insecure_mode_allowed() && spiffe::bearer_jwt_from_headers(headers).is_none() {
                // Local bypass: allow unsigned local traffic when explicitly opted in.
                return Ok(());
            }
            spiffe_ok(headers, node_id)
        }
    }
}

/// Requires the `x-parton-node-id` header to be present and equal to `claimed_node_id`.
///
/// The fleet-wide `PARTON_SHARED_TOKEN` authenticates *an* agent, not *which node* it is; without
/// this check, any holder of the shared token could claim, extend, or report results for a
/// different node's `node_id`. Agents must send `x-parton-node-id` matching the request body's
/// `node_id` on every claim / extend-lease / result / heartbeat call. Skipped when
/// [`insecure_mode_allowed`] (local dev only).
pub(super) fn authorize_node_binding(
    headers: &HeaderMap,
    claimed_node_id: &str,
) -> Result<(), (StatusCode, String)> {
    if insecure_mode_allowed() {
        return Ok(());
    }
    let expected = claimed_node_id.trim();
    let provided = headers
        .get("x-parton-node-id")
        .and_then(|value| value.to_str().ok())
        .map(str::trim)
        .filter(|v| !v.is_empty());
    match provided {
        Some(node_id) if node_id == expected => Ok(()),
        _ => {
            tracing::warn!(
                target: "security.authz",
                check = "node_binding",
                claimed_node_id = %expected,
                "agent request rejected: x-parton-node-id header missing or mismatched"
            );
            Err((
                StatusCode::FORBIDDEN,
                "x-parton-node-id header missing or does not match request node_id".to_string(),
            ))
        }
    }
}

/// Rate-limit key combining peer IP with `node_id` when available.
///
/// Keying on the pair (rather than IP alone) means one noisy/misbehaving node can't exhaust the
/// budget of every other agent sharing a NAT gateway, while still bounding an unauthenticated
/// flood from a single source IP with no claimed `node_id`.
fn rate_limit_key(peer_ip: &str, node_id: &str) -> String {
    let node_id = node_id.trim();
    if node_id.is_empty() {
        peer_ip.to_string()
    } else {
        format!("{peer_ip}|{node_id}")
    }
}

/// Enforces the `PION_AGENT_RATE_LIMIT_PER_MIN` token bucket for `peer_ip` (+ `node_id`).
///
/// Returns `429 Too Many Requests` when the caller has exceeded its budget. Checked before
/// authorization so a flood can't be used to brute-force the shared token or node binding
/// indefinitely without ever tripping a limit.
pub(super) fn check_agent_rate_limit(
    peer_ip: &str,
    node_id: &str,
) -> Result<(), (StatusCode, String)> {
    let key = rate_limit_key(peer_ip, node_id);
    if rate_limit::allow(&key, rate_limit::limit_per_min()) {
        Ok(())
    } else {
        tracing::warn!(
            target: "security.rate_limit",
            peer_ip = %peer_ip,
            node_id = %node_id,
            "agent request rate limit exceeded"
        );
        Err((
            StatusCode::TOO_MANY_REQUESTS,
            "rate limit exceeded; slow down".to_string(),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::{
        authorize_agent_request, authorize_node_binding, check_agent_rate_limit,
        insecure_mode_allowed, rate_limit_key,
    };
    use axum::http::{HeaderMap, HeaderValue, StatusCode};
    use serial_test::serial;

    /// Clears env vars this module reads so tests don't leak state into each other.
    fn clear_auth_env() {
        std::env::remove_var("PARTON_SHARED_TOKEN");
        std::env::remove_var("PION_ALLOW_INSECURE");
        std::env::remove_var("PION_AUTH_MODE");
        std::env::remove_var("PION_SPIFFE_TRUST_DOMAIN");
        std::env::remove_var("PION_SPIFFE_JWT_HMAC_SECRET");
        std::env::remove_var("PION_SPIFFE_JWT_KEY_PATH");
        std::env::remove_var("PION_SPIFFE_AUDIENCE");
    }

    fn hs256_svid_for(node_id: &str) -> String {
        let claims = serde_json::json!({
            "sub": format!("spiffe://example.org/parton/node/{node_id}"),
            "aud": "pion",
            "exp": (chrono::Utc::now() + chrono::Duration::hours(1)).timestamp(),
        });
        jsonwebtoken::encode(
            &jsonwebtoken::Header::new(jsonwebtoken::Algorithm::HS256),
            &claims,
            &jsonwebtoken::EncodingKey::from_secret(b"test-hmac-secret"),
        )
        .expect("encode")
    }

    fn configure_hs256_spiffe() {
        std::env::set_var("PION_SPIFFE_TRUST_DOMAIN", "example.org");
        std::env::set_var("PION_SPIFFE_JWT_HMAC_SECRET", "test-hmac-secret");
        std::env::set_var("PION_SPIFFE_AUDIENCE", "pion");
    }

    fn bearer_headers(jwt: &str) -> HeaderMap {
        let mut headers = HeaderMap::new();
        headers.insert(
            axum::http::header::AUTHORIZATION,
            HeaderValue::from_str(&format!("Bearer {jwt}")).expect("hdr"),
        );
        headers
    }

    #[test]
    #[serial]
    fn authorize_rejects_when_token_not_configured_and_not_insecure() {
        clear_auth_env();
        let headers = HeaderMap::new();
        let result = authorize_agent_request(&headers, "node-a");
        assert_eq!(
            result.map_err(|(status, _)| status),
            Err(StatusCode::INTERNAL_SERVER_ERROR)
        );
        clear_auth_env();
    }

    #[test]
    #[serial]
    fn authorize_allows_when_token_not_configured_and_insecure_opt_in() {
        clear_auth_env();
        std::env::set_var("PION_ALLOW_INSECURE", "1");
        let headers = HeaderMap::new();
        assert!(authorize_agent_request(&headers, "node-a").is_ok());
        clear_auth_env();
    }

    #[test]
    #[serial]
    fn insecure_mode_allowed_accepts_common_truthy_spellings() {
        clear_auth_env();
        for v in ["1", "true", "TRUE", "yes", "on", " On "] {
            std::env::set_var("PION_ALLOW_INSECURE", v);
            assert!(insecure_mode_allowed(), "expected {v:?} to be truthy");
        }
        for v in ["0", "false", "no", "off", ""] {
            std::env::set_var("PION_ALLOW_INSECURE", v);
            assert!(!insecure_mode_allowed(), "expected {v:?} to be falsy");
        }
        clear_auth_env();
    }

    #[test]
    #[serial]
    fn authorize_rejects_missing_token_when_configured() {
        clear_auth_env();
        std::env::set_var("PARTON_SHARED_TOKEN", "top-secret");
        let headers = HeaderMap::new();
        let result = authorize_agent_request(&headers, "node-a");
        assert_eq!(
            result,
            Err((
                StatusCode::UNAUTHORIZED,
                "Unauthorized agent request".to_string()
            ))
        );
        clear_auth_env();
    }

    #[test]
    #[serial]
    fn authorize_rejects_mismatched_token_when_configured() {
        clear_auth_env();
        std::env::set_var("PARTON_SHARED_TOKEN", "top-secret");
        let mut headers = HeaderMap::new();
        headers.insert("x-parton-token", HeaderValue::from_static("wrong-token"));
        let result = authorize_agent_request(&headers, "node-a");
        assert_eq!(
            result,
            Err((
                StatusCode::UNAUTHORIZED,
                "Unauthorized agent request".to_string()
            ))
        );
        clear_auth_env();
    }

    #[test]
    #[serial]
    fn authorize_accepts_matching_token_when_configured() {
        clear_auth_env();
        let token = "top-secret";
        std::env::set_var("PARTON_SHARED_TOKEN", token);
        let mut headers = HeaderMap::new();
        headers.insert(
            "x-parton-token",
            HeaderValue::from_str(token).expect("header value"),
        );
        assert!(authorize_agent_request(&headers, "node-a").is_ok());
        clear_auth_env();
    }

    #[test]
    #[serial]
    fn authorize_dual_accepts_spiffe_when_token_wrong() {
        clear_auth_env();
        std::env::set_var("PION_AUTH_MODE", "dual");
        std::env::set_var("PARTON_SHARED_TOKEN", "top-secret");
        configure_hs256_spiffe();
        let token = hs256_svid_for("node-a");
        let mut headers = bearer_headers(&token);
        headers.insert("x-parton-token", HeaderValue::from_static("wrong"));
        assert!(authorize_agent_request(&headers, "node-a").is_ok());
        clear_auth_env();
    }

    #[test]
    #[serial]
    fn authorize_spiffe_accepts_valid_jwt() {
        clear_auth_env();
        std::env::set_var("PION_AUTH_MODE", "spiffe");
        configure_hs256_spiffe();
        let headers = bearer_headers(&hs256_svid_for("node-a"));
        assert!(authorize_agent_request(&headers, "node-a").is_ok());
        clear_auth_env();
    }

    #[test]
    #[serial]
    fn authorize_spiffe_rejects_missing_bearer() {
        clear_auth_env();
        std::env::set_var("PION_AUTH_MODE", "spiffe");
        configure_hs256_spiffe();
        let headers = HeaderMap::new();
        let result = authorize_agent_request(&headers, "node-a");
        assert_eq!(
            result.map_err(|(status, _)| status),
            Err(StatusCode::UNAUTHORIZED)
        );
        clear_auth_env();
    }

    #[test]
    #[serial]
    fn authorize_spiffe_rejects_wrong_node() {
        clear_auth_env();
        std::env::set_var("PION_AUTH_MODE", "spiffe");
        configure_hs256_spiffe();
        let headers = bearer_headers(&hs256_svid_for("node-a"));
        let result = authorize_agent_request(&headers, "node-b");
        assert_eq!(
            result.map_err(|(status, _)| status),
            Err(StatusCode::FORBIDDEN)
        );
        clear_auth_env();
    }

    #[test]
    #[serial]
    fn authorize_spiffe_rejects_bad_signature() {
        clear_auth_env();
        std::env::set_var("PION_AUTH_MODE", "spiffe");
        configure_hs256_spiffe();
        let claims = serde_json::json!({
            "sub": "spiffe://example.org/parton/node/node-a",
            "aud": "pion",
            "exp": (chrono::Utc::now() + chrono::Duration::hours(1)).timestamp(),
        });
        let token = jsonwebtoken::encode(
            &jsonwebtoken::Header::new(jsonwebtoken::Algorithm::HS256),
            &claims,
            &jsonwebtoken::EncodingKey::from_secret(b"wrong-hmac-secret"),
        )
        .expect("encode");
        let headers = bearer_headers(&token);
        let result = authorize_agent_request(&headers, "node-a");
        assert_eq!(
            result.map_err(|(status, _)| status),
            Err(StatusCode::UNAUTHORIZED)
        );
        clear_auth_env();
    }

    #[test]
    #[serial]
    fn authorize_spiffe_insecure_allows_missing_bearer() {
        clear_auth_env();
        std::env::set_var("PION_AUTH_MODE", "spiffe");
        std::env::set_var("PION_ALLOW_INSECURE", "1");
        configure_hs256_spiffe();
        let headers = HeaderMap::new();
        assert!(authorize_agent_request(&headers, "node-a").is_ok());
        clear_auth_env();
    }

    #[test]
    #[serial]
    fn authorize_dual_accepts_good_token_without_jwt() {
        clear_auth_env();
        std::env::set_var("PION_AUTH_MODE", "dual");
        std::env::set_var("PARTON_SHARED_TOKEN", "top-secret");
        configure_hs256_spiffe();
        let mut headers = HeaderMap::new();
        headers.insert("x-parton-token", HeaderValue::from_static("top-secret"));
        assert!(authorize_agent_request(&headers, "node-a").is_ok());
        clear_auth_env();
    }

    #[test]
    #[serial]
    fn authorize_dual_rejects_bad_token_and_missing_jwt() {
        clear_auth_env();
        std::env::set_var("PION_AUTH_MODE", "dual");
        std::env::set_var("PARTON_SHARED_TOKEN", "top-secret");
        configure_hs256_spiffe();
        let mut headers = HeaderMap::new();
        headers.insert("x-parton-token", HeaderValue::from_static("wrong"));
        let result = authorize_agent_request(&headers, "node-a");
        assert_eq!(
            result.map_err(|(status, _)| status),
            Err(StatusCode::UNAUTHORIZED)
        );
        clear_auth_env();
    }

    #[test]
    #[serial]
    fn authorize_shared_token_rejects_jwt_only() {
        clear_auth_env();
        std::env::set_var("PION_AUTH_MODE", "shared_token");
        std::env::set_var("PARTON_SHARED_TOKEN", "top-secret");
        configure_hs256_spiffe();
        let headers = bearer_headers(&hs256_svid_for("node-a"));
        let result = authorize_agent_request(&headers, "node-a");
        assert_eq!(
            result.map_err(|(status, _)| status),
            Err(StatusCode::UNAUTHORIZED)
        );
        clear_auth_env();
    }

    #[test]
    #[serial]
    fn node_binding_rejects_missing_header_when_secure() {
        clear_auth_env();
        let headers = HeaderMap::new();
        let result = authorize_node_binding(&headers, "node-a");
        assert_eq!(
            result.map_err(|(status, _)| status),
            Err(StatusCode::FORBIDDEN)
        );
    }

    #[test]
    #[serial]
    fn node_binding_rejects_mismatched_header_when_secure() {
        clear_auth_env();
        let mut headers = HeaderMap::new();
        headers.insert("x-parton-node-id", HeaderValue::from_static("node-b"));
        let result = authorize_node_binding(&headers, "node-a");
        assert_eq!(
            result.map_err(|(status, _)| status),
            Err(StatusCode::FORBIDDEN)
        );
    }

    #[test]
    #[serial]
    fn node_binding_accepts_matching_header_when_secure() {
        clear_auth_env();
        let mut headers = HeaderMap::new();
        headers.insert("x-parton-node-id", HeaderValue::from_static("node-a"));
        assert!(authorize_node_binding(&headers, "node-a").is_ok());
    }

    #[test]
    fn rate_limit_key_combines_ip_and_node_id_when_present() {
        assert_eq!(rate_limit_key("1.2.3.4", "node-a"), "1.2.3.4|node-a");
    }

    #[test]
    fn rate_limit_key_falls_back_to_ip_alone_when_node_id_blank() {
        assert_eq!(rate_limit_key("1.2.3.4", ""), "1.2.3.4");
        assert_eq!(rate_limit_key("1.2.3.4", "   "), "1.2.3.4");
    }

    #[test]
    #[serial]
    fn check_agent_rate_limit_returns_429_once_budget_exhausted() {
        std::env::set_var("PION_AGENT_RATE_LIMIT_PER_MIN", "2");
        let peer_ip = "203.0.113.5"; // TEST-NET-3, unique per test to avoid cross-test bucket sharing
        let node_id = "rate-limit-test-node-exhausted";
        assert!(check_agent_rate_limit(peer_ip, node_id).is_ok());
        assert!(check_agent_rate_limit(peer_ip, node_id).is_ok());
        let result = check_agent_rate_limit(peer_ip, node_id);
        assert_eq!(
            result.map_err(|(status, _)| status),
            Err(StatusCode::TOO_MANY_REQUESTS)
        );
        std::env::remove_var("PION_AGENT_RATE_LIMIT_PER_MIN");
    }

    #[test]
    #[serial]
    fn check_agent_rate_limit_keys_are_independent_per_node() {
        std::env::set_var("PION_AGENT_RATE_LIMIT_PER_MIN", "1");
        let peer_ip = "203.0.113.6";
        assert!(check_agent_rate_limit(peer_ip, "node-independent-a").is_ok());
        assert!(check_agent_rate_limit(peer_ip, "node-independent-a").is_err());
        assert!(
            check_agent_rate_limit(peer_ip, "node-independent-b").is_ok(),
            "a different node_id behind the same peer IP must have its own budget"
        );
        std::env::remove_var("PION_AGENT_RATE_LIMIT_PER_MIN");
    }

    #[test]
    #[serial]
    fn check_agent_rate_limit_disabled_when_limit_is_zero() {
        std::env::set_var("PION_AGENT_RATE_LIMIT_PER_MIN", "0");
        let peer_ip = "203.0.113.7";
        let node_id = "rate-limit-test-node-disabled";
        for _ in 0..50 {
            assert!(check_agent_rate_limit(peer_ip, node_id).is_ok());
        }
        std::env::remove_var("PION_AGENT_RATE_LIMIT_PER_MIN");
    }

    #[test]
    #[serial]
    fn node_binding_skipped_when_insecure_opt_in() {
        clear_auth_env();
        std::env::set_var("PION_ALLOW_INSECURE", "1");
        let headers = HeaderMap::new();
        assert!(authorize_node_binding(&headers, "node-a").is_ok());
        clear_auth_env();
    }
}
