// Content for the rest of the marketing site — solutions, resources, company,
// and comparison pages. Each collection drives a dynamic route AND the footer
// columns, so the links and the pages can never drift. Fictional demo copy —
// swap it for your own.

export type ContentSection = { title: string; body: string };

export type SitePage = {
  slug: string;
  navLabel: string; // label in nav/footer
  eyebrow: string;
  title: string; // hero headline
  summary: string;
  sections: ContentSection[];
};

export const SOLUTIONS: SitePage[] = [
  {
    slug: "startups",
    navLabel: "For startups",
    eyebrow: "Solutions",
    title: "Move fast without losing the thread.",
    summary:
      "Keep a small team aligned as everything changes weekly. Acme gives you one place to plan, build, and ship before the next pivot.",
    sections: [
      { title: "One tool, not ten", body: "Projects, tasks, and docs in one place, so you are not paying for or stitching together five apps." },
      { title: "Set up in minutes", body: "No admin overhead. Invite the team and start working the same day." },
      { title: "Grows with you", body: "The same workspace works at five people and at fifty." },
    ],
  },
  {
    slug: "agencies",
    navLabel: "For agencies",
    eyebrow: "Solutions",
    title: "Run every client like clockwork.",
    summary:
      "Give each client their own space, keep the work organized, and show progress without a status meeting.",
    sections: [
      { title: "A space per client", body: "Separate workspaces keep every engagement tidy and private." },
      { title: "Shareable views", body: "Send clients a read-only view of exactly what is in flight." },
      { title: "Reusable templates", body: "Start every new engagement from a proven playbook." },
    ],
  },
  {
    slug: "enterprise",
    navLabel: "For enterprise",
    eyebrow: "Solutions",
    title: "Scale without the chaos.",
    summary:
      "Bring hundreds of people into one system of record, with the controls and visibility a larger org needs.",
    sections: [
      { title: "SSO and roles", body: "Single sign-on and granular roles keep access where it belongs." },
      { title: "Audit log", body: "A complete record of who changed what, and when." },
      { title: "Rollups", body: "See progress across teams and departments in one view." },
    ],
  },
  {
    slug: "teams",
    navLabel: "For teams",
    eyebrow: "Solutions",
    title: "Built for how your team works.",
    summary:
      "Whether you build, design, market, or support, Acme adapts to your process instead of forcing a new one.",
    sections: [
      { title: "Your workflow", body: "Custom statuses and fields match the way your team already works." },
      { title: "Cross-team work", body: "Hand work between teams without it falling through a crack." },
      { title: "Less status-chasing", body: "Everyone sees the same live picture, so updates write themselves." },
    ],
  },
];

export const RESOURCES: SitePage[] = [
  {
    slug: "docs",
    navLabel: "Docs",
    eyebrow: "Resources",
    title: "Documentation.",
    summary: "Everything you need to set up Acme and get your team productive.",
    sections: [
      { title: "Getting started", body: "Create a workspace, invite your team, and ship your first project." },
      { title: "Guides", body: "Deep dives on projects, tasks, docs, automations, and analytics." },
      { title: "API", body: "Build on the typed Acme API and webhooks." },
    ],
  },
  {
    slug: "guides",
    navLabel: "Guides",
    eyebrow: "Resources",
    title: "Guides and playbooks.",
    summary: "Practical walkthroughs for getting the most out of Acme.",
    sections: [
      { title: "Run a sprint", body: "Plan, track, and review a two-week cycle in Acme." },
      { title: "Automate intake", body: "Route incoming work to the right team automatically." },
      { title: "Report to leadership", body: "Build a dashboard that answers the questions you get asked." },
    ],
  },
  {
    slug: "changelog",
    navLabel: "Changelog",
    eyebrow: "Resources",
    title: "What's new.",
    summary: "Every improvement we ship, in one place.",
    sections: [
      { title: "This week", body: "Faster search, a redesigned task list, and new automation triggers." },
      { title: "Last week", body: "Timeline view for projects and CSV export for analytics." },
      { title: "Earlier", body: "Webhooks, custom fields, and version history for docs." },
    ],
  },
  {
    slug: "api",
    navLabel: "API reference",
    eyebrow: "Resources",
    title: "API reference.",
    summary: "A typed REST API and webhooks for everything in Acme.",
    sections: [
      { title: "Authentication", body: "API keys scoped to a workspace, revocable at any time." },
      { title: "Resources", body: "Projects, tasks, docs, and automations, all over the same API." },
      { title: "Webhooks", body: "Subscribe to events and react to changes in real time." },
    ],
  },
  {
    slug: "status",
    navLabel: "Status",
    eyebrow: "Resources",
    title: "System status.",
    summary: "Live status for every Acme service.",
    sections: [
      { title: "API", body: "Operational — 99.99% over the last 90 days." },
      { title: "Web app", body: "Operational — no incidents this week." },
      { title: "Webhooks", body: "Operational — delivering within seconds." },
    ],
  },
];

