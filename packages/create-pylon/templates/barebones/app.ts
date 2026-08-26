import {
  entity,
  field,
  policy,
  auth,
  buildManifest,
  discoverAppRoutes,
  discoverFunctions,
} from "@pylonsync/sdk";

// The smallest useful Pylon app: one entity, owned by its creator, with a
// live list and an optimistic create. `userId: field.owner()` stamps the
// signed-in (here: guest) user's id server-side and rejects any forged
// value, so the UI can do a plain `db.insert("Item", { text })` and the row
// stays unspoofably yours.
const Item = entity(
  "Item",
  {
    userId: field.string().owner(),
    text: field.string(),
    createdAt: field.datetime().defaultNow(),
  },
  { indexes: [{ name: "by_user", fields: ["userId"], unique: false }] },
);

// Items are private to their owner. An entity with NO policy is denied to
// clients by default, so this allow-list is what makes the live query +
// optimistic writes work — and only for your own rows.
const itemPolicy = policy({
  name: "item_access",
  entity: "Item",
  allowRead: "auth.userId == data.userId",
  allowInsert: "auth.userId != null",
  allowUpdate: "auth.userId == data.userId",
  allowDelete: "auth.userId == data.userId",
});

// The manifest is your whole app: data, policies, and the file-based routes
// under `app/`. `pylon dev` serves the SSR frontend and the API from one
// port. Add fields to `Item`, or a second `entity()`, and the typed client +
// REST/realtime API follow.
// Every non-internal function in functions/ becomes a manifest entry —
// this is what makes them show up in /api/manifest, the OpenAPI spec,
// and `pylon codegen`.
const fns = await discoverFunctions();

const manifest = buildManifest({
  name: "__APP_NAME__",
  version: "0.1.0",
  entities: [Item],
  queries: fns.queries,
  actions: fns.actions,
  policies: [itemPolicy],
  auth: auth(),
  routes: await discoverAppRoutes(),
});

// Not a debug leftover: the CLI runs `bun run app.ts` and parses stdout as
// your manifest (see crates/cli/src/bun.rs). Deleting this line breaks every
// pylon command that reads your schema.
console.log(JSON.stringify(manifest, null, 2));

export default manifest;
