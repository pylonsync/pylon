// THE single source of truth for everything business-specific on this agency
// site. Rebrand the whole studio by editing this ONE file — the landing page,
// layout, and the seedCapacity function all read from here. The create-pylon
// scaffolder and Mast target this file: a whole studio site is themed from one
// typed object.
//
// Colors live here (applied as CSS variables on <html> in app/layout.tsx).
// Fictional demo copy — replace the values, keep the shape. Anywhere a real
// photo belongs (case-study shots, team headshots) the page renders a clearly
// marked placeholder — swap those for <img>s when you have the assets.

/* ----------------------------- types ----------------------------- */

export type Social = { label: string; href: string; path: string };

export type BaseConfig = {
  brand: {
    name: string;
    letter: string; // monogram
    domain: string;
    email: string;
    footerBlurb: string;
    copyrightName: string;
    socials: Social[];
  };
  colors: { brand: string; brandSoft: string; paper: string };
  seo: { title: string; description: string };
};

export type Service = { title: string; body: string; icon?: string };

// A portfolio piece + case study. `seedProjects` writes these into the public
// Project entity on first visit; after that the owner curates them from the
// dashboard. `selected` features it on the homepage; the challenge/approach/
// outcome render on the /work/[slug] case-study page. `slug` is the URL segment
// (auto-derived from the title if omitted).
export type CaseStudy = {
  title: string;
  slug?: string;
  client: string; // display label, e.g. "Fintech · 0→1"
  summary: string;
  year?: string;
  tags: string[];
  selected?: boolean; // featured on the homepage "Selected work" grid
  challenge?: string;
  approach?: string;
  outcome?: string;
  liveUrl?: string;
};

export type ProcessStep = { title: string; body: string };
export type TeamMember = { name: string; role: string };
export type Testimonial = { quote: string; name: string; role: string };

// Demo back-office rows. `seedStudioBackoffice` (owner-gated) writes these into
// the private Client + Invoice entities on the owner's first dashboard visit, so
// the dashboard isn't an empty shell. `invoices[].client` matches a `clients[]`
// name; `invoices[].projectSlug` (optional) ties a bill to a case study.
export type ClientSeed = {
  name: string;
  company?: string;
  email?: string;
  phone?: string;
  status?: "prospect" | "active" | "past";
  notes?: string;
};
export type InvoiceSeed = {
  number: string;
  client: string; // matches a clients[].name
  projectSlug?: string; // matches a work.items[].slug
  amountCents: number;
  status?: "draft" | "sent" | "paid" | "overdue";
  issuedAt?: string; // "2026-04-01"
  dueAt?: string;
};

export type AgencyConfig = BaseConfig & {
  hero: {
    tagline: string;
    headline: string;
    subcopy: string;
    ctaLabel: string;
    secondaryCtaLabel: string;
  };
  // Seeds the public Capacity row on first visit; after that it lives in the DB
  // and the owner manages it from the dashboard. `openSlots` is the number the
  // hero shows live; `label` is the booking window it refers to.
  capacity: { label: string; openSlots: number };
  logos: { eyebrow: string; names: string[] };
  services: { eyebrow: string; headline: string; items: Service[] };
  work: { eyebrow: string; headline: string; items: CaseStudy[] };
  // Demo CRM + billing rows seeded into the owner dashboard on first visit.
  backoffice: { clients: ClientSeed[]; invoices: InvoiceSeed[] };
  process: { eyebrow: string; headline: string; steps: ProcessStep[] };
  team: { eyebrow: string; headline: string; members: TeamMember[] };
  testimonials?: { eyebrow: string; headline: string; items: Testimonial[] };
  contact: {
    eyebrow: string;
    headline: string;
    subcopy: string;
    projectTypes: string[];
    budgets: string[];
    confirmationMessage: string;
  };
};

/* ----------------------------- config ---------------------------- */

