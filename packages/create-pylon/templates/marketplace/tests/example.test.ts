import { expect, test } from "bun:test";
import {
  browseListings,
  listingSrcSet,
  pendingOfferCounts,
  percentOfAsk,
  suggestedOffers,
} from "../lib/catalog";
import { makeSlug, type Listing, type Offer } from "../client/market";

const listings: Listing[] = [
  {
    id: "1",
    sellerId: "seller-1",
    sellerName: "maple-fox",
    location: "Brooklyn, NY",
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

test("browse searches the seller's location", () => {
  expect(
    browseListings(listings, { query: "brooklyn" }).map((listing) => listing.id),
  ).toEqual(["1"]);
});

test("pending offer counts ignore answered offers", () => {
  const offer = (id: string, listingId: string, status: Offer["status"]): Offer => ({
    id,
    listingId,
    listingTitle: "",
    sellerId: "s",
    buyerId: "b",
    buyerName: "B",
    amount: 10,
    status,
    createdAt: "2026-01-01T00:00:00.000Z",
  });
  const counts = pendingOfferCounts([
    offer("a", "1", "pending"),
    offer("b", "1", "pending"),
    offer("c", "1", "declined"),
    offer("d", "2", "accepted"),
  ]);
  expect(counts.get("1")).toBe(2);
  expect(counts.has("2")).toBe(false);
});

test("suggested offers step below the asking price", () => {
  expect(suggestedOffers(680)).toEqual([650, 610, 580]);
  expect(suggestedOffers(95)).toEqual([90, 86, 81]);
  expect(suggestedOffers(1)).toEqual([]);
  expect(suggestedOffers(0)).toEqual([]);
  for (const price of [5, 49, 120, 240, 1150]) {
    for (const amount of suggestedOffers(price)) {
      expect(amount).toBeGreaterThan(0);
      expect(amount).toBeLessThan(price);
    }
  }
});

test("offer percentage of the asking price", () => {
  expect(percentOfAsk(620, 680)).toBe(91);
  expect(percentOfAsk(10, 0)).toBe(0);
});

test("bundled photos get a responsive srcset; others do not", () => {
  expect(listingSrcSet("/images/listings/watch.webp")).toBe(
    "/images/listings/watch-640.webp 640w, /images/listings/watch.webp 1120w",
  );
  expect(listingSrcSet("/api/files/abc")).toBeUndefined();
  expect(listingSrcSet("https://example.com/a.webp")).toBeUndefined();
  expect(listingSrcSet(undefined)).toBeUndefined();
});
