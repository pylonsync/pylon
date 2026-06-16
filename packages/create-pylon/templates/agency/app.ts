import {
  entity,
  field,
  policy,
  auth,
  buildManifest,
  discoverAppRoutes,
} from "@pylonsync/sdk";

// ---------------------------------------------------------------------------
// agency — a site for a boutique studio that takes on a LIMITED number of
// projects at a time. The realtime hook is scarcity: the hero shows how many
// project slots are open this quarter, and the moment the owner books a new
// client from the dashboard, that number drops for EVERYONE with the page
// open — no refresh. Open it in two tabs to see it.
//
// Three entities:
//   • Inquiry  — a "start a project" lead, with the prospect's name, email,
//                company + budget + message. Pure PII → denies ALL client
//                reads/writes. The public site never reads an Inquiry; the
//                owner sees them only through the owner-gated inquiriesForOwner.
//   • Capacity — a single, PII-FREE row the public page reads live: the current
//                booking period + how many project slots are open. This is what
//                makes the hero counter realtime. Booking an inquiry decrements
//                it; the owner can reset it any time.
//   • User     — the studio owner's account for the dashboard.
// ---------------------------------------------------------------------------

// A project inquiry. Everything here is PII or commercially sensitive, so the
// policy denies all client access; the only way in is the submitInquiry
// mutation, the only way to read is the owner-gated query. `status` tracks the
// pipeline: "new" → "booked" (consumes a slot) | "declined".
const Inquiry = entity(
  "Inquiry",
  {
    name: field.string(),
    email: field.string(),
    company: field.string().optional(),
    projectType: field.string().optional(),
    budget: field.string().optional(),
    message: field.string().optional(),
    status: field.string().default("new"), // "new" | "booked" | "declined"
    createdAt: field.datetime().defaultNow(),
  },
  { indexes: [{ name: "by_created", fields: ["createdAt"], unique: false }] },
);

// A single-row, PII-FREE aggregate the public page reads live. It holds only
// the current booking window label + the number of open project slots — no lead
// data. seedCapacity creates it from config on first visit; bookInquiry /
// setCapacity keep it current. The landing page subscribes with
// `db.useQuery("Capacity")`, so the "N slots open" counter ticks down across
// every open tab the instant the owner books someone — the cross-tab-safe
// realtime primitive (entity sync), not a per-connection server subscription.
const Capacity = entity(
  "Capacity",
  {
    label: field.string().default(""), // e.g. "Q3 2026"
    openSlots: field.int().default(0),
    updatedAt: field.datetime().defaultNow(),
  },
  {},
);

// The studio owner's account. Email/password auth is built in against an entity
// named "User" (passwordHash is server-only). The dashboard is gated to the
// owner — see PYLON_OWNER_EMAIL in lib/owner.ts + the owner-only functions.
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

// PRIVACY — Inquiry holds the prospect's name, email, company, and budget, so
// it denies EVERY client read and write. No `db.useQuery("Inquiry")` can pull a
// row; no client can write one directly. Writes happen only inside the
// submitInquiry / bookInquiry / declineInquiry mutations (functions bypass
// policies); reads happen only inside the owner-gated inquiriesForOwner. A
// studio site must never leak who's been talking to it — this guarantees it.
const inquiryPolicy = policy({
  name: "inquiry_private",
  entity: "Inquiry",
  allowRead: "false",
  allowInsert: "false",
  allowUpdate: "false",
  allowDelete: "false",
});

// The capacity row is public to READ (it's just a label + a number — the whole
// point is the landing page showing open slots live to everyone). Clients can't
// WRITE it; only seedCapacity / bookInquiry / setCapacity maintain it server-side.
const capacityPolicy = policy({
  name: "capacity_public_read",
  entity: "Capacity",
  allowRead: "true",
  allowInsert: "false",
  allowUpdate: "false",
  allowDelete: "false",
});

const userPolicy = policy({
  name: "user_self",
  entity: "User",
  allowRead: "auth.userId == data.id",
  allowInsert: "false",
  allowUpdate: "false",
  allowDelete: "false",
});

const manifest = buildManifest({
  name: "__APP_NAME__",
  version: "0.1.0",
  entities: [Inquiry, Capacity, User],
  // submitInquiry / seedCapacity (public) + inquiriesForOwner / bookInquiry /
  // declineInquiry / setCapacity (owner-gated) live in functions/ and are
  // discovered automatically — they don't need listing here.
  queries: [],
  actions: [],
  policies: [inquiryPolicy, capacityPolicy, userPolicy],
  // Email/password is on by default against the User entity above. No orgs, no
  // billing — a single studio is single-tenant (one business, one owner).
  auth: auth(),
  routes: await discoverAppRoutes(),
});

console.log(JSON.stringify(manifest, null, 2));

export default manifest;
