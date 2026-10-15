//! `ctx.apiKeys.create` / `ctx.apiKeys.revoke`: API keys for the app's own
//! users, minted from server code.
//!
//! The HTTP route `POST /api/auth/api-keys` mints a key for the signed-in
//! caller. An app that runs its own service users (for example one per
//! AI agent) needs to mint a key for a user that is not the caller. Before
//! this, the only way was `POST /api/auth/session` with the admin token,
//! which Pylon Cloud does not give apps. The runner has already refused
//! everything but actions, and callers that are neither internal nor elevated
//! (`api_key_op_gate` in pylon-functions); this module does the store work.

use pylon_auth::api_key::ApiKeyStore;
use pylon_functions::protocol::ApiKeyOpMessage;

const MAX_NAME_LEN: usize = 100;

/// Handle one `api_key_op` frame.
pub fn handle_op(
    store: &ApiKeyStore,
    req: &ApiKeyOpMessage,
) -> Result<serde_json::Value, (String, String)> {
    let user_id = req.user_id.trim();
    match req.op.as_str() {
        "create" => {
            let name = req
                .name
                .as_deref()
                .map(str::trim)
                .filter(|n| !n.is_empty())
                .unwrap_or("untitled");
            if name.chars().count() > MAX_NAME_LEN {
                return Err((
                    "INVALID_ARGS".into(),
                    format!("ctx.apiKeys.create name is longer than {MAX_NAME_LEN} characters"),
                ));
            }
            // A key that failed to persist is never returned: it would look
            // live and verify against nothing (same rule as the HTTP route).
            let (plaintext, key) = store
                .create(user_id.to_string(), name.to_string(), None, req.expires_at)
                .map_err(|e| {
                    (
                        "API_KEY_PERSIST_FAILED".to_string(),
                        format!("Could not persist the new API key: {e}"),
                    )
                })?;
            Ok(serde_json::json!({
                "key": plaintext,
                "id": key.id,
                "prefix": key.prefix,
                "expiresAt": key.expires_at,
            }))
        }
        "revoke" => {
            let key_id = req.key_id.as_deref().unwrap_or("").trim();
            // Only a key of the named user: a wrong pairing revokes nothing.
            match store.get(key_id) {
                Some(key) if key.user_id == user_id => {}
                _ => return Ok(serde_json::json!({ "revoked": false })),
            }
            if store.revoke(key_id) {
                return Ok(serde_json::json!({ "revoked": true }));
            }
            // The key exists and the delete reported nothing removed: a
            // storage failure, not "no such key". Say so, so the caller
            // does not treat a key that still works as gone.
            if store.get(key_id).is_some() {
                return Err((
                    "API_KEY_REVOKE_FAILED".into(),
                    "Could not revoke the API key; it is still active. Try again.".into(),
                ));
            }
            Ok(serde_json::json!({ "revoked": true }))
        }
        other => Err((
            "INVALID_ARGS".into(),
            format!("unknown ctx.apiKeys op: {other}"),
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn req(op: &str, user_id: &str, key_id: Option<&str>) -> ApiKeyOpMessage {
        ApiKeyOpMessage {
            call_id: "c_1".into(),
            op: op.into(),
            user_id: user_id.into(),
            name: Some("agent key".into()),
            expires_at: None,
            key_id: key_id.map(String::from),
        }
    }

    #[test]
    fn create_mints_a_key_that_verifies_for_that_user() {
        let store = ApiKeyStore::new();
        let out = handle_op(&store, &req("create", "svc_1", None)).unwrap();
        let key = out["key"].as_str().unwrap();
        assert!(key.starts_with("pk.key_"));
        let Ok(found) = store.verify(key) else {
            panic!("the minted key verifies")
        };
        assert_eq!(found.user_id, "svc_1");
        assert_eq!(found.name, "agent key");
    }

    #[test]
    fn revoke_needs_the_owning_user() {
        let store = ApiKeyStore::new();
        let out = handle_op(&store, &req("create", "svc_1", None)).unwrap();
        let id = out["id"].as_str().unwrap().to_string();
        let key = out["key"].as_str().unwrap().to_string();
        // Another user cannot revoke it.
        let other = handle_op(&store, &req("revoke", "svc_2", Some(&id))).unwrap();
        assert_eq!(other["revoked"], false);
        assert!(store.verify(&key).is_ok());
        // The owner can.
        let own = handle_op(&store, &req("revoke", "svc_1", Some(&id))).unwrap();
        assert_eq!(own["revoked"], true);
        assert!(store.verify(&key).is_err());
    }

    #[test]
    fn create_refuses_a_long_name() {
        let store = ApiKeyStore::new();
        let mut r = req("create", "svc_1", None);
        r.name = Some("x".repeat(MAX_NAME_LEN + 1));
        assert_eq!(handle_op(&store, &r).unwrap_err().0, "INVALID_ARGS");
    }
}
