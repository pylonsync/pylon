
use crate::Plugin;
use serde_json::Value;

/// MCP tool definition — describes a tool an AI agent can call.
#[derive(Debug, Clone)]
pub struct McpTool {
    pub name: String,
    pub description: String,
    pub input_schema: Value,
}

/// MCP resource — a readable data source.
#[derive(Debug, Clone)]
pub struct McpResource {
    pub uri: String,
    pub name: String,
    pub description: String,
    pub mime_type: String,
}

/// Result of calling an MCP tool.
#[derive(Debug, Clone)]
pub struct McpToolResult {
    pub content: Vec<McpContent>,
    pub is_error: bool,
}

#[derive(Debug, Clone)]
pub struct McpContent {
    pub content_type: String,
    pub text: String,
}

/// MCP Server plugin. Exposes statecraft as an MCP server for AI agents.
/// Provides tools for CRUD operations, queries, actions, and schema inspection.
pub struct McpPlugin {
    app_name: String,
    entities: Vec<String>,
    actions: Vec<String>,
    queries: Vec<String>,
}

impl McpPlugin {
    pub fn new(app_name: &str) -> Self {
        Self {
            app_name: app_name.to_string(),
            entities: vec![],
            actions: vec![],
            queries: vec![],
        }
    }

    pub fn with_entities(mut self, entities: Vec<String>) -> Self {
        self.entities = entities;
        self
    }

    pub fn with_actions(mut self, actions: Vec<String>) -> Self {
        self.actions = actions;
        self
    }

    pub fn with_queries(mut self, queries: Vec<String>) -> Self {
        self.queries = queries;
        self
    }

    /// Generate the list of MCP tools this server exposes.
    pub fn tools(&self) -> Vec<McpTool> {
        let mut tools = vec![
            McpTool {
                name: "list_entities".into(),
                description: format!("List all rows from an entity in the {} database", self.app_name),
                input_schema: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "entity": {
                            "type": "string",
                            "description": format!("Entity name. Available: [{}]", self.entities.join(", ")),
                        }
                    },
                    "required": ["entity"]
                }),
            },
            McpTool {
                name: "get_entity".into(),
                description: "Get a single row by ID".into(),
                input_schema: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "entity": { "type": "string", "description": "Entity name" },
                        "id": { "type": "string", "description": "Row ID" }
                    },
                    "required": ["entity", "id"]
                }),
            },
            McpTool {
                name: "insert_entity".into(),
                description: "Insert a new row into an entity".into(),
                input_schema: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "entity": { "type": "string", "description": "Entity name" },
                        "data": { "type": "object", "description": "Row data as key-value pairs" }
                    },
                    "required": ["entity", "data"]
                }),
            },
            McpTool {
                name: "update_entity".into(),
                description: "Update an existing row".into(),
                input_schema: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "entity": { "type": "string", "description": "Entity name" },
                        "id": { "type": "string", "description": "Row ID" },
                        "data": { "type": "object", "description": "Fields to update" }
                    },
                    "required": ["entity", "id", "data"]
                }),
            },
            McpTool {
                name: "delete_entity".into(),
                description: "Delete a row by ID".into(),
                input_schema: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "entity": { "type": "string", "description": "Entity name" },
                        "id": { "type": "string", "description": "Row ID" }
                    },
                    "required": ["entity", "id"]
                }),
            },
            McpTool {
                name: "search".into(),
                description: "Search across entities with a text query".into(),
                input_schema: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "entity": { "type": "string", "description": "Entity to search in" },
                        "query": { "type": "string", "description": "Search text" }
                    },
                    "required": ["entity", "query"]
                }),
            },
            McpTool {
                name: "inspect_schema".into(),
                description: format!("Get the full schema of the {} app including entities, fields, queries, actions, and policies", self.app_name),
                input_schema: serde_json::json!({
                    "type": "object",
                    "properties": {}
                }),
            },
        ];

        // Add action-specific tools.
        for action in &self.actions {
            tools.push(McpTool {
                name: format!("action_{action}"),
                description: format!("Execute the {action} action"),
                input_schema: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "input": { "type": "object", "description": "Action input data" }
                    },
                    "required": ["input"]
                }),
            });
        }

        tools
    }

    /// Generate MCP resources (readable data sources).
    pub fn resources(&self) -> Vec<McpResource> {
        let mut resources = vec![
            McpResource {
                uri: "statecraft://schema".into(),
                name: "App Schema".into(),
                description: "The full app manifest/schema".into(),
                mime_type: "application/json".into(),
            },
        ];

        for entity in &self.entities {
            resources.push(McpResource {
                uri: format!("statecraft://entities/{entity}"),
                name: format!("{entity} data"),
                description: format!("All rows in the {entity} entity"),
                mime_type: "application/json".into(),
            });
        }

        resources
    }

    /// Generate the MCP server manifest (for tool discovery).
    pub fn server_info(&self) -> Value {
        serde_json::json!({
            "name": format!("{}-statecraft", self.app_name),
            "version": "0.1.0",
            "description": format!("MCP server for {} powered by statecraft", self.app_name),
            "tools": self.tools().iter().map(|t| serde_json::json!({
                "name": t.name,
                "description": t.description,
                "inputSchema": t.input_schema,
            })).collect::<Vec<_>>(),
            "resources": self.resources().iter().map(|r| serde_json::json!({
                "uri": r.uri,
                "name": r.name,
                "description": r.description,
                "mimeType": r.mime_type,
            })).collect::<Vec<_>>(),
        })
    }
}

