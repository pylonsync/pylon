// THE single source of truth for everything business-specific on this directory
// site. Rebrand the whole thing by editing this ONE file — the landing page,
// layout, and the seedListings function all read from here. The create-pylon
// scaffolder and Mast target this file: a whole directory is themed + seeded
// from one typed object.
//
// Colors live here (applied as CSS variables on <html> in app/layout.tsx).
// Fictional demo copy — replace the values, keep the shape.

/* ----------------------------- types ----------------------------- */

export type Social = { label: string; href: string; path: string };

export type BaseConfig = {
  brand: {
    name: string;
    letter: string;
    domain: string;
    email: string;
    footerBlurb: string;
    copyrightName: string;
    socials: Social[];
  };
  colors: { brand: string; brandSoft: string; paper: string };
  seo: { title: string; description: string };
};

// A starter directory entry (seeds the public Listing table on first visit).
export type SeedListing = {
  name: string;
  tagline: string;
  url: string;
  category: string;
  tags?: string; // comma-separated
  description?: string;
  votes?: number;
  featured?: boolean;
};

export type DirectoryConfig = BaseConfig & {
  hero: {
    tagline: string;
    headline: string;
    subcopy: string;
    ctaLabel: string;
    searchPlaceholder: string;
  };
  browse: { eyebrow: string; headline: string };
  // The facet values offered in the submit form's category select. The browse
  // facet sidebar is built live from the data, so this only needs to cover what
  // submitters can pick.
  categories: string[];
  seedListings: SeedListing[];
  submit: {
    eyebrow: string;
    headline: string;
    subcopy: string;
    confirmationMessage: string;
  };
};

/* ----------------------------- config ---------------------------- */

export const siteConfig: DirectoryConfig = {
  brand: {
    name: "Stacked",
    letter: "S",
    domain: "stacked.dev",
    email: "hello@stacked.example",
    footerBlurb:
      "A hand-checked directory of the tools developers actually reach for. Search it, sort by votes, and submit the ones we're missing.",
    copyrightName: "Stacked",
    socials: [
      {
        label: "X",
        href: "https://x.com",
        path: "M18.244 2.25h3.308l-7.227 8.26 8.502 11.24H16.17l-5.214-6.817L4.99 21.75H1.68l7.73-8.835L1.254 2.25H8.08l4.713 6.231zm-1.161 17.52h1.833L7.084 4.126H5.117z",
      },
    ],
  },

  colors: { brand: "#7c3aed", brandSoft: "#ede9fe", paper: "#faf9fc" },

  seo: {
    title: "Stacked — the developer tools directory.",
    description:
      "A searchable, community-voted directory of developer tools. Full-text search, filter by category, upvote your favorites, and submit the ones we're missing.",
  },

  hero: {
    tagline: "The dev tools directory",
    headline: "The tools developers actually use.",
    subcopy:
      "Search a hand-checked directory of developer tools, filter by category, and upvote the ones that earn it. Live results, live votes — no signup to browse.",
    ctaLabel: "Submit a tool",
    searchPlaceholder: "Search tools — try “database”, “deploy”, “auth”…",
  },

  browse: {
    eyebrow: "Browse",
    headline: "Find your next tool.",
  },

  categories: ["Database", "Hosting", "Auth", "AI", "Analytics", "Design", "Productivity", "DevOps"],

  // Fictional-but-plausible starter entries across categories. Replace with your
  // own; `votes` seeds the leaderboard so "Top voted" isn't a flat list.
  seedListings: [
    { name: "Quill", tagline: "Postgres with a realtime sync engine baked in.", url: "https://example.com/quill", category: "Database", tags: "postgres, realtime, sync", votes: 342, featured: true },
    { name: "Tigris Store", tagline: "S3-compatible object storage at the edge.", url: "https://example.com/tigris", category: "Hosting", tags: "storage, edge, s3", votes: 218 },
    { name: "Gatekeep", tagline: "Drop-in auth: passkeys, OAuth, and magic links.", url: "https://example.com/gatekeep", category: "Auth", tags: "auth, passkeys, oauth", votes: 287, featured: true },
    { name: "Loom AI", tagline: "Self-hostable LLM gateway with caching + routing.", url: "https://example.com/loom", category: "AI", tags: "llm, gateway, cache", votes: 401, featured: true },
    { name: "Pulse", tagline: "Privacy-first product analytics you can self-host.", url: "https://example.com/pulse", category: "Analytics", tags: "analytics, privacy", votes: 174 },
    { name: "Frame", tagline: "A design-token pipeline from Figma to code.", url: "https://example.com/frame", category: "Design", tags: "design, tokens, figma", votes: 132 },
    { name: "Cadence", tagline: "Background jobs + cron with a visual dashboard.", url: "https://example.com/cadence", category: "DevOps", tags: "jobs, cron, queue", votes: 209 },
    { name: "Inbox Zero", tagline: "Transactional email with a real templating story.", url: "https://example.com/inbox", category: "Productivity", tags: "email, templates", votes: 96 },
    { name: "Shipyard", tagline: "Preview deploys for every PR, in seconds.", url: "https://example.com/shipyard", category: "Hosting", tags: "deploy, ci, preview", votes: 263 },
    { name: "Vector Vault", tagline: "Managed vector search without the ops.", url: "https://example.com/vectorvault", category: "AI", tags: "vector, search, rag", votes: 188 },
    { name: "Schema", tagline: "Type-safe migrations that review themselves.", url: "https://example.com/schema", category: "Database", tags: "migrations, types", votes: 151 },
    { name: "Watchtower", tagline: "Uptime + error tracking in one calm dashboard.", url: "https://example.com/watchtower", category: "DevOps", tags: "monitoring, errors", votes: 144 },
  ],

  submit: {
    eyebrow: "Submit a tool",
    headline: "Know one we're missing?",
    subcopy:
      "Send it over. Submissions land in the curator's queue — once it's approved it shows up in the directory for everyone.",
    confirmationMessage: "Thanks — your submission's in the queue. We review new tools a few times a week.",
  },
};
