import {
  entity,
  field,
  policy,
  auth,
  buildManifest,
  discoverAppRoutes,
  discoverFunctions,
} from "@pylonsync/sdk";

// A todo that belongs to one person. `userId: field.owner()` stamps the
// signed-in (here: guest) user's id server-side on insert and rejects any
// forged value — so the UI can do a plain, optimistic `db.insert("Todo",
// { title })` and never send (or spoof) userId. No createTodo function to
// write — every verb is a direct, policy-checked entity call.
const Todo = entity(
  "Todo",
  {
    userId: field.string().owner(),
    title: field.string(),
    done: field.boolean().default(false),
    // Float sort key so a drag-reorder can drop a row between two others by
    // taking their midpoint — no renumbering the whole list. New rows seed
    // with createdAt so the default order is chronological.
    sortKey: field.float().optional(),
    createdAt: field.datetime().defaultNow(),
  },
  { indexes: [{ name: "by_user", fields: ["userId"], unique: false }] },
);

// Todos are private — every read and write is gated to the owner. An entity
// with NO policy is denied to clients by default, so this is exactly what
// makes the live query + optimistic writes work, and only for your own rows.
// `auth.userId` is the session user (a guest id here); `data.userId` is the
// row's owner.
const todoPolicy = policy({
  name: "todo_access",
  entity: "Todo",
  allowRead: "auth.userId == data.userId",
  allowInsert: "auth.userId != null",
  allowUpdate: "auth.userId == data.userId",
  allowDelete: "auth.userId == data.userId",
});

// The manifest is your whole app in one object: data, policies, and the
// file-based routes under `app/`. `pylon dev` reads this, serves the SSR
// frontend and the API from one port, and regenerates a typed client on
// every change.
//
// This starter uses guest sessions: the page wraps its UI in `<EnsureGuest>`,
// which POSTs `/api/auth/guest` on first load so every visitor implicitly
// becomes their own user — no login wall, todos still private per browser.
// To require real accounts instead, enable email/password (it's built in
// against a `User` entity) and swap `<EnsureGuest>` for `<SignedIn>` /
// `<SignedOut>` from `@pylonsync/client`.
// Every non-internal function in functions/ becomes a manifest entry —
// this is what makes them show up in /api/manifest, the OpenAPI spec,
// and `pylon codegen`.
const fns = await discoverFunctions();

const manifest = buildManifest({
  name: "__APP_NAME__",
  version: "0.1.0",
  entities: [Todo],
  queries: fns.queries,
  actions: fns.actions,
  policies: [todoPolicy],
  auth: auth(),
  // File-based routing: `discoverAppRoutes()` walks `app/**/page.tsx` and
  // emits one route per page. Drop `app/about/page.tsx` to add `/about`.
  routes: await discoverAppRoutes(),
});

// Emit canonical manifest JSON to stdout for `pylon codegen`.
console.log(JSON.stringify(manifest, null, 2));

export default manifest;
