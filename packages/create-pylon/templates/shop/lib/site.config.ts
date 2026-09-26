// Business-specific copy and settings live here. The storefront pages and
// layout read this file. The product list, including starting stock, seeds the
// Product table on first visit; after that, stock lives in the database and
// updates live.
//
// Colors live here (applied as CSS variables on <html> in app/layout.tsx).
// Fictional demo copy. Replace the values and keep the shape.

/* ----------------------------- types ----------------------------- */

import type { ShippingTerms } from "./shop";

export type Social = { label: string; href: string; path: string };

export type BaseConfig = {
  brand: {
    name: string;
    letter: string; // the monogram on the sign-in and dashboard screens
    domain: string;
    email: string;
    announcement: string;
    footerBlurb: string;
    address: string[];
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
  size?: string; // one short line under the name, e.g. "8 oz, 60-hour burn"
  image: string; // a product photo URL or /public path (4:5 portrait works best)
  stock: number; // STARTING stock, seeded once, then live in the DB
};

export type Fact = { value: string; label: string };
export type Review = { quote: string; name: string; product?: string };
export type Policy = { title: string; body: string };

export type ShopConfig = BaseConfig & {
  hero: {
    headline: string;
    subcopy: string;
    ctaLabel: string;
    secondaryLabel: string;
    image: string;
    imageAlt: string;
  };
  products: { headline: string; subcopy: string; items: ProductItem[] };
  studio: { headline: string; body: string[]; facts: Fact[] };
  checkout: { shipping: ShippingTerms; reservedMessage: string; paidMessage: string; image: string };
  reviews?: { headline: string; items: Review[] };
  policies: { headline: string; items: Policy[] };
};

/* ----------------------------- config ---------------------------- */

export const siteConfig: ShopConfig = {
  brand: {
    name: "Ember Goods",
    letter: "E",
    domain: "embergoods.com",
    email: "hello@embergoods.example",
    announcement: "Free shipping on orders over $50. Ships from Dallas in two business days.",
    footerBlurb: "Candles poured by hand in small batches in Dallas, Texas.",
    address: ["2809 Elm Street", "Dallas, TX 75226", "Studio open Saturdays, 10 to 4"],
    copyrightName: "Ember Goods",
    socials: [
      {
        label: "Instagram",
        href: "https://instagram.com",
        path: "M12 2.2c3.2 0 3.6 0 4.85.07 1.17.05 1.8.25 2.23.41.56.22.96.48 1.38.9.42.42.68.82.9 1.38.16.42.36 1.06.41 2.23.06 1.27.07 1.65.07 4.85s0 3.58-.07 4.85c-.05 1.17-.25 1.8-.41 2.23-.22.56-.48.96-.9 1.38-.42.42-.82.68-1.38.9-.42.16-1.06.36-2.23.41-1.27.06-1.65.07-4.85.07s-3.58 0-4.85-.07c-1.17-.05-1.8-.25-2.23-.41a3.7 3.7 0 0 1-1.38-.9 3.7 3.7 0 0 1-.9-1.38c-.16-.42-.36-1.06-.41-2.23C2.2 15.58 2.2 15.2 2.2 12s0-3.58.07-4.85c.05-1.17.25-1.8.41-2.23.22-.56.48-.96.9-1.38.42-.42.82-.68 1.38-.9.42-.16 1.06-.36 2.23-.41C8.42 2.2 8.8 2.2 12 2.2zm0 1.8c-3.15 0-3.5 0-4.74.07-.9.04-1.38.19-1.7.32-.43.16-.74.36-1.06.68-.32.32-.52.63-.68 1.06-.13.32-.28.8-.32 1.7C3.8 8.5 3.8 8.85 3.8 12s0 3.5.07 4.74c.04.9.19 1.38.32 1.7.16.43.36.74.68 1.06.32.32.63.52 1.06.68.32.13.8.28 1.7.32 1.24.07 1.59.07 4.74.07s3.5 0 4.74-.07c.9-.04 1.38-.19 1.7-.32.43-.16.74-.36 1.06-.68.32-.32.52-.63.68-1.06.13-.32.28-.8.32-1.7.07-1.24.07-1.59.07-4.74s0-3.5-.07-4.74c-.04-.9-.19-1.38-.32-1.7a2.85 2.85 0 0 0-.68-1.06 2.85 2.85 0 0 0-1.06-.68c-.32-.13-.8-.28-1.7-.32C15.5 4 15.15 4 12 4zm0 3.06A4.94 4.94 0 1 0 12 16.94 4.94 4.94 0 0 0 12 7.06zm0 8.15A3.21 3.21 0 1 1 12 8.8a3.21 3.21 0 0 1 0 6.4zm6.3-8.35a1.15 1.15 0 1 1-2.3 0 1.15 1.15 0 0 1 2.3 0z",      },
    ],
  },

  colors: { brand: "#8b4a2b", brandSoft: "#efe3d6", paper: "#e9e1d5" },

  seo: {
    title: "Ember Goods: candles poured by hand in Dallas",
    description:
      "Small-batch candles poured in Dallas. Soy and coconut wax, cotton wicks, 60-hour burn. Stock counts match the studio shelf. Free shipping over $50.",
  },

  hero: {
    headline: "Candles poured by hand in Dallas.",
    subcopy:
      "Soy and coconut wax, cotton wicks, and scents we blend ourselves. We pour forty at a time, so the stock counts are the real shelf.",
    ctaLabel: "Shop candles",
    secondaryLabel: "See the studio",
    image: "/images/hero.jpg",
    imageAlt: "Three amber glass candles on a travertine shelf, the middle one lit, in late afternoon light.",
  },

  products: {
    headline: "The shelf",
    subcopy: "Stock updates live. When a batch sells out, it is gone until the next pour.",
    items: [
      {
        slug: "tobacco-oak",
        name: "Tobacco & Oak",
        priceCents: 4200,
        description: "Cured tobacco leaf, oak barrel, and amber. Our darkest scent, poured once a season.",
        size: "8 oz, 60-hour burn",
        image: "/images/products/tobacco-oak.jpg",
        stock: 2,
      },
      {
        slug: "cedar-smoke",
        name: "Cedar & Smoke",
        priceCents: 3800,
        description: "Cedarwood, smoked vanilla, and a little leather.",
        size: "8 oz, 60-hour burn",
        image: "/images/products/cedar-smoke.jpg",
        stock: 8,
      },
      {
        slug: "linen",
        name: "Linen",
        priceCents: 3800,
        description: "Clean cotton, white musk, and iris.",
        size: "8 oz, 60-hour burn",
        image: "/images/products/linen.jpg",
        stock: 3,
      },
      {
        slug: "fig-leaf",
        name: "Fig Leaf",
        priceCents: 3800,
        description: "Green fig, cedar, and coconut.",
        size: "8 oz, 60-hour burn",
        image: "/images/products/fig-leaf.jpg",
        stock: 11,
      },
      {
        slug: "sea-salt",
        name: "Sea Salt & Sage",
        priceCents: 3800,
        description: "Salt air, sage, and driftwood.",
        size: "8 oz, 60-hour burn",
        image: "/images/products/sea-salt.jpg",
        stock: 9,
      },
    ],
  },

  studio: {
    headline: "Poured in small batches, cured for two weeks.",
    body: [
      "Every candle is poured, cured, and trimmed by hand in our studio in Deep Ellum. We use a soy and coconut wax blend, cotton wicks, and phthalate-free fragrance oils.",
      "Each batch cures for fourteen days before it goes on the shelf, so the scent carries across the room from the first burn.",
    ],
    facts: [
      { value: "40", label: "candles per batch" },
      { value: "14 days", label: "cure before sale" },
      { value: "60 hr", label: "burn time, 8 oz" },
    ],
  },

  checkout: {
    shipping: { flatCents: 600, freeOverCents: 5000 },
    reservedMessage:
      "We are holding your items and will email a payment link within one business day. Orders ship from Dallas within two business days of payment.",
    paidMessage:
      "Your payment went through and the receipt is on its way to your inbox. Orders ship from Dallas within two business days.",
    image: "/images/products/tobacco-oak.jpg", // shown on /success
  },

  reviews: {
    headline: "Reviews",
    items: [
      {
        quote: "Cedar & Smoke fills the whole living room without being loud about it. On my third one.",
        name: "Maya C.",
        product: "Cedar & Smoke",
      },
      {
        quote: "Bought Linen for my mom. It arrived in two days, packed in paper, no plastic anywhere.",
        name: "Daniel R.",
        product: "Linen",
      },
      {
        quote: "Fig Leaf burned evenly all the way to the bottom of the jar. I kept the jar for pens.",
        name: "Hannah K.",
        product: "Fig Leaf",
      },
    ],
  },

  policies: {
    headline: "Shipping and returns",
    items: [
      { title: "Shipping", body: "Ships from Dallas in two business days. Free over $50, $6 flat otherwise." },
      { title: "Returns", body: "Return unused and unburned items within 30 days for a full refund." },
      { title: "Local pickup", body: "Pick up at the studio on Saturdays. Reply to your order email to switch to pickup." },
      { title: "Wholesale", body: "Email us for the line sheet and wholesale pricing." },
    ],
  },
};

export function productBySlug(slug: string): ProductItem | undefined {
  return siteConfig.products.items.find((p) => p.slug === slug);
}
