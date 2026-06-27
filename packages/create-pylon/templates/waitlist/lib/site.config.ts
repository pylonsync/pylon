// THE single source of truth for everything business-specific on this waitlist.
// Rebrand the whole page by editing this ONE file — the landing page and layout
// read from here and stay generic.
//
// Colors live here too (applied as CSS variables on <html> in app/layout.tsx),
// so you don't touch globals.css to re-theme.
//
// Fictional demo copy — replace the values, keep the shape.

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
  colors: { brand: string; brandSoft: string; paper: string };
  seo: { title: string; description: string };
};

export type ValueProp = { title: string; body: string; icon?: string };
export type Quote = { quote: string; name: string; role: string };
export type Faq = { q: string; a: string };

export type WaitlistConfig = BaseConfig & {
  hero: {
    badge: string;
    headline: string;
    subcopy: string;
    emailPlaceholder: string;
    ctaLabel: string;
    successMessage: string;
  };
  counter: {
    enabled: boolean;
    label: string; // e.g. "people on the list"
    // A vanity baseline added to the real, live signup count so a brand-new
    // page doesn't read "0". Set to 0 to show only genuine signups.
    seedCount?: number;
  };
  valueProps: { eyebrow: string; headline: string; items: ValueProp[] };
  socialProof?: {
    label: string;
    logos?: string[];
    quotes?: Quote[];
  };
  faq?: { eyebrow: string; headline: string; items: Faq[] };
};

/* ----------------------------- config ---------------------------- */

export const siteConfig: WaitlistConfig = {
  brand: {
    name: "Lumo",
    letter: "L",
    domain: "lumo.app",
    email: "hello@lumo.example",
    footerBlurb:
      "Lumo is the calm, focused home for everything you're working on. Launching soon — join the list to get an invite first.",
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

  colors: { brand: "#4f46e5", brandSoft: "#eef2ff", paper: "#fafafa" },

  seo: {
    title: "Lumo — the calm home for your work. Coming soon.",
    description:
      "Lumo brings your projects, notes, and tasks into one quiet, focused space. We're launching soon — join the waitlist for an early invite.",
  },

  hero: {
    badge: "Launching this fall",
    headline: "Your work, finally in one calm place.",
    subcopy:
      "Lumo is the focused home for your projects, notes, and tasks — no clutter, no busywork. We're opening the doors soon. Join the list and we'll send you an invite first.",
    emailPlaceholder: "you@work.com",
    ctaLabel: "Join the waitlist",
    successMessage: "You're on the list — we'll email your invite soon.",
  },

  counter: {
    enabled: true,
    label: "people already waiting",
    seedCount: 1200,
  },

  valueProps: {
    eyebrow: "Why Lumo",
    headline: "Built for focus, not for busywork.",
    items: [
      {
        icon: "◇",
        title: "One quiet space",
        body: "Projects, notes, and tasks live together — so nothing gets lost between five different apps.",
      },
      {
        icon: "◎",
        title: "For people who make things",
        body: "Designers, founders, writers, builders. If your work needs deep focus, Lumo is built for you.",
      },
      {
        icon: "✦",
        title: "Calm by default",
        body: "No noisy notifications, no endless feeds. Lumo shows you what matters now and gets out of the way.",
      },
    ],
  },

  socialProof: {
    label: "Loved by early testers",
    quotes: [
      {
        quote:
          "I've tried every productivity app out there. Lumo is the first one that actually made me feel calmer, not busier.",
        name: "Maya Chen",
        role: "Product designer",
      },
      {
        quote:
          "Finally, one place for all my half-finished projects. I check it every morning and actually know what to do next.",
        name: "Daniel Reyes",
        role: "Indie founder",
      },
      {
        quote:
          "It's fast, it's quiet, and it stays out of my way. I joined the waitlist the day I saw the first demo.",
        name: "Hannah Brooks",
        role: "Writer",
      },
    ],
  },

  faq: {
    eyebrow: "Questions",
    headline: "Good to know.",
    items: [
      {
        q: "When does Lumo launch?",
        a: "We're aiming for this fall. Waitlist members get an invite before the public launch.",
      },
      {
        q: "What do I get for joining?",
        a: "An early invite, founding-member pricing, and a direct line to shape what we build next.",
      },
      {
        q: "How much will it cost?",
        a: "There'll be a generous free tier. Waitlist members lock in founding-member pricing on paid plans.",
      },
      {
        q: "Will my email be shared?",
        a: "Never. We only use it to send your invite and the occasional launch update. Unsubscribe anytime.",
      },
    ],
  },
};
