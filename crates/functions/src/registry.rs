//! Function registry — tracks registered TypeScript functions and their metadata.

use std::collections::HashMap;
use std::sync::Mutex;

use serde::{Deserialize, Serialize};

use crate::protocol::FnType;

// ---------------------------------------------------------------------------
// Function definition
// ---------------------------------------------------------------------------

/// Declarative auth requirement set by the TS define API.
/// The router enforces this BEFORE the handler runs, so a handler
/// that forgets to check `ctx.auth.userId` doesn't leak data —
/// the request never reaches the handler in the first place.
///
/// Defaults to [`FnAuthMode::User`] in [`FnDef::default`]. The TS
/// runtime also emits `"user"` for any def that omits `auth`, so
/// the default-secure shape is enforced on both sides.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum FnAuthMode {
    /// Open to anyone, including unauthenticated callers. Must be
    /// explicit on the TS side — never inferred.
    Public,
    /// Guest sessions or authenticated users. For pre-login state
    /// (cart, preferences) carried by `/api/auth/guest` sessions.
    Guest,
    /// Real signed-in user required. The default for every
    /// declaration that omits `auth`. Guests are rejected.
    User,
    /// `is_admin == true` required. Use for ops endpoints exposed
    /// via `/api/fn/...`.
    Admin,
}

impl Default for FnAuthMode {
    fn default() -> Self {
        Self::User
    }
}

impl FnAuthMode {
    /// Stable wire-format string, matching the TS-side enum literals.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Public => "public",
            Self::Guest => "guest",
            Self::User => "user",
            Self::Admin => "admin",
        }
    }
}

/// Metadata about a registered TypeScript function.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FnDef {
    pub name: String,
    pub fn_type: FnType,
    /// JSON Schema for the function's args (from TypeScript validators).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub args_schema: Option<serde_json::Value>,
    /// When true, the function is callable only via `ctx.runQuery` /
    /// `ctx.runMutation` / `ctx.runAction` from another function — the
    /// router refuses external HTTP calls with `404 FN_NOT_FOUND` so
    /// probing can't even confirm the name exists.
    ///
    /// Set in the TypeScript define API via `internal: true` on the
    /// def. Apps mark their helper / "internal" mutations to prevent
    /// direct callers from bypassing the wrapping action's gating
    /// (see Pylon Cloud's deleteMachineRecord pattern for the rationale).
    #[serde(default, skip_serializing_if = "is_false")]
    pub internal: bool,
    /// Declarative auth gate. The router enforces this on every
    /// external `/api/fn/<name>` call BEFORE invoking the handler.
    /// Defaults to [`FnAuthMode::User`] — every function is
    /// signed-in-only unless explicitly opted out via
    /// `auth: "public"` in the TS define.
    ///
    /// The default applies in two places at once: the TS runtime
    /// emits `"user"` when the def omits `auth`, and this field
    /// also defaults to `User` if an older TS runtime doesn't
    /// emit the field at all. Both belts make sure existing
    /// deployments stay safe across a framework upgrade.
    #[serde(default)]
    pub auth: FnAuthMode,
    /// Per-function call deadline in seconds, from the TS def's `timeout`
    /// option. `None` → the host uses its global `PYLON_FN_CALL_TIMEOUT`.
    /// Drives both this function's call deadline and the wedge backstop for
    /// the worker running it, so a legitimately long call (heavy render, big
    /// batch) isn't force-killed mid-flight. `#[serde(default)]` keeps older
    /// TS runtimes (which don't emit the field) deserializing as `None`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timeout_secs: Option<u64>,
}

fn is_false(b: &bool) -> bool {
    !b
}

// ---------------------------------------------------------------------------
// Function registry
// ---------------------------------------------------------------------------

/// Registry of all TypeScript functions available to the runtime.
///
/// Populated at startup when the Bun process reports its registered functions.
/// Used by the router to validate function calls and by Studio to show
/// available functions.
pub struct FnRegistry {
    fns: Mutex<HashMap<String, FnDef>>,
}

