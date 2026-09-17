// Business-specific copy and settings live here. The landing page and layout
// both read this file.
//
// Colors live here too (applied as CSS variables on <html> in app/layout.tsx),
// so re-theming does not require changes to globals.css.
//
// Fictional demo copy. Replace the values and keep the shape.

/* ----------------------------- types ----------------------------- */

export type Social = { label: string; href: string; path: string };

// Shared by every archetype template: brand identity, accent colors, SEO.
export type BaseConfig = {
  brand: {
    name: string;
    letter: string; // logo monogram
    domain: string; // shown in the footer + metadata
    email: string;
    footerBlurb: string;
    copyrightName: string;
    socials: Social[];
  };
  // Marketing accent + surfaces. Applied as CSS vars on <html> in layout.tsx.
  // `ink` is the page background, `paper` the raised panel color, `brand` the
  // one accent, `brandSoft` the accent's dim fill.
  colors: { brand: string; brandSoft: string; paper: string; ink: string };
  seo: { title: string; description: string };
};

export type Fact = { title: string; body: string };
export type Faq = { q: string; a: string };

// A row in the product mock (a task or a note), drawn with plain divs.
export type MockRow = { title: string; tag: string; done?: boolean };

export type WaitlistConfig = BaseConfig & {
  hero: {
    launchNote: string; // one short line, set in mono: when it ships
    headline: string;
    subcopy: string;
    emailPlaceholder: string;
    ctaLabel: string;
    formHint: string;
    successMessage: string;
  };
  counter: {
    enabled: boolean;
    label: string; // e.g. "people on the list"
    // A vanity baseline added to the real, live signup count so a brand-new
    // page doesn't read "0". Set to 0 to show only genuine signups.
    seedCount?: number;
  };
  // The HTML/CSS product window on the right of the hero. No screenshot.
  mock: {
    windowTitle: string;
    sidebar: string[];
    activeSidebarIndex: number;
    rows: MockRow[];
  };
  facts: { headline: string; items: Fact[] };
  faq?: { headline: string; items: Faq[] };
};

/* ----------------------------- config ---------------------------- */

export const siteConfig: WaitlistConfig = {
  brand: {
    name: "Lumo",
    letter: "L",
    domain: "lumo.app",
    email: "hello@lumo.example",
    footerBlurb:
      "Lumo keeps projects, notes, and tasks in one workspace. Join the list for an early invite.",
    copyrightName: "Lumo, Inc.",
    socials: [
      {
        label: "X",
        href: "https://x.com/pylonsync",
        path: "M18.244 2.25h3.308l-7.227 8.26 8.502 11.24H16.17l-5.214-6.817L4.99 21.75H1.68l7.73-8.835L1.254 2.25H8.08l4.713 6.231zm-1.161 17.52h1.833L7.084 4.126H5.117z",
      },
      {
        label: "GitHub",
        href: "https://github.com/pylonsync/pylon",
        path: "M12 .297c-6.63 0-12 5.373-12 12 0 5.303 3.438 9.8 8.205 11.385.6.113.82-.258.82-.577 0-.285-.01-1.04-.015-2.04-3.338.724-4.042-1.61-4.042-1.61C4.422 18.07 3.633 17.7 3.633 17.7c-1.087-.744.084-.729.084-.729 1.205.084 1.838 1.236 1.838 1.236 1.07 1.835 2.809 1.305 3.495.998.108-.776.417-1.305.76-1.605-2.665-.3-5.466-1.332-5.466-5.93 0-1.31.465-2.38 1.235-3.22-.135-.303-.54-1.523.105-3.176 0 0 1.005-.322 3.3 1.23.96-.267 1.98-.399 3-.405 1.02.006 2.04.138 3 .405 2.28-1.552 3.285-1.23 3.285-1.23.645 1.653.24 2.873.12 3.176.765.84 1.23 1.91 1.23 3.22 0 4.61-2.805 5.625-5.475 5.92.42.36.81 1.096.81 2.22 0 1.606-.015 2.896-.015 3.286 0 .315.21.69.825.57C20.565 22.092 24 17.592 24 12.297c0-6.627-5.373-12-12-12",
      },
    ],
  },

  // Near-black page, one electric blue accent.
  colors: { brand: "#4f6bff", brandSoft: "#141a3d", paper: "#121214", ink: "#0b0b0c" },

  seo: {
    title: "Lumo — projects, notes, and tasks in one workspace. Coming soon.",
    description:
      "Lumo puts your projects, notes, and tasks in one workspace. It launches this fall. Join the waitlist for an early invite.",
  },

  hero: {
    launchNote: "Launch: fall 2026",
    headline: "Projects, notes, and tasks in one workspace.",
    subcopy:
      "Lumo is one app for the plan, the notes behind it, and the tasks that come out of it. Join the list and we send an invite before the public launch.",
    emailPlaceholder: "you@work.com",
    ctaLabel: "Join the waitlist",
    formHint: "One invite email. No newsletter.",
    successMessage: "You are on the list. We will email your invite.",
  },

  counter: {
    enabled: true,
    label: "people on the list",
    seedCount: 1200,
  },

  mock: {
    windowTitle: "Site redesign",
    sidebar: ["Inbox", "Site redesign", "Q4 plan", "Reading", "Archive"],
    activeSidebarIndex: 1,
    rows: [
      { title: "Draft the homepage copy", tag: "Today", done: true },
      { title: "Review nav on mobile", tag: "Today" },
      { title: "Pick the display face", tag: "Note" },
      { title: "Send pricing page to Priya", tag: "Tomorrow" },
      { title: "Set up the launch checklist", tag: "Next week" },
    ],
  },

  facts: {
    headline: "What Lumo does",
    items: [
      {
        title: "One place for a project",
        body: "Each project holds its plan, its notes, and its tasks. Nothing lives in a second app.",
      },
      {
        title: "Notes turn into tasks",
        body: "Select a line in a note and make it a task. The task keeps a link back to the note.",
      },
      {
        title: "No feed, no badges",
        body: "Lumo has no activity feed. You see the tasks due today and the project you opened last.",
      },
    ],
  },

  faq: {
    headline: "Questions",
    items: [
      {
        q: "When does Lumo launch?",
        a: "Fall 2026. People on the waitlist get an invite before the public launch.",
      },
      {
        q: "What do I get for joining?",
        a: "An early invite and founding-member pricing on paid plans.",
      },
      {
        q: "How much will it cost?",
        a: "There is a free tier. Paid plans are priced at launch. Waitlist members keep the founding-member price.",
      },
      {
        q: "Will my email be shared?",
        a: "No. We use it for the invite and launch updates only. You can unsubscribe at any time.",
      },
    ],
  },
};
