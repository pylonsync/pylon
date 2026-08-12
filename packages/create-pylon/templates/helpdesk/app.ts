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
// helpdesk — a support inbox for a small team: customers write in, agents triage
// by priority and SLA, and every reply lands on a shared thread.
//
// The realtime hook: the queue subscribes to Ticket, so a new ticket appears at
// the top of everyone's inbox as it arrives, and an assignment shows up before
// two agents start the same reply.
//
//   • Customer — the person who wrote in. Holds email, so never public.
//   • Ticket   — subject, status, priority, assignee. The queue.
//   • Message  — one entry on a thread; `internal` marks an agent-only note.
//   • User     — an agent (email/password).
//
// TRUST MODEL: an internal tool for ONE support team. Every entity is readable
// and writable by any SIGNED-IN user and by nobody else. There is no customer
// portal here — adding one means a separate, narrower policy scoped to the
// customer's own tickets, NOT loosening these.
// ---------------------------------------------------------------------------

const TEAM = "auth.userId != null";

const Customer = entity(
  "Customer",
  {
    name: field.string(),
    email: field.string(),
    company: field.string().optional(),
    createdAt: field.datetime().defaultNow(),
  },
  { indexes: [{ name: "by_email", fields: ["email"], unique: false }] },
);

const Ticket = entity(
  "Ticket",
  {
    subject: field.string(),
    // "open" | "pending" | "solved" | "closed" — see STATUSES in lib/tickets.ts.
    status: field.string().default("open"),
    // "urgent" | "high" | "normal" | "low" — sets the first-response window.
    priority: field.string().default("normal"),
    customerId: field.id("Customer").optional(),
    assigneeId: field.id("User").optional(),
    // When an agent first replied. Null means the customer is still waiting,
    // which is what the SLA state is computed from.
    firstRespondedAt: field.datetime().optional(),
    createdAt: field.datetime().defaultNow(),
    updatedAt: field.datetime().defaultNow(),
  },
  {
    indexes: [
      { name: "by_status", fields: ["status"], unique: false },
      { name: "by_assignee", fields: ["assigneeId"], unique: false },
    ],
  },
);

const Message = entity(
  "Message",
  {
    ticketId: field.id("Ticket"),
    body: field.string(),
    // True when the customer wrote it rather than an agent.
    fromCustomer: field.boolean().default(false),
    // An internal note stays with the team if you later add a customer portal.
    internal: field.boolean().default(false),
    authorId: field.id("User").readonly().optional(),
    createdAt: field.datetime().defaultNow(),
  },
  { indexes: [{ name: "by_ticket", fields: ["ticketId"], unique: false }] },
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
  entities: [Customer, Ticket, Message, User],
  queries: fns.queries,
  actions: fns.actions,
  policies: [
    teamPolicy("customer_team", "Customer"),
    teamPolicy("ticket_team", "Ticket"),
    teamPolicy("message_team", "Message"),
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
