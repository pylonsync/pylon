//! Function registry — tracks registered TypeScript functions and their metadata.

use std::collections::HashMap;
use std::sync::Mutex;

use serde::{Deserialize, Serialize};

use crate::protocol::FnType;

// ---------------------------------------------------------------------------
// Function definition
// ---------------------------------------------------------------------------

/// Metadata about a registered TypeScript function.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FnDef {
    pub name: String,
    pub fn_type: FnType,
    /// JSON Schema for the function's args (from TypeScript validators).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub args_schema: Option<serde_json::Value>,
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
        });
        reg.register(FnDef {
            name: "getLots".into(),
            fn_type: FnType::Query,
            args_schema: None,
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
            FnDef { name: "a".into(), fn_type: FnType::Mutation, args_schema: None },
            FnDef { name: "b".into(), fn_type: FnType::Query, args_schema: None },
            FnDef { name: "c".into(), fn_type: FnType::Mutation, args_schema: None },
            FnDef { name: "d".into(), fn_type: FnType::Action, args_schema: None },
        ]);

        assert_eq!(reg.list_by_type(FnType::Mutation).len(), 2);
        assert_eq!(reg.list_by_type(FnType::Query).len(), 1);
        assert_eq!(reg.list_by_type(FnType::Action).len(), 1);
    }
}
