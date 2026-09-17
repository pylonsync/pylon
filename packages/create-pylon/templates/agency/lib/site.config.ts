// Business-specific copy and settings live here. The landing page, layout, and
// seedCapacity function all read this file.
//
// Colors live here (applied as CSS variables on <html> in app/layout.tsx).
// Fictional demo copy. Replace the values and keep the shape. Anywhere a real
// photo belongs (case-study shots, team headshots) the page renders a clearly
// marked placeholder; swap those for <img>s when you have the assets.

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

export type Service = { title: string; body: string };

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
  // Line items drive the total — `amountCents` is computed from them on seed.
  lineItems: { description: string; quantity: number; unitCents: number }[];
  status?: "draft" | "sent" | "paid" | "overdue";
  issuedAt?: string; // "2026-04-01"
  dueAt?: string;
};

// The studio's billing identity — the "from" block + terms on every invoice and
// its PDF. Edit these to make the invoices yours.
export type Billing = {
  addressLines: string[]; // shown under the studio name on the invoice
  paymentTerms: string; // e.g. "Net 30"
  footerNote: string; // a thank-you / payment note in the invoice footer
};

export type AgencyConfig = BaseConfig & {
  hero: {
    headline: string;
    subcopy: string;
    ctaLabel: string;
    secondaryCtaLabel: string;
  };
  // Seeds the public Capacity row on first visit; after that it lives in the DB
  // and the owner manages it from the dashboard. `openSlots` is the number the
  // hero shows live; `label` is the booking window it refers to.
  capacity: { label: string; openSlots: number };
  services: { headline: string; items: Service[] };
  work: { headline: string; items: CaseStudy[] };
  // The studio's invoice "from" identity + terms.
  billing: Billing;
  // Demo CRM + billing rows seeded into the owner dashboard on first visit.
  backoffice: { clients: ClientSeed[]; invoices: InvoiceSeed[] };
  process: { headline: string; steps: ProcessStep[] };
  team: { headline: string; members: TeamMember[] };
  testimonials?: { headline: string; items: Testimonial[] };
  contact: {
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
      "A Dallas product studio. We design and build web and mobile software and take on a few projects at a time.",
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

  // `brand` colors links only. `paper` is the grey band behind the contact form.
  colors: { brand: "#1d4ed8", brandSoft: "#dbeafe", paper: "#f4f4f2" },

  seo: {
    title: "Halyard, a product studio in Dallas",
    description:
      "Halyard is a Dallas product studio that designs and builds web and mobile software. See how many project slots are open this quarter.",
  },

  hero: {
    headline: "Halyard designs and builds web and mobile products for funded teams that need to ship this quarter.",
    subcopy:
      "We are a small senior team in Dallas. We take on a few projects at a time and work in your repo from the first week.",
    ctaLabel: "Start a project",
    secondaryCtaLabel: "See our work",
  },

  capacity: { label: "Q3 2026", openSlots: 3 },

  services: {
    headline: "What we do",
    items: [
      {
        title: "Product design",
        body: "Research, flows, and interface design. We test a clickable prototype with real users before anything is built.",
      },
      {
        title: "Web and mobile engineering",
        body: "Production code for web, iOS, and Android. Typed, tested, and committed to your repository from day one.",
      },
      {
        title: "Brand and identity",
        body: "Naming, logo, type, and a visual system with the files and rules your team needs to keep using it.",
      },
      {
        title: "Embedded team",
        body: "Two or three of us join your team for a quarter. We plan, build, and hand off with documentation.",
      },
    ],
  },

  work: {
    headline: "Work",
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
          "A two-founder fintech had funding, a thesis, and 12 weeks to create its product, brand, and App Store-ready launch.",
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
        lineItems: [
          { description: "Product design & iOS build — Ledger (12-week engagement)", quantity: 1, unitCents: 4800000 },
        ],
        status: "paid",
        issuedAt: "2026-01-15",
        dueAt: "2026-02-14",
      },
      {
        number: "INV-002",
        client: "Tom Reyes",
        projectSlug: "atlas-health",
        lineItems: [
          { description: "Platform rebuild & design system", quantity: 1, unitCents: 5500000 },
          { description: "Discovery & scoping sprint", quantity: 1, unitCents: 700000 },
        ],
        status: "paid",
        issuedAt: "2026-03-01",
        dueAt: "2026-03-31",
      },
      {
        number: "INV-003",
        client: "Tom Reyes",
        projectSlug: "atlas-health",
        lineItems: [
          { description: "Maintenance retainer — June", quantity: 1, unitCents: 850000 },
        ],
        status: "sent",
        issuedAt: "2026-06-01",
        dueAt: "2026-07-01",
      },
      {
        number: "INV-004",
        client: "Sofia Marin",
        projectSlug: "cohort",
        lineItems: [
          { description: "Brand identity & system", quantity: 1, unitCents: 1400000 },
          { description: "Marketing site design & build", quantity: 1, unitCents: 1000000 },
        ],
        status: "overdue",
        issuedAt: "2026-04-10",
        dueAt: "2026-05-10",
      },
    ],
  },

  billing: {
    addressLines: ["Halyard Studio", "Dallas, TX", "hello@halyard.example"],
    paymentTerms: "Net 30",
    footerNote: "Thank you. Please reference the invoice number with payment.",
  },

  process: {
    headline: "How a project runs",
    steps: [
      {
        title: "Scope",
        body: "One week. We agree on the problem, the riskiest unknowns, and what launch has to include.",
      },
      {
        title: "Design",
        body: "Within two weeks, you can test a clickable prototype running on real data.",
      },
      {
        title: "Build",
        body: "We ship every week to a staging build you can open. All code lives in your repository.",
      },
      {
        title: "Launch and hand off",
        body: "We release, write the docs, and stay for two weeks after launch to fix what comes up.",
      },
    ],
  },

  team: {
    headline: "Who you will work with",
    members: [
      { name: "Claire Donovan", role: "Principal, Design" },
      { name: "Marcus Lee", role: "Principal, Engineering" },
      { name: "Dana Okafor", role: "Brand & Strategy" },
    ],
  },

  testimonials: {
    headline: "From clients",
    items: [
      {
        quote:
          "Halyard shipped in a quarter what our last agency could not in a year. Senior people, no hand-offs.",
        name: "Erin Caldwell",
        role: "CEO, Ledger",
      },
      {
        quote:
          "They worked like part of our team. The rollout had zero downtime, which we had never managed before.",
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
    headline: "Tell us what you are building",
    subcopy:
      "We take on a few projects at a time. Send a note and we reply within two business days. If we are full, we say so and suggest another studio.",
    projectTypes: ["New product (0→1)", "Existing product", "Rebrand", "Not sure yet"],
    budgets: ["$25–50k", "$50–100k", "$100k+", "Let's talk"],
    confirmationMessage:
      "Thanks. Your note is in. We reply within two business days.",
  },
};
