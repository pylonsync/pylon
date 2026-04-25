use pylon_kernel::AppManifest;
use serde_json::{json, Value};

/// Generate a complete OpenAPI 3.0.3 specification from an `AppManifest`.
///
/// The `base_url` is used as the server URL in the spec. Pass an empty string
/// or "/" if the server URL should be relative to the host.
pub fn generate_openapi(manifest: &AppManifest, base_url: &str) -> Value {
    let mut paths = serde_json::Map::new();
    let mut schemas = serde_json::Map::new();

    // -----------------------------------------------------------------------
    // Fixed paths
    // -----------------------------------------------------------------------

    paths.insert(
        "/health".into(),
        json!({
            "get": {
                "operationId": "healthCheck",
                "summary": "Health check",
                "tags": ["system"],
                "responses": {
                    "200": {
                        "description": "Server is healthy",
                        "content": { "application/json": { "schema": {
                            "type": "object",
                            "properties": {
                                "status": { "type": "string" },
                                "version": { "type": "string" },
                                "uptime_secs": { "type": "integer" }
                            }
                        }}}
                    }
                }
            }
        }),
    );

    paths.insert("/api/manifest".into(), json!({
        "get": {
            "operationId": "getManifest",
            "summary": "Get application manifest",
            "tags": ["system"],
            "responses": {
                "200": { "description": "Application manifest", "content": { "application/json": { "schema": { "type": "object" } } } }
            }
        }
    }));

    paths.insert("/api/openapi.json".into(), json!({
        "get": {
            "operationId": "getOpenApiSpec",
            "summary": "Get OpenAPI specification",
            "tags": ["system"],
            "responses": {
                "200": { "description": "OpenAPI 3.0.3 spec", "content": { "application/json": { "schema": { "type": "object" } } } }
            }
        }
    }));

    paths.insert("/api/query".into(), json!({
        "post": {
            "operationId": "graphQuery",
            "summary": "Execute a graph query",
            "tags": ["query"],
            "security": [{ "BearerAuth": [] }],
            "requestBody": {
                "required": true,
                "content": { "application/json": { "schema": { "type": "object" } } }
            },
            "responses": {
                "200": { "description": "Query result", "content": { "application/json": { "schema": { "type": "object" } } } },
                "400": { "description": "Invalid query", "content": { "application/json": { "schema": { "$ref": "#/components/schemas/Error" } } } }
            }
        }
    }));

    paths.insert("/api/batch".into(), json!({
        "post": {
            "operationId": "batchOperations",
            "summary": "Execute batch operations",
            "tags": ["batch"],
            "security": [{ "BearerAuth": [] }],
            "requestBody": {
                "required": true,
                "content": { "application/json": { "schema": {
                    "type": "object",
                    "properties": {
                        "operations": {
                            "type": "array",
                            "items": { "type": "object" }
                        }
                    },
                    "required": ["operations"]
                }}}
            },
            "responses": {
                "200": { "description": "Batch results", "content": { "application/json": { "schema": { "type": "object" } } } }
            }
        }
    }));

    paths.insert("/api/transact".into(), json!({
        "post": {
            "operationId": "atomicTransaction",
            "summary": "Execute an atomic transaction",
            "tags": ["batch"],
            "security": [{ "BearerAuth": [] }],
            "requestBody": {
                "required": true,
                "content": { "application/json": { "schema": {
                    "type": "array",
                    "items": { "type": "object" }
                }}}
            },
            "responses": {
                "200": { "description": "Transaction committed", "content": { "application/json": { "schema": { "type": "object" } } } },
                "400": { "description": "Transaction rolled back", "content": { "application/json": { "schema": { "$ref": "#/components/schemas/Error" } } } }
            }
        }
    }));

    paths.insert("/api/export".into(), json!({
        "get": {
            "operationId": "exportAll",
            "summary": "Export all data (admin only)",
            "tags": ["admin"],
            "security": [{ "BearerAuth": [] }],
            "responses": {
                "200": { "description": "Full data export", "content": { "application/json": { "schema": { "type": "object" } } } },
                "403": { "description": "Forbidden", "content": { "application/json": { "schema": { "$ref": "#/components/schemas/Error" } } } }
            }
        }
    }));

    // -----------------------------------------------------------------------
    // Rooms
    // -----------------------------------------------------------------------

    paths.insert("/api/rooms".into(), json!({
        "get": {
            "operationId": "listRooms",
            "summary": "List active rooms",
            "tags": ["rooms"],
            "responses": {
                "200": { "description": "List of rooms", "content": { "application/json": { "schema": {
                    "type": "array",
                    "items": {
                        "type": "object",
                        "properties": {
                            "name": { "type": "string" },
                            "members": { "type": "integer" }
                        }
                    }
                }}}}
            }
        }
    }));

    for (path, op_id, summary) in [
        ("/api/rooms/join", "joinRoom", "Join a room"),
        ("/api/rooms/leave", "leaveRoom", "Leave a room"),
        (
            "/api/rooms/presence",
            "updatePresence",
            "Update presence in a room",
        ),
        (
            "/api/rooms/broadcast",
            "broadcastToRoom",
            "Broadcast a message to a room",
        ),
    ] {
        paths.insert(path.into(), json!({
            "post": {
                "operationId": op_id,
                "summary": summary,
                "tags": ["rooms"],
                "security": [{ "BearerAuth": [] }],
                "requestBody": {
                    "required": true,
                    "content": { "application/json": { "schema": { "type": "object" } } }
                },
                "responses": {
                    "200": { "description": "Success", "content": { "application/json": { "schema": { "type": "object" } } } },
                    "401": { "description": "Auth required", "content": { "application/json": { "schema": { "$ref": "#/components/schemas/Error" } } } }
                }
            }
        }));
    }

    // -----------------------------------------------------------------------
    // Auth endpoints
    // -----------------------------------------------------------------------

    paths.insert("/api/auth/session".into(), json!({
        "post": {
            "operationId": "createSession",
            "summary": "Create a session",
            "tags": ["auth"],
            "requestBody": {
                "required": true,
                "content": { "application/json": { "schema": {
                    "type": "object",
                    "properties": { "user_id": { "type": "string" } },
                    "required": ["user_id"]
                }}}
            },
            "responses": {
                "201": { "description": "Session created", "content": { "application/json": { "schema": {
                    "type": "object",
                    "properties": {
                        "token": { "type": "string" },
                        "user_id": { "type": "string" }
                    }
                }}}}
            }
        },
        "delete": {
            "operationId": "revokeSession",
            "summary": "Revoke current session",
            "tags": ["auth"],
            "security": [{ "BearerAuth": [] }],
            "responses": {
                "200": { "description": "Session revoked", "content": { "application/json": { "schema": { "type": "object" } } } }
            }
        }
    }));

    paths.insert("/api/auth/guest".into(), json!({
        "post": {
            "operationId": "createGuestSession",
            "summary": "Create a guest session",
            "tags": ["auth"],
            "responses": {
                "201": { "description": "Guest session created", "content": { "application/json": { "schema": {
                    "type": "object",
                    "properties": {
                        "token": { "type": "string" },
                        "user_id": { "type": "string" },
                        "guest": { "type": "boolean" }
                    }
                }}}}
            }
        }
    }));

    paths.insert("/api/auth/magic/send".into(), json!({
        "post": {
            "operationId": "sendMagicCode",
            "summary": "Send a magic login code",
            "tags": ["auth"],
            "requestBody": {
                "required": true,
                "content": { "application/json": { "schema": {
                    "type": "object",
                    "properties": { "email": { "type": "string", "format": "email" } },
                    "required": ["email"]
                }}}
            },
            "responses": {
                "200": { "description": "Code sent", "content": { "application/json": { "schema": { "type": "object" } } } }
            }
        }
    }));

    paths.insert("/api/auth/magic/verify".into(), json!({
        "post": {
            "operationId": "verifyMagicCode",
            "summary": "Verify a magic login code",
            "tags": ["auth"],
            "requestBody": {
                "required": true,
                "content": { "application/json": { "schema": {
                    "type": "object",
                    "properties": {
                        "email": { "type": "string", "format": "email" },
                        "code": { "type": "string" }
                    },
                    "required": ["email", "code"]
                }}}
            },
            "responses": {
                "200": { "description": "Verified and session created", "content": { "application/json": { "schema": {
                    "type": "object",
                    "properties": {
                        "token": { "type": "string" },
                        "user_id": { "type": "string" }
                    }
                }}}},
                "401": { "description": "Invalid code", "content": { "application/json": { "schema": { "$ref": "#/components/schemas/Error" } } } }
            }
        }
    }));

    paths.insert("/api/auth/providers".into(), json!({
        "get": {
            "operationId": "listAuthProviders",
            "summary": "List available OAuth providers",
            "tags": ["auth"],
            "responses": {
                "200": { "description": "Provider list", "content": { "application/json": { "schema": {
                    "type": "array",
                    "items": {
                        "type": "object",
                        "properties": {
                            "provider": { "type": "string" },
                            "auth_url": { "type": "string" }
                        }
                    }
                }}}}
            }
        }
    }));

    // -----------------------------------------------------------------------
    // Entity CRUD paths (generated from manifest)
    // -----------------------------------------------------------------------

    for entity in &manifest.entities {
        let entity_lower = entity.name.to_lowercase();
        let schema_ref = format!("#/components/schemas/{}", entity.name);
        let tag = entity.name.clone();

        // Build the schema for this entity.
        let entity_schema = build_entity_schema(entity);
        schemas.insert(entity.name.clone(), entity_schema);

        // GET + POST /api/entities/{entity}
        let collection_path = format!("/api/entities/{entity_lower}");
        paths.insert(collection_path, json!({
            "get": {
                "operationId": format!("list{}", entity.name),
                "summary": format!("List all {} entities", entity.name),
                "tags": [tag],
                "security": [{ "BearerAuth": [] }],
                "parameters": [
                    { "name": "limit", "in": "query", "schema": { "type": "integer" }, "description": "Maximum number of results" },
                    { "name": "offset", "in": "query", "schema": { "type": "integer", "default": 0 }, "description": "Number of results to skip" }
                ],
                "responses": {
                    "200": { "description": format!("List of {}", entity.name), "content": { "application/json": { "schema": {
                        "type": "object",
                        "properties": {
                            "data": { "type": "array", "items": { "$ref": &schema_ref } },
                            "total": { "type": "integer" },
                            "offset": { "type": "integer" },
                            "limit": { "type": "integer", "nullable": true }
                        }
                    }}}}
                }
            },
            "post": {
                "operationId": format!("create{}", entity.name),
                "summary": format!("Create a new {}", entity.name),
                "tags": [tag],
                "security": [{ "BearerAuth": [] }],
                "requestBody": {
                    "required": true,
                    "content": { "application/json": { "schema": { "$ref": &schema_ref } } }
                },
                "responses": {
                    "201": { "description": "Created", "content": { "application/json": { "schema": {
                        "type": "object",
                        "properties": { "id": { "type": "string" } }
                    }}}},
                    "400": { "description": "Validation error", "content": { "application/json": { "schema": { "$ref": "#/components/schemas/Error" } } } }
                }
            }
        }));

        // GET + PATCH + DELETE /api/entities/{entity}/{id}
        let item_path = format!("/api/entities/{entity_lower}/{{id}}");
        paths.insert(item_path, json!({
            "get": {
                "operationId": format!("get{}ById", entity.name),
                "summary": format!("Get a {} by ID", entity.name),
                "tags": [tag],
                "security": [{ "BearerAuth": [] }],
                "parameters": [
                    { "name": "id", "in": "path", "required": true, "schema": { "type": "string" } }
                ],
                "responses": {
                    "200": { "description": format!("{} found", entity.name), "content": { "application/json": { "schema": { "$ref": &schema_ref } } } },
                    "404": { "description": "Not found", "content": { "application/json": { "schema": { "$ref": "#/components/schemas/Error" } } } }
                }
            },
            "patch": {
                "operationId": format!("update{}", entity.name),
                "summary": format!("Update a {}", entity.name),
                "tags": [tag],
                "security": [{ "BearerAuth": [] }],
                "parameters": [
                    { "name": "id", "in": "path", "required": true, "schema": { "type": "string" } }
                ],
                "requestBody": {
                    "required": true,
                    "content": { "application/json": { "schema": { "$ref": &schema_ref } } }
                },
                "responses": {
                    "200": { "description": "Updated", "content": { "application/json": { "schema": { "type": "object", "properties": { "updated": { "type": "boolean" } } } } } },
                    "404": { "description": "Not found", "content": { "application/json": { "schema": { "$ref": "#/components/schemas/Error" } } } }
                }
            },
            "delete": {
                "operationId": format!("delete{}", entity.name),
                "summary": format!("Delete a {}", entity.name),
                "tags": [tag],
                "security": [{ "BearerAuth": [] }],
                "parameters": [
                    { "name": "id", "in": "path", "required": true, "schema": { "type": "string" } }
                ],
                "responses": {
                    "200": { "description": "Deleted", "content": { "application/json": { "schema": { "type": "object", "properties": { "deleted": { "type": "boolean" } } } } } },
                    "404": { "description": "Not found", "content": { "application/json": { "schema": { "$ref": "#/components/schemas/Error" } } } }
                }
            }
        }));

        // GET /api/entities/{entity}/cursor
        let cursor_path = format!("/api/entities/{entity_lower}/cursor");
        paths.insert(cursor_path, json!({
            "get": {
                "operationId": format!("list{}ByCursor", entity.name),
                "summary": format!("Cursor-paginated list of {}", entity.name),
                "tags": [tag],
                "security": [{ "BearerAuth": [] }],
                "parameters": [
                    { "name": "after", "in": "query", "schema": { "type": "string" }, "description": "Cursor: ID of the last item from the previous page" },
                    { "name": "limit", "in": "query", "schema": { "type": "integer", "default": 20, "maximum": 100 }, "description": "Maximum number of results" }
                ],
                "responses": {
                    "200": { "description": format!("Paginated {} list", entity.name), "content": { "application/json": { "schema": {
                        "type": "object",
                        "properties": {
                            "data": { "type": "array", "items": { "$ref": &schema_ref } },
                            "next_cursor": { "type": "string", "nullable": true },
                            "has_more": { "type": "boolean" }
                        }
                    }}}}
                }
            }
        }));
    }

    // -----------------------------------------------------------------------
    // Action paths (generated from manifest)
    // -----------------------------------------------------------------------

    for action in &manifest.actions {
        let action_lower = action.name.to_lowercase();
        let input_schema_name = format!("{}Input", action.name);
        let input_schema = build_fields_schema(&action.input);
        schemas.insert(input_schema_name.clone(), input_schema);

        let path = format!("/api/actions/{action_lower}");
        paths.insert(path, json!({
            "post": {
                "operationId": format!("execute{}", action.name),
                "summary": format!("Execute the {} action", action.name),
                "tags": ["actions"],
                "security": [{ "BearerAuth": [] }],
                "requestBody": {
                    "required": true,
                    "content": { "application/json": { "schema": { "$ref": format!("#/components/schemas/{input_schema_name}") } } }
                },
                "responses": {
                    "200": { "description": "Action executed", "content": { "application/json": { "schema": {
                        "type": "object",
                        "properties": {
                            "action": { "type": "string" },
                            "input": { "type": "object" },
                            "executed": { "type": "boolean" }
                        }
                    }}}},
                    "400": { "description": "Validation error", "content": { "application/json": { "schema": { "$ref": "#/components/schemas/Error" } } } },
                    "404": { "description": "Action not found", "content": { "application/json": { "schema": { "$ref": "#/components/schemas/Error" } } } }
                }
            }
        }));
    }

    // -----------------------------------------------------------------------
    // Shared schemas
    // -----------------------------------------------------------------------

    schemas.insert(
        "Error".into(),
        json!({
            "type": "object",
            "properties": {
                "error": {
                    "type": "object",
                    "properties": {
                        "code": { "type": "string" },
                        "message": { "type": "string" },
                        "hint": { "type": "string" }
                    },
                    "required": ["code", "message"]
                }
            }
        }),
    );

    // -----------------------------------------------------------------------
    // Assemble final spec
    // -----------------------------------------------------------------------

    json!({
        "openapi": "3.0.3",
        "info": {
            "title": manifest.name,
            "version": manifest.version,
            "description": format!("Auto-generated API documentation for {}", manifest.name)
        },
        "servers": [{ "url": base_url }],
        "paths": Value::Object(paths),
        "components": {
            "schemas": Value::Object(schemas),
            "securitySchemes": {
                "BearerAuth": {
                    "type": "http",
                    "scheme": "bearer"
                }
            }
        }
    })
}

