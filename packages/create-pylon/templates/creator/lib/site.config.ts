// THE single source of truth for everything personal/business-specific on this
// creator site. Rebrand the whole page by editing this ONE file — the landing
// page and layout read from here and stay generic. The create-pylon scaffolder
// and Mast target this file too.
//
// Colors live here (applied as CSS variables on <html> in app/layout.tsx).
// Fictional demo copy — replace the values, keep the shape.

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

export type Offering = { title: string; body: string; price?: string };
export type Testimonial = { quote: string; name: string; role: string };
export type LinkItem = { label: string; href: string; note?: string };

export type CreatorConfig = BaseConfig & {
  hero: {
    badge: string;
    name: string; // the person / personal brand
    tagline: string; // one-line "what you do"
    intro: string; // a short paragraph
  };
  about: { eyebrow: string; headline: string; paragraphs: string[] };
  offerings: { eyebrow: string; headline: string; items: Offering[] };
  testimonials?: { eyebrow: string; headline: string; items: Testimonial[] };
  // The realtime feature: a live newsletter subscriber count.
  newsletter: {
    eyebrow: string;
    headline: string;
    subcopy: string;
    emailPlaceholder: string;
    ctaLabel: string;
    successMessage: string;
    counterLabel: string; // e.g. "readers subscribed"
    seedCount?: number; // vanity baseline added to the real live count
  };
  links?: { eyebrow: string; headline: string; items: LinkItem[] };
};

/* ----------------------------- config ---------------------------- */

export const siteConfig: CreatorConfig = {
  brand: {
    name: "Maya Rivera",
    letter: "M",
    domain: "mayarivera.co",
    email: "hello@mayarivera.example",
    footerBlurb:
      "Product design coach and writer. I help designers do braver work — through 1:1 coaching, portfolio reviews, and a weekly newsletter.",
    copyrightName: "Maya Rivera",
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

  colors: { brand: "#0d9488", brandSoft: "#ccfbf1", paper: "#fafafa" },

  seo: {
    title: "Maya Rivera — product design coach & writer",
    description:
      "Product design coaching, portfolio reviews, and a weekly newsletter for designers who want to do braver work.",
  },

  hero: {
    badge: "Now coaching · 2 spots open",
    name: "Maya Rivera",
    tagline: "Product design coach & writer.",
    intro:
      "I help product designers get unstuck, sharpen their portfolios, and do the bravest work of their careers. Fifteen years in the room; now I spend it in yours.",
  },

  about: {
    eyebrow: "About",
    headline: "Hi, I'm Maya.",
    paragraphs: [
      "I've led design at two startups and a public company, shipped products to millions, and mentored designers who now lead teams of their own.",
      "These days I coach one-on-one, review portfolios, and write a weekly newsletter about the craft and the career. No fluff, no hustle — just the things that actually move your work forward.",
    ],
  },

  offerings: {
    eyebrow: "Work with me",
    headline: "Three ways I can help.",
    items: [
      {
        title: "1:1 coaching",
        body: "Monthly sessions on whatever's in your way — craft, career, confidence. We build a plan and I hold you to it.",
        price: "from $400/mo",
      },
      {
        title: "Portfolio review",
        body: "A deep, honest teardown of your portfolio and a recorded walkthrough with everything I'd change.",
        price: "$250",
      },
      {
        title: "Team workshops",
        body: "Half-day workshops on critique, design systems, and shipping faster without lowering the bar.",
        price: "let's talk",
      },
    ],
  },

  testimonials: {
    eyebrow: "Kind words",
    headline: "From people I've worked with.",
    items: [
      {
        quote:
          "Maya helped me land a senior role in three months. The portfolio review alone was worth ten times the price.",
        name: "Daniel Reyes",
        role: "Senior Product Designer",
      },
      {
        quote:
          "Our whole team got sharper after Maya's critique workshop. Calmer feedback, better work, less ego.",
        name: "Priya Sharma",
        role: "Design Manager",
      },
      {
        quote:
          "The newsletter is the only one I actually read every week. It's like a coaching session in my inbox.",
        name: "Marcus Bell",
        role: "Product Designer",
      },
    ],
  },

  newsletter: {
    eyebrow: "The Studio Notes",
    headline: "A weekly letter on the craft and the career.",
    subcopy:
      "One short, useful email every Sunday — on design, taste, and doing brave work. No spam, unsubscribe anytime.",
    emailPlaceholder: "you@email.com",
    ctaLabel: "Subscribe",
    successMessage: "You're in — watch your inbox this Sunday.",
    counterLabel: "designers reading",
    seedCount: 2400,
  },

  links: {
    eyebrow: "Elsewhere",
    headline: "Find me around the web.",
    items: [
      { label: "Portfolio", href: "#", note: "Selected work, 2010–today" },
      { label: "Read the archive", href: "#", note: "Every past issue of The Studio Notes" },
      { label: "Book a free intro call", href: "#", note: "15 minutes, no pitch" },
    ],
  },
};
