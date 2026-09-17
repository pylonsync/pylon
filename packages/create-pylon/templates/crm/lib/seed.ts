// Demo data for a brand-new workspace.
//
// The first sign-in seeds a working pipeline: two dozen companies, a contact or
// two at each, deals in every stage, and a few weeks of activity. The app
// opens with something to look at, then never touches the data again.
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
  /** Days since the deal was created. */
  age: number;
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
  { key: "larkspur", name: "Larkspur Clinics", domain: "larkspurclinics.com", industry: "Healthcare", size: "200-500" },
  { key: "meridian", name: "Meridian Freight", domain: "meridianfreight.com", industry: "Logistics", size: "500+" },
  { key: "copperline", name: "Copperline Brewing", domain: "copperline.beer", industry: "Food & Beverage", size: "10-50" },
  { key: "tidewater", name: "Tidewater Marine", domain: "tidewatermarine.com", industry: "Manufacturing", size: "50-200" },
  { key: "sable", name: "Sable & Finch", domain: "sableandfinch.com", industry: "Retail", size: "10-50" },
  { key: "orchard", name: "Orchard Learning", domain: "orchardlearning.org", industry: "Education", size: "50-200" },
  { key: "granite", name: "Granite Peak Outfitters", domain: "granitepeak.com", industry: "Retail", size: "10-50" },
  { key: "fernhill", name: "Fernhill Veterinary", domain: "fernhillvet.com", industry: "Healthcare", size: "10-50" },
  { key: "kestrel", name: "Kestrel Aviation Services", domain: "kestrelaviation.com", industry: "Aviation", size: "200-500" },
  { key: "bluefin", name: "Bluefin Analytics", domain: "bluefin.io", industry: "Software", size: "50-200" },
  { key: "harbor", name: "Harbor Point Hotels", domain: "harborpointhotels.com", industry: "Hospitality", size: "500+" },
  { key: "wren", name: "Wren Architecture", domain: "wren.studio", industry: "Agency", size: "10-50" },
  { key: "summit", name: "Summit Credit Union", domain: "summitcu.org", industry: "Finance", size: "200-500" },
  { key: "pinecrest", name: "Pinecrest Farms", domain: "pinecrestfarms.com", industry: "Agriculture", size: "50-200" },
  { key: "vantage", name: "Vantage Solar", domain: "vantagesolar.com", industry: "Energy", size: "50-200" },
  { key: "ironwood", name: "Ironwood Furniture", domain: "ironwoodfurniture.com", industry: "Manufacturing", size: "10-50" },
  { key: "cobalt", name: "Cobalt Staffing", domain: "cobaltstaffing.com", industry: "Recruiting", size: "50-200" },
  { key: "halcyon", name: "Halcyon Senior Living", domain: "halcyonliving.com", industry: "Healthcare", size: "200-500" },
];

