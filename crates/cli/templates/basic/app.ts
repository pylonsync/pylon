import { entity, field, policy, defineRoute, buildManifest } from "./sdk";

// ---------------------------------------------------------------------------
// Schema
// ---------------------------------------------------------------------------

const Post = entity("Post", {
  title: field.string(),
  slug: field.string().unique(),
  body: field.richtext(),
  publishedAt: field.datetime().optional(),
});

// ---------------------------------------------------------------------------
// Routes
// ---------------------------------------------------------------------------

const home = defineRoute({
  path: "/",
  mode: "server",
});

const postBySlug = defineRoute({
  path: "/posts/:slug",
  mode: "static",
});

// ---------------------------------------------------------------------------
// Policies
//
// Pylon is SECURE BY DEFAULT: an entity with NO policy is denied — every read
// and write returns 403 until you declare access explicitly. (That's on
// purpose: it stops you from accidentally shipping a table wide open.) This
// policy makes posts publicly readable but writable only by a signed-in user.
// Edit the `allow*` expressions — they can reference `auth` (the caller) and
// `data` (the row) — e.g. `allowUpdate: "auth.userId == data.authorId"`.
// ---------------------------------------------------------------------------

const postPolicy = policy({
  name: "post_access",
  entity: "Post",
  allowRead: "true",
  allowInsert: "auth.userId != null",
  allowUpdate: "auth.userId != null",
  allowDelete: "auth.userId != null",
});

// ---------------------------------------------------------------------------
// Manifest
// ---------------------------------------------------------------------------

const manifest = buildManifest({
  name: "__APP_NAME__",
  version: "0.1.0",
  entities: [Post],
  policies: [postPolicy],
  routes: [home, postBySlug],
});

// Emit canonical manifest JSON to stdout.
// Used by: pylon codegen app.ts
console.log(JSON.stringify(manifest, null, 2));
