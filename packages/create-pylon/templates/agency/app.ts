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
// agency — a site + back-office for a boutique studio that takes on a LIMITED
// number of projects at a time. The public site is a marketing page (hero,
// services, selected work, case studies, process, team, testimonials, contact);
// the owner dashboard is the studio's back-office (pipeline, portfolio, clients,
// invoices). The realtime hook is scarcity: the hero shows how many project
// slots are open this quarter, and the moment the owner books a client, that
// number drops for EVERYONE with the page open — no refresh.
//
// Entities, split by who can see them:
//
//   PUBLIC (marketing — read by anyone, written only by the owner's functions)
//   • Capacity — a single PII-FREE row: the booking window + open slots. Drives
//                the live "N slots open" hero counter.
//   • Project  — a portfolio piece + case study (title, summary, tags, the
//                challenge/approach/outcome write-up). `selected` flags the ones
//                featured on the homepage; `published` hides drafts. Public-read
//                so the marketing site + case-study pages render server-side.
//
//   PRIVATE (back-office — deny ALL client access; owner-gated functions only)
//   • Inquiry  — a "start a project" lead (name, email, company, budget, msg).
//   • Client   — a CRM contact (name, company, email, phone, notes, status).
//   • Invoice  — a bill tied to a client (+ optional project): amount, status,
//                issue/due dates. Money + client data, so it never leaves the
//                owner-gated functions.
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

// A portfolio piece + its case study. PUBLIC marketing content — no PII — so it
// reads publicly: the homepage shows the `selected` ones, /work lists every
// `published` one, and /work/[slug] renders the full case study, all
// server-side for SEO + first paint. Only the owner's functions write it
// (upsertProject / setProjectFlags / deleteProject / seedProjects), so a visitor
// can read the portfolio but never edit it.
//
//   • selected  — featured on the homepage "Selected work" grid. Toggling it in
//                 the dashboard re-curates the homepage.
//   • published — false = a draft, hidden from the public site (the owner still
//                 sees it in the dashboard, which reads every row).
//   • order     — manual sort (ascending) within the grids.
const Project = entity(
  "Project",
  {
    title: field.string(),
    slug: field.string(), // URL segment for /work/[slug]
    client: field.string().default(""), // display label, e.g. "Fintech · 0→1"
    summary: field.string().default(""), // one-liner on the card
    year: field.string().optional(), // "2026"
    tags: field.string().optional(), // comma-separated → chips
    selected: field.boolean().default(false), // featured on the homepage
    published: field.boolean().default(true), // false = draft, hidden publicly
    order: field.int().default(0), // manual sort
    // Case-study body — three short sections render on /work/[slug].
    challenge: field.string().optional(),
    approach: field.string().optional(),
    outcome: field.string().optional(),
    liveUrl: field.string().optional(),
    createdAt: field.datetime().defaultNow(),
  },
  {
    indexes: [
      { name: "by_slug", fields: ["slug"], unique: true },
      { name: "by_created", fields: ["createdAt"], unique: false },
    ],
  },
);

// A CRM contact — the people behind booked projects. Name/email/phone are PII,
// so it denies ALL client access; the dashboard reads + writes it only through
// the owner-gated functions (clientsForOwner / upsertClient / deleteClient).
const Client = entity(
  "Client",
  {
    name: field.string(),
    company: field.string().optional(),
    email: field.string().optional(),
    phone: field.string().optional(),
    status: field.string().default("prospect"), // "prospect" | "active" | "past"
    notes: field.string().optional(),
    createdAt: field.datetime().defaultNow(),
  },
  { indexes: [{ name: "by_created", fields: ["createdAt"], unique: false }] },
);

// A bill tied to a client (and optionally a project). This is MONEY plus client
// data, so like Client it denies all client access — invoices are owner-only,
// read + written through invoicesForOwner / upsertInvoice / setInvoiceStatus /
// deleteInvoice. `amountCents` is stored as integer cents to avoid float money
// bugs. `clientName` / `projectTitle` are denormalized for display so the list
// renders without extra joins.
const Invoice = entity(
  "Invoice",
  {
    number: field.string(), // "INV-001"
    clientId: field.string(),
    clientName: field.string().default(""),
    projectId: field.string().optional(),
    projectTitle: field.string().optional(),
    // Line items as a JSON-encoded array of { description, quantity, unitCents }
    // (Pylon has no JSON column type, so it's a string). `amountCents` is the
    // computed total, kept in sync on write so the list + totals don't have to
    // re-parse every row. The case-study PDF + the invoice view render the items.
    lineItems: field.string().optional(),
    amountCents: field.int().default(0),
    status: field.string().default("draft"), // "draft" | "sent" | "paid" | "overdue"
    issuedAt: field.string().optional(), // ISO date "2026-06-01"
    dueAt: field.string().optional(),
    notes: field.string().optional(),
    createdAt: field.datetime().defaultNow(),
  },
  {
    indexes: [
      { name: "by_client", fields: ["clientId"], unique: false },
      { name: "by_created", fields: ["createdAt"], unique: false },
    ],
  },
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

// Projects are PUBLIC to READ — they're marketing content (portfolio + case
// studies), so the site renders them server-side for SEO. Clients can't WRITE
// them: only the owner-gated functions do. Drafts (`published == false`) are
// filtered out in the public read paths (the homepage, /work, /work/[slug]);
// the dashboard reads every row. (We keep the read open rather than gating on
// `published` in the policy so the owner's dashboard can list drafts with the
// same live `db.useQuery` the public uses — simplest path, and a draft case
// study is not sensitive the way a lead or an invoice is.)
const projectPolicy = policy({
  name: "project_public_read",
  entity: "Project",
  allowRead: "true",
  allowInsert: "false",
  allowUpdate: "false",
  allowDelete: "false",
});

// Clients + Invoices are the studio's back-office: contact PII and money. Both
// deny ALL client access, exactly like Inquiry — they're never read or written
// over entity sync. The owner reaches them only through the owner-gated
// functions, which check PYLON_OWNER_EMAIL and use ctx.db.unsafe.
const clientPolicy = policy({
  name: "client_private",
  entity: "Client",
  allowRead: "false",
  allowInsert: "false",
  allowUpdate: "false",
  allowDelete: "false",
});

const invoicePolicy = policy({
  name: "invoice_private",
  entity: "Invoice",
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

const manifest = buildManifest({
  name: "__APP_NAME__",
  version: "0.1.0",
  entities: [Inquiry, Capacity, Project, Client, Invoice, User],
  // Functions live in functions/ and are discovered automatically:
  //   public:  submitInquiry, seedCapacity, seedProjects
  //   owner:   inquiriesForOwner, bookInquiry, declineInquiry, setCapacity,
  //            upsertProject, setProjectFlags, deleteProject,
  //            clientsForOwner, upsertClient, deleteClient,
  //            invoicesForOwner, upsertInvoice, setInvoiceStatus, deleteInvoice,
  //            seedStudioBackoffice
  queries: [],
  actions: [],
  policies: [
    inquiryPolicy,
    capacityPolicy,
    projectPolicy,
    clientPolicy,
    invoicePolicy,
    userPolicy,
  ],
  // Email/password is on by default against the User entity above. No orgs, no
  // billing — a single studio is single-tenant (one business, one owner).
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

console.log(JSON.stringify(manifest, null, 2));

export default manifest;
