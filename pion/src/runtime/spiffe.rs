//! JWT-SVID verification for SPIFFE workload identity.
//!
//! Implemented and ready for operators who run SPIRE. Expected SPIFFE ID shape:
//! `spiffe://{trust_domain}/parton/node/{node_id}`.
//!
//! Configure `PION_SPIFFE_TRUST_DOMAIN` and either:
//! - `PION_SPIFFE_JWT_KEY_PATH` — PEM public key (`RS256` / `ES256` / `EdDSA`), or
//! - `PION_SPIFFE_JWT_HMAC_SECRET` — HS256 shared secret (labs / unit tests only).
//!
//! Auth modes (`PION_AUTH_MODE`): `shared_token` (default), `dual`, `spiffe`.
//! Rollout guide: repository `docs/spiffe.md`. Vulnerability reporting: `SECURITY.md`.

use axum::http::StatusCode;
use jsonwebtoken::{decode, Algorithm, DecodingKey, Validation};
use serde::Deserialize;

/// Authentication mode for `/api/parton/*` (`PION_AUTH_MODE`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum AuthMode {
    /// Shared token only (default).
    SharedToken,
    /// Accept either a valid shared token or a valid JWT-SVID.
    Dual,
    /// JWT-SVID required; shared token ignored (except the insecure local bypass).
    Spiffe,
}

impl AuthMode {
    pub(super) fn from_env() -> Self {
        Self::from_raw(std::env::var("PION_AUTH_MODE").ok().as_deref())
    }

    pub(super) fn from_raw(raw: Option<&str>) -> Self {
        match raw.map_or("", str::trim).to_ascii_lowercase().as_str() {
            "spiffe" => Self::Spiffe,
            "dual" => Self::Dual,
            _ => Self::SharedToken,
        }
    }
}

/// Build the canonical SPIFFE ID for a Parton agent node.
#[must_use]
pub(super) fn expected_spiffe_id(trust_domain: &str, node_id: &str) -> String {
    format!(
        "spiffe://{}/parton/node/{}",
        trust_domain.trim().trim_end_matches('/'),
        node_id.trim()
    )
}

fn trust_domain() -> Option<String> {
    std::env::var("PION_SPIFFE_TRUST_DOMAIN")
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

fn audience() -> String {
    std::env::var("PION_SPIFFE_AUDIENCE")
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "pion".to_string())
}

#[derive(Debug, Deserialize)]
struct JwtClaims {
    sub: String,
    #[serde(default)]
    aud: Aud,
}

#[derive(Debug, Deserialize, Default)]
#[serde(untagged)]
enum Aud {
    #[default]
    Missing,
    One(String),
    Many(Vec<String>),
}

impl Aud {
    fn contains(&self, expected: &str) -> bool {
        match self {
            Self::Missing => false,
            Self::One(s) => s == expected,
            Self::Many(v) => v.iter().any(|s| s == expected),
        }
    }
}

fn decoding_key() -> Result<(DecodingKey, Vec<Algorithm>), String> {
    if let Ok(path) = std::env::var("PION_SPIFFE_JWT_KEY_PATH") {
        let path = path.trim();
        if !path.is_empty() {
            let pem = std::fs::read(path)
                .map_err(|e| format!("failed to read PION_SPIFFE_JWT_KEY_PATH `{path}`: {e}"))?;
            // Prefer RSA; fall back to EC / EdDSA encodings that jsonwebtoken accepts from PEM.
            if let Ok(key) = DecodingKey::from_rsa_pem(&pem) {
                return Ok((
                    key,
                    vec![Algorithm::RS256, Algorithm::RS384, Algorithm::RS512],
                ));
            }
            if let Ok(key) = DecodingKey::from_ec_pem(&pem) {
                return Ok((key, vec![Algorithm::ES256, Algorithm::ES384]));
            }
            if let Ok(key) = DecodingKey::from_ed_pem(&pem) {
                return Ok((key, vec![Algorithm::EdDSA]));
            }
            return Err(format!(
                "PION_SPIFFE_JWT_KEY_PATH `{path}` is not a usable RSA/EC/Ed25519 public key PEM"
            ));
        }
    }
    if let Ok(secret) = std::env::var("PION_SPIFFE_JWT_HMAC_SECRET") {
        let secret = secret.trim();
        if !secret.is_empty() {
            return Ok((
                DecodingKey::from_secret(secret.as_bytes()),
                vec![Algorithm::HS256],
            ));
        }
    }
    Err("SPIFFE auth requires PION_SPIFFE_JWT_KEY_PATH or PION_SPIFFE_JWT_HMAC_SECRET".to_string())
}

