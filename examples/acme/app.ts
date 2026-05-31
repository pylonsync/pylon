import { buildManifest, discoverAppRoutes, entity, field } from "@pylonsync/sdk";

// Acme — the default Pylon startup template.
//
// All pages are server-rendered from app/**/page.tsx, layouts
// compose via app/**/layout.tsx, styles compile from app/globals.css
// via the bundler's Tailwind v4 integration. Drop this template
// into a fresh repo, customize the copy, ship.

const Lead = entity("Lead", {
  email: field.string(),
  source: field.string(),
  message: field.string(),
});

const manifest = buildManifest({
  name: "acme",
  version: "0.1.0",
  entities: [Lead],
  queries: [],
  actions: [],
  policies: [],
  routes: await discoverAppRoutes(),
});

console.log(JSON.stringify(manifest, null, 2));

export default manifest;
