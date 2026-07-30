import type { Listing } from "../client/market";

export type CatalogSort = "latest" | "price-low" | "price-high";

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
    ].some((value) => value.toLocaleLowerCase().includes(query));
  });

  return [...filtered].sort((a, b) => {
    if (sort === "price-low") return a.price - b.price;
    if (sort === "price-high") return b.price - a.price;
    return b.createdAt.localeCompare(a.createdAt);
  });
}