export const COMPANY: SitePage[] = [
  {
    slug: "about",
    navLabel: "About",
    eyebrow: "Company",
    title: "About Acme.",
    summary: "We build the workspace we always wanted: fast, focused, and a pleasure to use.",
    sections: [
      { title: "Our mission", body: "Help teams do their best work without fighting their tools." },
      { title: "How we work", body: "Small team, weekly releases, every decision close to the user." },
      { title: "Where we are", body: "Remote-first, with people across a dozen time zones." },
    ],
  },
  {
    slug: "blog",
    navLabel: "Blog",
    eyebrow: "Company",
    title: "The Acme blog.",
    summary: "Notes on building Acme, and on building product in general.",
    sections: [
      { title: "Why one tool beats ten", body: "The hidden cost of stitching your stack together." },
      { title: "Shipping weekly", body: "How a small team keeps a steady release cadence." },
      { title: "Designing for focus", body: "The principles behind the Acme interface." },
    ],
  },
  {
    slug: "careers",
    navLabel: "Careers",
    eyebrow: "Company",
    title: "Work at Acme.",
    summary: "We are a small team that ships a lot. If that sounds good, come build with us.",
    sections: [
      { title: "Engineering", body: "Full-stack engineers who care about craft and speed." },
      { title: "Design", body: "Product designers who sweat the details." },
      { title: "Support", body: "People who love helping customers succeed." },
    ],
  },
  {
    slug: "contact",
    navLabel: "Contact",
    eyebrow: "Company",
    title: "Get in touch.",
    summary: "Questions, feedback, or just want to say hi? We would love to hear from you.",
    sections: [
      { title: "Sales", body: "Talk through whether Acme is a fit for your team." },
      { title: "Support", body: "Get help from a human, usually within a few hours." },
      { title: "Press", body: "Logos, screenshots, and company facts for the press." },
    ],
  },
  {
    slug: "privacy",
    navLabel: "Privacy",
    eyebrow: "Company",
    title: "Privacy.",
    summary: "How Acme handles your data, in plain language.",
    sections: [
      { title: "What we collect", body: "Only what we need to run the product and support you." },
      { title: "How we use it", body: "To operate Acme — never sold, never rented." },
      { title: "Your control", body: "Export or delete your data at any time." },
    ],
  },
];

export type Comparison = {
  slug: string;
  navLabel: string;
  competitor: string;
  title: string;
  summary: string;
  rows: { dim: string; acme: string; them: string }[];
};

// Generic, made-up competitors so the template ships no real brand names.
export const COMPARISONS: Comparison[] = [
  {
    slug: "beacon",
    navLabel: "Acme vs Beacon",
    competitor: "Beacon",
    title: "Acme vs Beacon",
    summary:
      "Beacon is a capable tool, but it splits projects, docs, and automation across separate products. Acme brings them into one fast workspace.",
    rows: [
      { dim: "Projects, tasks, and docs", acme: "In one workspace", them: "Separate products" },
      { dim: "Real-time sync", acme: "Built in", them: "Add-on" },
      { dim: "Automations", acme: "Included", them: "Higher tier" },
      { dim: "Typed API", acme: "Yes", them: "Partial" },
      { dim: "Setup time", acme: "Minutes", them: "Hours" },
    ],
  },
  {
    slug: "orbit",
    navLabel: "Acme vs Orbit",
    competitor: "Orbit",
    title: "Acme vs Orbit",
    summary:
      "Orbit is flexible but slow to set up and heavy to run. Acme gives you the same power with a fraction of the overhead.",
    rows: [
      { dim: "Time to first project", acme: "Same day", them: "Onboarding required" },
      { dim: "Speed", acme: "Instant, real-time", them: "Page reloads" },
      { dim: "Per-seat pricing", acme: "No surprises", them: "Adds up fast" },
      { dim: "Analytics", acme: "Built in", them: "Separate tool" },
      { dim: "Learning curve", acme: "Gentle", them: "Steep" },
    ],
  },
  {
    slug: "tempo",
    navLabel: "Acme vs Tempo",
    competitor: "Tempo",
    title: "Acme vs Tempo",
    summary:
      "Tempo is built for managers; Acme is built for the whole team. Everyone gets a fast, shared view of the work.",
    rows: [
      { dim: "Designed for", acme: "The whole team", them: "Managers" },
      { dim: "Daily driver", acme: "Yes", them: "Reporting layer" },
      { dim: "Docs included", acme: "Yes", them: "No" },
      { dim: "Automations", acme: "Included", them: "Limited" },
      { dim: "Self-serve", acme: "Yes", them: "Sales-led" },
    ],
  },
];

export function bySlug<T extends { slug: string }>(
  list: T[],
  slug: string,
): T | undefined {
  return list.find((x) => x.slug === slug);
}
