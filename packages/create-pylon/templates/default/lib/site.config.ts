// Business-specific copy and settings live here. The marketing components,
// `create-pylon` scaffolder, and automated generators all read this file.
//
// Colors live here (applied as CSS variables on <html> in app/layout.tsx), so
// you don't touch globals.css to re-theme the marketing pages.
//
// Fictional demo copy. Replace the values and keep the shape.

/* ----------------------------- types ----------------------------- */

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

export type ContentSection = { title: string; body: string };

export type SitePage = {
  slug: string;
  navLabel: string; // label in nav/footer
  eyebrow: string;
  title: string; // hero headline
  summary: string;
  sections: ContentSection[];
};

export type Comparison = {
  slug: string;
  navLabel: string;
  competitor: string;
  title: string;
  summary: string;
  rows: { dim: string; acme: string; them: string }[];
};

export type Social = { label: string; href: string; path: string };
export type TextItem = { title: string; body: string };
export type IconItem = { icon: string; title: string; body: string };
export type Quote = { quote: string; name: string; role: string };
export type Plan = {
  name: string;
  tagline: string;
  price: string;
  unit: string;
  cta: string;
  featured: boolean;
  features: string[];
};
export type Faq = { q: string; a: string };

export type SiteConfig = {
  brand: {
    name: string;
    letter: string; // logo monogram
    domain: string; // shown in mockup browser frames, e.g. "acme.app"
    email: string;
    footerBlurb: string;
    copyrightName: string;
    socials: Social[];
  };
  // Marketing accent + surfaces. Applied as CSS vars on <html> in layout.tsx.
  colors: { brand: string; brandSoft: string; paper: string };
  seo: { title: string; description: string };
  hero: {
    badge: string;
    headline: string;
    subcopy: string;
    mockupLabel: string;
  };
  logoCloud: { label: string; companies: string[] };
  outcomes: { eyebrow: string; headline: string; body: string; items: TextItem[] };
  featuredTestimonial: { quote: string; name: string; role: string };
  entryPoints: { eyebrow: string; title: string; body: string; items: IconItem[] };
  engagement: {
    eyebrow: string;
    headline: string;
    paragraphs: string[];
    items: TextItem[];
  };
  customers: { eyebrow: string; title: string; body: string; quotes: Quote[] };
  gettingStarted: { eyebrow: string; headline: string; body: string; steps: TextItem[] };
  pricing: { eyebrow: string; headline: string; body: string; plans: Plan[] };
  team: { eyebrow: string; headline: string; body: string; items: TextItem[] };
  finalCta: {
    eyebrow: string;
    headline: string;
    bodyLead: string;
    highlight: string;
    bodyTail: string;
    cta: string;
    footnote: string;
  };
  faq: { eyebrow: string; headline: string; items: Faq[] };
  products: Product[];
  solutions: SitePage[];
  resources: SitePage[];
  company: SitePage[];
  comparisons: Comparison[];
};

/* ----------------------------- config ---------------------------- */

