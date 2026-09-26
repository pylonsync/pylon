// Demo back-office data for local development.
//
// The owner's first dashboard load in `pylon dev` fills an empty studio with
// fictional leads, clients, and invoices (functions/seedStudioBackoffice.ts),
// so the pipeline, CRM, and billing tabs have rows. A deploy does not seed:
// `demoDataEnabled` is true only when PYLON_DEMO_DATA is on, or when it is
// unset and the process runs under `pylon dev`. Set PYLON_DEMO_DATA=0 to turn
// it off in development too.
//
// The public portfolio (functions/seedProjects.ts) is different: it copies the
// case studies from lib/site.config.ts, which is the studio's own content, so it
// runs in every environment.

const ON = new Set(["1", "true", "yes", "on"]);
const OFF = new Set(["0", "false", "no", "off"]);

/**
 * True when demo rows may be written. An explicit PYLON_DEMO_DATA wins.
 * Otherwise it needs both PYLON_DEV_MODE on and PYLON_DEV_WATCH_DIR set. Only
 * `pylon dev` sets PYLON_DEV_WATCH_DIR; `pylon start` and the Docker image do
 * not, and the image turns PYLON_DEV_MODE on by default, so dev mode alone is
 * not proof of a local machine.
 */
export function demoDataEnabled(env: Record<string, string | undefined>): boolean {
  const explicit = env.PYLON_DEMO_DATA?.trim().toLowerCase();
  if (explicit && ON.has(explicit)) return true;
  if (explicit && OFF.has(explicit)) return false;
  const dev = env.PYLON_DEV_MODE?.trim().toLowerCase();
  const devMode = dev === "1" || dev === "true";
  return devMode && Boolean(env.PYLON_DEV_WATCH_DIR?.trim());
}

/** Local "YYYY-MM-DD" for `daysAgo` days before `nowMs` (negative = ahead). */
export function daysAgoDate(daysAgo: number, nowMs: number): string {
  const d = new Date(nowMs);
  d.setDate(d.getDate() - daysAgo);
  return `${d.getFullYear()}-${String(d.getMonth() + 1).padStart(2, "0")}-${String(d.getDate()).padStart(2, "0")}`;
}

export interface DemoInquiry {
  name: string;
  email: string;
  company: string;
  projectType: string;
  budget: string;
  message: string;
  status: "new" | "booked" | "declined";
  /** Hours before the seed runs. */
  hoursAgo: number;
}

// Fictional leads. Project types and budgets use the contact form's options.
export const DEMO_INQUIRIES: DemoInquiry[] = [
  {
    name: "Priya Raman",
    email: "priya@fernhillvet.example",
    company: "Fernhill Veterinary",
    projectType: "New product (0→1)",
    budget: "$50–100k",
    message:
      "We run six clinics and want an app for booking visits and refilling prescriptions. We have an API from our practice software. Hoping to launch in spring.",
    status: "new",
    hoursAgo: 3,
  },
  {
    name: "Marcus Iyer",
    email: "marcus@northwind.example",
    company: "Northwind Logistics",
    projectType: "Existing product",
    budget: "$100k+",
    message:
      "Our dispatch team works out of spreadsheets. We need a web app for load planning with live driver locations. Two engineers in-house can take it over after launch.",
    status: "new",
    hoursAgo: 20,
  },
  {
    name: "Lena Fischer",
    email: "lena@beaconproperty.example",
    company: "Beacon Property",
    projectType: "Existing product",
    budget: "$25–50k",
    message:
      "Three product teams, three sets of buttons. We want one component library in Figma and React, and help rolling it out.",
    status: "new",
    hoursAgo: 46,
  },
  {
    name: "Ben Osei",
    email: "ben@quarry.example",
    company: "Quarry Design Co",
    projectType: "Rebrand",
    budget: "$25–50k",
    message: "Looking for a partner to build our new marketing site in a headless CMS. Designs are about 70% done.",
    status: "booked",
    hoursAgo: 120,
  },
  {
    name: "Tom Alvarez",
    email: "tom@riverbed.example",
    company: "Riverbed Coffee",
    projectType: "New product (0→1)",
    budget: "Let's talk",
    message: "A loyalty app for our four cafes. Punch card, order ahead, and push offers.",
    status: "declined",
    hoursAgo: 200,
  },
  {
    name: "Simone Clarke",
    email: "simone@atlasfitness.example",
    company: "Atlas Fitness",
    projectType: "New product (0→1)",
    budget: "$50–100k",
    message:
      "Members book classes through a third-party tool that charges per booking. We want our own booking and billing, with a coach view for check-ins.",
    status: "new",
    hoursAgo: 70,
  },
];