impl Plugin for McpPlugin {
    fn name(&self) -> &str {
        "mcp"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_plugin() -> McpPlugin {
        McpPlugin::new("test-app")
            .with_entities(vec!["User".into(), "Todo".into()])
            .with_actions(vec!["createTodo".into(), "toggleTodo".into()])
            .with_queries(vec!["allTodos".into()])
    }

    #[test]
    fn generates_tools() {
        let plugin = test_plugin();
        let tools = plugin.tools();
        // 7 base tools + 2 action tools
        assert_eq!(tools.len(), 9);
        assert!(tools.iter().any(|t| t.name == "list_entities"));
        assert!(tools.iter().any(|t| t.name == "action_createTodo"));
        assert!(tools.iter().any(|t| t.name == "inspect_schema"));
    }

    #[test]
    fn generates_resources() {
        let plugin = test_plugin();
        let resources = plugin.resources();
        // 1 schema + 2 entity resources
        assert_eq!(resources.len(), 3);
        assert!(resources.iter().any(|r| r.uri == "statecraft://schema"));
        assert!(resources.iter().any(|r| r.uri == "statecraft://entities/User"));
    }

    #[test]
    fn server_info_is_valid_json() {
        let plugin = test_plugin();
        let info = plugin.server_info();
        assert!(info.get("name").is_some());
        assert!(info.get("tools").unwrap().as_array().unwrap().len() > 0);
        assert!(info.get("resources").unwrap().as_array().unwrap().len() > 0);
    }

    #[test]
    fn tool_schemas_have_required_fields() {
        let plugin = test_plugin();
        let tools = plugin.tools();
        for tool in &tools {
            assert!(!tool.name.is_empty());
            assert!(!tool.description.is_empty());
            assert!(tool.input_schema.is_object());
        }
    }

    #[test]
    fn entity_names_in_tool_description() {
        let plugin = test_plugin();
        let tools = plugin.tools();
        let list_tool = tools.iter().find(|t| t.name == "list_entities").unwrap();
        let schema_str = serde_json::to_string(&list_tool.input_schema).unwrap();
        assert!(schema_str.contains("User"));
        assert!(schema_str.contains("Todo"));
    }

    #[test]
    fn empty_plugin() {
        let plugin = McpPlugin::new("empty");
        let tools = plugin.tools();
        assert_eq!(tools.len(), 7); // base tools only
        let resources = plugin.resources();
        assert_eq!(resources.len(), 1); // schema only
    }
}
