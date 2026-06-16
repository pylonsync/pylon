// THE single source of truth for everything business-specific. Rebrand the
// whole site — and reconfigure the reservation engine — by editing this ONE
// file. The landing page, layout, AND the createReservation server function all
// read from here, so the menu, seating hours, and table count stay in lockstep.
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

export type MenuItem = { name: string; price: string; desc?: string; tags?: string[] };
export type MenuSection = { name: string; items: MenuItem[] };
export type DayHours = { open: string; close: string } | null; // last-seating window
export type Review = { quote: string; name: string; rating?: number };
export type Faq = { q: string; a: string };

export type RestaurantConfig = BaseConfig & {
  hero: {
    tagline: string;
    headline: string;
    subcopy: string;
    ctaLabel: string;
    quickFacts: { hours: string; area: string; phone: string };
  };
  menu: { eyebrow: string; headline: string; sections: MenuSection[] };
  reservations: {
    enabled: boolean;
    eyebrow: string;
    headline: string;
    subcopy: string;
    slotMinutes: number; // gap between seatings, e.g. 30
    leadTimeHours: number;
    daysAhead: number;
    tablesPerSlot: number; // capacity per seating time
    maxPartySize: number;
    hours: Record<number, DayHours>; // 0=Sun … 6=Sat; null = closed
    confirmationMessage: string;
  };
  reviews?: { eyebrow: string; headline: string; items: Review[] };
  location: {
    eyebrow: string;
    headline: string;
    address: string;
    mapEmbedUrl?: string;
    hoursText: string;
    phone: string;
    email: string;
  };
  faq?: { eyebrow: string; headline: string; items: Faq[] };
};

/* ----------------------------- config ---------------------------- */

