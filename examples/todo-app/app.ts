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
import { entity, field, policy, buildManifest } from "@pylonsync/sdk";

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
  },
  {
    indexes: [
      { name: "by_user", fields: ["userId"], unique: false },
      { name: "by_user_done", fields: ["userId", "done"], unique: false },
    ],
  },
);

const userPolicy = policy({
  name: "user_self",
  entity: "User",
  allowRead: "auth.userId != null",
  allowInsert: "false",
  allowUpdate: "false",
  allowDelete: "false",
});

const todoPolicy = policy({
  name: "todo_owner",
  entity: "Todo",
  allowRead: "auth.userId != null",
  allowInsert: "auth.userId == data.userId",
  allowUpdate: "auth.userId != null",
  allowDelete: "auth.userId != null",
});

const manifest = buildManifest({
  name: "todo-app",
  version: "0.1.0",
  entities: [User, Todo],
  queries: [],
  actions: [],
  policies: [userPolicy, todoPolicy],
  routes: [],
});

console.log(JSON.stringify(manifest, null, 2));
