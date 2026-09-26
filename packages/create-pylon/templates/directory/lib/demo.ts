// Demo review queue for local development.
//
// The curator's first dashboard load in `pylon dev` fills an empty Submission
// table with fictional tool submissions (functions/seedDemo.ts), so the queue
// has entries to approve or reject. A deploy does not seed: `demoDataEnabled`
// is true only when PYLON_DEMO_DATA is on, or when it is unset and the process
// runs under `pylon dev`. Set PYLON_DEMO_DATA=0 to turn it off in development too.
//
// The starter listings (functions/seedListings.ts) are different: they are the
// directory's own content from lib/site.config.ts, so they seed everywhere.

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

export interface DemoSubmission {
  submitterName: string;
  submitterEmail: string;
  name: string;
  tagline: string;
  url: string;
  category: string;
  tags: string;
  description: string;
  status: "new" | "approved" | "rejected";
  /** Hours before the seed runs. */
  hoursAgo: number;
}

// Fictional tools and submitters. The two "approved" rows name starter
// listings from lib/site.config.ts, so every approved submission is live.
// Starter listings added from config have no submission row.
export const DEMO_SUBMISSIONS: DemoSubmission[] = [
  {
    submitterName: "Nadia Haddad",
    submitterEmail: "nadia@example.com",
    name: "Querybird",
    tagline: "Explain plans for Postgres, drawn as a tree you can click through.",
    url: "https://example.com/querybird",
    category: "Database",
    tags: "postgres, performance",
    description: "Paste a query, get the plan with the slow node marked. Runs in the browser; nothing leaves your machine.",
    status: "new",
    hoursAgo: 2,
  },
  {
    submitterName: "Owen Reilly",
    submitterEmail: "owen@example.com",
    name: "Flagpost",
    tagline: "Feature flags stored in your own database, with a small admin UI.",
    url: "https://example.com/flagpost",
    category: "DevOps",
    tags: "feature flags, self-hosted",
    description: "One table, one SDK call, and an admin page you mount in your app.",
    status: "new",
    hoursAgo: 9,
  },
  {
    submitterName: "Hana Sato",
    submitterEmail: "hana@example.com",
    name: "Palettier",
    tagline: "Generate accessible color scales from one brand color.",
    url: "https://example.com/palettier",
    category: "Design",
    tags: "color, accessibility, tokens",
    description: "Every step is checked for WCAG contrast against white and black. Exports CSS variables and a Figma file.",
    status: "new",
    hoursAgo: 26,
  },
  {
    submitterName: "Felix Wagner",
    submitterEmail: "felix@example.com",
    name: "Logline",
    tagline: "Structured logs you can search from the terminal.",
    url: "https://example.com/logline",
    category: "DevOps",
    tags: "logging, cli",
    description: "Pipe JSON logs in, then filter with a short query language. Keeps a week of history on disk.",
    status: "new",
    hoursAgo: 40,
  },
  {
    submitterName: "Isabel Cruz",
    submitterEmail: "isabel@example.com",
    name: "Promptdeck",
    tagline: "Version and diff the prompts your app sends to a model.",
    url: "https://example.com/promptdeck",
    category: "AI",
    tags: "llm, prompts, versioning",
    description: "Each prompt change gets a version and a side-by-side diff of outputs on your saved test inputs.",
    status: "new",
    hoursAgo: 55,
  },
  {
    submitterName: "Ruth Castillo",
    submitterEmail: "ruth@example.com",
    name: "Brineway",
    tagline: "S3-compatible object storage at the edge.",
    url: "https://example.com/brineway",
    category: "Hosting",
    tags: "storage, edge, s3",
    description: "",
    status: "approved",
    hoursAgo: 150,
  },
  {
    submitterName: "Daniel Park",
    submitterEmail: "daniel@example.com",
    name: "Countwell",
    tagline: "Privacy-first product analytics you can self-host.",
    url: "https://example.com/countwell",
    category: "Analytics",
    tags: "analytics, privacy",
    description: "",
    status: "approved",
    hoursAgo: 210,
  },
  {
    submitterName: "Luca Romano",
    submitterEmail: "luca@example.com",
    name: "Best AI Tools 2026",
    tagline: "A list of 500 AI tools.",
    url: "https://example.com/best-ai-tools",
    category: "AI",
    tags: "list",
    description: "",
    status: "rejected",
    hoursAgo: 96,
  },
];
