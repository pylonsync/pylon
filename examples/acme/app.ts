import { buildManifest, discoverAppRoutes, entity, field, policy } from "@pylonsync/sdk";

// Acme — the default Pylon startup template.
//
// All pages are server-rendered from app/**/page.tsx, layouts
// compose via app/**/layout.tsx, styles compile from app/globals.css
// via the bundler's Tailwind v4 integration. Drop this template
// into a fresh repo, customize the copy, ship.
//
// Functions in functions/*.ts are auto-discovered by the runtime —
// no explicit import here needed. submitLead is auth:"guest" and
// inserts a Lead row, wired to the early-access + contact forms.

// Leads: inbound interest from the landing page and /contact form.
// Guest-accessible insert (no login needed), admin-only read.
const Lead = entity("Lead", {
  email: field.string(),
  source: field.string(),
  message: field.string(),
  createdAt: field.string(),
});

// Guests can submit their email; nobody reads leads through the sync
// client (only server-side admin tooling does). Default-deny is the
// framework baseline — this policy opens only insert.
const leadPolicy = policy({
  name: "lead_submit",
  entity: "Lead",
  allowInsert: "true",
  allowRead: "false",
  allowUpdate: "false",
  allowDelete: "false",
});

const manifest = buildManifest({
  name: "acme",
  version: "0.1.0",
  entities: [Lead],
  queries: [],
  actions: [],
  policies: [leadPolicy],
  routes: await discoverAppRoutes(),
});

console.log(JSON.stringify(manifest, null, 2));

export default manifest;
