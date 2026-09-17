// Personal and business-specific copy lives here. The landing page, layout,
// create-pylon scaffolder, and Mast all read this file.
//
// Colors live here (applied as CSS variables on <html> in app/layout.tsx).
// Fictional demo copy. Replace the values and keep the shape.

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
  // brand: the one accent. paper: page background. ink: running text.
  colors: { brand: string; brandSoft: string; paper: string; ink: string };
  seo: { title: string; description: string };
};

export type Offering = { title: string; body: string; price?: string };
export type Testimonial = { quote: string; name: string; role: string };
export type LinkItem = { label: string; href: string; note?: string };
export type Issue = { title: string; date: string; href: string; summary?: string };

export type CreatorConfig = BaseConfig & {
  hero: {
    name: string; // the person / personal brand
    tagline: string; // one-line "what you do"
    intro: string; // two sentences of running text under the name
  };
  about: { headline: string; paragraphs: string[] };
  offerings: { headline: string; items: Offering[] };
  // Recent newsletter issues or posts. Omit to hide the section.
  writing?: { headline: string; items: Issue[] };
  testimonials?: { headline: string; items: Testimonial[] };
  // The realtime feature: a live newsletter subscriber count. The form sits in
  // the hero, under the introduction.
  newsletter: {
    name: string; // the newsletter's name, e.g. "The Studio Notes"
    subcopy: string;
    emailPlaceholder: string;
    ctaLabel: string;
    successMessage: string;
    counterLabel: string; // e.g. "readers subscribed"
    seedCount?: number; // vanity baseline added to the real live count
  };
  links?: { headline: string; body: string; items: LinkItem[] };
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

  colors: { brand: "#4a6b3f", brandSoft: "#e6ecdf", paper: "#f7f3ec", ink: "#2a2622" },

  seo: {
    title: "Maya Rivera — product design coach & writer",
    description:
      "Product design coaching, portfolio reviews, and a weekly newsletter for designers who want to do braver work.",
  },

  hero: {
    name: "Maya Rivera",
    tagline: "Product design coach & writer.",
    intro:
      "I help product designers get unstuck, sharpen their portfolios, and do the bravest work of their careers. I spent fifteen years leading design teams, and now I spend that time with you.",
  },

  about: {
    headline: "About",
    paragraphs: [
      "I've led design at two startups and a public company, shipped products to millions, and mentored designers who now lead teams of their own.",
      "These days I coach one-on-one, review portfolios, and write a weekly newsletter about the craft and the career. Every issue focuses on practical ways to move the work forward.",
    ],
  },

  offerings: {
    headline: "What I do",
    items: [
      {
        title: "1:1 coaching",
        body: "Monthly sessions on whatever's in your way — craft, career, confidence. We build a plan and I hold you to it.",
        price: "from $400/mo",
      },
      {
        title: "Portfolio review",
        body: "A detailed portfolio critique and a recorded walkthrough of the changes I'd make.",
        price: "$250",
      },
      {
        title: "Team workshops",
        body: "Half-day workshops on critique, design systems, and shipping faster without lowering the bar.",
        price: "priced per team",
      },
    ],
  },

  writing: {
    headline: "Recent issues",
    items: [
      {
        title: "Critique is a skill, not a meeting",
        date: "Sep 7, 2026",
        href: "#",
        summary: "How to run a critique that ends with decisions instead of notes.",
      },
      {
        title: "Your portfolio has too many projects",
        date: "Aug 31, 2026",
        href: "#",
        summary: "Three case studies, told well, beat twelve told badly.",
      },
      {
        title: "What senior designers stop asking permission for",
        date: "Aug 24, 2026",
        href: "#",
        summary: "The small decisions that mark the change from mid-level to senior.",
      },
    ],
  },

  testimonials: {
    headline: "From people I have worked with",
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
        name: "Hannah Brooks",
        role: "Design Manager",
      },
      {
        quote:
          "I read the newsletter every week. It feels like a coaching session in my inbox.",
        name: "Marcus Bell",
        role: "Product Designer",
      },
    ],
  },

  newsletter: {
    name: "The Studio Notes",
    subcopy: "One short email every Sunday on design, taste, and doing brave work.",
    emailPlaceholder: "you@email.com",
    ctaLabel: "Subscribe",
    successMessage: "You are in. The next issue arrives on Sunday.",
    counterLabel: "designers reading",
    seedCount: 2400,
  },

  links: {
    headline: "Work with me",
    body:
      "If you want coaching or a portfolio review, book a short intro call first. We talk through what you need, and I tell you whether I can help.",
    items: [
      { label: "Book a free intro call", href: "#", note: "15 minutes, no pitch" },
      { label: "Read the archive", href: "#", note: "Every past issue of The Studio Notes" },
      { label: "Portfolio", href: "#", note: "Selected work, 2010 to today" },
    ],
  },
};
