// Demo data for a brand-new delivery workspace.
//
// The first sign-in seeds a working studio: eight clients, ten projects in
// every status, sixty tasks, and the time logged against them. Two active
// projects are over budget, so the budget states are visible rather than
// theoretical.

export interface SeedClient {
  key: string;
  name: string;
  email: string;
}

export interface SeedProject {
  key: string;
  client: string;
  name: string;
  status: string;
  /** Agreed budget in hours. */
  budgetHours: number;
  hourlyRate: number;
  dueInDays: number;
  /** Days since the project was created. */
  age: number;
}

export interface SeedTask {
  key: string;
  project: string;
  title: string;
  status: string;
  estimateHours: number;
  /** Hours logged against it; each becomes one entry. */
  loggedHours: number[];
}

export const SEED_CLIENTS: SeedClient[] = [
  { key: "northwind", name: "Northwind Logistics", email: "dana@northwind.co" },
  { key: "hallmark", name: "Hallmark Dental", email: "priya@hallmarkdental.com" },
  { key: "larkspur", name: "Larkspur Clinics", email: "kdoyle@larkspurclinics.com" },
  { key: "meridian", name: "Meridian Freight", email: "rstone@meridianfreight.com" },
  { key: "bluefin", name: "Bluefin Analytics", email: "elena@bluefin.io" },
  { key: "harbor", name: "Harbor Point Hotels", email: "rchen@harborpointhotels.com" },
  { key: "kestrel", name: "Kestrel Aviation Services", email: "jwhitaker@kestrelaviation.com" },
  { key: "orchard", name: "Orchard Learning", email: "dpark@orchardlearning.org" },
];

export const SEED_PROJECTS: SeedProject[] = [
  { key: "dispatch", client: "northwind", name: "Dispatch portal", status: "active", budgetHours: 120, hourlyRate: 165, dueInDays: 24, age: 45 },
  // Over budget: 40h agreed, about 50h logged.
  { key: "intake", client: "hallmark", name: "Patient intake rebuild", status: "active", budgetHours: 40, hourlyRate: 150, dueInDays: 9, age: 38 },
  { key: "scheduling", client: "larkspur", name: "Multi-site scheduling", status: "active", budgetHours: 200, hourlyRate: 165, dueInDays: 60, age: 30 },
  { key: "onboarding", client: "meridian", name: "Carrier onboarding workflow", status: "active", budgetHours: 160, hourlyRate: 165, dueInDays: 41, age: 52 },
  // Over budget: 60h agreed, about 68h logged.
  { key: "reporting", client: "bluefin", name: "Embedded reporting", status: "active", budgetHours: 60, hourlyRate: 175, dueInDays: 5, age: 40 },
  { key: "messaging", client: "harbor", name: "Guest messaging pilot", status: "active", budgetHours: 90, hourlyRate: 160, dueInDays: 18, age: 28 },
  { key: "maintenance", client: "kestrel", name: "Maintenance log digitisation", status: "paused", budgetHours: 240, hourlyRate: 165, dueInDays: 120, age: 20 },
  { key: "parents", client: "orchard", name: "Parent communication hub", status: "paused", budgetHours: 80, hourlyRate: 140, dueInDays: 75, age: 33 },
  { key: "parts", client: "kestrel", name: "Parts inventory system", status: "complete", budgetHours: 180, hourlyRate: 165, dueInDays: -35, age: 150 },
  { key: "referrals", client: "larkspur", name: "Referral tracking", status: "complete", budgetHours: 100, hourlyRate: 165, dueInDays: -20, age: 110 },
];

