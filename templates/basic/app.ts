import { entity, field, defineRoute, buildManifest } from "./sdk";

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
// Manifest
// ---------------------------------------------------------------------------

const manifest = buildManifest({
  name: "__APP_NAME__",
  version: "0.1.0",
  entities: [Post],
  routes: [home, postBySlug],
});

// Emit canonical manifest JSON to stdout.
// Used by: statecraft codegen app.ts
console.log(JSON.stringify(manifest, null, 2));
