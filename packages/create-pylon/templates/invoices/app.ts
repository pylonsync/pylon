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
// invoices — billing for a small business: clients, invoices with line items,
// and payments against them.
//
// The realtime hook: recording a payment updates the balance and flips the
// status everywhere at once, so two people chasing the same overdue invoice
// stop the moment one of them marks it paid.
//
//   • Client   — who gets billed. Holds email + address, so never public.
//   • Invoice  — number, dates, tax rate, status. Totals are DERIVED.
//   • LineItem — description, quantity, unit price. Belongs to one invoice.
//   • Payment  — an amount received against an invoice.
//   • User     — a team member (email/password).
//
// MONEY IS INTEGER CENTS. Floating-point dollars is how an invoice ends up a
// penny off after tax, and a penny on a bill is a support ticket. Quantities
// are thousandths for the same reason. See lib/billing.ts.
//
// Totals and "overdue" are NOT stored. An invoice becomes overdue by the clock
// passing; deriving it means no nightly job has to flip a column, and the
// number on screen always matches the line items you can see.
//
// TRUST MODEL: an internal tool for ONE business. Every entity is readable and
// writable by any SIGNED-IN user and by nobody else. There is no client portal —
// adding one means a separate, narrower policy for that client's own invoices,
// NOT loosening these.
// ---------------------------------------------------------------------------

const TEAM = "auth.userId != null";

const Client = entity(
  "Client",
  {
    name: field.string(),
    email: field.string().optional(),
    address: field.string().optional(),
    // Appears on the invoice — VAT number, company registration, whatever the
    // jurisdiction wants.
    taxId: field.string().optional(),
    createdAt: field.datetime().defaultNow(),
  },
  { indexes: [{ name: "by_name", fields: ["name"], unique: false }] },
);

const Invoice = entity(
  "Invoice",
  {
    // Human-facing series, e.g. INV-2026-0007. Unique so two drafts can't claim
    // the same number — an accountant WILL ask about a duplicate.
    number: field.string().unique(),
    clientId: field.id("Client").optional(),
    // "draft" | "sent" | "paid" | "void" — "overdue" is derived, not stored.
    status: field.string().default("draft"),
    // Basis points: 875 = 8.75%. Integer, like every other money field.
    taxRateBps: field.number().default(0),
    issueDate: field.datetime().optional(),
    dueDate: field.datetime().optional(),
    notes: field.string().optional(),
    ownerId: field.id("User").readonly().optional(),
    createdAt: field.datetime().defaultNow(),
    updatedAt: field.datetime().defaultNow(),
  },
  {
    indexes: [
      { name: "by_status", fields: ["status"], unique: false },
      { name: "by_client", fields: ["clientId"], unique: false },
    ],
  },
);

const LineItem = entity(
  "LineItem",
  {
    invoiceId: field.id("Invoice"),
    description: field.string(),
    // Thousandths of a unit: 1.5 hours is 1500.
    quantityMilli: field.number().default(1000),
    unitPriceCents: field.number().default(0),
    position: field.number().default(0),
  },
  { indexes: [{ name: "by_invoice", fields: ["invoiceId"], unique: false }] },
);

const Payment = entity(
  "Payment",
  {
    invoiceId: field.id("Invoice"),
    amountCents: field.number(),
    // "bank" | "card" | "cash" | "other" — free text; every business differs.
    method: field.string().optional(),
    reference: field.string().optional(),
    paidAt: field.datetime().defaultNow(),
    recordedBy: field.id("User").readonly().optional(),
  },
  { indexes: [{ name: "by_invoice", fields: ["invoiceId"], unique: false }] },
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

const userPolicy = policy({
  name: "user_team_read",
  entity: "User",
  allowRead: TEAM,
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
  entities: [Client, Invoice, LineItem, Payment, User],
  queries: fns.queries,
  actions: fns.actions,
  policies: [
    teamPolicy("client_team", "Client"),
    teamPolicy("invoice_team", "Invoice"),
    teamPolicy("line_item_team", "LineItem"),
    teamPolicy("payment_team", "Payment"),
    userPolicy,
  ],
  auth: auth(),
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
