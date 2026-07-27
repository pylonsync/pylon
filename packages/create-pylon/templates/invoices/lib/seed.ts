// Demo data for a brand-new billing workspace.
//
// An empty invoice list shows you nothing about totals, ageing, or partial
// payment. The first sign-in seeds a realistic book — including one invoice
// already overdue and one part-paid — so those states are visible rather than
// theoretical.
//
// Pure data + pure shaping; `functions/seedWorkspace.ts` stays a thin wrapper.

export interface SeedClient {
  key: string;
  name: string;
  email: string;
  address: string;
}

export interface SeedLine {
  description: string;
  /** Whole units; shaped into thousandths. */
  quantity: number;
  /** Whole dollars; shaped into cents. */
  unitPrice: number;
}

export interface SeedInvoice {
  key: string;
  client: string;
  number: string;
  status: string;
  taxRateBps: number;
  /** Days from today; negative is in the past. */
  issuedInDays: number;
  dueInDays: number;
  lines: SeedLine[];
  /** Dollars already received; 0 means nothing paid. */
  paid?: number;
  paidInDays?: number;
}

export const SEED_CLIENTS: SeedClient[] = [
  {
    key: "northwind",
    name: "Northwind Logistics",
    email: "ap@northwind.co",
    address: "412 Dock Road\nRotterdam, NL",
  },
  {
    key: "hallmark",
    name: "Hallmark Dental",
    email: "billing@hallmarkdental.com",
    address: "88 Mercer Street\nAustin, TX 78701",
  },
  {
    key: "riverbed",
    name: "Riverbed Coffee",
    email: "tom@riverbed.coffee",
    address: "5 Mill Lane\nPortland, OR 97209",
  },
  {
    key: "quarry",
    name: "Quarry Design Co",
    email: "ben@quarry.design",
    address: "19 Foundry Street\nManchester, UK",
  },
];

export const SEED_INVOICES: SeedInvoice[] = [
  // Overdue: sent, past due, nothing paid.
  {
    key: "overdue",
    client: "northwind",
    number: "INV-2026-0001",
    status: "sent",
    taxRateBps: 0,
    issuedInDays: -52,
    dueInDays: -22,
    lines: [
      { description: "Dispatch integration — discovery", quantity: 1, unitPrice: 4800 },
      { description: "Senior engineering", quantity: 32, unitPrice: 165 },
    ],
  },
  // Part-paid: a payment against a larger balance.
  {
    key: "partial",
    client: "hallmark",
    number: "INV-2026-0002",
    status: "sent",
    taxRateBps: 875,
    issuedInDays: -18,
    dueInDays: 12,
    lines: [
      { description: "Patient intake portal — build", quantity: 1, unitPrice: 12_400 },
      { description: "Training session", quantity: 2, unitPrice: 450 },
    ],
    paid: 5000,
    paidInDays: -6,
  },
  // Settled.
  {
    key: "paid",
    client: "riverbed",
    number: "INV-2026-0003",
    status: "paid",
    taxRateBps: 0,
    issuedInDays: -40,
    dueInDays: -10,
    lines: [{ description: "Inventory sync — monthly", quantity: 1, unitPrice: 1800 }],
    paid: 1800,
    paidInDays: -12,
  },
  // Still being written.
  {
    key: "draft",
    client: "quarry",
    number: "INV-2026-0004",
    status: "draft",
    taxRateBps: 2000,
    issuedInDays: 0,
    dueInDays: 30,
    lines: [
      { description: "Client portal — phase 1", quantity: 1, unitPrice: 9500 },
      { description: "SSO configuration", quantity: 4, unitPrice: 180 },
    ],
  },
];

export interface ShapedSeed {
  clients: Array<{ key: string; row: Record<string, unknown> }>;
  invoices: Array<{ key: string; client: string; row: Record<string, unknown> }>;
  lines: Array<{ invoice: string; row: Record<string, unknown> }>;
  payments: Array<{ invoice: string; row: Record<string, unknown> }>;
}

export function shapeSeed(now: number = Date.now()): ShapedSeed {
  const at = (days: number) => new Date(now + days * 86_400_000).toISOString();

  const lines: ShapedSeed["lines"] = [];
  const payments: ShapedSeed["payments"] = [];

  for (const invoice of SEED_INVOICES) {
    invoice.lines.forEach((line, index) => {
      lines.push({
        invoice: invoice.key,
        row: {
          description: line.description,
          quantityMilli: Math.round(line.quantity * 1000),
          unitPriceCents: Math.round(line.unitPrice * 100),
          position: index,
        },
      });
    });
    if (invoice.paid) {
      payments.push({
        invoice: invoice.key,
        row: {
          amountCents: Math.round(invoice.paid * 100),
          method: "bank",
          paidAt: at(invoice.paidInDays ?? 0),
        },
      });
    }
  }

  return {
    clients: SEED_CLIENTS.map((c) => ({
      key: c.key,
      row: {
        name: c.name,
        email: c.email,
        address: c.address,
        createdAt: at(-90),
      },
    })),
    invoices: SEED_INVOICES.map((i) => ({
      key: i.key,
      client: i.client,
      row: {
        number: i.number,
        status: i.status,
        taxRateBps: i.taxRateBps,
        issueDate: at(i.issuedInDays),
        dueDate: at(i.dueInDays),
        createdAt: at(i.issuedInDays),
        updatedAt: at(i.paidInDays ?? i.issuedInDays),
      },
    })),
    lines,
    payments,
  };
}
