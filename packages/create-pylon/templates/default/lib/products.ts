// Single source of truth for the marketing "products". The nav dropdown, the
// homepage feature sections, and the /products/[slug] pages all read from here,
// so a product is defined once. Add an entry and it shows up everywhere.
// Fictional demo copy — rename these to your real product modules.

export type ProductFeature = { title: string; body: string };

export type Product = {
  slug: string;
  icon: string;
  title: string; // nav label + page <h1> subject
  tagline: string; // one-line blurb for the nav dropdown
  eyebrow: string; // section/page eyebrow
  headline: string; // page hero headline
  summary: string; // page hero paragraph
  features: ProductFeature[];
  mockupUrl: string; // fake browser URL in the screenshot frame
  mockupLabel: string; // placeholder label inside the frame
};

export const PRODUCTS: Product[] = [
  {
    slug: "projects",
    icon: "▤",
    title: "Projects",
    tagline: "Plan and track every initiative.",
    eyebrow: "Projects",
    headline: "Plan every project in one place.",
    summary:
      "Give your team one place to plan the work, see what is in flight, and keep every project moving toward done.",
    mockupUrl: "acme.app/projects",
    mockupLabel: "Projects board",
    features: [
      { title: "Flexible views", body: "See the work as a board, a list, or a timeline — whatever fits the team." },
      { title: "Milestones", body: "Group work into milestones so everyone knows what ships next." },
      { title: "Dependencies", body: "Link related work so blockers surface before they bite." },
      { title: "Custom fields", body: "Track the details that matter to your team, your way." },
      { title: "Templates", body: "Start new projects from a template instead of a blank page." },
      { title: "Saved filters", body: "Slice the work by owner, status, or label in a single click." },
    ],
  },
  {
    slug: "tasks",
    icon: "✓",
    title: "Tasks",
    tagline: "Assign, prioritize, and finish work.",
    eyebrow: "Tasks",
    headline: "Turn plans into finished work.",
    summary:
      "Break projects into tasks, assign owners, and watch progress update in real time across every screen.",
    mockupUrl: "acme.app/tasks",
    mockupLabel: "Task list",
    features: [
      { title: "Assignees and due dates", body: "Every task has a clear owner and a clear deadline." },
      { title: "Subtasks", body: "Break big tasks into small, checkable steps." },
      { title: "Priorities", body: "Sort by what matters most so the right work happens first." },
      { title: "Comments", body: "Discuss the work where it lives, with full context attached." },
      { title: "My work", body: "A personal view of everything on your plate, across projects." },
      { title: "Recurring tasks", body: "Set it once and the task comes back when it should." },
    ],
  },
  {
    slug: "docs",
    icon: "≡",
    title: "Docs",
    tagline: "Write and share team knowledge.",
    eyebrow: "Docs",
    headline: "Keep what your team knows in one place.",
    summary:
      "Write docs, notes, and specs alongside the work, so the context never lives in just one person's head.",
    mockupUrl: "acme.app/docs",
    mockupLabel: "Doc editor",
    features: [
      { title: "Rich editor", body: "Headings, checklists, tables, and embeds in a clean editor." },
      { title: "Linked to work", body: "Connect a doc to the project or task it describes." },
      { title: "Real-time co-editing", body: "Write together, with changes syncing as you type." },
      { title: "Version history", body: "Roll back to any earlier version in a click." },
      { title: "Templates", body: "Spin up specs, briefs, and notes from reusable templates." },
      { title: "Instant search", body: "Find any doc by title or content in milliseconds." },
    ],
  },
  {
    slug: "automations",
    icon: "⟳",
    title: "Automations",
    tagline: "Automate the busywork.",
    eyebrow: "Automations",
    headline: "Let the routine work run itself.",
    summary:
      "Build simple rules that move work forward automatically, so your team spends its time on what actually matters.",
    mockupUrl: "acme.app/automations",
    mockupLabel: "Automation builder",
    features: [
      { title: "Rules", body: "When this happens, do that — no code required." },
      { title: "Scheduled runs", body: "Kick off routine work on a schedule you set." },
      { title: "Webhooks", body: "Trigger automations from anything that can send a request." },
      { title: "Run history", body: "See exactly what ran, when, and why." },
    ],
  },
  {
    slug: "analytics",
    icon: "◔",
    title: "Analytics",
    tagline: "Measure what actually matters.",
    eyebrow: "Analytics",
    headline: "See how the work is really going.",
    summary:
      "Track throughput, cycle time, and progress across projects, so you can spot what is stuck before it slips.",
    mockupUrl: "acme.app/analytics",
    mockupLabel: "Analytics dashboard",
    features: [
      { title: "Dashboards", body: "Build views that answer the questions your team asks most." },
      { title: "Cycle time", body: "See how long work takes from start to done." },
      { title: "Throughput", body: "Track how much ships each week, by team or project." },
      { title: "Exports", body: "Send any view to CSV or your warehouse." },
    ],
  },
];

export function productBySlug(slug: string): Product | undefined {
  return PRODUCTS.find((p) => p.slug === slug);
}
