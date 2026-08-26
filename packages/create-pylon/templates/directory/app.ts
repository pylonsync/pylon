import {
  entity,
  field,
  policy,
  auth,
  buildManifest,
  discoverAppRoutes,
  discoverFunctions,
  font,
} from "@pylonsync/sdk";

// ---------------------------------------------------------------------------
// directory — a curated, searchable listing site (a "best X" / tools / local
// directory). It's the template that shows off Pylon's FULL-TEXT SEARCH +
// FACETS: the browse page is a live `db.useSearch` over the Listing table —
// type in the box and results + facet counts update instantly, all server-
// backed, no search service. The realtime hook is live upvotes: tap the arrow
// and the count ticks up for EVERYONE with the page open (no refresh).
//
// Three entities:
//   • Listing    — an approved, public entry: name, what it is, the link, a
//                  category + tags, and a live vote count. PII-free, so it's
//                  PUBLIC-READ and indexed for search. Only functions write it.
//   • Submission — a "submit your listing" lead. Holds the submitter's name +
//                  email (PII) alongside the proposed entry, so it denies ALL
//                  client reads/writes. The owner reviews submissions and
//                  approves them into public Listings; the email never leaks.
//   • User       — the curator/owner's account for the moderation dashboard.
// ---------------------------------------------------------------------------

// A public directory entry. No PII — everything here is meant to be browsed.
// The `search:` block tells Pylon to build FTS5 + facet shadow tables: `text`
// fields are full-text indexed, `facets` get live count sidebars, `sortable`
// fields can order results. `db.useSearch("Listing", …)` drives the browse UI.
const Listing = entity(
  "Listing",
  {
    name: field.string(),
    tagline: field.string(),
    url: field.string(), // the link this entry points to
    category: field.string(),
    tags: field.string().optional(), // comma-separated, shown as chips
    description: field.string().optional(),
    votes: field.int().default(0),
    featured: field.boolean().default(false),
    createdAt: field.datetime().defaultNow(),
  },
  {
    indexes: [
      { name: "by_category", fields: ["category"], unique: false },
      { name: "by_votes", fields: ["votes"], unique: false },
      { name: "by_created", fields: ["createdAt"], unique: false },
    ],
    search: {
      text: ["name", "tagline", "description"],
      facets: ["category"],
      sortable: ["votes", "createdAt"],
    },
  },
);

// A pending submission. PII (submitter email) + un-vetted content, so it's
// deny-all; submitListing writes it, the owner reads it via submissionsForOwner.
// `status` tracks moderation: "new" → "approved" (becomes a Listing) | "rejected".
const Submission = entity(
  "Submission",
  {
    submitterName: field.string(),
    submitterEmail: field.string(),
    name: field.string(),
    tagline: field.string(),
    url: field.string(),
    category: field.string(),
    tags: field.string().optional(),
    description: field.string().optional(),
    status: field.string().default("new"), // "new" | "approved" | "rejected"
    createdAt: field.datetime().defaultNow(),
  },
  { indexes: [{ name: "by_created", fields: ["createdAt"], unique: false }] },
);

// The curator's account. Email/password auth is built in against "User"
// (passwordHash is server-only). The dashboard is gated to PYLON_OWNER_EMAIL.
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

// Listings are PUBLIC-READ — the whole point is a browsable, searchable
// directory. Clients can't WRITE them; only seedListings / approveSubmission
// create them and the upvote mutation bumps the vote count, all server-side.
const listingPolicy = policy({
  name: "listing_public_read",
  entity: "Listing",
  allowRead: "true",
  allowInsert: "false",
  allowUpdate: "false",
  allowDelete: "false",
});

// PRIVACY — Submission holds the submitter's name + email, so it denies EVERY
// client read and write. No `db.useQuery("Submission")` can pull a row; writes
// happen only inside submitListing / approveSubmission / rejectSubmission; reads
// only inside the owner-gated submissionsForOwner. The directory must never leak
// who submitted what — this guarantees it.
const submissionPolicy = policy({
  name: "submission_private",
  entity: "Submission",
  allowRead: "false",
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

// Every non-internal function in functions/ becomes a manifest entry —
// this is what makes them show up in /api/manifest, the OpenAPI spec,
// and `pylon codegen`.
const fns = await discoverFunctions();

const manifest = buildManifest({
  name: "__APP_NAME__",
  version: "0.1.0",
  entities: [Listing, Submission, User],
  // seedListings / submitListing / upvote (public) + submissionsForOwner /
  // approveSubmission / rejectSubmission (owner-gated) live in functions/ and
  // are discovered automatically.
  queries: fns.queries,
  actions: fns.actions,
  policies: [listingPolicy, submissionPolicy, userPolicy],
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

// Not a debug leftover: the CLI runs `bun run app.ts` and parses stdout as
// your manifest (see crates/cli/src/bun.rs). Deleting this line breaks every
// pylon command that reads your schema.
console.log(JSON.stringify(manifest, null, 2));

export default manifest;