export const SEED_CONTACTS: SeedContact[] = [
  { company: "northwind", name: "Dana Whitfield", email: "dana@northwind.co", title: "VP Operations" },
  { company: "northwind", name: "Marcus Iyer", email: "marcus@northwind.co", title: "Head of IT" },
  { company: "hallmark", name: "Priya Raman", email: "priya@hallmarkdental.com", title: "Practice Manager" },
  { company: "riverbed", name: "Tom Alvarez", email: "tom@riverbed.coffee", title: "Founder" },
  { company: "riverbed", name: "Jess Nakamura", email: "jess@riverbed.coffee", title: "Operations Lead" },
  { company: "atlas", name: "Simone Clarke", email: "simone@atlasfitness.io", title: "Owner" },
  { company: "quarry", name: "Ben Osei", email: "ben@quarry.design", title: "Principal" },
  { company: "beacon", name: "Lena Fischer", email: "lena@beaconproperty.com", title: "Director of Sales" },
  { company: "beacon", name: "Owen Hart", email: "owen@beaconproperty.com", title: "IT Manager" },
  { company: "larkspur", name: "Dr. Amara Bello", email: "abello@larkspurclinics.com", title: "Medical Director" },
  { company: "larkspur", name: "Kevin Doyle", email: "kdoyle@larkspurclinics.com", title: "COO" },
  { company: "meridian", name: "Rachel Stone", email: "rstone@meridianfreight.com", title: "VP Technology" },
  { company: "meridian", name: "Luis Herrera", email: "lherrera@meridianfreight.com", title: "Procurement Manager" },
  { company: "copperline", name: "Mike Brennan", email: "mike@copperline.beer", title: "Co-founder" },
  { company: "tidewater", name: "Sarah Lindqvist", email: "sarah@tidewatermarine.com", title: "Plant Manager" },
  { company: "sable", name: "Isabel Moreau", email: "isabel@sableandfinch.com", title: "Owner" },
  { company: "orchard", name: "David Park", email: "dpark@orchardlearning.org", title: "Director of Technology" },
  { company: "granite", name: "Hannah Weiss", email: "hannah@granitepeak.com", title: "General Manager" },
  { company: "fernhill", name: "Dr. Chris Okafor", email: "chris@fernhillvet.com", title: "Owner" },
  { company: "kestrel", name: "James Whitaker", email: "jwhitaker@kestrelaviation.com", title: "Director of Maintenance" },
  { company: "kestrel", name: "Nadia Petrov", email: "npetrov@kestrelaviation.com", title: "Systems Analyst" },
  { company: "bluefin", name: "Elena Vasquez", email: "elena@bluefin.io", title: "Head of Product" },
  { company: "harbor", name: "Robert Chen", email: "rchen@harborpointhotels.com", title: "VP Guest Experience" },
  { company: "harbor", name: "Monica Reyes", email: "mreyes@harborpointhotels.com", title: "Regional Manager" },
  { company: "wren", name: "Alex Duffy", email: "alex@wren.studio", title: "Partner" },
  { company: "summit", name: "Patricia Nguyen", email: "pnguyen@summitcu.org", title: "SVP Member Services" },
  { company: "summit", name: "Greg Holloway", email: "gholloway@summitcu.org", title: "Information Security" },
  { company: "pinecrest", name: "Walt Jensen", email: "walt@pinecrestfarms.com", title: "Owner" },
  { company: "vantage", name: "Anita Rao", email: "anita@vantagesolar.com", title: "Head of Installations" },
  { company: "ironwood", name: "Paul Marchetti", email: "paul@ironwoodfurniture.com", title: "Owner" },
  { company: "cobalt", name: "Tara Singh", email: "tara@cobaltstaffing.com", title: "Managing Director" },
  { company: "cobalt", name: "Derek Lowe", email: "derek@cobaltstaffing.com", title: "Operations Manager" },
  { company: "halcyon", name: "Margaret O'Neill", email: "moneill@halcyonliving.com", title: "Executive Director" },
  { company: "halcyon", name: "Sam Kowalski", email: "skowalski@halcyonliving.com", title: "IT Lead" },
];