export const siteConfig: AgencyConfig = {
  brand: {
    name: "Halyard",
    letter: "H",
    domain: "halyard.studio",
    email: "hello@halyard.example",
    footerBlurb:
      "A product studio in Dallas. We design and build the software ambitious teams bet on — and we only take on a few projects at a time, so the work stays sharp.",
    copyrightName: "Halyard Studio",
    socials: [
      {
        label: "X",
        href: "https://x.com",
        path: "M18.244 2.25h3.308l-7.227 8.26 8.502 11.24H16.17l-5.214-6.817L4.99 21.75H1.68l7.73-8.835L1.254 2.25H8.08l4.713 6.231zm-1.161 17.52h1.833L7.084 4.126H5.117z",
      },
      {
        label: "LinkedIn",
        href: "https://linkedin.com",
        path: "M20.45 20.45h-3.56v-5.57c0-1.33-.02-3.04-1.85-3.04-1.85 0-2.13 1.45-2.13 2.94v5.67H9.35V9h3.42v1.56h.05c.48-.9 1.64-1.85 3.37-1.85 3.6 0 4.27 2.37 4.27 5.46v6.28zM5.34 7.43a2.06 2.06 0 1 1 0-4.13 2.06 2.06 0 0 1 0 4.13zM7.12 20.45H3.55V9h3.57v11.45zM22.22 0H1.77C.79 0 0 .77 0 1.73v20.54C0 23.22.79 24 1.77 24h20.45c.98 0 1.78-.78 1.78-1.73V1.73C24 .77 23.2 0 22.22 0z",
      },
    ],
  },

  colors: { brand: "#4f46e5", brandSoft: "#e0e7ff", paper: "#fafafa" },

  seo: {
    title: "Halyard — a product studio for ambitious teams.",
    description:
      "Halyard is a Dallas product studio. We design and build web and mobile software end-to-end. We take on a few projects at a time — see how many slots are open this quarter.",
  },

  hero: {
    tagline: "Product studio · Dallas",
    headline: "We build the products teams bet on.",
    subcopy:
      "Halyard is a small, senior team that designs and ships web and mobile software end-to-end. We take on a handful of projects at a time, so the work — and your launch — stays sharp.",
    ctaLabel: "Start a project",
    secondaryCtaLabel: "See our work",
  },

  capacity: { label: "Q3 2026", openSlots: 3 },

  logos: {
    eyebrow: "Trusted by teams at",
    names: ["Northwind", "Lumen", "Foundry", "Atlas", "Cohort", "Vela"],
  },

  services: {
    eyebrow: "What we do",
    headline: "One team, the whole build.",
    items: [
      {
        icon: "◆",
        title: "Product design",
        body: "Research, flows, and interface design that turns a fuzzy idea into something people understand in seconds.",
      },
      {
        icon: "◇",
        title: "Web & mobile",
        body: "Production engineering across web, iOS, and Android — typed, tested, and built to scale past launch day.",
      },
      {
        icon: "◈",
        title: "Brand & identity",
        body: "Naming, logo, and a system that makes a young product feel like it's been around — and worth paying for.",
      },
      {
        icon: "✦",
        title: "Fractional team",
        body: "Embed with your team for a quarter. We plan, build, and hand off — leaving you faster than we found you.",
      },
    ],
  },

  work: {
    eyebrow: "Selected work",
    headline: "A few things we've shipped.",
    items: [
      {
        title: "Ledger",
        slug: "ledger",
        client: "Fintech · 0→1",
        year: "2026",
        summary: "A consumer banking app from first sketch to App Store launch in one quarter.",
        tags: ["Product design", "iOS", "Brand"],
        selected: true,
        challenge:
          "A two-founder fintech had funding and a thesis, but no product, no brand, and a hard 12-week runway to a launchable app the App Store would approve.",
        approach:
          "We ran a one-week scope sprint, then designed and built in parallel — a clickable prototype on real data by week three, weekly TestFlight builds after that. Brand and UI moved together so nothing felt bolted on.",
        outcome:
          "Shipped to the App Store in the 11th week with a 4.8★ launch rating. The founders closed their seed round two weeks later using the live app as the demo.",
        liveUrl: "https://example.com",
      },
      {
        title: "Atlas Health",
        slug: "atlas-health",
        client: "Healthcare · Platform",
        year: "2025",
        summary: "Rebuilt a clinical scheduling tool used by 4,000 providers, with zero downtime.",
        tags: ["Web", "Design system"],
        selected: true,
        challenge:
          "A scheduling platform 4,000 clinicians depended on daily had become impossible to change — every release risked an outage no hospital could tolerate.",
        approach:
          "We introduced a typed design system and migrated screen by screen behind feature flags, shipping to a few clinics at a time and watching the metrics before widening the rollout.",
        outcome:
          "Replaced the entire front end over a quarter with zero scheduled downtime, and cut the time to ship a new screen from two weeks to two days.",
      },
      {
        title: "Cohort",
        slug: "cohort",
        client: "B2B SaaS · Rebrand",
        year: "2025",
        summary: "New identity and marketing site that lifted demo requests 40% in six weeks.",
        tags: ["Brand", "Web"],
        selected: true,
        challenge:
          "A profitable B2B SaaS had outgrown the brand it launched with — the site read like a side project and was quietly losing enterprise deals at the first click.",
        approach:
          "A focused rebrand: new name treatment, a confident visual system, and a marketing site rebuilt around the two proof points buyers actually asked about.",
        outcome:
          "Demo requests rose 40% in the first six weeks, and the sales team stopped apologizing for the website on calls.",
      },
      {
        title: "Vela",
        slug: "vela",
        client: "Logistics · Mobile",
        year: "2024",
        summary: "A driver app with live routing that cut dispatch calls in half.",
        tags: ["Product design", "Android"],
        selected: false,
        challenge:
          "Dispatchers spent their day on the phone because drivers had no live view of their own routes — every change meant a call.",
        approach:
          "We designed an offline-first driver app with live routing and a single, glanceable 'what's next' screen, built for one-handed use in a moving vehicle.",
        outcome:
          "Dispatch calls dropped by half within a month, and on-time delivery climbed eight points.",
      },
    ],
  },

  backoffice: {
    clients: [
      {
        name: "Erin Caldwell",
        company: "Ledger",
        email: "erin@ledger.example",
        phone: "+1 (214) 555-0142",
        status: "active",
        notes: "Founder. Seed round closed; discussing a phase-2 retainer for Q4.",
      },
      {
        name: "Tom Reyes",
        company: "Atlas Health",
        email: "tom@atlashealth.example",
        status: "active",
        notes: "VP Product. Rollout complete; on a monthly maintenance retainer.",
      },
      {
        name: "Sofia Marin",
        company: "Cohort",
        email: "sofia@cohort.example",
        status: "past",
        notes: "Rebrand shipped. Happy to be a reference.",
      },
      {
        name: "Grant Whitaker",
        company: "Northwind",
        email: "grant@northwind.example",
        status: "prospect",
        notes: "Inbound about a 0→1 mobile app. Sent a proposal; awaiting reply.",
      },
    ],
    invoices: [
      {
        number: "INV-001",
        client: "Erin Caldwell",
        projectSlug: "ledger",
        amountCents: 4800000,
        status: "paid",
        issuedAt: "2026-01-15",
        dueAt: "2026-02-14",
      },
      {
        number: "INV-002",
        client: "Tom Reyes",
        projectSlug: "atlas-health",
        amountCents: 6200000,
        status: "paid",
        issuedAt: "2026-03-01",
        dueAt: "2026-03-31",
      },
      {
        number: "INV-003",
        client: "Tom Reyes",
        projectSlug: "atlas-health",
        amountCents: 850000,
        status: "sent",
        issuedAt: "2026-06-01",
        dueAt: "2026-07-01",
      },
      {
        number: "INV-004",
        client: "Sofia Marin",
        projectSlug: "cohort",
        amountCents: 2400000,
        status: "overdue",
        issuedAt: "2026-04-10",
        dueAt: "2026-05-10",
      },
    ],
  },

  process: {
    eyebrow: "How we work",
    headline: "Senior, hands-on, and fast.",
    steps: [
      {
        title: "Scope",
        body: "A focused kickoff week. We pin down the real problem, the riskiest unknowns, and what a great launch looks like.",
      },
      {
        title: "Design",
        body: "Clickable, real-data prototypes — not slide decks. You react to something you can use within two weeks.",
      },
      {
        title: "Build",
        body: "Weekly shipping you can watch. Typed, reviewed code in your repo from day one, no black box.",
      },
      {
        title: "Launch & hand off",
        body: "We ship it, document it, and leave your team able to run and extend it without us.",
      },
    ],
  },

  team: {
    eyebrow: "Who you'll work with",
    headline: "A small, senior team.",
    members: [
      { name: "Claire Donovan", role: "Principal, Design" },
      { name: "Marcus Lee", role: "Principal, Engineering" },
      { name: "Dana Okafor", role: "Brand & Strategy" },
    ],
  },

  testimonials: {
    eyebrow: "Kind words",
    headline: "Teams we've worked with.",
    items: [
      {
        quote:
          "Halyard shipped in a quarter what our last agency couldn't in a year. Senior people, no hand-offs, no drama.",
        name: "Erin Caldwell",
        role: "CEO, Ledger",
      },
      {
        quote:
          "They felt like our team, not a vendor. The work was sharp and the launch was the smoothest we've had.",
        name: "Tom Reyes",
        role: "VP Product, Atlas Health",
      },
      {
        quote:
          "The rebrand paid for itself in two months. We still use the system they built every single day.",
        name: "Sofia Marin",
        role: "Founder, Cohort",
      },
    ],
  },

  contact: {
    eyebrow: "Start a project",
    headline: "Tell us what you're building.",
    subcopy:
      "We take on a few projects at a time. Send a note and we'll reply within two business days — if we're full, we'll tell you straight and point you somewhere good.",
    projectTypes: ["New product (0→1)", "Existing product", "Rebrand", "Not sure yet"],
    budgets: ["$25–50k", "$50–100k", "$100k+", "Let's talk"],
    confirmationMessage:
      "Thanks — your note's in. We'll get back to you within two business days.",
  },
};