/// Map an pylon field type string to an OpenAPI schema fragment.
fn map_field_type(field_type: &str) -> Value {
    match field_type {
        "string" => json!({ "type": "string" }),
        "int" => json!({ "type": "integer" }),
        "float" => json!({ "type": "number" }),
        "bool" => json!({ "type": "boolean" }),
        "datetime" => json!({ "type": "string", "format": "date-time" }),
        "richtext" => json!({ "type": "string" }),
        t if t.starts_with("id(") => json!({ "type": "string" }),
        _ => json!({ "type": "string" }),
    }
}

/// Build an OpenAPI schema object from a `ManifestEntity`.
fn build_entity_schema(entity: &pylon_kernel::ManifestEntity) -> Value {
    build_fields_schema_with_id(&entity.fields)
}

/// Build an OpenAPI schema from a slice of fields, prepending an `id` property.
fn build_fields_schema_with_id(fields: &[pylon_kernel::ManifestField]) -> Value {
    let mut properties = serde_json::Map::new();
    let mut required = vec!["id".to_string()];

    properties.insert("id".into(), json!({ "type": "string" }));

    for field in fields {
        properties.insert(field.name.clone(), map_field_type(&field.field_type));
        if !field.optional {
            required.push(field.name.clone());
        }
    }

    json!({
        "type": "object",
        "properties": Value::Object(properties),
        "required": required
    })
}