export const SEED_DEALS: SeedDeal[] = [
  // Lead
  { company: "atlas", contact: "Simone Clarke", title: "Class booking system", value: 7400, stage: "lead", closeInDays: 40, age: 6 },
  { company: "beacon", contact: "Lena Fischer", title: "Listing sync integration", value: 64000, stage: "lead", closeInDays: 55, age: 4 },
  { company: "copperline", contact: "Mike Brennan", title: "Taproom POS integration", value: 12500, stage: "lead", closeInDays: 45, age: 3 },
  { company: "granite", contact: "Hannah Weiss", title: "Rental fleet tracking", value: 9800, stage: "lead", closeInDays: 60, age: 2 },
  { company: "pinecrest", contact: "Walt Jensen", title: "CSA subscription portal", value: 16000, stage: "lead", closeInDays: 50, age: 8 },
  { company: "ironwood", contact: "Paul Marchetti", title: "Custom order configurator", value: 21000, stage: "lead", closeInDays: 70, age: 1 },
  { company: "wren", contact: "Alex Duffy", title: "Project archive search", value: 8200, stage: "lead", closeInDays: 35, age: 5 },
  { company: "fernhill", contact: "Dr. Chris Okafor", title: "Appointment reminders", value: 4600, stage: "lead", closeInDays: 30, age: 9 },
  { company: "harbor", contact: "Monica Reyes", title: "Housekeeping dispatch app", value: 38000, stage: "lead", closeInDays: 75, age: 2 },
  { company: "halcyon", contact: "Sam Kowalski", title: "Family portal", value: 27000, stage: "lead", closeInDays: 65, age: 7 },

  // Qualified
  { company: "northwind", contact: "Marcus Iyer", title: "Warehouse scanner pilot", value: 9500, stage: "qualified", closeInDays: 26, age: 18 },
  { company: "quarry", contact: "Ben Osei", title: "Client portal build", value: 31000, stage: "qualified", closeInDays: 18, age: 22 },
  { company: "larkspur", contact: "Kevin Doyle", title: "Multi-site scheduling", value: 72000, stage: "qualified", closeInDays: 34, age: 15 },
  { company: "meridian", contact: "Luis Herrera", title: "Carrier onboarding workflow", value: 44000, stage: "qualified", closeInDays: 28, age: 20 },
  { company: "orchard", contact: "David Park", title: "Parent communication hub", value: 19500, stage: "qualified", closeInDays: 21, age: 12 },
  { company: "kestrel", contact: "Nadia Petrov", title: "Maintenance log digitisation", value: 56000, stage: "qualified", closeInDays: 42, age: 25 },
  { company: "summit", contact: "Patricia Nguyen", title: "Member onboarding forms", value: 33500, stage: "qualified", closeInDays: 30, age: 16 },
  { company: "cobalt", contact: "Tara Singh", title: "Candidate pipeline dashboard", value: 14800, stage: "qualified", closeInDays: 24, age: 11 },
  { company: "vantage", contact: "Anita Rao", title: "Installer scheduling", value: 26500, stage: "qualified", closeInDays: 38, age: 14 },
  { company: "tidewater", contact: "Sarah Lindqvist", title: "Shop floor work orders", value: 41000, stage: "qualified", closeInDays: 33, age: 19 },

  // Proposal
  { company: "northwind", contact: "Dana Whitfield", title: "Fleet dispatch rollout", value: 48000, stage: "proposal", closeInDays: 12, age: 34 },
  { company: "riverbed", contact: "Tom Alvarez", title: "Multi-store inventory", value: 22000, stage: "proposal", closeInDays: 5, age: 29 },
  { company: "meridian", contact: "Rachel Stone", title: "Customer tracking portal", value: 88000, stage: "proposal", closeInDays: 9, age: 41 },
  { company: "bluefin", contact: "Elena Vasquez", title: "Embedded reporting module", value: 36000, stage: "proposal", closeInDays: 14, age: 27 },
  { company: "harbor", contact: "Robert Chen", title: "Guest messaging platform", value: 67000, stage: "proposal", closeInDays: 7, age: 38 },
  { company: "halcyon", contact: "Margaret O'Neill", title: "Care plan tracking", value: 52000, stage: "proposal", closeInDays: 16, age: 31 },
  { company: "summit", contact: "Greg Holloway", title: "Security review tooling", value: 18000, stage: "proposal", closeInDays: 3, age: 24 },
  { company: "sable", contact: "Isabel Moreau", title: "Loyalty programme", value: 11200, stage: "proposal", closeInDays: 10, age: 26 },

  // Won
  { company: "hallmark", contact: "Priya Raman", title: "Patient intake portal", value: 15200, stage: "won", closeInDays: -8, age: 52 },
  { company: "riverbed", contact: "Jess Nakamura", title: "Roastery ordering app", value: 13400, stage: "won", closeInDays: -30, age: 74 },
  { company: "larkspur", contact: "Dr. Amara Bello", title: "Referral tracking", value: 29000, stage: "won", closeInDays: -19, age: 63 },
  { company: "kestrel", contact: "James Whitaker", title: "Parts inventory system", value: 47500, stage: "won", closeInDays: -44, age: 90 },
  { company: "orchard", contact: "David Park", title: "Enrolment forms", value: 8900, stage: "won", closeInDays: -12, age: 48 },
  { company: "cobalt", contact: "Derek Lowe", title: "Timesheet approvals", value: 12600, stage: "won", closeInDays: -61, age: 102 },
  { company: "vantage", contact: "Anita Rao", title: "Site survey app", value: 19800, stage: "won", closeInDays: -25, age: 70 },
  { company: "beacon", contact: "Owen Hart", title: "Tenant maintenance requests", value: 23000, stage: "won", closeInDays: -3, age: 45 },
  { company: "wren", contact: "Alex Duffy", title: "Proposal templates", value: 6400, stage: "won", closeInDays: -37, age: 66 },

  // Lost
  { company: "beacon", contact: "Lena Fischer", title: "Agent CRM migration", value: 18500, stage: "lost", closeInDays: -21, age: 58 },
  { company: "tidewater", contact: "Sarah Lindqvist", title: "Quality inspection app", value: 28000, stage: "lost", closeInDays: -40, age: 81 },
  { company: "granite", contact: "Hannah Weiss", title: "Online store rebuild", value: 15500, stage: "lost", closeInDays: -14, age: 49 },
  { company: "bluefin", contact: "Elena Vasquez", title: "Data warehouse sync", value: 42000, stage: "lost", closeInDays: -55, age: 95 },
  { company: "ironwood", contact: "Paul Marchetti", title: "Showroom kiosk", value: 9200, stage: "lost", closeInDays: -9, age: 40 },
];

