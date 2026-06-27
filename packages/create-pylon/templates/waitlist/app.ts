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
// waitlist — a pre-launch / coming-soon landing page with a LIVE signup
// counter. The realtime hook: open the page in two tabs, submit an email in
// one, and the counter on the other ticks up with no refresh.
//
// The data model is deliberately tiny — three entities:
//   • Signup       — one row per email. Holds visitor PII, so it denies ALL
//                    client reads/writes (writes go through the joinWaitlist
//                    mutation; the public page only ever sees an aggregate
//                    count, never an email).
//   • WaitlistStat — a single-row, PII-free aggregate (just the count) the
//                    public page reads live for the counter.
//   • User         — the business owner's account (email/password is built in),
//                    so the owner can sign in to the dashboard and see signups.
// ---------------------------------------------------------------------------

// One waitlist signup. `email` is the only PII; `createdAt` powers the
// signups-over-time chart on the dashboard. The unique index on email dedupes
// at the database level — a duplicate insert is rejected even under a race, so
// joinWaitlist can treat the conflict as "already joined".
const Signup = entity(
  "Signup",
  {
    email: field.string(),
    createdAt: field.datetime().defaultNow(),
  },
  {
    indexes: [
      { name: "by_email", fields: ["email"], unique: true },
      { name: "by_created", fields: ["createdAt"], unique: false },
    ],
  },
);

// A single-row, PII-FREE aggregate the public page can safely read live. It
// holds only the signup count — no emails. `joinWaitlist` keeps it in sync with
// the real Signup count on every new join. The landing page subscribes with
// `db.useQuery("WaitlistStat")`, which syncs across every open tab through the
// replica — so the counter ticks up everywhere the instant someone joins. This
// is the cross-tab-safe realtime primitive (entity sync), not a per-connection
// server-query subscription.
const WaitlistStat = entity(
  "WaitlistStat",
  {
    count: field.int().default(0),
    updatedAt: field.datetime().defaultNow(),
  },
  {},
);

// The business owner's account. Email/password auth is built in against an
// entity named "User" (passwordHash is server-only; the register route stamps
// avatarColor). The dashboard is gated to the owner — see PYLON_OWNER_EMAIL in
// functions/waitlistStats.ts and app/dashboard/page.tsx.
const User = entity(
  "User",
  {
    email: field.string(),
    displayName: field.string().optional(),
    passwordHash: field.string().serverOnly().optional(),
    avatarColor: field.string().optional(),
    // Set when the owner verifies their email; unused by the waitlist flow but
    // declared so the framework's email-verification routes have a column.
    emailVerified: field.datetime().optional(),
    createdAt: field.datetime().defaultNow(),
  },
  { indexes: [{ name: "by_email", fields: ["email"], unique: true }] },
);

// PRIVACY — the heart of the spec. Signup holds visitor emails, so it denies
// EVERY client read and write. No `db.useQuery("Signup")` can ever pull a row,
// and no client can insert/update/delete directly. Writes happen only inside
// the server-side `joinWaitlist` mutation (functions bypass policies); reads
// happen only inside `waitlistCount` (returns a bare integer) and the
// owner-gated `waitlistStats`. A marketing site must never leak its own
// customers' emails — this policy is what guarantees it.
const signupPolicy = policy({
  name: "signup_private",
  entity: "Signup",
  allowRead: "false",
  allowInsert: "false",
  allowUpdate: "false",
  allowDelete: "false",
});

// The aggregate count is public to READ (it's just a number — the whole point
// is that the landing page shows it live to everyone). Clients can't WRITE it;
// only the joinWaitlist mutation maintains it server-side.
const waitlistStatPolicy = policy({
  name: "waitlist_stat_public_read",
  entity: "WaitlistStat",
  allowRead: "true",
  allowInsert: "false",
  allowUpdate: "false",
  allowDelete: "false",
});

// The owner reads their own User row (the dashboard resolves their email this
// way to check ownership). The auth subsystem owns all writes.
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
  entities: [Signup, WaitlistStat, User],
  // joinWaitlist (public mutation) + waitlistStats (owner-gated query) live in
  // functions/ and are discovered automatically — they don't need listing here.
  queries: [],
  actions: [],
  policies: [signupPolicy, waitlistStatPolicy, userPolicy],
  // Email/password is on by default against the User entity above. No orgs,
  // no billing — a waitlist is single-tenant (one business, one owner).
  auth: auth(),
  // Self-hosted Inter (next/font parity): the build fetches the woff2, serves it
  // same-origin (no third-party request, no FOUT), preloads it, and synthesizes a
  // size-adjusted fallback face so there's no layout shift. globals.css reads it
  // via `var(--font-sans, …)`; layout.tsx carries no font <link>.
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

// Emit the canonical manifest JSON to stdout — `pylon dev` captures this.
console.log(JSON.stringify(manifest, null, 2));

export default manifest;
