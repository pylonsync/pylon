// THE single source of truth for everything business-specific. Rebrand the
// whole site — and reconfigure the booking engine — by editing this ONE file.
// The landing page, layout, AND the createBooking server function all read from
// here, so services, prices, weekly hours, and lead time stay in lockstep. The
// create-pylon scaffolder and Mast target this file: a whole appointment site
// is themed + configured by producing one typed object.
//
// Colors live here (applied as CSS variables on <html> in app/layout.tsx).
//
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

export type ServiceItem = {
  slug: string;
  name: string;
  durationMin: number; // drives the booking slot length
  price: string; // display only, e.g. "$35"
  description?: string;
};

export type DayHours = { open: string; close: string } | null; // "09:00".."18:00"

export type Review = { quote: string; name: string; rating?: number };
export type Faq = { q: string; a: string };

export type LocalServiceConfig = BaseConfig & {
  hero: {
    tagline: string;
    headline: string;
    subcopy: string;
    ctaLabel: string;
    quickFacts: { hours: string; area: string; phone: string };
  };
  services: {
    eyebrow: string;
    headline: string;
    items: ServiceItem[];
  };
  booking: {
    enabled: boolean;
    eyebrow: string;
    headline: string;
    subcopy: string;
    slotMinutes: number; // granularity of offered start times, e.g. 30
    leadTimeHours: number; // earliest bookable lead time from now
    daysAhead: number; // how many days the picker offers
    // Weekly hours keyed by day-of-week (0=Sun … 6=Sat). null = closed.
    hours: Record<number, DayHours>;
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

export const siteConfig: LocalServiceConfig = {
  brand: {
    name: "Northgate Barbers",
    letter: "N",
    domain: "northgatebarbers.com",
    email: "hello@northgatebarbers.example",
    footerBlurb:
      "A neighborhood barbershop in Dallas. Classic cuts, hot-towel shaves, and a chair that's always ready. Book in ten seconds.",
    copyrightName: "Northgate Barbers",
    socials: [
      {
        label: "Instagram",
        href: "https://instagram.com",
        path: "M12 2.2c3.2 0 3.6 0 4.85.07 1.17.05 1.8.25 2.23.41.56.22.96.48 1.38.9.42.42.68.82.9 1.38.16.42.36 1.06.41 2.23.06 1.27.07 1.65.07 4.85s0 3.58-.07 4.85c-.05 1.17-.25 1.8-.41 2.23-.22.56-.48.96-.9 1.38-.42.42-.82.68-1.38.9-.42.16-1.06.36-2.23.41-1.27.06-1.65.07-4.85.07s-3.58 0-4.85-.07c-1.17-.05-1.8-.25-2.23-.41a3.7 3.7 0 0 1-1.38-.9 3.7 3.7 0 0 1-.9-1.38c-.16-.42-.36-1.06-.41-2.23C2.2 15.58 2.2 15.2 2.2 12s0-3.58.07-4.85c.05-1.17.25-1.8.41-2.23.22-.56.48-.96.9-1.38.42-.42.82-.68 1.38-.9.42-.16 1.06-.36 2.23-.41C8.42 2.2 8.8 2.2 12 2.2zm0 1.8c-3.15 0-3.5 0-4.74.07-.9.04-1.38.19-1.7.32-.43.16-.74.36-1.06.68-.32.32-.52.63-.68 1.06-.13.32-.28.8-.32 1.7C3.8 8.5 3.8 8.85 3.8 12s0 3.5.07 4.74c.04.9.19 1.38.32 1.7.16.43.36.74.68 1.06.32.32.63.52 1.06.68.32.13.8.28 1.7.32 1.24.07 1.59.07 4.74.07s3.5 0 4.74-.07c.9-.04 1.38-.19 1.7-.32.43-.16.74-.36 1.06-.68.32-.32.52-.63.68-1.06.13-.32.28-.8.32-1.7.07-1.24.07-1.59.07-4.74s0-3.5-.07-4.74c-.04-.9-.19-1.38-.32-1.7a2.85 2.85 0 0 0-.68-1.06 2.85 2.85 0 0 0-1.06-.68c-.32-.13-.8-.28-1.7-.32C15.5 4 15.15 4 12 4zm0 3.06A4.94 4.94 0 1 0 12 16.94 4.94 4.94 0 0 0 12 7.06zm0 8.15A3.21 3.21 0 1 1 12 8.8a3.21 3.21 0 0 1 0 6.4zm6.3-8.35a1.15 1.15 0 1 1-2.3 0 1.15 1.15 0 0 1 2.3 0z",
      },
    ],
  },

  colors: { brand: "#b45309", brandSoft: "#fef3c7", paper: "#fafaf9" },

  seo: {
    title: "Northgate Barbers — classic cuts in Dallas. Book online.",
    description:
      "A neighborhood barbershop in Dallas. Haircuts, beard trims, and hot-towel shaves. See live availability and book your chair in seconds.",
  },

  hero: {
    tagline: "Dallas · est. 2014",
    headline: "A proper haircut, booked in ten seconds.",
    subcopy:
      "Classic cuts, beard work, and hot-towel shaves from barbers who've been at it a while. Pick a time that's actually open — availability updates live — and you're set.",
    ctaLabel: "Book a chair",
    quickFacts: {
      hours: "Tue–Sat, 9–6",
      area: "Lower Greenville, Dallas",
      phone: "(214) 555-0148",
    },
  },

  services: {
    eyebrow: "Services",
    headline: "Simple menu, honest prices.",
    items: [
      {
        slug: "haircut",
        name: "Haircut",
        durationMin: 45,
        price: "$35",
        description: "Consultation, cut, and a clean finish. The classic.",
      },
      {
        slug: "beard-trim",
        name: "Beard trim",
        durationMin: 20,
        price: "$18",
        description: "Shape-up, line work, and hot-towel finish.",
      },
      {
        slug: "cut-and-beard",
        name: "Cut + beard",
        durationMin: 60,
        price: "$48",
        description: "The full sit-down. Haircut and beard, start to finish.",
      },
      {
        slug: "kids-cut",
        name: "Kids' cut",
        durationMin: 30,
        price: "$22",
        description: "For the under-12s. Patient barbers, no rush.",
      },
    ],
  },

  booking: {
    enabled: true,
    eyebrow: "Book",
    headline: "Find a time that's open.",
    subcopy:
      "Pick a service and a day — open slots are live, so what you see is what's actually free. No account, no phone tag.",
    slotMinutes: 30,
    leadTimeHours: 2,
    daysAhead: 14,
    hours: {
      0: null, // Sun — closed
      1: null, // Mon — closed
      2: { open: "09:00", close: "18:00" },
      3: { open: "09:00", close: "18:00" },
      4: { open: "09:00", close: "19:00" },
      5: { open: "09:00", close: "19:00" },
      6: { open: "09:00", close: "16:00" }, // Sat — shorter
    },
    confirmationMessage:
      "You're booked. We'll see you then — a reminder will go out the day before.",
  },

  reviews: {
    eyebrow: "Reviews",
    headline: "Regulars say it best.",
    items: [
      {
        quote:
          "Best fade in Dallas, and I can finally book online instead of waiting around. Booked, in, out, sharp.",
        name: "Marcus B.",
        rating: 5,
      },
      {
        quote:
          "Been coming for three years. Same great cut every time, and the online booking is dead simple.",
        name: "Daniel R.",
        rating: 5,
      },
      {
        quote:
          "Took my son for his first real haircut. Patient, friendly, and the chair was ready right on time.",
        name: "Hannah K.",
        rating: 5,
      },
    ],
  },

  location: {
    eyebrow: "Visit",
    headline: "Find us on Greenville.",
    address: "1845 Greenville Ave, Dallas, TX 75206",
    mapEmbedUrl: "",
    hoursText: "Tue–Wed 9–6 · Thu–Fri 9–7 · Sat 9–4 · Sun–Mon closed",
    phone: "(214) 555-0148",
    email: "hello@northgatebarbers.example",
  },

  faq: {
    eyebrow: "Questions",
    headline: "Good to know.",
    items: [
      {
        q: "Do you take walk-ins?",
        a: "When a chair's open, sure — but booking online guarantees your time, and you can see exactly what's free.",
      },
      {
        q: "What if I need to cancel?",
        a: "Just give us a call. No charge for cancellations with a few hours' notice.",
      },
      {
        q: "How should I pay?",
        a: "Cash or card in the shop. Booking online doesn't charge you anything — you pay after the cut.",
      },
    ],
  },
};
