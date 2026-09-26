import type { Listing, Offer } from "../client/market";

export type CatalogSort = "latest" | "price-low" | "price-high";

export interface Category {
  id: string;
  label: string;
}

// Browse + sell categories. The id is what a Listing stores; the label is
// what the UI shows.
export const CATEGORIES: Category[] = [
  { id: "furniture", label: "Furniture" },
  { id: "lighting", label: "Lighting" },
  { id: "audio", label: "Audio" },
  { id: "cameras", label: "Cameras" },
  { id: "watches", label: "Watches" },
  { id: "clothing", label: "Clothing" },
  { id: "shoes", label: "Shoes" },
  { id: "bags", label: "Bags" },
  { id: "bikes", label: "Bikes" },
  { id: "instruments", label: "Instruments" },
  { id: "kitchen", label: "Kitchen" },
  { id: "other", label: "Other" },
];

export const CONDITIONS = ["new", "like-new", "good", "fair"] as const;

export function categoryLabel(id: string): string {
  return CATEGORIES.find((c) => c.id === id)?.label ?? id;
}

export function isCategory(id: string): boolean {
  return CATEGORIES.some((c) => c.id === id);
}

export function browseListings(
  listings: Listing[],
  options: { category?: string; query?: string; sort?: string },
): Listing[] {
  const category = options.category || "all";
  const query = options.query?.trim().toLocaleLowerCase() ?? "";
  const sort: CatalogSort =
    options.sort === "price-low" || options.sort === "price-high"
      ? options.sort
      : "latest";

  const filtered = listings.filter((listing) => {
    if (category !== "all" && listing.category !== category) return false;
    if (!query) return true;
    return [
      listing.title,
      listing.description,
      listing.category,
      listing.sellerName,
      listing.location ?? "",
    ].some((value) => value.toLocaleLowerCase().includes(query));
  });

  return [...filtered].sort((a, b) => {
    if (sort === "price-low") return a.price - b.price;
    if (sort === "price-high") return b.price - a.price;
    return b.createdAt.localeCompare(a.createdAt);
  });
}

/** Pending offers per listing id. */
export function pendingOfferCounts(offers: Offer[]): Map<string, number> {
  const counts = new Map<string, number>();
  for (const offer of offers) {
    if (offer.status !== "pending") continue;
    counts.set(offer.listingId, (counts.get(offer.listingId) ?? 0) + 1);
  }
  return counts;
}

/** An offer as a whole-number percentage of the asking price. */
export function percentOfAsk(amount: number, price: number): number {
  if (!(price > 0)) return 0;
  return Math.round((amount / price) * 100);
}

/**
 * Suggested offer amounts: 95%, 90%, and 85% of the asking price, rounded to
 * a clean step, deduplicated, and always below the asking price.
 */
export function suggestedOffers(price: number): number[] {
  if (!(price > 0)) return [];
  const step = price >= 500 ? 10 : price >= 100 ? 5 : 1;
  const amounts = [0.95, 0.9, 0.85].map((ratio) =>
    Math.max(step, Math.round((price * ratio) / step) * step),
  );
  return Array.from(new Set(amounts)).filter((amount) => amount < price);
}

const BUNDLED_PHOTO = /^(\/images\/listings\/[a-z0-9-]+)\.webp$/;

/** Responsive `srcset` for a bundled listing photo; undefined for others. */
export function listingSrcSet(imageUrl: string | undefined): string | undefined {
  const match = imageUrl?.match(BUNDLED_PHOTO);
  if (!match) return undefined;
  return `${match[1]}-640.webp 640w, ${imageUrl} 1120w`;
}