impl FnRegistry {
    pub fn new() -> Self {
        Self {
            fns: Mutex::new(HashMap::new()),
        }
    }

    /// Register a function. Called during startup handshake with the Bun process.
    pub fn register(&self, def: FnDef) {
        self.fns.lock().unwrap().insert(def.name.clone(), def);
    }

    /// Register multiple functions at once (from startup handshake).
    pub fn register_all(&self, defs: Vec<FnDef>) {
        let mut fns = self.fns.lock().unwrap();
        for def in defs {
            fns.insert(def.name.clone(), def);
        }
    }

    /// Atomically replace the entire registered set. Used after the runtime
    /// supervisor respawns Bun: any function that was deleted between
    /// processes must stop being callable, and `register_all()` alone won't
    /// remove stale entries.
    pub fn replace_all(&self, defs: Vec<FnDef>) {
        let mut fns = self.fns.lock().unwrap();
        fns.clear();
        for def in defs {
            fns.insert(def.name.clone(), def);
        }
    }

    /// Look up a function by name.
    pub fn get(&self, name: &str) -> Option<FnDef> {
        self.fns.lock().unwrap().get(name).cloned()
    }

    /// List all registered functions.
    pub fn list(&self) -> Vec<FnDef> {
        self.fns.lock().unwrap().values().cloned().collect()
    }

    /// List functions of a specific type.
    pub fn list_by_type(&self, fn_type: FnType) -> Vec<FnDef> {
        self.fns
            .lock()
            .unwrap()
            .values()
            .filter(|f| f.fn_type == fn_type)
            .cloned()
            .collect()
    }

    /// Check if a function is registered.
    pub fn exists(&self, name: &str) -> bool {
        self.fns.lock().unwrap().contains_key(name)
    }

    /// Number of registered functions.
    pub fn count(&self) -> usize {
        self.fns.lock().unwrap().len()
    }
}

impl Default for FnRegistry {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn register_and_lookup() {
        let reg = FnRegistry::new();
        reg.register(FnDef {
            name: "placeBid".into(),
            fn_type: FnType::Mutation,
            args_schema: None,
            internal: false,
            auth: FnAuthMode::User,
            timeout_secs: None,
        });
        reg.register(FnDef {
            name: "getLots".into(),
            fn_type: FnType::Query,
            args_schema: None,
            internal: false,
            auth: FnAuthMode::User,
            timeout_secs: None,
        });

        assert_eq!(reg.count(), 2);
        assert!(reg.exists("placeBid"));
        assert!(!reg.exists("nonexistent"));

        let def = reg.get("placeBid").unwrap();
        assert_eq!(def.fn_type, FnType::Mutation);
    }

    #[test]
    fn list_by_type() {
        let reg = FnRegistry::new();
        reg.register_all(vec![
            FnDef {
                name: "a".into(),
                fn_type: FnType::Mutation,
                args_schema: None,
                internal: false,
                auth: FnAuthMode::User,
                timeout_secs: None,
            },
            FnDef {
                name: "b".into(),
                fn_type: FnType::Query,
                args_schema: None,
                internal: false,
                auth: FnAuthMode::User,
                timeout_secs: None,
            },
            FnDef {
                name: "c".into(),
                fn_type: FnType::Mutation,
                args_schema: None,
                internal: false,
                auth: FnAuthMode::User,
                timeout_secs: None,
            },
            FnDef {
                name: "d".into(),
                fn_type: FnType::Action,
                args_schema: None,
                internal: false,
                auth: FnAuthMode::User,
                timeout_secs: None,
            },
        ]);

        assert_eq!(reg.list_by_type(FnType::Mutation).len(), 2);
        assert_eq!(reg.list_by_type(FnType::Query).len(), 1);
        assert_eq!(reg.list_by_type(FnType::Action).len(), 1);
    }
}