/// Verify a bearer JWT-SVID for `node_id`.
///
/// # Errors
///
/// Returns `(StatusCode, message)` when the token is missing/invalid or the SPIFFE ID does not
/// match `spiffe://{trust}/parton/node/{node_id}`.
pub(super) fn verify_jwt_svid(bearer_jwt: &str, node_id: &str) -> Result<(), (StatusCode, String)> {
    let trust = trust_domain().ok_or_else(|| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            "PION_SPIFFE_TRUST_DOMAIN is not configured".to_string(),
        )
    })?;
    let (key, algs) =
        decoding_key().map_err(|message| (StatusCode::INTERNAL_SERVER_ERROR, message))?;

    let mut validation = Validation::new(algs[0]);
    validation.algorithms = algs;
    validation.set_audience(&[audience()]);
    validation.validate_exp = true;

    let token = decode::<JwtClaims>(bearer_jwt.trim(), &key, &validation).map_err(|e| {
        tracing::warn!(
            target: "security.authz",
            check = "spiffe_jwt",
            error = %e,
            "agent request rejected: JWT-SVID verification failed"
        );
        (
            StatusCode::UNAUTHORIZED,
            "Unauthorized agent request (invalid SPIFFE JWT-SVID)".to_string(),
        )
    })?;

    let expected = expected_spiffe_id(&trust, node_id);
    if token.claims.sub != expected {
        tracing::warn!(
            target: "security.authz",
            check = "spiffe_id",
            expected = %expected,
            actual = %token.claims.sub,
            "agent request rejected: SPIFFE ID does not match node_id"
        );
        return Err((
            StatusCode::FORBIDDEN,
            "SPIFFE ID does not match request node_id".to_string(),
        ));
    }
    if !token.claims.aud.contains(&audience()) {
        // jsonwebtoken already validates aud when set_audience is used; keep a belt-and-suspenders
        // check for clarity in audits.
        return Err((
            StatusCode::UNAUTHORIZED,
            "Unauthorized agent request (audience mismatch)".to_string(),
        ));
    }
    Ok(())
}

