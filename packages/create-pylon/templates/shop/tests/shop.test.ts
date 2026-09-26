import { describe, expect, test } from "bun:test";
import { existsSync } from "node:fs";
import { join } from "node:path";
import { formatPrice, freeShippingRemaining, shippingCents, stockStatus } from "../lib/shop";
import { productBySlug, siteConfig } from "../lib/site.config";

const terms = { flatCents: 600, freeOverCents: 5000 };

describe("stockStatus", () => {
  test("zero or less is sold out", () => {
    expect(stockStatus(0)).toEqual({ kind: "sold_out", label: "Sold out" });
    expect(stockStatus(-2).kind).toBe("sold_out");
  });

  test("three or fewer is low", () => {
    expect(stockStatus(1)).toEqual({ kind: "low", label: "Last one" });
    expect(stockStatus(3)).toEqual({ kind: "low", label: "Only 3 left" });
  });

  test("more than three shows the count", () => {
    expect(stockStatus(4)).toEqual({ kind: "in_stock", label: "4 in stock" });
  });
});

describe("shipping", () => {
  test("flat rate below the threshold", () => {
    expect(shippingCents(4999, terms)).toBe(600);
    expect(freeShippingRemaining(3800, terms)).toBe(1200);
  });

  test("free at and above the threshold", () => {
    expect(shippingCents(5000, terms)).toBe(0);
    expect(freeShippingRemaining(8400, terms)).toBe(0);
  });
});

test("formatPrice drops zero cents", () => {
  expect(formatPrice(3800)).toBe("$38");
  expect(formatPrice(1850)).toBe("$18.50");
});

describe("catalog config", () => {
  const items = siteConfig.products.items;

  test("slugs are unique", () => {
    expect(new Set(items.map((p) => p.slug)).size).toBe(items.length);
  });

  test("productBySlug finds a product and misses an unknown slug", () => {
    expect(productBySlug(items[0].slug)?.name).toBe(items[0].name);
    expect(productBySlug("nope")).toBeUndefined();
  });

  // Every local image the storefront references must ship in public/.
  test("every local image exists in public/", () => {
    const paths = [
      siteConfig.hero.image,
      siteConfig.checkout.image,
      ...items.map((p) => p.image),
    ].filter((src) => src.startsWith("/"));
    const missing = paths.filter((src) => !existsSync(join(import.meta.dir, "..", "public", src)));
    expect(missing).toEqual([]);
  });
});
