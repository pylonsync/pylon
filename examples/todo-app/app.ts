import { entity, field, defineRoute, query, action, policy, buildManifest } from "@statecraft/sdk";

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
// Queries
// ---------------------------------------------------------------------------

const todosByAuthor = query("todosByAuthor", {
  input: [{ name: "authorId", type: "id(User)" }],
});

const allTodos = query("allTodos", {
  input: [{ name: "done", type: "bool", optional: true }],
});

const todoById = query("todoById", {
  input: [{ name: "todoId", type: "id(Todo)" }],
});

// ---------------------------------------------------------------------------
// Actions
// ---------------------------------------------------------------------------

const createTodo = action("createTodo", {
  input: [
    { name: "title", type: "string" },
    { name: "authorId", type: "id(User)" },
  ],
});

const toggleTodo = action("toggleTodo", {
  input: [{ name: "todoId", type: "id(Todo)" }],
});

// ---------------------------------------------------------------------------
// Policies
// ---------------------------------------------------------------------------

const authenticatedCreate = policy({
  name: "authenticatedCreate",
  action: "createTodo",
  allow: "auth.userId != null",
});

const ownerToggle = policy({
  name: "ownerToggle",
  action: "toggleTodo",
  allow: "auth.userId == input.authorId",
});

const ownerReadTodos = policy({
  name: "ownerReadTodos",
  entity: "Todo",
  allow: "auth.userId == data.authorId",
});

// ---------------------------------------------------------------------------
// Routes
// ---------------------------------------------------------------------------

const home = defineRoute({
  path: "/",
  mode: "server",
  query: "allTodos",
  auth: "public",
});

const todoList = defineRoute({
  path: "/todos",
  mode: "live",
  query: "todosByAuthor",
  auth: "user",
});

const todoDetail = defineRoute({
  path: "/todos/:todoId",
  mode: "server",
  query: "todoById",
  auth: "user",
});

// ---------------------------------------------------------------------------
// Manifest
// ---------------------------------------------------------------------------

const manifest = buildManifest({
  name: "todo-app",
  version: "0.1.0",
  entities: [User, Todo],
  queries: [todosByAuthor, allTodos, todoById],
  actions: [createTodo, toggleTodo],
  policies: [authenticatedCreate, ownerToggle, ownerReadTodos],
  routes: [home, todoList, todoDetail],
});

// Emit canonical manifest JSON to stdout.
// Used by: statecraft codegen examples/todo-app/app.ts
console.log(JSON.stringify(manifest, null, 2));
