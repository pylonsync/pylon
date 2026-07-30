import { expect, test } from "bun:test";
import { browseListings } from "../lib/catalog";
import { makeSlug, type Listing } from "../client/market";

const listings: Listing[] = [
  {
    id: "1",
    sellerId: "seller-1",
    sellerName: "maple-fox",
    title: "Walnut lounge chair",
    slug: "walnut-lounge-chair-1",
    description: "Restored frame with new upholstery.",
    price: 420,
    category: "furniture",
    condition: "good",
    status: "active",
    seed: "one",
    createdAt: "2026-01-01T00:00:00.000Z",
  },
  {
    id: "2",
    sellerId: "seller-2",
    sellerName: "slate-heron",
    title: "Compact mirrorless camera",
    slug: "compact-mirrorless-camera-2",
    description: "Clean sensor and two batteries.",
    price: 680,
    category: "cameras",
    condition: "like-new",
    status: "active",
    seed: "two",
    createdAt: "2026-02-01T00:00:00.000Z",
  },
  {
    id: "3",
    sellerId: "seller-3",
    sellerName: "amber-lynx",
    title: "Teak side table",
    slug: "teak-side-table-3",
    description: "Compact bedside table.",
    price: 140,
    category: "furniture",
    condition: "good",
    status: "active",
    seed: "three",
    createdAt: "2026-03-01T00:00:00.000Z",
  },
];

test("browse filters by category and searches listing copy", () => {
  expect(
    browseListings(listings, { category: "furniture", query: "walnut" }).map(
      (listing) => listing.id,
    ),
  ).toEqual(["1"]);
  expect(
    browseListings(listings, { query: "slate-heron" }).map(
      (listing) => listing.id,
    ),
  ).toEqual(["2"]);
});

test("browse sorts newest and by price", () => {
  expect(browseListings(listings, {}).map((listing) => listing.id)).toEqual([
    "3",
    "2",
    "1",
  ]);
  expect(
    browseListings(listings, { sort: "price-low" }).map(
      (listing) => listing.price,
    ),
  ).toEqual([140, 420, 680]);
});

test("listing slugs remain readable and unique", () => {
  expect(makeSlug("  Vintage Technics SL-1200 MK2  ", "a1f3")).toBe(
    "vintage-technics-sl-1200-mk2-a1f3",
  );
});
