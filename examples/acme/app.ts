import { buildManifest, discoverAppRoutes, entity, field, font, policy } from "@pylonsync/sdk";

// Acme — the default Pylon startup template.
//
// Pages server-render from app/**/page.tsx, layouts compose via
// app/**/layout.tsx, styles compile from app/globals.css (Tailwind v4).
// Functions in functions/*.ts are auto-discovered by the runtime —
// no explicit import here needed.

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
  // Self-hosted Inter (next/font parity) — served same-origin, preloaded,
  // with a size-adjusted fallback; globals.css reads it via --font-sans.
  fonts: [
    font({
      family: "Inter",
      variable: "--font-sans",
      weights: ["400", "500", "600", "700", "800"],
      subsets: ["latin"],
      display: "swap",
      preload: true,
    }),
  ],
  routes: await discoverAppRoutes(),
});

console.log(JSON.stringify(manifest, null, 2));

export default manifest;
