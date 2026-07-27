// Demo data for a brand-new helpdesk.
//
// An empty inbox shows you nothing about triage, SLA, or the thread view. The
// first sign-in seeds a realistic queue — including one ticket already past its
// first-response window, so the breach state is visible rather than theoretical.
//
// Pure data + pure shaping; `functions/seedWorkspace.ts` stays a thin wrapper.

export interface SeedCustomer {
  key: string;
  name: string;
  email: string;
  company: string;
}

export interface SeedTicket {
  key: string;
  customer: string;
  subject: string;
  status: string;
  priority: string;
  /** Hours since it arrived. */
  age: number;
  /** Hours since the first agent reply; null means nobody has answered. */
  respondedAgo: number | null;
}

export interface SeedMessage {
  ticket: string;
  body: string;
  fromCustomer: boolean;
  internal?: boolean;
  /** Hours ago. */
  age: number;
}

export const SEED_CUSTOMERS: SeedCustomer[] = [
  { key: "dana", name: "Dana Whitfield", email: "dana@northwind.co", company: "Northwind Logistics" },
  { key: "tom", name: "Tom Alvarez", email: "tom@riverbed.coffee", company: "Riverbed Coffee" },
  { key: "priya", name: "Priya Raman", email: "priya@hallmarkdental.com", company: "Hallmark Dental" },
  { key: "ben", name: "Ben Osei", email: "ben@quarry.design", company: "Quarry Design Co" },
  { key: "lena", name: "Lena Fischer", email: "lena@beaconproperty.com", company: "Beacon Property" },
];

export const SEED_TICKETS: SeedTicket[] = [
  // Urgent, unanswered, 3h old against a 1h target — a live breach on first load.
  {
    key: "export",
    customer: "dana",
    subject: "Export is timing out on large date ranges",
    status: "open",
    priority: "urgent",
    age: 3,
    respondedAgo: null,
  },
  {
    key: "invite",
    customer: "tom",
    subject: "Can't invite a second manager to the account",
    status: "open",
    priority: "high",
    age: 6,
    respondedAgo: 5,
  },
  {
    key: "invoice",
    customer: "priya",
    subject: "Invoice shows last month's plan",
    status: "pending",
    priority: "normal",
    age: 26,
    respondedAgo: 24,
  },
  {
    key: "sso",
    customer: "ben",
    subject: "SSO with Google Workspace — is it supported?",
    status: "open",
    priority: "normal",
    age: 14,
    respondedAgo: null,
  },
  {
    key: "mobile",
    customer: "lena",
    subject: "Mobile layout cuts off the listing photos",
    status: "solved",
    priority: "low",
    age: 120,
    respondedAgo: 110,
  },
  {
    key: "webhook",
    customer: "dana",
    subject: "Webhook retries are firing twice",
    status: "closed",
    priority: "high",
    age: 300,
    respondedAgo: 290,
  },
];

export const SEED_MESSAGES: SeedMessage[] = [
  {
    ticket: "export",
    body: "Pulling a 90-day export just spins and eventually errors. 30 days is fine. This is blocking our month-end reporting.",
    fromCustomer: true,
    age: 3,
  },
  {
    ticket: "invite",
    body: "I added marcus@northwind.co but he never got the email and doesn't show in the members list.",
    fromCustomer: true,
    age: 6,
  },
  {
    ticket: "invite",
    body: "Thanks Tom — I can see the invite was created but the delivery bounced. Re-sending now and checking the domain's SPF record.",
    fromCustomer: false,
    age: 5,
  },
  {
    ticket: "invite",
    body: "Their MX is misconfigured — flagging for the infra team rather than sending again blind.",
    fromCustomer: false,
    internal: true,
    age: 5,
  },
  {
    ticket: "invoice",
    body: "We upgraded on the 3rd but the invoice still lists the old plan. Can you re-issue?",
    fromCustomer: true,
    age: 26,
  },
  {
    ticket: "invoice",
    body: "Re-issued and emailed — the proration lands on next month's invoice.",
    fromCustomer: false,
    age: 24,
  },
  {
    ticket: "sso",
    body: "We're standardising on Google Workspace this quarter. Does your plan include SAML, or only OAuth sign-in?",
    fromCustomer: true,
    age: 14,
  },
];

export interface ShapedSeed {
  customers: Array<{ key: string; row: Record<string, unknown> }>;
  tickets: Array<{ key: string; customer: string; row: Record<string, unknown> }>;
  messages: Array<{ ticket: string; row: Record<string, unknown> }>;
}

export function shapeSeed(now: number = Date.now()): ShapedSeed {
  const hoursAgo = (hours: number) => new Date(now - hours * 3_600_000).toISOString();

  return {
    customers: SEED_CUSTOMERS.map((c) => ({
      key: c.key,
      row: { name: c.name, email: c.email, company: c.company, createdAt: hoursAgo(720) },
    })),
    tickets: SEED_TICKETS.map((t) => ({
      key: t.key,
      customer: t.customer,
      row: {
        subject: t.subject,
        status: t.status,
        priority: t.priority,
        firstRespondedAt: t.respondedAgo === null ? null : hoursAgo(t.respondedAgo),
        createdAt: hoursAgo(t.age),
        updatedAt: hoursAgo(t.respondedAgo ?? t.age),
      },
    })),
    messages: SEED_MESSAGES.map((m) => ({
      ticket: m.ticket,
      row: {
        body: m.body,
        fromCustomer: m.fromCustomer,
        internal: m.internal ?? false,
        createdAt: hoursAgo(m.age),
      },
    })),
  };
}