/// Extract `Authorization: Bearer <jwt>` from headers.
pub(super) fn bearer_jwt_from_headers(headers: &axum::http::HeaderMap) -> Option<String> {
    let value = headers
        .get(axum::http::header::AUTHORIZATION)?
        .to_str()
        .ok()?;
    let value = value.trim();
    let rest = value
        .strip_prefix("Bearer ")
        .or_else(|| value.strip_prefix("bearer "))?;
    let jwt = rest.trim();
    if jwt.is_empty() {
        None
    } else {
        Some(jwt.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use jsonwebtoken::{encode, EncodingKey, Header};
    use serial_test::serial;

    fn clear_spiffe_env() {
        for k in [
            "PION_AUTH_MODE",
            "PION_SPIFFE_TRUST_DOMAIN",
            "PION_SPIFFE_AUDIENCE",
            "PION_SPIFFE_JWT_KEY_PATH",
            "PION_SPIFFE_JWT_HMAC_SECRET",
        ] {
            std::env::remove_var(k);
        }
    }

    #[test]
    fn expected_spiffe_id_format() {
        assert_eq!(
            expected_spiffe_id("example.org", "node-a"),
            "spiffe://example.org/parton/node/node-a"
        );
    }

    #[test]
    fn auth_mode_from_raw() {
        assert_eq!(AuthMode::from_raw(None), AuthMode::SharedToken);
        assert_eq!(AuthMode::from_raw(Some("dual")), AuthMode::Dual);
        assert_eq!(AuthMode::from_raw(Some("SPIFFE")), AuthMode::Spiffe);
    }

    #[derive(serde::Serialize)]
    struct TestClaims {
        sub: String,
        aud: String,
        exp: usize,
    }

    fn jwt_exp_one_hour() -> usize {
        let ts = (chrono::Utc::now() + chrono::Duration::hours(1)).timestamp();
        usize::try_from(ts).unwrap_or(usize::MAX)
    }

    #[test]
    #[serial]
    fn verify_accepts_matching_hs256_svid() {
        clear_spiffe_env();
        std::env::set_var("PION_SPIFFE_TRUST_DOMAIN", "example.org");
        std::env::set_var("PION_SPIFFE_JWT_HMAC_SECRET", "test-hmac-secret");
        std::env::set_var("PION_SPIFFE_AUDIENCE", "pion");
        let claims = TestClaims {
            sub: expected_spiffe_id("example.org", "node-a"),
            aud: "pion".to_string(),
            exp: jwt_exp_one_hour(),
        };
        let token = encode(
            &Header::new(Algorithm::HS256),
            &claims,
            &EncodingKey::from_secret(b"test-hmac-secret"),
        )
        .expect("encode");
        assert!(verify_jwt_svid(&token, "node-a").is_ok());
        clear_spiffe_env();
    }

    #[test]
    #[serial]
    fn verify_rejects_wrong_node_id() {
        clear_spiffe_env();
        std::env::set_var("PION_SPIFFE_TRUST_DOMAIN", "example.org");
        std::env::set_var("PION_SPIFFE_JWT_HMAC_SECRET", "test-hmac-secret");
        let claims = TestClaims {
            sub: expected_spiffe_id("example.org", "node-a"),
            aud: "pion".to_string(),
            exp: jwt_exp_one_hour(),
        };
        let token = encode(
            &Header::new(Algorithm::HS256),
            &claims,
            &EncodingKey::from_secret(b"test-hmac-secret"),
        )
        .expect("encode");
        let err = verify_jwt_svid(&token, "node-b").expect_err("wrong node");
        assert_eq!(err.0, StatusCode::FORBIDDEN);
        clear_spiffe_env();
    }

    const RSA_PRIVATE_PEM: &str = "-----BEGIN PRIVATE KEY-----
MIIEvAIBADANBgkqhkiG9w0BAQEFAASCBKYwggSiAgEAAoIBAQCMM4SE/+izpHUy
eFyYHFKqc+RUZ26yy71u1WOlLS7lWYT02RcWIzJTVieyUGFsIoACVgzx1VI6arqx
5xMHcwCukGFbM2hzOr/p1erkcSXN+XRzTk0Um7jF7vcpcq0y1hP0waWprC0l8hV7
40lqGycftLTD9ifFKXFj5+db78vrOBc+/Buu6QuIT1GpOoJxL7FLREyZN8ctxoJn
Wo4AdjTALAXyDh3m7ysiCMm41W0ixuADodraXkNOJe9zg81GsWyk5zeIpxk2thhn
BstktMqZLM4oENBe8vTi7FIfZWEhZeWWfyG65OixccP8sPiArs3gSJzoWpz+u/nQ
KZEunwz9AgMBAAECggEABRIn80q+Q+3pP0LgA0oJXP9SfUSjpsJJWItNtPLsnoCd
l0cmCp4jR9hdGM0t30qWjrJGE/xY1dWrNjWny1eUte7XvQPslzf2XdYBTo0fhoYR
rDBjOu39W0tiLS5n2hrcH4VeVJC7R7Z5CHR9ra5lvKo9XFXrtYavzzx02PwHYbtY
ujy6E/hHNI002K9aj8Bo8SjmNKkA8YUWYt3sNMJzNzpeLWVb1HoQ92zyxqB/Uf7o
2/v3kk15ZDVLbJ5IFAtKOIx9zGZo19zeXaphfIUJTfzzitQ0i6iLYGgHuYGS+LdK
QD6Kq86uGX8Skb/UUd4+s4SWg8TjAteewFkjABYPAQKBgQDAladg5WDZtnc3Ub1W
DXzUs+iO6fjOXF+ELWFRNzxoUgf6fCRO+hplHy6JKjutZduMPdzfxIrLk4CQUWTo
WwHYrSKdZ/0NOd21gzsCFtt7ZK8WEDGXortSlAjXbA1US8BoYN5M7o7jmAafeLMx
7NW+dy+E40WrX7JG6xBsJhgajQKBgQC6Xhfe6cN3ilkIKqBVLhHTWY6crz62XSY0
OxA7uAE1vfmk2BzS9d78SAYqDYM/bO6lzFU6hCGE2lLQXlRMx7nJo5TqYd962e2S
kUpzzTAHmWurVHcBdGN0wnfP5ZMCTkY3XPDH7iUVwVZ8d6LrdZcpqIFUoiIfLsfd
D55bEsfYMQKBgENiWb1ypO+og5AsnpYEhCAcjwuqXC9AbP9frYRwUkeiJD6Fv2KO
6jtlrK+7wkPKtwYcAzOcnn/arYpRoIAYj83Tzp/K6eT90VCYiUYS31sOgKC0q9WF
0At11p4hpYxQMROUiVPbyM1jvTWBUxnt52AMbekOaKstAcEEsKtWrOplAoGACQO4
H94qyEN23wBA1R3vWsvALDAF1ohW6rvYoyrZVCImSyTw7/tYl9dcBPi2WoEIYhiq
HrR5cpWk39NQPI6EnA4/i77EMosMBMTmVwebxSJUpOrm/rkEfodRiErQe5IRr2fd
da49OPorFsYqqTz83NT7vH5DLEL1A+pXfIxCAmECgYBmhiSsJGYPkb0uNDRK4VLH
a8sO1jLnZYLGfoeUXl90jrLOdm70AN/X3wKnRSuR4BoAcwBNy5rLAfXuoALjrKam
5/rXmYbiW7wdFTdoVlG0vaz2qKtQGcWSpeqRo5rDyAxePnohRGR7TH63IRHFt1p0
4OM7llxYe4Zc2dmBApVeSA==
-----END PRIVATE KEY-----
";

    const RSA_PUBLIC_PEM: &str = "-----BEGIN PUBLIC KEY-----
MIIBIjANBgkqhkiG9w0BAQEFAAOCAQ8AMIIBCgKCAQEAjDOEhP/os6R1MnhcmBxS
qnPkVGdussu9btVjpS0u5VmE9NkXFiMyU1YnslBhbCKAAlYM8dVSOmq6secTB3MA
rpBhWzNoczq/6dXq5HElzfl0c05NFJu4xe73KXKtMtYT9MGlqawtJfIVe+NJahsn
H7S0w/YnxSlxY+fnW+/L6zgXPvwbrukLiE9RqTqCcS+xS0RMmTfHLcaCZ1qOAHY0
wCwF8g4d5u8rIgjJuNVtIsbgA6Ha2l5DTiXvc4PNRrFspOc3iKcZNrYYZwbLZLTK
mSzOKBDQXvL04uxSH2VhIWXlln8huuTosXHD/LD4gK7N4Eic6Fqc/rv50CmRLp8M
/QIDAQAB
-----END PUBLIC KEY-----
";

    const RSA_OTHER_PUBLIC_PEM: &str = "-----BEGIN PUBLIC KEY-----
MIIBIjANBgkqhkiG9w0BAQEFAAOCAQ8AMIIBCgKCAQEAjMUz5IDOUAhSHcVm0hp6
lO7MYMnsGoMqqoh5UfAiUnnwuJjqDYTu7cN0DBLH0uZ3FhfjHAEDYoo6/4Za6hD1
FX3h/1geF86eJdSFtaMOaulgGPrTF0qL2qsVq/Dq9H/ZxzE2akpWx9QrfD68tuGS
u9BsfQy3xIIBRYc/lKjHkvnMM6tiAI/TTdGv+TKb+XenMC2anZMXbPPbqlRVDqIX
SDPGuEekoQRcZybULpvCSAw5DU0vDSz8v+Kt0/vGpnkQE1TctAsLDL6q4zRxsVUJ
73VteORaqwfVf/xlNoHBGtILMrU+7xG4cow8zk5mX8Sc85CNOivy7+6O2tHHMacm
/wIDAQAB
-----END PUBLIC KEY-----
";

    fn sign_rs256(claims: &TestClaims) -> String {
        encode(
            &Header::new(Algorithm::RS256),
            claims,
            &EncodingKey::from_rsa_pem(RSA_PRIVATE_PEM.as_bytes()).expect("priv pem"),
        )
        .expect("encode rs256")
    }

    #[test]
    #[serial]
    fn verify_accepts_matching_rs256_pem_key_path() {
        clear_spiffe_env();
        let dir = tempfile::tempdir().expect("tempdir");
        let pub_path = dir.path().join("spiffe.pub");
        std::fs::write(&pub_path, RSA_PUBLIC_PEM).expect("write pub");
        std::env::set_var("PION_SPIFFE_TRUST_DOMAIN", "example.org");
        std::env::set_var("PION_SPIFFE_AUDIENCE", "pion");
        std::env::set_var("PION_SPIFFE_JWT_KEY_PATH", pub_path.to_str().expect("utf8"));
        let claims = TestClaims {
            sub: expected_spiffe_id("example.org", "node-a"),
            aud: "pion".to_string(),
            exp: jwt_exp_one_hour(),
        };
        let token = sign_rs256(&claims);
        assert!(verify_jwt_svid(&token, "node-a").is_ok());
        clear_spiffe_env();
    }

    #[test]
    #[serial]
    fn verify_rejects_wrong_rs256_public_pem() {
        clear_spiffe_env();
        let dir = tempfile::tempdir().expect("tempdir");
        let pub_path = dir.path().join("wrong.pub");
        std::fs::write(&pub_path, RSA_OTHER_PUBLIC_PEM).expect("write pub");
        std::env::set_var("PION_SPIFFE_TRUST_DOMAIN", "example.org");
        std::env::set_var("PION_SPIFFE_AUDIENCE", "pion");
        std::env::set_var("PION_SPIFFE_JWT_KEY_PATH", pub_path.to_str().expect("utf8"));
        let claims = TestClaims {
            sub: expected_spiffe_id("example.org", "node-a"),
            aud: "pion".to_string(),
            exp: jwt_exp_one_hour(),
        };
        let token = sign_rs256(&claims);
        let err = verify_jwt_svid(&token, "node-a").expect_err("wrong key");
        assert_eq!(err.0, StatusCode::UNAUTHORIZED);
        clear_spiffe_env();
    }

    #[test]
    #[serial]
    fn verify_rejects_expired_rs256_svid() {
        clear_spiffe_env();
        let dir = tempfile::tempdir().expect("tempdir");
        let pub_path = dir.path().join("spiffe.pub");
        std::fs::write(&pub_path, RSA_PUBLIC_PEM).expect("write pub");
        std::env::set_var("PION_SPIFFE_TRUST_DOMAIN", "example.org");
        std::env::set_var("PION_SPIFFE_AUDIENCE", "pion");
        std::env::set_var("PION_SPIFFE_JWT_KEY_PATH", pub_path.to_str().expect("utf8"));
        let expired = (chrono::Utc::now() - chrono::Duration::hours(1)).timestamp();
        let claims = TestClaims {
            sub: expected_spiffe_id("example.org", "node-a"),
            aud: "pion".to_string(),
            exp: usize::try_from(expired).unwrap_or(0),
        };
        let token = sign_rs256(&claims);
        let err = verify_jwt_svid(&token, "node-a").expect_err("expired");
        assert_eq!(err.0, StatusCode::UNAUTHORIZED);
        clear_spiffe_env();
    }
}