export const siteConfig: SiteConfig = {
  brand: {
    name: "Acme",
    letter: "A",
    domain: "acme.app",
    email: "hello@acme.example",
    footerBlurb:
      "Plan, build, and ship with projects, docs, and automation in one workspace.",
    copyrightName: "Acme, Inc.",
    socials: [
      {
        label: "X",
        href: "https://x.com/acme",
        path: "M18.244 2.25h3.308l-7.227 8.26 8.502 11.24H16.17l-5.214-6.817L4.99 21.75H1.68l7.73-8.835L1.254 2.25H8.08l4.713 6.231zm-1.161 17.52h1.833L7.084 4.126H5.117z",
      },
      {
        label: "GitHub",
        href: "https://github.com/acme",
        path: "M12 .297c-6.63 0-12 5.373-12 12 0 5.303 3.438 9.8 8.205 11.385.6.113.82-.258.82-.577 0-.285-.01-1.04-.015-2.04-3.338.724-4.042-1.61-4.042-1.61C4.422 18.07 3.633 17.7 3.633 17.7c-1.087-.744.084-.729.084-.729 1.205.084 1.838 1.236 1.838 1.236 1.07 1.835 2.809 1.305 3.495.998.108-.776.417-1.305.76-1.605-2.665-.3-5.466-1.332-5.466-5.93 0-1.31.465-2.38 1.235-3.22-.135-.303-.54-1.523.105-3.176 0 0 1.005-.322 3.3 1.23.96-.267 1.98-.399 3-.405 1.02.006 2.04.138 3 .405 2.28-1.552 3.285-1.23 3.285-1.23.645 1.653.24 2.873.12 3.176.765.84 1.23 1.91 1.23 3.22 0 4.61-2.805 5.625-5.475 5.92.42.36.81 1.096.81 2.22 0 1.606-.015 2.896-.015 3.286 0 .315.21.69.825.57C20.565 22.092 24 17.592 24 12.297c0-6.627-5.373-12-12-12",
      },
    ],
  },

  colors: { brand: "#2563eb", brandSoft: "#e8f0fe", paper: "#fafafa" },

  seo: {
    title: "Acme — the workspace where work gets done",
    description:
      "Acme brings your projects, your people, and your updates into one fast, real-time workspace. Plan together, ship together, keep everyone in the loop.",
  },

  hero: {
    badge: "Acme for teams is here →",
    headline: "The workspace where work gets done.",
    subcopy:
      "Acme brings your projects, your people, and your updates into one fast, real-time workspace. Plan together, ship together, and keep everyone in the loop without the busywork.",
    mockupLabel: "Dashboard preview",
  },

  logoCloud: {
    label: "Powering fast-moving teams",
    companies: ["Northwind", "Globex", "Initech", "Umbrella", "Soylent", "Hooli"],
  },

  outcomes: {
    eyebrow: "Why Acme",
    headline: "Keep work moving from kickoff to launch.",
    body: "Most teams lose work somewhere between the kickoff and the ship date. Acme keeps the whole path in one place, so every idea has a clear route from planned, to in progress, to done.",
    items: [
      {
        title: "Work in one place",
        body: "Plans, tasks, and docs live together, so nothing gets lost between tools.",
      },
      {
        title: "Clear priorities",
        body: "Everyone can see what is planned, in progress, and done — and why.",
      },
      {
        title: "Real-time by default",
        body: "Every change syncs instantly, so the workspace is the same on every screen.",
      },
      {
        title: "Less busywork",
        body: "Automations handle the routine, so your team focuses on the work that matters.",
      },
    ],
  },

  featuredTestimonial: {
    quote:
      "Acme gives our team one view of the work. We can see what is happening, decide what is next, and move in the same direction.",
    name: "Maya Chen",
    role: "Head of Product, Northwind",
  },

  entryPoints: {
    eyebrow: "Anywhere",
    title: "Meet your team where they already are.",
    body: "Web, desktop, and a typed API all share the same workspace underneath, so nothing lives in two places.",
    items: [
      {
        icon: "◇",
        title: "Cloud workspace",
        body: "Your whole team works in the browser. Nothing to install, always up to date, and secure by default.",
      },
      {
        icon: "◆",
        title: "Open API",
        body: "A typed API and webhooks for everything in Acme, so you can wire it into the rest of your stack.",
      },
    ],
  },

  engagement: {
    eyebrow: "Momentum",
    headline: "Most tools go quiet after the kickoff.",
    paragraphs: [
      "Someone shares an idea, it lands in a list, and that is the last anyone hears of it. Acme treats every idea as the start of a loop, not the end of one.",
      "When the status moves to planned, the people who care hear about it. When it ships, they hear about it first. And every week a digest pulls them back with the work worth weighing in on.",
      "The loop starts running the same day, without a separate configuration step.",
    ],
    items: [
      {
        title: "The loop",
        body: "Submission, acknowledgment, progress, launch. Four moments every person gets to feel heard.",
      },
      {
        title: "Instant alerts",
        body: "The moment someone votes or comments on their idea, social proof brings them back.",
      },
      {
        title: "Launch alerts",
        body: "The note that tells a person the thing they asked for just shipped. Nothing builds loyalty faster.",
      },
      {
        title: "Weekly digest",
        body: "The top ideas across your boards, delivered every week, with a one-click way to weigh in.",
      },
    ],
  },

  customers: {
    eyebrow: "Customers",
    title: "Teams that ship with Acme.",
    body: "A few words from the people who run their work in Acme every day.",
    quotes: [
      {
        quote:
          "The whole team sees the same work, so we spend less time in status meetings.",
        name: "Daniel Reyes",
        role: "Founder, Globex",
      },
      {
        quote:
          "We replaced five disconnected tools with one place to plan the work.",
        name: "Hannah Brooks",
        role: "Founder, OpenLane",
      },
      {
        quote:
          "People can follow their ideas from submission to launch, which helps us keep them involved.",
        name: "Marcus Bell",
        role: "Cofounder, Initech",
      },
    ],
  },

  gettingStarted: {
    eyebrow: "Get started",
    headline: "Up and running in 60 seconds.",
    body: "Start with an email and a workspace name. No credit card or sales call required.",
    steps: [
      {
        title: "Create a workspace",
        body: "Enter an email and choose a workspace name.",
      },
      {
        title: "Invite your team",
        body: "Add teammates, share the board URL, or link it from your site. Work starts flowing in.",
      },
      {
        title: "Watch the loop start",
        body: "People add ideas, votes pile up, you ship, and everyone hears about it and comes back with more.",
      },
    ],
  },

  pricing: {
    eyebrow: "Pricing",
    headline: "Simple pricing. Every plan.",
    body: "Start free and upgrade as your team grows, with predictable pricing and no annual commitment.",
    plans: [
      {
        name: "Free",
        tagline: "For getting started.",
        price: "$0",
        unit: "forever",
        cta: "Get started",
        featured: false,
        features: [
          "Up to 3 projects",
          "Unlimited members",
          "Real-time board",
          "Community support",
        ],
      },
      {
        name: "Team",
        tagline: "For small teams.",
        price: "$29",
        unit: "/ month",
        cta: "Start free trial",
        featured: true,
        features: [
          "Unlimited projects",
          "Roadmap and updates",
          "Private boards",
          "Priority support",
        ],
      },
      {
        name: "Business",
        tagline: "For growing teams.",
        price: "$59",
        unit: "/ month",
        cta: "Start free trial",
        featured: false,
        features: [
          "Everything in Team",
          "SSO and audit log",
          "Custom domain",
          "Onboarding help",
        ],
      },
    ],
  },

  team: {
    eyebrow: "The team",
    headline: "Built by people who use it every day.",
    body: "Acme is built by a small team that got tired of clunky, overpriced tools. Every feature earns its place by being something worth using every day.",
    items: [
      {
        title: "A small team",
        body: "A small team decides what is worth building and ships it.",
      },
      {
        title: "Design first",
        body: "Beautiful software makes people want to use it. Every screen is built with that in mind.",
      },
      {
        title: "Ships every week",
        body: "Updates go out weekly, often based on work posted directly to our own board.",
      },
      {
        title: "Customer funded",
        body: "We optimize for your renewal, not an exit. If Acme works for you, it works for us.",
      },
    ],
  },

  finalCta: {
    eyebrow: "Start building",
    headline: "Stop losing momentum to busywork.",
    bodyLead: "Plan, decide, and ship in ",
    highlight: "one shared workspace",
    bodyTail: ".",
    cta: "Start building with Acme",
    footnote: "Free to start · No credit card · Cancel anytime",
  },

  faq: {
    eyebrow: "Questions",
    headline: "Frequently asked.",
    items: [
      {
        q: "Is Acme a fit for my team?",
        a: "Acme works for any team that plans and ships work together — product, design, engineering, or ops.",
      },
      {
        q: "What about migrating from another tool?",
        a: "Import your existing projects and pick up where you left off.",
      },
      {
        q: "Is there an API or a way to script this?",
        a: "Yes — every action in Acme is available over a typed API and an MCP server.",
      },
      {
        q: "Do you offer SSO?",
        a: "SSO and audit logging are included on the Business plan.",
      },
    ],
  },

  products: [
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
  ],

  solutions: [
    {
      slug: "startups",
      navLabel: "For startups",
      eyebrow: "Solutions",
      title: "Move fast without losing the thread.",
      summary:
        "Keep a small team aligned as everything changes weekly. Acme gives you one place to plan, build, and ship before the next pivot.",
      sections: [
        { title: "One workspace", body: "Keep projects, tasks, and docs together instead of stitching together separate apps." },
        { title: "Set up in minutes", body: "Invite the team and start working the same day." },
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
      title: "Match Acme to your workflow.",
      summary:
        "Custom fields and statuses adapt Acme to product, design, marketing, and support workflows.",
      sections: [
        { title: "Your workflow", body: "Custom statuses and fields match the way your team already works." },
        { title: "Cross-team work", body: "Hand work between teams without it falling through a crack." },
        { title: "Less status-chasing", body: "Everyone sees the same live picture, so updates write themselves." },
      ],
    },
  ],

  resources: [
    {
      slug: "docs",
      navLabel: "Docs",
      eyebrow: "Resources",
      title: "Documentation.",
      summary: "Set up Acme, invite your team, and configure projects, automations, and integrations.",
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
  ],

  company: [
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
      summary: "Contact the Acme team about sales, support, or press.",
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
    {
      slug: "terms",
      navLabel: "Terms",
      eyebrow: "Company",
      title: "Terms of Service.",
      summary: "The rules for using Acme, in plain language.",
      sections: [
        { title: "Using Acme", body: "Use it for lawful work; don't abuse the service or other customers." },
        { title: "Your content", body: "You own your data — you grant us only what's needed to run the product for you." },
        { title: "Changes & cancellation", body: "Cancel anytime; we give notice before any material change to these terms." },
      ],
    },
  ],

  // Generic, made-up competitors so the template ships no real brand names.
  comparisons: [
    {
      slug: "beacon",
      navLabel: "Acme vs Beacon",
      competitor: "Beacon",
      title: "Acme vs Beacon",
      summary:
        "Beacon separates projects, docs, and automation across products. Acme keeps them in one workspace.",
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
        "Orbit requires more setup and administration. Acme focuses on same-day setup and a smaller operational footprint.",
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
        "Tempo centers management reporting. Acme gives the whole team a shared view of daily work.",
      rows: [
        { dim: "Designed for", acme: "The whole team", them: "Managers" },
        { dim: "Daily driver", acme: "Yes", them: "Reporting layer" },
        { dim: "Docs included", acme: "Yes", them: "No" },
        { dim: "Automations", acme: "Included", them: "Limited" },
        { dim: "Self-serve", acme: "Yes", them: "Sales-led" },
      ],
    },
  ],
};

/* ------------------- back-compat exports + helpers ---------------- */
// Existing imports (`@/lib/products`, `@/lib/site`) keep working via these.

export const PRODUCTS = siteConfig.products;
export function productBySlug(slug: string): Product | undefined {
  return PRODUCTS.find((p) => p.slug === slug);
}

export const SOLUTIONS = siteConfig.solutions;
export const RESOURCES = siteConfig.resources;
export const COMPANY = siteConfig.company;
export const COMPARISONS = siteConfig.comparisons;

export function bySlug<T extends { slug: string }>(
  list: T[],
  slug: string,
): T | undefined {
  return list.find((x) => x.slug === slug);
}
