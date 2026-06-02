import { buildManifest, discoverAppRoutes, entity, field } from "@pylonsync/sdk";

// Your data model. Every `entity()` becomes a synced table with a REST +
// realtime API and a typed client — no migrations to write, no resolvers
// to wire. Add fields here and they show up everywhere.
const Note = entity("Note", {
  body: field.string(),
  done: field.boolean().default(false),
});

// The manifest is your whole app in one object: data, server functions,
// access policies, and the file-based routes under `app/`. `pylon dev`
// reads this, serves the SSR frontend and the API from one port, and
// regenerates a typed client on every change.
//
// File-based routing: `discoverAppRoutes()` walks `app/**/page.tsx` and
// emits one route per page. Drop `app/about/page.tsx` to add `/about` —
// no route table to maintain.
const manifest = buildManifest({
  name: "__APP_NAME__",
  version: "0.1.0",
  entities: [Note],
  queries: [],
  actions: [],
  policies: [],
  routes: await discoverAppRoutes(),
});

// Emit canonical manifest JSON to stdout for `pylon codegen`.
console.log(JSON.stringify(manifest, null, 2));

export default manifest;
