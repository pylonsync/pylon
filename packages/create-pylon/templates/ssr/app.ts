import {
  entity,
  field,
  policy,
  auth,
  buildManifest,
  discoverAppRoutes,
} from "@pylonsync/sdk";

// Accounts. Email/password auth is built in: POST /api/auth/password/register
// hashes the password and writes this row; /api/auth/password/login mints a
// session and sets an HttpOnly cookie. The framework treats the entity named
// "User" as the account table — `passwordHash` is server-only and never
// serialized to a client.
const User = entity(
  "User",
  {
    email: field.string(),
    displayName: field.string().optional(),
    passwordHash: field.string().serverOnly().optional(),
    // The framework's /api/auth/password/register stamps a generated avatar
    // color here, so the User entity must declare it.
    avatarColor: field.string().optional(),
    createdAt: field.datetime().defaultNow(),
  },
  { indexes: [{ name: "by_email", fields: ["email"], unique: true }] },
);

// A note that belongs to one user. `ownerId: field.owner()` is the key move:
// the framework stamps the signed-in user's id server-side on insert and
// rejects any forged value — so the dashboard can do a plain, optimistic
// `db.insert("Note", { body })` (the row shows instantly, no round-trip) while
// the owner stays unspoofable. No createNote function to write.
const Note = entity(
  "Note",
  {
    ownerId: field.string().owner(),
    body: field.string(),
    done: field.boolean().default(false),
    createdAt: field.datetime().defaultNow(),
  },
  { indexes: [{ name: "by_owner", fields: ["ownerId"], unique: false }] },
);

// Notes are private — every read and write is gated to the owner. An entity
// with NO policy is denied to clients by default, so this is exactly what
// makes the dashboard's live query + optimistic writes work, and only for
// your own rows. `auth.userId` is the session user; `data.ownerId` is the row.
const notePolicy = policy({
  name: "note_access",
  entity: "Note",
  allowRead: "auth.userId == data.ownerId",
  allowInsert: "auth.userId != null",
  allowUpdate: "auth.userId == data.ownerId",
  allowDelete: "auth.userId == data.ownerId",
});

// User rows are read-only to clients, and only your own (so the dashboard can
// read your display name). The auth subsystem owns writes — registration and
// login go through /api/auth/password/*, never the entity API.
const userPolicy = policy({
  name: "user_access",
  entity: "User",
  allowRead: "auth.userId == data.id",
  allowInsert: "false",
  allowUpdate: "false",
  allowDelete: "false",
});

// The manifest is your whole app in one object: data, policies, and the
// file-based routes under `app/`. `pylon dev` reads this, serves the SSR
// frontend and the API from one port, and regenerates a typed client on
// every change.
const manifest = buildManifest({
  name: "__APP_NAME__",
  version: "0.1.0",
  entities: [User, Note],
  queries: [],
  actions: [],
  policies: [userPolicy, notePolicy],
  // Email/password is on by default against the User entity above. `auth()`
  // is the knob for session lifetime, exposed fields, orgs, and trusted
  // origins — `auth({ session: { expiresIn: 60 * 60 * 24 * 7 } })` for a
  // 7-day session, etc.
  auth: auth(),
  // File-based routing: `discoverAppRoutes()` walks `app/**/page.tsx` and
  // emits one route per page. Drop `app/about/page.tsx` to add `/about`.
  routes: await discoverAppRoutes(),
});

// Emit canonical manifest JSON to stdout for `pylon codegen`.
console.log(JSON.stringify(manifest, null, 2));

export default manifest;
