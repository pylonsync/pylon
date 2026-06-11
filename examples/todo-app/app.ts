/**
 * Pylon Todo — the canonical Pylon hello-world.
 *
 * Smallest possible app that exercises every core primitive:
 *   - Entity declaration with indexes
 *   - Per-user policies (`auth.userId == data.userId`)
 *   - Live queries via `db.useQuery`
 *   - Optimistic mutations via `db.useEntity`
 *   - Email/password auth out of the box
 *
 * No server-side functions are needed — todos are CRUD'd directly
 * through `/api/entities/Todo`, with policies enforcing ownership.
 */
import {
  entity,
  field,
  policy,
  buildManifest,
  discoverAppRoutes,
} from "@pylonsync/sdk";

const User = entity(
  "User",
  {
    email: field.string(),
    displayName: field.string(),
    avatarColor: field.string().optional(),
    passwordHash: field.string().optional(),
    createdAt: field.datetime(),
  },
  {
    indexes: [{ name: "by_email", fields: ["email"], unique: true }],
  },
);

const Todo = entity(
  "Todo",
  {
    userId: field.string(),
    title: field.string(),
    notes: field.string().optional(),
    done: field.bool(),
    priority: field.string(), // "low" | "med" | "high"
    dueAt: field.datetime().optional(),
    completedAt: field.datetime().optional(),
    createdAt: field.datetime(),
    // Manual sort key — set by the client when reordering via DnD.
    // Floats let inserts between two adjacent rows reuse the midpoint
    // without renumbering the whole list (`(a.sortKey + b.sortKey) / 2`).
    // New todos seed with createdAt's epoch millis so the default
    // ordering still feels chronological.
    sortKey: field.float().optional(),
  },
  {
    indexes: [
      { name: "by_user", fields: ["userId"], unique: false },
      { name: "by_user_done", fields: ["userId", "done"], unique: false },
    ],
  },
);

// You can only read your own User row. Scoping read to `data.id` (the row's
// primary key) keeps one user's email/profile from leaking to every other
// logged-in user over `/api/entities/User`.
const userPolicy = policy({
  name: "user_self",
  entity: "User",
  allowRead: "auth.userId == data.id",
  allowInsert: "false",
  allowUpdate: "false",
  allowDelete: "false",
});

// Every Todo is private to its owner. ALL four verbs scope to
// `auth.userId == data.userId` — `auth.userId != null` (any logged-in user)
// would let any account read, edit, and delete everyone else's todos. The
// read scope matters most: the list endpoint enforces it per row, so a peer's
// `GET /api/entities/Todo` returns only their own rows.
const todoPolicy = policy({
  name: "todo_owner",
  entity: "Todo",
  allowRead: "auth.userId == data.userId",
  allowInsert: "auth.userId == data.userId",
  allowUpdate: "auth.userId == data.userId",
  allowDelete: "auth.userId == data.userId",
});

const manifest = buildManifest({
  name: "todo-app",
  version: "0.1.0",
  entities: [User, Todo],
  queries: [],
  actions: [],
  policies: [userPolicy, todoPolicy],
  // File-based SSR routing: app/page.tsx → "/". The single binary serves
  // the frontend and the API on one port — no separate Next.js app.
  routes: await discoverAppRoutes(),
});

console.log(JSON.stringify(manifest, null, 2));