export const SEED_ACTIVITIES: SeedActivity[] = [
  { deal: "Fleet dispatch rollout", kind: "call", body: "Walked Dana through the dispatch flow. Wants a security review before signing. Sending the SOC 2 summary.", age: 5 },
  { deal: "Fleet dispatch rollout", kind: "email", body: "Sent revised proposal with the 3-year term and onboarding included.", age: 30 },
  { deal: "Fleet dispatch rollout", kind: "meeting", body: "Pricing workshop with Dana and Marcus. They asked for a per-vehicle price instead of per-seat.", age: 120 },
  { deal: "Warehouse scanner pilot", kind: "note", body: "Marcus has 12 Zebra scanners on hand. Pilot will run in the Rotterdam warehouse only.", age: 54 },
  { deal: "Patient intake portal", kind: "note", body: "Signed. Kickoff scheduled for the 14th. Priya is the day-to-day contact.", age: 190 },
  { deal: "Multi-store inventory", kind: "meeting", body: "Demo with Tom and the two store leads. Main question is offline mode at the market stall.", age: 48 },
  { deal: "Multi-store inventory", kind: "email", body: "Sent the offline sync write-up and the pricing for a third store.", age: 20 },
  { deal: "Client portal build", kind: "note", body: "Ben wants SSO with their Google Workspace. Confirmed that is in scope.", age: 72 },
  { deal: "Client portal build", kind: "call", body: "Ben is comparing us to a Webflow build. Sent three portal references.", age: 8 },
  { deal: "Listing sync integration", kind: "call", body: "Intro call. They are evaluating two other vendors. Decision in Q3.", age: 96 },
  { deal: "Multi-site scheduling", kind: "meeting", body: "Site visit at the Westgate clinic. 14 practitioners, 3 rooms, paper diary today.", age: 140 },
  { deal: "Multi-site scheduling", kind: "email", body: "Kevin asked for HIPAA documentation before the next meeting. Sent the BAA template.", age: 36 },
  { deal: "Customer tracking portal", kind: "meeting", body: "Executive review with Rachel and the CFO. Budget approved pending legal.", age: 26 },
  { deal: "Customer tracking portal", kind: "note", body: "Legal wants a data residency clause. Ours already covers EU, so this should be quick.", age: 12 },
  { deal: "Customer tracking portal", kind: "call", body: "Rachel confirmed the board wants this live before peak season.", age: 200 },
  { deal: "Carrier onboarding workflow", kind: "email", body: "Luis sent the current onboarding checklist. 47 steps, 6 systems.", age: 90 },
  { deal: "Embedded reporting module", kind: "meeting", body: "Technical review with Elena's engineers. They want the embed to support row-level permissions.", age: 44 },
  { deal: "Embedded reporting module", kind: "email", body: "Sent proposal v2 with the permissions work broken out as a phase 2 option.", age: 18 },
  { deal: "Guest messaging platform", kind: "call", body: "Robert wants SMS and WhatsApp on day one. Email can wait.", age: 60 },
  { deal: "Guest messaging platform", kind: "meeting", body: "Pilot proposal for the two Boston properties. Monica will be the operational sponsor.", age: 15 },
  { deal: "Care plan tracking", kind: "note", body: "Margaret needs sign-off from the clinical board. They meet on the first Tuesday.", age: 70 },
  { deal: "Care plan tracking", kind: "email", body: "Sent the audit log and access control write-up Sam asked for.", age: 33 },
  { deal: "Security review tooling", kind: "call", body: "Greg walked through their current spreadsheet process. Wants the pilot scoped to vendor reviews only.", age: 9 },
  { deal: "Loyalty programme", kind: "meeting", body: "Isabel liked the points model. Asked whether it can work with their Square terminals.", age: 52 },
  { deal: "Member onboarding forms", kind: "call", body: "Patricia confirmed they process about 900 applications a month. Paper today.", age: 80 },
  { deal: "Candidate pipeline dashboard", kind: "email", body: "Tara sent sample data. Their ATS export is a CSV with 23 columns.", age: 100 },
  { deal: "Installer scheduling", kind: "meeting", body: "Rode along with an install crew. Real pain is the morning dispatch call, not the calendar.", age: 150 },
  { deal: "Shop floor work orders", kind: "note", body: "Sarah wants tablets on the floor. Rugged cases are their cost, not ours.", age: 64 },
  { deal: "Maintenance log digitisation", kind: "meeting", body: "FAA compliance is the whole conversation. Nadia is pulling the relevant Part 145 sections.", age: 112 },
  { deal: "Parent communication hub", kind: "call", body: "David wants a pilot with two schools this term.", age: 30 },
  { deal: "Roastery ordering app", kind: "note", body: "Live since March. Jess mentioned a wholesale tier as a possible expansion.", age: 400 },
  { deal: "Referral tracking", kind: "note", body: "Went live across all four clinics. Amara is the reference for Halcyon.", age: 320 },
  { deal: "Parts inventory system", kind: "note", body: "Renewal due in eleven months. James is happy with the barcode workflow.", age: 700 },
  { deal: "Tenant maintenance requests", kind: "email", body: "Contract countersigned. Onboarding starts Monday with Owen's team.", age: 50 },
  { deal: "Agent CRM migration", kind: "note", body: "Lost to the incumbent. Lena said the switching cost for 80 agents was the blocker.", age: 500 },
  { deal: "Quality inspection app", kind: "note", body: "Lost. They built it in-house with their ERP vendor.", age: 960 },
  { deal: "Online store rebuild", kind: "note", body: "Lost to a Shopify agency. Budget was smaller than we scoped for.", age: 330 },
  { deal: "Data warehouse sync", kind: "note", body: "Lost. Elena's team chose Fivetran for the sync and kept us for reporting, which is the open proposal.", age: 1300 },
  { deal: "Taproom POS integration", kind: "email", body: "Mike asked which POS systems we already integrate with. Sent the list.", age: 40 },
  { deal: "CSA subscription portal", kind: "call", body: "Walt runs 340 CSA boxes a week on a spreadsheet. Wants pickup site management too.", age: 180 },
  { deal: "Housekeeping dispatch app", kind: "note", body: "Inbound from the Guest messaging conversation. Monica introduced us to her ops lead.", age: 44 },
  { deal: "Family portal", kind: "email", body: "Sam sent the current family newsletter process. Wants photo sharing with consent controls.", age: 160 },
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

  const latestActivity = new Map<string, number>();
  for (const a of SEED_ACTIVITIES) {
    const current = latestActivity.get(a.deal);
    if (current === undefined || a.age < current) latestActivity.set(a.deal, a.age);
  }

  return {
    companies: SEED_COMPANIES.map((c, index) => ({
      key: c.key,
      row: {
        name: c.name,
        domain: c.domain,
        industry: c.industry,
        size: c.size,
        createdAt: at(-120 + index * 3),
      },
    })),
    contacts: SEED_CONTACTS.map((c, index) => ({
      company: c.company,
      row: { name: c.name, email: c.email, title: c.title, createdAt: at(-110 + index * 2) },
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
        createdAt: at(-d.age),
        updatedAt: hoursAgo(Math.min(latestActivity.get(d.title) ?? d.age * 24, d.age * 24)),
      },
    })),
    activities: SEED_ACTIVITIES.map((a) => ({
      deal: a.deal,
      row: { kind: a.kind, body: a.body, createdAt: hoursAgo(a.age) },
    })),
  };
}
