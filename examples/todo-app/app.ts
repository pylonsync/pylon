import { entity, field, defineRoute, buildManifest } from "@agentdb/sdk";

// ---------------------------------------------------------------------------
// Schema
// ---------------------------------------------------------------------------

const User = entity("User", {
  email: field.string().unique(),
  displayName: field.string(),
  createdAt: field.datetime(),
});

const Todo = entity("Todo", {
  title: field.string(),
  done: field.bool(),
  authorId: field.id("User"),
  createdAt: field.datetime(),
}, {
  indexes: [
    { name: "by_author", fields: ["authorId"], unique: false },
  ],
});

// ---------------------------------------------------------------------------
// Routes
// ---------------------------------------------------------------------------

const home = defineRoute({
  path: "/",
  mode: "server",
});

const todoList = defineRoute({
  path: "/todos",
  mode: "live",
});

// ---------------------------------------------------------------------------
// Manifest
// ---------------------------------------------------------------------------

const manifest = buildManifest({
  name: "todo-app",
  version: "0.1.0",
  entities: [User, Todo],
  routes: [home, todoList],
});

// When run directly, print the manifest JSON
console.log(JSON.stringify(manifest, null, 2));
