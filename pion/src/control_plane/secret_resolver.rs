//! Claim-time resolution of `{"$secret_ref": ...}` markers in `PionNodeActionCommand.payload_json`.
//!
//! The wire shape is a JSON object (for example in a `command` array entry, or nested as the
//! `value` of a [`crate::DeploySecretEnvVar`] in `deploy_handoff` payloads):
//! `{"$secret_ref": {"id": "neutrino_secret:…", "version": 1}, "field": "password"}`.
//! At claim, the object is replaced with a **string** (the field value from the sealed JSON).

use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, OnceLock};

use anyhow::Context;
use async_trait::async_trait;
use serde_json::Value;
use valence::Valence;

/// Resolves Neutrino secret refs to field values (for example the `password` in
/// `{"user":"root","password":"…"}`), without persisting secrets in the queue.
#[async_trait]
pub trait SecretResolver: Send + Sync {
    /// Resolves one field from a sealed Neutrino secret JSON blob.
    ///
    /// # Contract
    ///
    /// `secret_id` and `version` select the Neutrino secret row; `field` is a string key in the
    /// decrypted JSON (for example `password` next to `user`).
    ///
    /// # Errors
    ///
    /// Returns `Err` when the secret is missing, decryption fails, or `field` is absent.
    async fn resolve_secret_root_field(
        &self,
        valence: &Valence,
        secret_id: &str,
        version: i64,
        field: &str,
    ) -> Result<String, anyhow::Error>;
}

static DEFAULT_SECRET_RESOLVER: OnceLock<Arc<dyn SecretResolver + Send + Sync>> = OnceLock::new();

/// One-time install (typically from the host binary at startup).
///
/// # Errors
///
/// Returns `Err` when a resolver was already installed.
pub fn set_default_secret_resolver(
    r: Arc<dyn SecretResolver + Send + Sync>,
) -> Result<(), Arc<dyn SecretResolver + Send + Sync>> {
    DEFAULT_SECRET_RESOLVER.set(r)
}

/// Returns the process-wide resolver installed by [`set_default_secret_resolver`], if any.
pub fn default_secret_resolver() -> Option<Arc<dyn SecretResolver + Send + Sync>> {
    DEFAULT_SECRET_RESOLVER.get().cloned()
}

/// True if `v` (recursively) contains a `$secret_ref` placeholder object.
pub fn value_contains_secret_ref_placeholder(v: &Value) -> bool {
    match v {
        Value::Array(a) => a.iter().any(value_contains_secret_ref_placeholder),
        Value::Object(m) => {
            if m.get(SECRET_REF_WRAPPER_KEY).is_some() && m.get("field").is_some() {
                return true;
            }
            m.values().any(value_contains_secret_ref_placeholder)
        }
        _ => false,
    }
}

const SECRET_REF_WRAPPER_KEY: &str = "$secret_ref";

/// If `v` is a secret-ref placeholder, resolve and return `Ok(Some(String))`. Otherwise `Ok(None)`.
///
/// Wire shape: `{"$secret_ref": {"id": "neutrino_secret:…", "version": 1}, "field": "password"}`.
/// Crate-root guide: [Secret-ref at claim](crate#secret-ref-at-claim).
///
/// # Errors
///
/// Returns `Err` when the object looks like a secret-ref but `field` / `$secret_ref.id` /
/// `version` are missing or wrong types, or when [`SecretResolver::resolve_secret_root_field`]
/// fails.
pub async fn try_resolve_one_secret_ref(
    valence: &Valence,
    resolver: &dyn SecretResolver,
    v: &Value,
) -> Result<Option<String>, anyhow::Error> {
    let Some(m) = v.as_object() else {
        return Ok(None);
    };
    if m.get(SECRET_REF_WRAPPER_KEY).is_none() {
        return Ok(None);
    }
    let field = m
        .get("field")
        .and_then(|x| x.as_str())
        .ok_or_else(|| anyhow::anyhow!("secret ref missing string field 'field'"))?;
    let refv = m
        .get(SECRET_REF_WRAPPER_KEY)
        .ok_or_else(|| anyhow::anyhow!("secret ref missing"))?;
    let id = refv
        .get("id")
        .and_then(|x| x.as_str())
        .ok_or_else(|| anyhow::anyhow!("$secret_ref.id missing or not a string"))?;
    let version = refv
        .get("version")
        .and_then(serde_json::Value::as_i64)
        .ok_or_else(|| anyhow::anyhow!("$secret_ref.version missing or not an integer"))?;
    let s = resolver
        .resolve_secret_root_field(valence, id, version, field)
        .await
        .with_context(|| {
            format!("resolve secret ref field '{field}' from secret {id} v{version}")
        })?;
    Ok(Some(s))
}

/// Walks `v` in place, replacing any secret-ref placeholder with a JSON string.
///
/// # Errors
///
/// Propagates secret resolution failures from nested placeholders.
pub async fn resolve_secrets_in_json(
    valence: &Valence,
    resolver: &dyn SecretResolver,
    v: &mut Value,
) -> Result<(), anyhow::Error> {
    resolve_secrets_in_json_inner(valence, resolver, v).await
}

fn resolve_secrets_in_json_inner<'a>(
    valence: &'a Valence,
    resolver: &'a dyn SecretResolver,
    v: &'a mut Value,
) -> Pin<Box<dyn Future<Output = Result<(), anyhow::Error>> + Send + 'a>> {
    Box::pin(async move {
        match v {
            Value::Array(a) => {
                for item in a.iter_mut() {
                    if let Some(s) = try_resolve_one_secret_ref(valence, resolver, item).await? {
                        *item = Value::String(s);
                    } else {
                        resolve_secrets_in_json_inner(valence, resolver, item).await?;
                    }
                }
            }
            Value::Object(_) => {
                if let Some(s) = try_resolve_one_secret_ref(valence, resolver, v).await? {
                    *v = Value::String(s);
                    return Ok(());
                }
                // Matched `Object` above; `as_object_mut` is infallible here.
                let Some(map) = v.as_object_mut() else {
                    return Ok(());
                };
                for child in map.values_mut() {
                    resolve_secrets_in_json_inner(valence, resolver, child).await?;
                }
            }
            _ => {}
        }
        Ok(())
    })
}
