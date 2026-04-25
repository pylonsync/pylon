use pylon_kernel::{AppManifest, ManifestAction, ManifestField};

// ---------------------------------------------------------------------------
// Action descriptor — runtime-facing action metadata
// ---------------------------------------------------------------------------

/// An action descriptor holds the contract for a single named action.
/// It describes what inputs the action accepts, derived from the manifest.
/// It does not implement execution.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ActionDescriptor {
    pub name: String,
    pub input: Vec<InputField>,
}

/// An input field descriptor for an action.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InputField {
    pub name: String,
    pub field_type: String,
    pub optional: bool,
}

impl InputField {
    pub fn from_manifest_field(f: &ManifestField) -> Self {
        Self {
            name: f.name.clone(),
            field_type: f.field_type.clone(),
            optional: f.optional,
        }
    }
}

impl ActionDescriptor {
    pub fn from_manifest(ma: &ManifestAction) -> Self {
        Self {
            name: ma.name.clone(),
            input: ma
                .input
                .iter()
                .map(InputField::from_manifest_field)
                .collect(),
        }
    }
}

// ---------------------------------------------------------------------------
// Action registry — all actions from a manifest
// ---------------------------------------------------------------------------

/// A registry of action descriptors, built from a manifest.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ActionRegistry {
    pub actions: Vec<ActionDescriptor>,
}

impl ActionRegistry {
    pub fn from_manifest(manifest: &AppManifest) -> Self {
        Self {
            actions: manifest
                .actions
                .iter()
                .map(ActionDescriptor::from_manifest)
                .collect(),
        }
    }

    pub fn get(&self, name: &str) -> Option<&ActionDescriptor> {
        self.actions.iter().find(|a| a.name == name)
    }

    pub fn names(&self) -> Vec<&str> {
        self.actions.iter().map(|a| a.name.as_str()).collect()
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use pylon_kernel::ManifestField;

    fn test_manifest() -> AppManifest {
        serde_json::from_str(include_str!(
            "../../../examples/todo-app/pylon.manifest.json"
        ))
        .unwrap()
    }

    #[test]
    fn registry_from_manifest() {
        let reg = ActionRegistry::from_manifest(&test_manifest());
        assert_eq!(reg.actions.len(), 2);
        assert_eq!(reg.names(), vec!["createTodo", "toggleTodo"]);
    }

    #[test]
    fn get_action_by_name() {
        let reg = ActionRegistry::from_manifest(&test_manifest());
        let a = reg.get("createTodo").unwrap();
        assert_eq!(a.name, "createTodo");
        assert_eq!(a.input.len(), 2);
        assert_eq!(a.input[0].name, "title");
        assert_eq!(a.input[0].field_type, "string");
        assert_eq!(a.input[1].name, "authorId");
        assert_eq!(a.input[1].field_type, "id(User)");
    }

    #[test]
    fn get_missing_action_returns_none() {
        let reg = ActionRegistry::from_manifest(&test_manifest());
        assert!(reg.get("nonexistent").is_none());
    }

    #[test]
    fn descriptor_from_manifest_action() {
        let ma = ManifestAction {
            name: "doThing".into(),
            input: vec![
                ManifestField {
                    name: "x".into(),
                    field_type: "string".into(),
                    optional: false,
                    unique: false,
                    crdt: None,
                },
                ManifestField {
                    name: "y".into(),
                    field_type: "int".into(),
                    optional: true,
                    unique: false,
                    crdt: None,
                },
            ],
        };
        let desc = ActionDescriptor::from_manifest(&ma);
        assert_eq!(desc.name, "doThing");
        assert_eq!(desc.input.len(), 2);
        assert!(!desc.input[0].optional);
        assert!(desc.input[1].optional);
    }
}
