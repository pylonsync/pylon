import {
  entity,
  field,
  policy,
  auth,
  buildManifest,
  discoverAppRoutes,
  font,
} from "@pylonsync/sdk";

// ---------------------------------------------------------------------------
// crm — a sales CRM for a small team: companies, contacts, deals moving through
// a pipeline, and an activity log.
//
// The realtime hook: the board subscribes to Deal, so when a teammate drags a
// deal to "Won" it moves on everyone's board instantly. No refresh, and no
// standup spent reconciling two people's idea of the forecast.
//
//   • Company  — the account. Contacts and deals hang off it.
//   • Contact  — a person at a company. Holds email/phone, so never public.
//   • Deal     — the opportunity: value, stage, expected close. The pipeline.
//   • Activity — a note/call/email/meeting logged against a deal or contact.
//   • User     — a team member (email/password).
//
// TRUST MODEL: an internal tool for ONE team. Every entity is readable and
// writable by any SIGNED-IN user, and by nobody else — a CRM's whole point is a
// shared pipeline. `ownerId` records who created a row (for "my deals" and
// accountability) without partitioning access. For per-rep isolation, tighten
// allowRead to "auth.userId == data.ownerId" and add a manager role.
// ---------------------------------------------------------------------------

// Signed-in team members only. Deliberately not `"true"`: an anonymous visitor
// must never read the customer list, and `pylon lint` flags wide-open policies
// for exactly this reason.
const TEAM = "auth.userId != null";

const Company = entity(
  "Company",
  {
    name: field.string(),
    domain: field.string().optional(),
    industry: field.string().optional(),
    // Free text ("1-10", "50-200") rather than an enum — headcount bands vary
    // by who you ask, and a CRM shouldn't argue with its user about it.
    size: field.string().optional(),
    notes: field.string().optional(),
    ownerId: field.id("User").readonly().optional(),
    createdAt: field.datetime().defaultNow(),
  },
  { indexes: [{ name: "by_name", fields: ["name"], unique: false }] },
);

const Contact = entity(
  "Contact",
  {
    companyId: field.id("Company").optional(),
    name: field.string(),
    email: field.string().optional(),
    phone: field.string().optional(),
    title: field.string().optional(),
    ownerId: field.id("User").readonly().optional(),
    createdAt: field.datetime().defaultNow(),
  },
  { indexes: [{ name: "by_company", fields: ["companyId"], unique: false }] },
);

// `stage` drives the board; PIPELINE in lib/pipeline.ts owns the ordered stages
// and which of them count as closed.
const Deal = entity(
  "Deal",
  {
    title: field.string(),
    companyId: field.id("Company").optional(),
    contactId: field.id("Contact").optional(),
    // Whole currency units, not cents — a forecast is read by humans, and this
    // keeps the card, the column total, and the input in one unit.
    value: field.number().default(0),
    stage: field.string().default("lead"),
    closeDate: field.datetime().optional(),
    ownerId: field.id("User").readonly().optional(),
    createdAt: field.datetime().defaultNow(),
    updatedAt: field.datetime().defaultNow(),
  },
  { indexes: [{ name: "by_stage", fields: ["stage"], unique: false }] },
);

// The timeline. Attached to a deal, a contact, or both — logging a call against
// a contact shouldn't require inventing a deal to hang it on.
const Activity = entity(
  "Activity",
  {
    // "note" | "call" | "email" | "meeting" — see ACTIVITY_KINDS.
    kind: field.string().default("note"),
    body: field.string(),
    dealId: field.id("Deal").optional(),
    contactId: field.id("Contact").optional(),
    companyId: field.id("Company").optional(),
    ownerId: field.id("User").readonly().optional(),
    createdAt: field.datetime().defaultNow(),
  },
  { indexes: [{ name: "by_deal", fields: ["dealId"], unique: false }] },
);

const User = entity(
  "User",
  {
    email: field.string(),
    displayName: field.string().optional(),
    passwordHash: field.string().serverOnly().optional(),
    avatarColor: field.string().optional(),
    emailVerified: field.datetime().optional(),
    createdAt: field.datetime().defaultNow(),
  },
  { indexes: [{ name: "by_email", fields: ["email"], unique: true }] },
);

const teamPolicy = (name: string, entityName: string) =>
  policy({
    name,
    entity: entityName,
    allowRead: TEAM,
    allowInsert: TEAM,
    allowUpdate: TEAM,
    allowDelete: TEAM,
  });

// Team members read each other's rows so a deal can show who owns it. Nobody
// writes User through the client — auth owns that, and passwordHash is
// serverOnly regardless.
const userPolicy = policy({
  name: "user_team_read",
  entity: "User",
  allowRead: TEAM,
  allowInsert: "false",
  allowUpdate: "false",
  allowDelete: "false",
});

const manifest = buildManifest({
  name: "__APP_NAME__",
  version: "0.1.0",
  entities: [Company, Contact, Deal, Activity, User],
  queries: [],
  actions: [],
  policies: [
    teamPolicy("company_team", "Company"),
    teamPolicy("contact_team", "Contact"),
    teamPolicy("deal_team", "Deal"),
    teamPolicy("activity_team", "Activity"),
    userPolicy,
  ],
  auth: auth(),
  // Self-hosted Inter: the build fetches the woff2, serves it same-origin (no
  // third-party request, no FOUT), preloads it, and synthesizes a size-adjusted
  // fallback so there's no layout shift.
  fonts: [
    font({
      family: "Inter",
      variable: "--font-sans",
      weights: ["400", "500", "600", "700"],
      subsets: ["latin"],
      display: "swap",
      preload: true,
    }),
  ],
  routes: await discoverAppRoutes(),
});

console.log(JSON.stringify(manifest, null, 2));

export default manifest;
