import {
	entity,
	field,
	query,
	action,
	policy,
	buildManifest,
} from "@pylonsync/sdk";

// ---------------------------------------------------------------------------
// Schema — one entity, two operations. The smallest thing that
// demonstrates the loop: declare → handler → live query.
// ---------------------------------------------------------------------------

const Widget = entity("Widget", {
	name: field.string(),
	count: field.int(),
	createdAt: field.datetime(),
});

// ---------------------------------------------------------------------------
// Function declarations — names only. Implementations live under
// functions/<name>.ts and are auto-discovered by the runtime.
// ---------------------------------------------------------------------------

const listWidgets = query("listWidgets");

const createWidget = action("createWidget", {
	input: [{ name: "name", type: "string" }],
});

// ---------------------------------------------------------------------------
// Policies — wide-open by default. Tighten for production.
// ---------------------------------------------------------------------------

const widgetPolicy = policy({
	name: "widget_open",
	entity: "Widget",
	allowRead: "true",
	allowInsert: "true",
	allowUpdate: "true",
	allowDelete: "true",
});

// ---------------------------------------------------------------------------
// Manifest — pylon codegen reads this and emits pylon.manifest.json.
// pylon dev / pylon codegen run `bun run schema.ts` and read the
// manifest off stdout — every entry file ends with this console.log.
// ---------------------------------------------------------------------------

const manifest = buildManifest({
	name: "__APP_NAME_SNAKE__",
	version: "0.0.1",
	entities: [Widget],
	queries: [listWidgets],
	actions: [createWidget],
	policies: [widgetPolicy],
	routes: [],
});

console.log(JSON.stringify(manifest));
