import {
	entity,
	field,
	query,
	action,
	policy,
	buildManifest,
} from "@pylonsync/sdk";

// ---------------------------------------------------------------------------
// Schema
// ---------------------------------------------------------------------------

const Todo = entity("Todo", {
	title: field.string(),
	done: field.bool(),
	createdAt: field.datetime(),
	// Float position so drag-reorder can insert between two existing
	// rows without renumbering the whole list. Frontend computes
	// (prev.position + next.position) / 2 on drop. Optional for
	// backwards compat with legacy rows.
	position: field.float().optional(),
});

// ---------------------------------------------------------------------------
// Function declarations — names only. Implementations live under
// functions/<name>.ts and are auto-discovered by the runtime.
// ---------------------------------------------------------------------------

const listTodos = query("listTodos");

const addTodo = action("addTodo", {
	input: [{ name: "title", type: "string" }],
});

const toggleTodo = action("toggleTodo", {
	input: [{ name: "id", type: "id(Todo)" }, { name: "done", type: "bool" }],
});

const deleteTodo = action("deleteTodo", {
	input: [{ name: "id", type: "id(Todo)" }],
});

const editTodo = action("editTodo", {
	input: [
		{ name: "id", type: "id(Todo)" },
		{ name: "title", type: "string" },
	],
});

const reorderTodo = action("reorderTodo", {
	input: [
		{ name: "id", type: "id(Todo)" },
		{ name: "position", type: "float" },
	],
});

// ---------------------------------------------------------------------------
// Policies — wide-open by default. Tighten for production.
// ---------------------------------------------------------------------------

const todoPolicy = policy({
	name: "todo_open",
	entity: "Todo",
	allowRead: "true",
	allowInsert: "true",
	allowUpdate: "true",
	allowDelete: "true",
});

// ---------------------------------------------------------------------------
// Manifest
// ---------------------------------------------------------------------------

const manifest = buildManifest({
	name: "__APP_NAME_SNAKE__",
	version: "0.0.1",
	entities: [Todo],
	queries: [listTodos],
	actions: [addTodo, toggleTodo, deleteTodo, editTodo, reorderTodo],
	policies: [todoPolicy],
	routes: [],
});

// Not a debug leftover: the CLI runs `bun run` on this file and parses stdout
// as your manifest (see crates/cli/src/bun.rs). Deleting this line breaks every
// pylon command that reads your schema.
console.log(JSON.stringify(manifest));
