// THE single source of truth for everything business-specific. Rebrand the
// whole store by editing this ONE file — the landing page and layout read from
// here. The product list (incl. starting stock) seeds the Product table on
// first visit; after that, stock lives in the database and updates live.
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

export type ProductItem = {
  slug: string;
  name: string;
  priceCents: number;
  description?: string;
  image: string; // emoji or image URL (rendered big in the card)
  stock: number; // STARTING stock — seeded once, then live in the DB
};

export type ValueProp = { title: string; body: string; icon?: string };
export type Review = { quote: string; name: string; rating?: number };
export type Policy = { title: string; body: string };

export type ShopConfig = BaseConfig & {
  hero: { tagline: string; headline: string; subcopy: string; ctaLabel: string };
  products: { eyebrow: string; headline: string; items: ProductItem[] };
  checkout: { confirmationMessage: string };
  valueProps: { eyebrow: string; headline: string; items: ValueProp[] };
  reviews?: { eyebrow: string; headline: string; items: Review[] };
  policies: { eyebrow: string; headline: string; items: Policy[] };
};

/* ----------------------------- config ---------------------------- */

export const siteConfig: ShopConfig = {
  brand: {
    name: "Ember Goods",
    letter: "E",
    domain: "embergoods.com",
    email: "hello@embergoods.example",
    footerBlurb:
      "Hand-poured candles and small-batch home goods, made in Dallas. Slow, simple, and built to last. Free shipping over $50.",
    copyrightName: "Ember Goods",
    socials: [
      {
        label: "Instagram",
        href: "https://instagram.com",
        path: "M12 2.2c3.2 0 3.6 0 4.85.07 1.17.05 1.8.25 2.23.41.56.22.96.48 1.38.9.42.42.68.82.9 1.38.16.42.36 1.06.41 2.23.06 1.27.07 1.65.07 4.85s0 3.58-.07 4.85c-.05 1.17-.25 1.8-.41 2.23-.22.56-.48.96-.9 1.38-.42.42-.82.68-1.38.9-.42.16-1.06.36-2.23.41-1.27.06-1.65.07-4.85.07s-3.58 0-4.85-.07c-1.17-.05-1.8-.25-2.23-.41a3.7 3.7 0 0 1-1.38-.9 3.7 3.7 0 0 1-.9-1.38c-.16-.42-.36-1.06-.41-2.23C2.2 15.58 2.2 15.2 2.2 12s0-3.58.07-4.85c.05-1.17.25-1.8.41-2.23.22-.56.48-.96.9-1.38.42-.42.82-.68 1.38-.9.42-.16 1.06-.36 2.23-.41C8.42 2.2 8.8 2.2 12 2.2zm0 1.8c-3.15 0-3.5 0-4.74.07-.9.04-1.38.19-1.7.32-.43.16-.74.36-1.06.68-.32.32-.52.63-.68 1.06-.13.32-.28.8-.32 1.7C3.8 8.5 3.8 8.85 3.8 12s0 3.5.07 4.74c.04.9.19 1.38.32 1.7.16.43.36.74.68 1.06.32.32.63.52 1.06.68.32.13.8.28 1.7.32 1.24.07 1.59.07 4.74.07s3.5 0 4.74-.07c.9-.04 1.38-.19 1.7-.32.43-.16.74-.36 1.06-.68.32-.32.52-.63.68-1.06.13-.32.28-.8.32-1.7.07-1.24.07-1.59.07-4.74s0-3.5-.07-4.74c-.04-.9-.19-1.38-.32-1.7a2.85 2.85 0 0 0-.68-1.06 2.85 2.85 0 0 0-1.06-.68c-.32-.13-.8-.28-1.7-.32C15.5 4 15.15 4 12 4zm0 3.06A4.94 4.94 0 1 0 12 16.94 4.94 4.94 0 0 0 12 7.06zm0 8.15A3.21 3.21 0 1 1 12 8.8a3.21 3.21 0 0 1 0 6.4zm6.3-8.35a1.15 1.15 0 1 1-2.3 0 1.15 1.15 0 0 1 2.3 0z",
      },
    ],
  },

  colors: { brand: "#9a3412", brandSoft: "#ffedd5", paper: "#faf8f5" },

  seo: {
    title: "Ember Goods — hand-poured candles, made in Dallas.",
    description:
      "Small-batch candles and home goods, hand-poured in Dallas. Live stock — what you see is what's in the studio. Free shipping over $50.",
  },

  hero: {
    tagline: "Small batch · Made in Dallas",
    headline: "Hand-poured candles, made to last.",
    subcopy:
      "We pour everything by hand in small batches, so quantities are real and limited. The stock you see below is exactly what's on the studio shelf right now.",
    ctaLabel: "Shop the shelf",
  },

  products: {
    eyebrow: "The shelf",
    headline: "What's in the studio right now.",
    items: [
      {
        slug: "cedar-smoke",
        name: "Cedar & Smoke",
        priceCents: 3200,
        description: "Cedarwood, smoked vanilla, a little leather. 60-hour burn.",
        image: "🕯️",
        stock: 8,
      },
      {
        slug: "linen",
        name: "Fresh Linen",
        priceCents: 3200,
        description: "Clean cotton and white musk. The everyday one.",
        image: "🤍",
        stock: 3,
      },
      {
        slug: "brass-holder",
        name: "Brass matchstick holder",
        priceCents: 2400,
        description: "Solid brass, striker on the base. Ages beautifully.",
        image: "🟫",
        stock: 12,
      },
      {
        slug: "ceramic-vessel",
        name: "Ceramic vessel candle",
        priceCents: 4800,
        description: "Hand-thrown stoneware you'll keep long after the wax.",
        image: "🏺",
        stock: 5,
      },
      {
        slug: "wick-trimmer",
        name: "Wick trimmer",
        priceCents: 1800,
        description: "The one tool that makes a candle last. Matte black.",
        image: "✂️",
        stock: 0,
      },
      {
        slug: "gift-set",
        name: "The trio gift set",
        priceCents: 8400,
        description: "Three minis, boxed and ready to give.",
        image: "🎁",
        stock: 6,
      },
    ],
  },

  checkout: {
    confirmationMessage:
      "Order placed! We'll email you a payment link and ship within two business days.",
  },

  valueProps: {
    eyebrow: "Why Ember",
    headline: "Made slow, on purpose.",
    items: [
      {
        icon: "◍",
        title: "Hand-poured",
        body: "Every candle is poured, wicked, and labelled by hand in our Dallas studio.",
      },
      {
        icon: "◇",
        title: "Real small batch",
        body: "We make what we can do well. When the shelf says 3 left, there are 3 left.",
      },
      {
        icon: "✦",
        title: "Free shipping over $50",
        body: "Carbon-neutral shipping, plastic-free packaging, and easy returns.",
      },
    ],
  },

  reviews: {
    eyebrow: "Reviews",
    headline: "What people say.",
    items: [
      {
        quote: "Cedar & Smoke is the best candle I've ever bought, full stop. The brass holder is gorgeous too.",
        name: "Maya C.",
        rating: 5,
      },
      {
        quote: "Ordered the gift set for my mom. Beautiful packaging and it arrived in two days. Will reorder.",
        name: "Daniel R.",
        rating: 5,
      },
      {
        quote: "You can tell these are made by hand. The ceramic vessel is on my shelf forever now.",
        name: "Priya S.",
        rating: 5,
      },
    ],
  },

  policies: {
    eyebrow: "Good to know",
    headline: "Shipping & returns.",
    items: [
      { title: "Shipping", body: "Ships in 2 business days from Dallas. Free over $50, $6 flat otherwise." },
      { title: "Returns", body: "Unused and unburned? Return within 30 days for a full refund." },
      { title: "Wholesale", body: "Run a shop? Email us for the line sheet and wholesale pricing." },
    ],
  },
};
