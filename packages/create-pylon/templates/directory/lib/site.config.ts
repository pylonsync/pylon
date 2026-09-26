// Business-specific copy and settings live here. The landing page, layout,
// seedListings function, create-pylon scaffolder, and Mast all read this typed
// object.
//
// Colors live here (applied as CSS variables on <html> in app/layout.tsx).
// Fictional demo copy. Replace the values and keep the shape.

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
  // The page opens with a headline, one line of description, and the search
  // box. The nav already shows the brand name, so the headline says what the
  // directory holds.
  intro: {
    headline: string;
    description: string;
    ctaLabel: string;
    searchPlaceholder: string;
  };
  // The facet values offered in the submit form's category select. The browse
  // facet sidebar is built live from the data, so this only needs to cover what
  // submitters can pick.
  categories: string[];
  seedListings: SeedListing[];
  submit: {
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
      "A hand-checked directory of developer tools. Search it, sort by votes, and submit missing entries.",
    copyrightName: "Stacked",
    socials: [
      {
        label: "X",
        href: "https://x.com",
        path: "M18.244 2.25h3.308l-7.227 8.26 8.502 11.24H16.17l-5.214-6.817L4.99 21.75H1.68l7.73-8.835L1.254 2.25H8.08l4.713 6.231zm-1.161 17.52h1.833L7.084 4.126H5.117z",
      },
    ],
  },

  colors: { brand: "#1d4ed8", brandSoft: "#eff6ff", paper: "#ffffff" },

  seo: {
    title: "Stacked, a developer tools directory",
    description:
      "A searchable, community-voted directory of developer tools. Full-text search, category filters, upvotes, and a submit form.",
  },

  intro: {
    headline: "Developer tools, checked by hand.",
    description:
      "Every entry is reviewed before it goes live. Search, filter by category, and upvote. No account needed.",
    ctaLabel: "Submit a tool",
    searchPlaceholder: "search: database, deploy, auth",
  },

  categories: ["Database", "Hosting", "Auth", "AI", "Analytics", "Design", "Productivity", "DevOps"],

  // Fictional starter entries across categories. The names are made up so no
  // entry reads as a real product. Replace them with your own; `votes` seeds the
  // leaderboard so "Top voted" isn't a flat list.
  seedListings: [
    { name: "Tidepool DB", tagline: "Postgres with a realtime sync engine baked in.", url: "https://example.com/tidepool", category: "Database", tags: "postgres, realtime, sync", votes: 342, featured: true },
    { name: "Brineway", tagline: "S3-compatible object storage at the edge.", url: "https://example.com/brineway", category: "Hosting", tags: "storage, edge, s3", votes: 218 },
    { name: "Keyloft", tagline: "Drop-in auth: passkeys, OAuth, and magic links.", url: "https://example.com/keyloft", category: "Auth", tags: "auth, passkeys, oauth", votes: 287, featured: true },
    { name: "Relaygate", tagline: "Self-hostable LLM gateway with caching + routing.", url: "https://example.com/relaygate", category: "AI", tags: "llm, gateway, cache", votes: 401, featured: true },
    { name: "Countwell", tagline: "Privacy-first product analytics you can self-host.", url: "https://example.com/countwell", category: "Analytics", tags: "analytics, privacy", votes: 174 },
    { name: "Tokenmill", tagline: "A design-token pipeline from Figma to code.", url: "https://example.com/tokenmill", category: "Design", tags: "design, tokens, figma", votes: 132 },
    { name: "Cronhollow", tagline: "Background jobs + cron with a visual dashboard.", url: "https://example.com/cronhollow", category: "DevOps", tags: "jobs, cron, queue", votes: 209 },
    { name: "Letterpane", tagline: "Transactional email with a real templating story.", url: "https://example.com/letterpane", category: "Productivity", tags: "email, templates", votes: 96 },
    { name: "Branchlight", tagline: "Preview deploys for every PR, in seconds.", url: "https://example.com/branchlight", category: "Hosting", tags: "deploy, ci, preview", votes: 263 },
    { name: "Nearfield", tagline: "Managed vector search without the ops.", url: "https://example.com/nearfield", category: "AI", tags: "vector, search, rag", votes: 188 },
    { name: "Migratory", tagline: "Type-safe migrations that review themselves.", url: "https://example.com/migratory", category: "Database", tags: "migrations, types", votes: 151 },
    { name: "Lookoutly", tagline: "Uptime + error tracking in one calm dashboard.", url: "https://example.com/lookoutly", category: "DevOps", tags: "monitoring, errors", votes: 144 },
  ],

  submit: {
    headline: "Submit a tool",
    subcopy:
      "Submissions go to the review queue. Approved tools appear in the directory.",
    confirmationMessage: "Your submission is in the queue. We review new tools a few times a week.",
  },
};