export const SEED_TASKS: SeedTask[] = [
  { key: "d-auth", project: "dispatch", title: "SSO with their identity provider", status: "done", estimateHours: 8, loggedHours: [3, 4.5] },
  { key: "d-model", project: "dispatch", title: "Route and stop data model", status: "done", estimateHours: 6, loggedHours: [5] },
  { key: "d-map", project: "dispatch", title: "Map tiles and stop markers", status: "done", estimateHours: 10, loggedHours: [4, 6, 1.5] },
  { key: "d-board", project: "dispatch", title: "Dispatcher board UI", status: "doing", estimateHours: 20, loggedHours: [6, 4, 5] },
  { key: "d-assign", project: "dispatch", title: "Drag stops between drivers", status: "doing", estimateHours: 8, loggedHours: [3] },
  { key: "d-mobile", project: "dispatch", title: "Driver mobile view", status: "review", estimateHours: 12, loggedHours: [9, 2] },
  { key: "d-report", project: "dispatch", title: "Weekly utilisation report", status: "todo", estimateHours: 6, loggedHours: [] },
  { key: "d-import", project: "dispatch", title: "Import legacy stop history", status: "todo", estimateHours: 10, loggedHours: [] },
  { key: "d-alerts", project: "dispatch", title: "Late delivery alerts", status: "todo", estimateHours: 6, loggedHours: [] },

  { key: "i-forms", project: "intake", title: "Form builder", status: "done", estimateHours: 16, loggedHours: [8, 7, 4] },
  { key: "i-sig", project: "intake", title: "Consent signatures", status: "done", estimateHours: 8, loggedHours: [6, 5] },
  { key: "i-insurance", project: "intake", title: "Insurance card capture", status: "done", estimateHours: 6, loggedHours: [4, 3.5] },
  { key: "i-sync", project: "intake", title: "Sync to practice software", status: "doing", estimateHours: 12, loggedHours: [9, 7] },
  { key: "i-kiosk", project: "intake", title: "Waiting room kiosk mode", status: "review", estimateHours: 4, loggedHours: [5] },
  { key: "i-train", project: "intake", title: "Staff training session", status: "todo", estimateHours: 4, loggedHours: [] },

  { key: "s-discovery", project: "scheduling", title: "Site visits and workflow mapping", status: "done", estimateHours: 18, loggedHours: [6, 6, 7] },
  { key: "s-model", project: "scheduling", title: "Practitioner, room, and slot model", status: "done", estimateHours: 10, loggedHours: [5, 4] },
  { key: "s-calendar", project: "scheduling", title: "Week calendar with room lanes", status: "doing", estimateHours: 30, loggedHours: [7, 6, 8] },
  { key: "s-rules", project: "scheduling", title: "Booking rules per clinic", status: "doing", estimateHours: 16, loggedHours: [4] },
  { key: "s-reminders", project: "scheduling", title: "SMS reminders", status: "todo", estimateHours: 8, loggedHours: [] },
  { key: "s-waitlist", project: "scheduling", title: "Cancellation waitlist", status: "todo", estimateHours: 12, loggedHours: [] },
  { key: "s-reports", project: "scheduling", title: "Utilisation by site", status: "todo", estimateHours: 8, loggedHours: [] },
  { key: "s-migration", project: "scheduling", title: "Import the paper diaries", status: "todo", estimateHours: 20, loggedHours: [] },

  { key: "o-checklist", project: "onboarding", title: "Digitise the 47-step checklist", status: "done", estimateHours: 12, loggedHours: [6, 6.5] },
  { key: "o-docs", project: "onboarding", title: "Document upload and expiry", status: "done", estimateHours: 14, loggedHours: [5, 6, 4] },
  { key: "o-approvals", project: "onboarding", title: "Approval chain", status: "doing", estimateHours: 16, loggedHours: [7, 3] },
  { key: "o-portal", project: "onboarding", title: "Carrier self-service portal", status: "doing", estimateHours: 24, loggedHours: [8, 6] },
  { key: "o-tms", project: "onboarding", title: "Push approved carriers to the TMS", status: "review", estimateHours: 10, loggedHours: [9] },
  { key: "o-audit", project: "onboarding", title: "Audit trail export", status: "todo", estimateHours: 6, loggedHours: [] },
  { key: "o-insurance", project: "onboarding", title: "Insurance certificate verification", status: "todo", estimateHours: 12, loggedHours: [] },

  { key: "r-embed", project: "reporting", title: "Embed SDK", status: "done", estimateHours: 12, loggedHours: [7, 6] },
  { key: "r-charts", project: "reporting", title: "Chart components", status: "done", estimateHours: 16, loggedHours: [8, 9, 3] },
  { key: "r-filters", project: "reporting", title: "Cross-chart filters", status: "done", estimateHours: 10, loggedHours: [6, 7] },
  { key: "r-perms", project: "reporting", title: "Row-level permissions", status: "doing", estimateHours: 14, loggedHours: [9, 8] },
  { key: "r-export", project: "reporting", title: "PDF and CSV export", status: "review", estimateHours: 6, loggedHours: [5] },
  { key: "r-docs", project: "reporting", title: "Integration guide", status: "todo", estimateHours: 4, loggedHours: [] },

  { key: "m-sms", project: "messaging", title: "SMS channel", status: "done", estimateHours: 10, loggedHours: [6, 4.5] },
  { key: "m-whatsapp", project: "messaging", title: "WhatsApp channel", status: "done", estimateHours: 12, loggedHours: [7, 6] },
  { key: "m-inbox", project: "messaging", title: "Shared inbox for front desk", status: "doing", estimateHours: 20, loggedHours: [8, 7] },
  { key: "m-templates", project: "messaging", title: "Message templates", status: "doing", estimateHours: 6, loggedHours: [2] },
  { key: "m-pms", project: "messaging", title: "Link guests from the PMS", status: "review", estimateHours: 8, loggedHours: [7] },
  { key: "m-rollout", project: "messaging", title: "Roll out to the two Boston properties", status: "todo", estimateHours: 8, loggedHours: [] },
  { key: "m-metrics", project: "messaging", title: "Response time dashboard", status: "todo", estimateHours: 6, loggedHours: [] },

  { key: "k-discovery", project: "maintenance", title: "Part 145 compliance review", status: "done", estimateHours: 16, loggedHours: [8, 8] },
  { key: "k-model", project: "maintenance", title: "Work order and sign-off model", status: "doing", estimateHours: 12, loggedHours: [4] },
  { key: "k-forms", project: "maintenance", title: "Inspection forms", status: "todo", estimateHours: 30, loggedHours: [] },
  { key: "k-signatures", project: "maintenance", title: "Technician signatures", status: "todo", estimateHours: 10, loggedHours: [] },
  { key: "k-archive", project: "maintenance", title: "Scan and index paper logs", status: "todo", estimateHours: 40, loggedHours: [] },

  { key: "p-pilot", project: "parents", title: "Pilot scope with two schools", status: "done", estimateHours: 6, loggedHours: [3, 3] },
  { key: "p-announce", project: "parents", title: "Announcements feed", status: "doing", estimateHours: 12, loggedHours: [5] },
  { key: "p-consent", project: "parents", title: "Trip consent forms", status: "todo", estimateHours: 10, loggedHours: [] },
  { key: "p-translate", project: "parents", title: "Translations for the top five languages", status: "todo", estimateHours: 8, loggedHours: [] },

  { key: "pi-model", project: "parts", title: "Parts and location model", status: "done", estimateHours: 12, loggedHours: [6, 6] },
  { key: "pi-barcode", project: "parts", title: "Barcode scanning", status: "done", estimateHours: 20, loggedHours: [8, 7, 6] },
  { key: "pi-reorder", project: "parts", title: "Reorder points and alerts", status: "done", estimateHours: 10, loggedHours: [5, 5] },
  { key: "pi-import", project: "parts", title: "Import the vendor catalogue", status: "done", estimateHours: 16, loggedHours: [8, 9] },
  { key: "pi-reports", project: "parts", title: "Usage reports", status: "done", estimateHours: 8, loggedHours: [4, 4.5] },
  { key: "pi-training", project: "parts", title: "Stores team training", status: "done", estimateHours: 4, loggedHours: [4] },

  { key: "rf-model", project: "referrals", title: "Referral lifecycle model", status: "done", estimateHours: 8, loggedHours: [4, 4] },
  { key: "rf-intake", project: "referrals", title: "Fax and email intake", status: "done", estimateHours: 14, loggedHours: [7, 8] },
  { key: "rf-tracking", project: "referrals", title: "Status board per clinic", status: "done", estimateHours: 16, loggedHours: [8, 6, 3] },
  { key: "rf-reports", project: "referrals", title: "Turnaround reports", status: "done", estimateHours: 8, loggedHours: [4, 5] },
];

