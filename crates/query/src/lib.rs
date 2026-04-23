use pylon_kernel::{AppManifest, ManifestField, ManifestQuery};

// ---------------------------------------------------------------------------
// Query descriptor — runtime-facing query metadata
// ---------------------------------------------------------------------------

/// A query descriptor holds the contract for a single named query.
/// It describes what inputs the query accepts, derived from the manifest.
/// It does not implement execution.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QueryDescriptor {
    pub name: String,
    pub input: Vec<InputField>,
}

/// An input field descriptor for a query or action.
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

impl QueryDescriptor {
    pub fn from_manifest(mq: &ManifestQuery) -> Self {
        Self {
            name: mq.name.clone(),
            input: mq
                .input
                .iter()
                .map(InputField::from_manifest_field)
                .collect(),
        }
    }
}

// ---------------------------------------------------------------------------
// Query registry — all queries from a manifest
// ---------------------------------------------------------------------------

/// A registry of query descriptors, built from a manifest.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QueryRegistry {
    pub queries: Vec<QueryDescriptor>,
}

impl QueryRegistry {
    pub fn from_manifest(manifest: &AppManifest) -> Self {
        Self {
            queries: manifest
                .queries
                .iter()
                .map(QueryDescriptor::from_manifest)
                .collect(),
        }
    }

    pub fn get(&self, name: &str) -> Option<&QueryDescriptor> {
        self.queries.iter().find(|q| q.name == name)
    }

    pub fn names(&self) -> Vec<&str> {
        self.queries.iter().map(|q| q.name.as_str()).collect()
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
        let reg = QueryRegistry::from_manifest(&test_manifest());
        assert_eq!(reg.queries.len(), 3);
        assert_eq!(reg.names(), vec!["todosByAuthor", "allTodos", "todoById"]);
    }

    #[test]
    fn get_query_by_name() {
        let reg = QueryRegistry::from_manifest(&test_manifest());
        let q = reg.get("todosByAuthor").unwrap();
        assert_eq!(q.name, "todosByAuthor");
        assert_eq!(q.input.len(), 1);
        assert_eq!(q.input[0].name, "authorId");
        assert_eq!(q.input[0].field_type, "id(User)");
        assert!(!q.input[0].optional);
    }

    #[test]
    fn get_query_with_optional_input() {
        let reg = QueryRegistry::from_manifest(&test_manifest());
        let q = reg.get("allTodos").unwrap();
        assert_eq!(q.input.len(), 1);
        assert_eq!(q.input[0].name, "done");
        assert!(q.input[0].optional);
    }

    #[test]
    fn get_missing_query_returns_none() {
        let reg = QueryRegistry::from_manifest(&test_manifest());
        assert!(reg.get("nonexistent").is_none());
    }

    #[test]
    fn descriptor_from_manifest_query() {
        let mq = ManifestQuery {
            name: "test".into(),
            input: vec![ManifestField {
                name: "id".into(),
                field_type: "string".into(),
                optional: false,
                unique: false,
            }],
        };
        let desc = QueryDescriptor::from_manifest(&mq);
        assert_eq!(desc.name, "test");
        assert_eq!(desc.input.len(), 1);
        assert_eq!(desc.input[0].name, "id");
    }
}