export const siteConfig: RestaurantConfig = {
  brand: {
    name: "Cedar & Vine",
    letter: "C",
    domain: "cedarandvine.com",
    email: "hello@cedarandvine.example",
    footerBlurb:
      "A seasonal neighborhood bistro in Dallas. Wood-fired plates, a short natural-wine list, and a table waiting for you. Reserve in seconds.",
    copyrightName: "Cedar & Vine",
    socials: [
      {
        label: "Instagram",
        href: "https://instagram.com",
        path: "M12 2.2c3.2 0 3.6 0 4.85.07 1.17.05 1.8.25 2.23.41.56.22.96.48 1.38.9.42.42.68.82.9 1.38.16.42.36 1.06.41 2.23.06 1.27.07 1.65.07 4.85s0 3.58-.07 4.85c-.05 1.17-.25 1.8-.41 2.23-.22.56-.48.96-.9 1.38-.42.42-.82.68-1.38.9-.42.16-1.06.36-2.23.41-1.27.06-1.65.07-4.85.07s-3.58 0-4.85-.07c-1.17-.05-1.8-.25-2.23-.41a3.7 3.7 0 0 1-1.38-.9 3.7 3.7 0 0 1-.9-1.38c-.16-.42-.36-1.06-.41-2.23C2.2 15.58 2.2 15.2 2.2 12s0-3.58.07-4.85c.05-1.17.25-1.8.41-2.23.22-.56.48-.96.9-1.38.42-.42.82-.68 1.38-.9.42-.16 1.06-.36 2.23-.41C8.42 2.2 8.8 2.2 12 2.2zm0 1.8c-3.15 0-3.5 0-4.74.07-.9.04-1.38.19-1.7.32-.43.16-.74.36-1.06.68-.32.32-.52.63-.68 1.06-.13.32-.28.8-.32 1.7C3.8 8.5 3.8 8.85 3.8 12s0 3.5.07 4.74c.04.9.19 1.38.32 1.7.16.43.36.74.68 1.06.32.32.63.52 1.06.68.32.13.8.28 1.7.32 1.24.07 1.59.07 4.74.07s3.5 0 4.74-.07c.9-.04 1.38-.19 1.7-.32.43-.16.74-.36 1.06-.68.32-.32.52-.63.68-1.06.13-.32.28-.8.32-1.7.07-1.24.07-1.59.07-4.74s0-3.5-.07-4.74c-.04-.9-.19-1.38-.32-1.7a2.85 2.85 0 0 0-.68-1.06 2.85 2.85 0 0 0-1.06-.68c-.32-.13-.8-.28-1.7-.32C15.5 4 15.15 4 12 4zm0 3.06A4.94 4.94 0 1 0 12 16.94 4.94 4.94 0 0 0 12 7.06zm0 8.15A3.21 3.21 0 1 1 12 8.8a3.21 3.21 0 0 1 0 6.4zm6.3-8.35a1.15 1.15 0 1 1-2.3 0 1.15 1.15 0 0 1 2.3 0z",
      },
    ],
  },

  colors: { brand: "#9f1239", brandSoft: "#ffe4e6", paper: "#fbf7f5" },

  seo: {
    title: "Cedar & Vine — a seasonal bistro in Dallas. Reserve a table.",
    description:
      "Wood-fired seasonal plates and natural wine in Dallas. See live table availability and reserve in seconds — no phone tag.",
  },

  hero: {
    tagline: "Dallas · Bishop Arts",
    headline: "A table by the fire, whenever you're ready.",
    subcopy:
      "Seasonal small plates, wood-fired mains, and a short natural-wine list. Reserve a table in seconds — availability is live, so the time you see is a time you can actually get.",
    ctaLabel: "Reserve a table",
    quickFacts: {
      hours: "Wed–Sun, 5–10",
      area: "Bishop Arts, Dallas",
      phone: "(214) 555-0172",
    },
  },

  menu: {
    eyebrow: "Menu",
    headline: "Short, seasonal, wood-fired.",
    sections: [
      {
        name: "To start",
        items: [
          { name: "Wood-fired bread", price: "$9", desc: "Cultured butter, sea salt." },
          { name: "Little gem salad", price: "$14", desc: "Anchovy, lemon, parmesan.", tags: ["GF"] },
          { name: "Charred carrots", price: "$13", desc: "Whipped feta, dukkah, honey.", tags: ["V"] },
          { name: "Beef tartare", price: "$18", desc: "Smoked egg yolk, grilled sourdough." },
        ],
      },
      {
        name: "Mains",
        items: [
          { name: "Hearth chicken", price: "$28", desc: "Half bird, schmaltz potatoes, jus." },
          { name: "Wood-fired branzino", price: "$34", desc: "Fennel, citrus, salsa verde.", tags: ["GF"] },
          { name: "Rigatoni", price: "$24", desc: "Pork sugo, parmesan, chili.", },
          { name: "Roasted cauliflower", price: "$22", desc: "Romesco, golden raisin, almond.", tags: ["V", "GF"] },
        ],
      },
      {
        name: "Sweets",
        items: [
          { name: "Olive oil cake", price: "$11", desc: "Mascarpone, citrus." },
          { name: "Chocolate budino", price: "$12", desc: "Sea salt, crème fraîche." },
        ],
      },
    ],
  },

  reservations: {
    enabled: true,
    eyebrow: "Reserve",
    headline: "Find a table that's open.",
    subcopy:
      "Pick a date, a time, and your party size — open tables are live, so what you see is what's actually free. We hold it; you just show up.",
    slotMinutes: 30,
    leadTimeHours: 2,
    daysAhead: 21,
    tablesPerSlot: 6,
    maxPartySize: 8,
    hours: {
      0: { open: "17:00", close: "21:00" }, // Sun
      1: null, // Mon — closed
      2: null, // Tue — closed
      3: { open: "17:00", close: "21:30" },
      4: { open: "17:00", close: "21:30" },
      5: { open: "17:00", close: "22:00" },
      6: { open: "17:00", close: "22:00" },
    },
    confirmationMessage:
      "Your table is reserved. We'll hold it for 15 minutes past your time — see you soon.",
  },

  reviews: {
    eyebrow: "Reviews",
    headline: "Regulars keep coming back.",
    items: [
      {
        quote:
          "The branzino is perfect and the wine list is exactly the right size. Booking online means we actually get a table on Fridays now.",
        name: "Maya C.",
        rating: 5,
      },
      {
        quote:
          "Our neighborhood spot. Warm room, real cooking, and the reservation page tells you the truth about what's open.",
        name: "Daniel R.",
        rating: 5,
      },
      {
        quote:
          "Took my parents for their anniversary. The team held a quiet corner table — exactly what I asked for in the notes.",
        name: "Hannah K.",
        rating: 5,
      },
    ],
  },

  location: {
    eyebrow: "Visit",
    headline: "Find us in Bishop Arts.",
    address: "412 N Bishop Ave, Dallas, TX 75208",
    mapEmbedUrl: "",
    hoursText: "Wed–Thu 5–9:30 · Fri–Sat 5–10 · Sun 5–9 · Mon–Tue closed",
    phone: "(214) 555-0172",
    email: "hello@cedarandvine.example",
  },

  faq: {
    eyebrow: "Questions",
    headline: "Good to know.",
    items: [
      {
        q: "How big a party can I book online?",
        a: "Up to eight. For larger groups or private dining, give us a call — we'll take good care of you.",
      },
      {
        q: "What if I'm running late?",
        a: "We hold your table for 15 minutes past your time. Running later than that? Call and we'll do our best.",
      },
      {
        q: "Do you take walk-ins?",
        a: "Always, at the bar when there's room — but reserving guarantees a table and you can see exactly what's open.",
      },
    ],
  },
};
