// Demo data for a brand-new workspace.
//
// An empty CRM teaches you nothing: no board, no forecast, no sense of what the
// thing is for. The first sign-in seeds a realistic pipeline so the app opens
// with something to look at, then never touches the data again.
//
// Pure data + pure shaping, so the seeding function stays a thin wrapper and
// this file can be edited (or emptied) without reading any server code.

export interface SeedCompany {
  key: string;
  name: string;
  domain: string;
  industry: string;
  size: string;
}

export interface SeedContact {
  company: string;
  name: string;
  email: string;
  title: string;
}

export interface SeedDeal {
  company: string;
  contact: string;
  title: string;
  value: number;
  stage: string;
  /** Days from today; negative is in the past. */
  closeInDays: number;
}

export interface SeedActivity {
  deal: string;
  kind: string;
  body: string;
  /** Hours ago. */
  age: number;
}

export const SEED_COMPANIES: SeedCompany[] = [
  { key: "northwind", name: "Northwind Logistics", domain: "northwind.co", industry: "Logistics", size: "200-500" },
  { key: "hallmark", name: "Hallmark Dental", domain: "hallmarkdental.com", industry: "Healthcare", size: "10-50" },
  { key: "riverbed", name: "Riverbed Coffee", domain: "riverbed.coffee", industry: "Food & Beverage", size: "50-200" },
  { key: "atlas", name: "Atlas Fitness", domain: "atlasfitness.io", industry: "Fitness", size: "10-50" },
  { key: "quarry", name: "Quarry Design Co", domain: "quarry.design", industry: "Agency", size: "1-10" },
  { key: "beacon", name: "Beacon Property", domain: "beaconproperty.com", industry: "Real Estate", size: "50-200" },
];

export const SEED_CONTACTS: SeedContact[] = [
  { company: "northwind", name: "Dana Whitfield", email: "dana@northwind.co", title: "VP Operations" },
  { company: "northwind", name: "Marcus Iyer", email: "marcus@northwind.co", title: "Head of IT" },
  { company: "hallmark", name: "Priya Raman", email: "priya@hallmarkdental.com", title: "Practice Manager" },
  { company: "riverbed", name: "Tom Alvarez", email: "tom@riverbed.coffee", title: "Founder" },
  { company: "atlas", name: "Simone Clarke", email: "simone@atlasfitness.io", title: "Owner" },
  { company: "quarry", name: "Ben Osei", email: "ben@quarry.design", title: "Principal" },
  { company: "beacon", name: "Lena Fischer", email: "lena@beaconproperty.com", title: "Director of Sales" },
];

export const SEED_DEALS: SeedDeal[] = [
  { company: "northwind", contact: "Dana Whitfield", title: "Fleet dispatch rollout", value: 48000, stage: "proposal", closeInDays: 12 },
  { company: "northwind", contact: "Marcus Iyer", title: "Warehouse scanner pilot", value: 9500, stage: "qualified", closeInDays: 26 },
  { company: "hallmark", contact: "Priya Raman", title: "Patient intake portal", value: 15200, stage: "won", closeInDays: -8 },
  { company: "riverbed", contact: "Tom Alvarez", title: "Multi-store inventory", value: 22000, stage: "proposal", closeInDays: 5 },
  { company: "atlas", contact: "Simone Clarke", title: "Class booking system", value: 7400, stage: "lead", closeInDays: 40 },
  { company: "quarry", contact: "Ben Osei", title: "Client portal build", value: 31000, stage: "qualified", closeInDays: 18 },
  { company: "beacon", contact: "Lena Fischer", title: "Listing sync integration", value: 64000, stage: "lead", closeInDays: 55 },
  { company: "beacon", contact: "Lena Fischer", title: "Agent CRM migration", value: 18500, stage: "lost", closeInDays: -21 },
];

export const SEED_ACTIVITIES: SeedActivity[] = [
  { deal: "Fleet dispatch rollout", kind: "call", body: "Walked Dana through the dispatch flow. Wants a security review before signing — sending the SOC 2 summary.", age: 5 },
  { deal: "Fleet dispatch rollout", kind: "email", body: "Sent revised proposal with the 3-year term and onboarding included.", age: 30 },
  { deal: "Patient intake portal", kind: "note", body: "Signed. Kickoff scheduled for the 14th — Priya is the day-to-day contact.", age: 190 },
  { deal: "Multi-store inventory", kind: "meeting", body: "Demo with Tom and the two store leads. Main question is offline mode at the market stall.", age: 48 },
  { deal: "Client portal build", kind: "note", body: "Ben wants SSO with their Google Workspace. Confirmed that's in scope.", age: 72 },
  { deal: "Listing sync integration", kind: "call", body: "Intro call. They're evaluating two other vendors; decision in Q3.", age: 96 },
];

export interface ShapedSeed {
  companies: Array<{ key: string; row: Record<string, unknown> }>;
  contacts: Array<{ company: string; row: Record<string, unknown> }>;
  deals: Array<{ company: string; contact: string; title: string; row: Record<string, unknown> }>;
  activities: Array<{ deal: string; row: Record<string, unknown> }>;
}

/**
 * Turn the fixtures into insertable rows with dates relative to `now`, so a
 * workspace seeded today shows deals closing next week rather than in 2024.
 */
export function shapeSeed(now: number = Date.now()): ShapedSeed {
  const at = (days: number) => new Date(now + days * 86_400_000).toISOString();
  const hoursAgo = (hours: number) => new Date(now - hours * 3_600_000).toISOString();

  return {
    companies: SEED_COMPANIES.map((c) => ({
      key: c.key,
      row: {
        name: c.name,
        domain: c.domain,
        industry: c.industry,
        size: c.size,
        createdAt: hoursAgo(240),
      },
    })),
    contacts: SEED_CONTACTS.map((c) => ({
      company: c.company,
      row: { name: c.name, email: c.email, title: c.title, createdAt: hoursAgo(200) },
    })),
    deals: SEED_DEALS.map((d) => ({
      company: d.company,
      contact: d.contact,
      title: d.title,
      row: {
        title: d.title,
        value: d.value,
        stage: d.stage,
        closeDate: at(d.closeInDays),
        createdAt: hoursAgo(160),
        updatedAt: hoursAgo(4),
      },
    })),
    activities: SEED_ACTIVITIES.map((a) => ({
      deal: a.deal,
      row: { kind: a.kind, body: a.body, createdAt: hoursAgo(a.age) },
    })),
  };
}
