// Demo data for a brand-new billing workspace.
//
// The first sign-in seeds a working book: a dozen clients and half a year of
// invoices in every status. Several are overdue, a few are part-paid, and the
// most recent ones are still drafts, so those states are visible rather than
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
  status: string;
  taxRateBps: number;
  /** Days from today; negative is in the past. */
  issuedInDays: number;
  dueInDays: number;
  lines: SeedLine[];
  /** Dollars already received; 0 means nothing paid. */
  paid?: number;
  paidInDays?: number;
  method?: string;
}

export const SEED_CLIENTS: SeedClient[] = [
  { key: "northwind", name: "Northwind Logistics", email: "ap@northwind.co", address: "412 Dock Road\nRotterdam, NL" },
  { key: "hallmark", name: "Hallmark Dental", email: "billing@hallmarkdental.com", address: "88 Mercer Street\nAustin, TX 78701" },
  { key: "riverbed", name: "Riverbed Coffee", email: "tom@riverbed.coffee", address: "5 Mill Lane\nPortland, OR 97209" },
  { key: "quarry", name: "Quarry Design Co", email: "ben@quarry.design", address: "19 Foundry Street\nManchester, UK" },
  { key: "larkspur", name: "Larkspur Clinics", email: "accounts@larkspurclinics.com", address: "1200 Westgate Avenue\nDenver, CO 80204" },
  { key: "meridian", name: "Meridian Freight", email: "payables@meridianfreight.com", address: "7 Harbour Way\nSouthampton, UK" },
  { key: "kestrel", name: "Kestrel Aviation Services", email: "finance@kestrelaviation.com", address: "Hangar 4, Field Road\nWichita, KS 67209" },
  { key: "orchard", name: "Orchard Learning", email: "finance@orchardlearning.org", address: "30 Chapel Street\nBristol, UK" },
  { key: "bluefin", name: "Bluefin Analytics", email: "ap@bluefin.io", address: "550 Bryant Street\nSan Francisco, CA 94107" },
  { key: "harbor", name: "Harbor Point Hotels", email: "invoices@harborpointhotels.com", address: "2 Seaport Boulevard\nBoston, MA 02210" },
  { key: "cobalt", name: "Cobalt Staffing", email: "accounts@cobaltstaffing.com", address: "14 King Street West\nToronto, ON M5H 1A1" },
  { key: "vantage", name: "Vantage Solar", email: "ap@vantagesolar.com", address: "900 Solar Drive\nPhoenix, AZ 85004" },
];

const ENGINEERING = "Senior engineering";
const DESIGN = "Product design";
const SUPPORT = "Support retainer";