export interface ShapedSeed {
  clients: Array<{ key: string; row: Record<string, unknown> }>;
  projects: Array<{ key: string; client: string; row: Record<string, unknown> }>;
  tasks: Array<{ key: string; project: string; row: Record<string, unknown> }>;
  entries: Array<{ task: string; project: string; row: Record<string, unknown> }>;
}

export function shapeSeed(now: number = Date.now()): ShapedSeed {
  const at = (days: number) => new Date(now + days * 86_400_000).toISOString();
  const projectAge = new Map(SEED_PROJECTS.map((p) => [p.key, p.age]));

  const tasks: ShapedSeed["tasks"] = [];
  const entries: ShapedSeed["entries"] = [];
  const positions = new Map<string, number>();

  SEED_TASKS.forEach((task, index) => {
    const columnKey = `${task.project}:${task.status}`;
    const position = positions.get(columnKey) ?? 0;
    positions.set(columnKey, position + 1);

    const age = projectAge.get(task.project) ?? 30;
    // Tasks were created over the first half of the project, in order.
    const created = -age + (index % 6) * Math.max(1, Math.floor(age / 12));
    const lastTouched = task.status === "todo" ? created : -Math.min(age, 1 + (index % 9));

    tasks.push({
      key: task.key,
      project: task.project,
      row: {
        title: task.title,
        status: task.status,
        estimateMinutes: Math.round(task.estimateHours * 60),
        position,
        createdAt: at(created),
        updatedAt: at(lastTouched),
      },
    });

    // Entries spread from creation towards today, one every few days.
    const span = Math.max(1, -created - 1);
    const step = task.loggedHours.length > 1 ? span / task.loggedHours.length : 0;
    task.loggedHours.forEach((hours, entryIndex) => {
      entries.push({
        task: task.key,
        project: task.project,
        row: {
          minutes: Math.round(hours * 60),
          note: null,
          spentOn: at(Math.round(created + 1 + step * entryIndex)),
        },
      });
    });
  });

  return {
    clients: SEED_CLIENTS.map((c, index) => ({
      key: c.key,
      row: { name: c.name, email: c.email, createdAt: at(-160 + index * 10) },
    })),
    projects: SEED_PROJECTS.map((p) => ({
      key: p.key,
      client: p.client,
      row: {
        name: p.name,
        status: p.status,
        budgetMinutes: Math.round(p.budgetHours * 60),
        hourlyRateCents: Math.round(p.hourlyRate * 100),
        dueDate: at(p.dueInDays),
        createdAt: at(-p.age),
      },
    })),
    tasks,
    entries,
  };
}