/// Build an OpenAPI schema from a slice of fields (no implicit `id`).
fn build_fields_schema(fields: &[pylon_kernel::ManifestField]) -> Value {
    let mut properties = serde_json::Map::new();
    let mut required = Vec::new();

    for field in fields {
        properties.insert(field.name.clone(), map_field_type(&field.field_type));
        if !field.optional {
            required.push(field.name.clone());
        }
    }

    let mut schema = json!({
        "type": "object",
        "properties": Value::Object(properties)
    });

    if !required.is_empty() {
        schema["required"] = json!(required);
    }

    schema
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use pylon_kernel::{ManifestAction, ManifestEntity, ManifestField, ManifestIndex};

    fn sample_manifest() -> AppManifest {
        AppManifest {
            manifest_version: 1,
            name: "TestApp".into(),
            version: "0.1.0".into(),
            entities: vec![
                ManifestEntity {
                    name: "User".into(),
                    fields: vec![
                        ManifestField {
                            name: "email".into(),
                            field_type: "string".into(),
                            optional: false,
                            unique: true,
                        },
                        ManifestField {
                            name: "age".into(),
                            field_type: "int".into(),
                            optional: true,
                            unique: false,
                        },
                        ManifestField {
                            name: "score".into(),
                            field_type: "float".into(),
                            optional: true,
                            unique: false,
                        },
                        ManifestField {
                            name: "active".into(),
                            field_type: "bool".into(),
                            optional: false,
                            unique: false,
                        },
                        ManifestField {
                            name: "createdAt".into(),
                            field_type: "datetime".into(),
                            optional: true,
                            unique: false,
                        },
                        ManifestField {
                            name: "bio".into(),
                            field_type: "richtext".into(),
                            optional: true,
                            unique: false,
                        },
                    ],
                    indexes: vec![ManifestIndex {
                        name: "email_idx".into(),
                        fields: vec!["email".into()],
                        unique: true,
                    }],
                    relations: vec![],
                    search: None,
                                    crdt: true,
                },
                ManifestEntity {
                    name: "Post".into(),
                    fields: vec![
                        ManifestField {
                            name: "title".into(),
                            field_type: "string".into(),
                            optional: false,
                            unique: false,
                        },
                        ManifestField {
                            name: "authorId".into(),
                            field_type: "id(User)".into(),
                            optional: false,
                            unique: false,
                        },
                    ],
                    indexes: vec![],
                    relations: vec![],
                    search: None,
                                    crdt: true,
                },
            ],
            routes: vec![],
            queries: vec![],
            actions: vec![ManifestAction {
                name: "PublishPost".into(),
                input: vec![
                    ManifestField {
                        name: "postId".into(),
                        field_type: "id(Post)".into(),
                        optional: false,
                        unique: false,
                    },
                    ManifestField {
                        name: "notify".into(),
                        field_type: "bool".into(),
                        optional: true,
                        unique: false,
                    },
                ],
            }],
            policies: vec![],
        }
    }

    #[test]
    fn spec_has_correct_structure() {
        let spec = generate_openapi(&sample_manifest(), "http://localhost:3000");

        assert_eq!(spec["openapi"], "3.0.3");
        assert_eq!(spec["info"]["title"], "TestApp");
        assert_eq!(spec["info"]["version"], "0.1.0");
        assert!(spec["info"]["description"]
            .as_str()
            .unwrap()
            .contains("TestApp"));
        assert_eq!(spec["servers"][0]["url"], "http://localhost:3000");
        assert!(spec["paths"].is_object());
        assert!(spec["components"]["schemas"].is_object());
        assert!(spec["components"]["securitySchemes"]["BearerAuth"].is_object());
    }

    #[test]
    fn spec_is_valid_json() {
        let spec = generate_openapi(&sample_manifest(), "/");
        // Round-trip through string to verify it's valid JSON.
        let json_str = serde_json::to_string(&spec).unwrap();
        let reparsed: Value = serde_json::from_str(&json_str).unwrap();
        assert_eq!(spec, reparsed);
    }

    #[test]
    fn entity_paths_generated_for_each_entity() {
        let spec = generate_openapi(&sample_manifest(), "/");
        let paths = spec["paths"].as_object().unwrap();

        // User entity
        assert!(
            paths.contains_key("/api/entities/user"),
            "missing collection path for User"
        );
        assert!(
            paths.contains_key("/api/entities/user/{id}"),
            "missing item path for User"
        );
        assert!(
            paths.contains_key("/api/entities/user/cursor"),
            "missing cursor path for User"
        );

        // Post entity
        assert!(
            paths.contains_key("/api/entities/post"),
            "missing collection path for Post"
        );
        assert!(
            paths.contains_key("/api/entities/post/{id}"),
            "missing item path for Post"
        );
        assert!(
            paths.contains_key("/api/entities/post/cursor"),
            "missing cursor path for Post"
        );

        // Collection path has GET and POST
        let user_collection = &paths["/api/entities/user"];
        assert!(user_collection.get("get").is_some());
        assert!(user_collection.get("post").is_some());

        // Item path has GET, PATCH, DELETE
        let user_item = &paths["/api/entities/user/{id}"];
        assert!(user_item.get("get").is_some());
        assert!(user_item.get("patch").is_some());
        assert!(user_item.get("delete").is_some());
    }

    #[test]
    fn action_paths_generated() {
        let spec = generate_openapi(&sample_manifest(), "/");
        let paths = spec["paths"].as_object().unwrap();

        assert!(
            paths.contains_key("/api/actions/publishpost"),
            "missing action path"
        );
        let action_path = &paths["/api/actions/publishpost"];
        assert!(action_path.get("post").is_some());
        assert_eq!(action_path["post"]["operationId"], "executePublishPost");
    }

    #[test]
    fn action_input_schema_generated() {
        let spec = generate_openapi(&sample_manifest(), "/");
        let schemas = spec["components"]["schemas"].as_object().unwrap();

        assert!(
            schemas.contains_key("PublishPostInput"),
            "missing action input schema"
        );
        let input = &schemas["PublishPostInput"];
        assert!(input["properties"]["postId"].is_object());
        assert!(input["properties"]["notify"].is_object());

        // postId is required, notify is optional
        let required = input["required"].as_array().unwrap();
        assert!(required.contains(&json!("postId")));
        assert!(!required.contains(&json!("notify")));
    }

    #[test]
    fn entity_schemas_generated() {
        let spec = generate_openapi(&sample_manifest(), "/");
        let schemas = spec["components"]["schemas"].as_object().unwrap();

        assert!(schemas.contains_key("User"));
        assert!(schemas.contains_key("Post"));

        let user = &schemas["User"];
        assert!(user["properties"]["id"].is_object());
        assert!(user["properties"]["email"].is_object());
        assert!(user["properties"]["age"].is_object());
    }

    #[test]
    fn field_types_mapped_correctly() {
        let spec = generate_openapi(&sample_manifest(), "/");
        let user = &spec["components"]["schemas"]["User"];

        // string -> string
        assert_eq!(user["properties"]["email"]["type"], "string");
        // int -> integer
        assert_eq!(user["properties"]["age"]["type"], "integer");
        // float -> number
        assert_eq!(user["properties"]["score"]["type"], "number");
        // bool -> boolean
        assert_eq!(user["properties"]["active"]["type"], "boolean");
        // datetime -> string with format date-time
        assert_eq!(user["properties"]["createdAt"]["type"], "string");
        assert_eq!(user["properties"]["createdAt"]["format"], "date-time");
        // richtext -> string
        assert_eq!(user["properties"]["bio"]["type"], "string");

        let post = &spec["components"]["schemas"]["Post"];
        // id(User) -> string
        assert_eq!(post["properties"]["authorId"]["type"], "string");
    }

    #[test]
    fn required_fields_in_schema() {
        let spec = generate_openapi(&sample_manifest(), "/");
        let user = &spec["components"]["schemas"]["User"];
        let required = user["required"].as_array().unwrap();

        // id, email, active are required (non-optional)
        assert!(required.contains(&json!("id")));
        assert!(required.contains(&json!("email")));
        assert!(required.contains(&json!("active")));

        // age, score, createdAt, bio are optional
        assert!(!required.contains(&json!("age")));
        assert!(!required.contains(&json!("score")));
        assert!(!required.contains(&json!("createdAt")));
        assert!(!required.contains(&json!("bio")));
    }

    #[test]
    fn fixed_paths_present() {
        let spec = generate_openapi(&sample_manifest(), "/");
        let paths = spec["paths"].as_object().unwrap();

        assert!(paths.contains_key("/health"));
        assert!(paths.contains_key("/api/manifest"));
        assert!(paths.contains_key("/api/query"));
        assert!(paths.contains_key("/api/batch"));
        assert!(paths.contains_key("/api/transact"));
        assert!(paths.contains_key("/api/export"));
        assert!(paths.contains_key("/api/rooms"));
        assert!(paths.contains_key("/api/rooms/join"));
        assert!(paths.contains_key("/api/rooms/leave"));
        assert!(paths.contains_key("/api/rooms/presence"));
        assert!(paths.contains_key("/api/rooms/broadcast"));
        assert!(paths.contains_key("/api/auth/session"));
        assert!(paths.contains_key("/api/auth/guest"));
        assert!(paths.contains_key("/api/auth/magic/send"));
        assert!(paths.contains_key("/api/auth/magic/verify"));
        assert!(paths.contains_key("/api/auth/providers"));
    }

    #[test]
    fn empty_manifest_produces_valid_spec() {
        let manifest = AppManifest {
            manifest_version: 1,
            name: "Empty".into(),
            version: "0.0.0".into(),
            entities: vec![],
            routes: vec![],
            queries: vec![],
            actions: vec![],
            policies: vec![],
        };
        let spec = generate_openapi(&manifest, "");

        assert_eq!(spec["openapi"], "3.0.3");
        assert_eq!(spec["info"]["title"], "Empty");
        // Only fixed paths + Error schema should exist.
        let schemas = spec["components"]["schemas"].as_object().unwrap();
        assert!(schemas.contains_key("Error"));
        assert_eq!(schemas.len(), 1);
    }
}