export const SEED_INVOICES: SeedInvoice[] = [
  // Six months ago: all settled.
  { key: "i01", client: "kestrel", status: "paid", taxRateBps: 0, issuedInDays: -180, dueInDays: -150, lines: [{ description: "Parts inventory system, phase 1", quantity: 1, unitPrice: 18_000 }, { description: ENGINEERING, quantity: 40, unitPrice: 165 }], paid: 24_600, paidInDays: -158, method: "bank" },
  { key: "i02", client: "cobalt", status: "paid", taxRateBps: 1300, issuedInDays: -176, dueInDays: -146, lines: [{ description: "Timesheet approvals, build", quantity: 1, unitPrice: 9_800 }], paid: 11_074, paidInDays: -140, method: "card" },
  { key: "i03", client: "riverbed", status: "paid", taxRateBps: 0, issuedInDays: -170, dueInDays: -140, lines: [{ description: "Inventory sync, monthly", quantity: 1, unitPrice: 1_800 }], paid: 1_800, paidInDays: -138, method: "bank" },
  { key: "i04", client: "orchard", status: "paid", taxRateBps: 2000, issuedInDays: -165, dueInDays: -135, lines: [{ description: "Enrolment forms", quantity: 1, unitPrice: 6_400 }, { description: "Training session", quantity: 1, unitPrice: 450 }], paid: 8_220, paidInDays: -130, method: "bank" },

  // Five months ago.
  { key: "i05", client: "kestrel", status: "paid", taxRateBps: 0, issuedInDays: -150, dueInDays: -120, lines: [{ description: "Parts inventory system, phase 2", quantity: 1, unitPrice: 16_500 }, { description: ENGINEERING, quantity: 24, unitPrice: 165 }], paid: 20_460, paidInDays: -118, method: "bank" },
  { key: "i06", client: "riverbed", status: "paid", taxRateBps: 0, issuedInDays: -140, dueInDays: -110, lines: [{ description: "Inventory sync, monthly", quantity: 1, unitPrice: 1_800 }], paid: 1_800, paidInDays: -111, method: "bank" },
  { key: "i07", client: "bluefin", status: "paid", taxRateBps: 0, issuedInDays: -138, dueInDays: -108, lines: [{ description: "Reporting discovery workshop", quantity: 2, unitPrice: 2_400 }], paid: 4_800, paidInDays: -100, method: "card" },
  { key: "i08", client: "vantage", status: "paid", taxRateBps: 860, issuedInDays: -132, dueInDays: -102, lines: [{ description: "Site survey app, build", quantity: 1, unitPrice: 14_200 }, { description: DESIGN, quantity: 18, unitPrice: 140 }], paid: 18_149.52, paidInDays: -95, method: "bank" },
  { key: "i09", client: "larkspur", status: "void", taxRateBps: 0, issuedInDays: -128, dueInDays: -98, lines: [{ description: "Referral tracking, phase 1", quantity: 1, unitPrice: 12_000 }] },

  // Four months ago.
  { key: "i10", client: "larkspur", status: "paid", taxRateBps: 0, issuedInDays: -126, dueInDays: -96, lines: [{ description: "Referral tracking, phase 1 (reissued)", quantity: 1, unitPrice: 12_000 }, { description: ENGINEERING, quantity: 30, unitPrice: 165 }], paid: 16_950, paidInDays: -90, method: "bank" },
  { key: "i11", client: "riverbed", status: "paid", taxRateBps: 0, issuedInDays: -110, dueInDays: -80, lines: [{ description: "Inventory sync, monthly", quantity: 1, unitPrice: 1_800 }], paid: 1_800, paidInDays: -82, method: "bank" },
  { key: "i12", client: "meridian", status: "paid", taxRateBps: 2000, issuedInDays: -105, dueInDays: -75, lines: [{ description: "Carrier onboarding, discovery", quantity: 1, unitPrice: 7_500 }], paid: 9_000, paidInDays: -60, method: "bank" },
  { key: "i13", client: "harbor", status: "paid", taxRateBps: 625, issuedInDays: -100, dueInDays: -70, lines: [{ description: "Guest messaging pilot, setup", quantity: 1, unitPrice: 11_000 }, { description: "SMS credits", quantity: 5000, unitPrice: 0.04 }], paid: 11_900, paidInDays: -68, method: "bank" },
  { key: "i14", client: "cobalt", status: "paid", taxRateBps: 1300, issuedInDays: -96, dueInDays: -66, lines: [{ description: SUPPORT, quantity: 1, unitPrice: 1_200 }], paid: 1_356, paidInDays: -64, method: "card" },

  // Three months ago.
  { key: "i15", client: "larkspur", status: "paid", taxRateBps: 0, issuedInDays: -92, dueInDays: -62, lines: [{ description: "Referral tracking, phase 2", quantity: 1, unitPrice: 9_500 }, { description: ENGINEERING, quantity: 22, unitPrice: 165 }], paid: 13_130, paidInDays: -55, method: "bank" },
  { key: "i16", client: "riverbed", status: "paid", taxRateBps: 0, issuedInDays: -80, dueInDays: -50, lines: [{ description: "Inventory sync, monthly", quantity: 1, unitPrice: 1_800 }], paid: 1_800, paidInDays: -52, method: "bank" },
  { key: "i17", client: "bluefin", status: "paid", taxRateBps: 0, issuedInDays: -78, dueInDays: -48, lines: [{ description: "Embedded reporting, prototype", quantity: 1, unitPrice: 8_800 }, { description: DESIGN, quantity: 12, unitPrice: 140 }], paid: 10_480, paidInDays: -40, method: "bank" },
  { key: "i18", client: "vantage", status: "paid", taxRateBps: 860, issuedInDays: -74, dueInDays: -44, lines: [{ description: SUPPORT, quantity: 1, unitPrice: 1_500 }], paid: 1_629, paidInDays: -42, method: "card" },
  { key: "i19", client: "harbor", status: "sent", taxRateBps: 625, issuedInDays: -70, dueInDays: -40, lines: [{ description: "Guest messaging pilot, month 1", quantity: 1, unitPrice: 4_500 }, { description: "SMS credits", quantity: 12000, unitPrice: 0.04 }] },

  // Two months ago. The first overdue invoices start here.
  { key: "i20", client: "northwind", status: "sent", taxRateBps: 0, issuedInDays: -52, dueInDays: -22, lines: [{ description: "Dispatch integration, discovery", quantity: 1, unitPrice: 4_800 }, { description: ENGINEERING, quantity: 32, unitPrice: 165 }] },
  { key: "i21", client: "meridian", status: "sent", taxRateBps: 2000, issuedInDays: -50, dueInDays: -20, lines: [{ description: "Carrier onboarding, build sprint 1", quantity: 1, unitPrice: 14_000 }, { description: ENGINEERING, quantity: 16, unitPrice: 165 }], paid: 10_000, paidInDays: -15, method: "bank" },
  { key: "i22", client: "riverbed", status: "sent", taxRateBps: 0, issuedInDays: -50, dueInDays: -20, lines: [{ description: "Inventory sync, monthly", quantity: 1, unitPrice: 1_800 }] },
  { key: "i23", client: "cobalt", status: "paid", taxRateBps: 1300, issuedInDays: -48, dueInDays: -18, lines: [{ description: SUPPORT, quantity: 1, unitPrice: 1_200 }], paid: 1_356, paidInDays: -20, method: "card" },
  { key: "i24", client: "quarry", status: "paid", taxRateBps: 2000, issuedInDays: -45, dueInDays: -15, lines: [{ description: "Client portal, discovery", quantity: 1, unitPrice: 3_200 }], paid: 3_840, paidInDays: -16, method: "bank" },
  { key: "i25", client: "orchard", status: "sent", taxRateBps: 2000, issuedInDays: -40, dueInDays: -10, lines: [{ description: "Parent communication hub, pilot", quantity: 1, unitPrice: 7_800 }, { description: "Training session", quantity: 2, unitPrice: 450 }] },

  // This month.
  { key: "i26", client: "hallmark", status: "sent", taxRateBps: 875, issuedInDays: -18, dueInDays: 12, lines: [{ description: "Patient intake portal, build", quantity: 1, unitPrice: 12_400 }, { description: "Training session", quantity: 2, unitPrice: 450 }], paid: 5_000, paidInDays: -6, method: "bank" },
  { key: "i27", client: "riverbed", status: "sent", taxRateBps: 0, issuedInDays: -20, dueInDays: 10, lines: [{ description: "Inventory sync, monthly", quantity: 1, unitPrice: 1_800 }] },
  { key: "i28", client: "bluefin", status: "sent", taxRateBps: 0, issuedInDays: -14, dueInDays: 16, lines: [{ description: "Embedded reporting, sprint 1", quantity: 1, unitPrice: 12_000 }, { description: DESIGN, quantity: 20, unitPrice: 140 }] },
  { key: "i29", client: "vantage", status: "sent", taxRateBps: 860, issuedInDays: -12, dueInDays: 18, lines: [{ description: SUPPORT, quantity: 1, unitPrice: 1_500 }, { description: "Installer scheduling, discovery", quantity: 1, unitPrice: 2_800 }] },
  { key: "i30", client: "kestrel", status: "sent", taxRateBps: 0, issuedInDays: -9, dueInDays: 21, lines: [{ description: "Maintenance log, discovery", quantity: 1, unitPrice: 6_000 }, { description: ENGINEERING, quantity: 12, unitPrice: 165 }] },
  { key: "i31", client: "harbor", status: "sent", taxRateBps: 625, issuedInDays: -8, dueInDays: 22, lines: [{ description: "Guest messaging pilot, month 2", quantity: 1, unitPrice: 4_500 }, { description: "SMS credits", quantity: 15000, unitPrice: 0.04 }] },
  { key: "i32", client: "cobalt", status: "sent", taxRateBps: 1300, issuedInDays: -5, dueInDays: 25, lines: [{ description: SUPPORT, quantity: 1, unitPrice: 1_200 }, { description: "Candidate dashboard, discovery", quantity: 1, unitPrice: 3_600 }] },

  // Still being written.
  { key: "i33", client: "quarry", status: "draft", taxRateBps: 2000, issuedInDays: 0, dueInDays: 30, lines: [{ description: "Client portal, phase 1", quantity: 1, unitPrice: 9_500 }, { description: "SSO configuration", quantity: 4, unitPrice: 180 }] },
  { key: "i34", client: "northwind", status: "draft", taxRateBps: 0, issuedInDays: 0, dueInDays: 30, lines: [{ description: "Fleet dispatch rollout, sprint 1", quantity: 1, unitPrice: 16_000 }, { description: ENGINEERING, quantity: 36, unitPrice: 165 }] },
  { key: "i35", client: "meridian", status: "draft", taxRateBps: 2000, issuedInDays: 0, dueInDays: 30, lines: [{ description: "Carrier onboarding, build sprint 2", quantity: 1, unitPrice: 14_000 }] },
  { key: "i36", client: "larkspur", status: "draft", taxRateBps: 0, issuedInDays: 0, dueInDays: 30, lines: [{ description: "Multi-site scheduling, discovery", quantity: 1, unitPrice: 8_500 }, { description: "Site visits", quantity: 3, unitPrice: 600 }] },
];

export interface ShapedSeed {
  clients: Array<{ key: string; row: Record<string, unknown> }>;
  invoices: Array<{ key: string; client: string; row: Record<string, unknown> }>;
  lines: Array<{ invoice: string; row: Record<string, unknown> }>;
  payments: Array<{ invoice: string; row: Record<string, unknown> }>;
}

export function shapeSeed(now: number = Date.now()): ShapedSeed {
  const at = (days: number) => new Date(now + days * 86_400_000).toISOString();
  const year = new Date(now).getUTCFullYear();

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
          method: invoice.method ?? "bank",
          paidAt: at(invoice.paidInDays ?? 0),
        },
      });
    }
  }

  return {
    clients: SEED_CLIENTS.map((c, index) => ({
      key: c.key,
      row: {
        name: c.name,
        email: c.email,
        address: c.address,
        createdAt: at(-200 + index * 12),
      },
    })),
    invoices: SEED_INVOICES.map((i, index) => ({
      key: i.key,
      client: i.client,
      row: {
        number: `INV-${year}-${String(index + 1).padStart(4, "0")}`,
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
