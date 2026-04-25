/**
 * Catalog page — server-rendered product grid + facet sidebar.
 *
 * The `searchParams` URL state is the source of truth: `q`, facets,
 * `sort`, `page`. The server runs the search via Pylon's REST surface
 * and ships fully-formed product cards down — good for SEO + LCP.
 *
 * Client interactivity (typing in the search box, toggling a facet)
 * is delegated to small Client Components that update the URL via
 * `useRouter`, which re-renders this Server Component with fresh
 * results.
 */
import type { Metadata } from "next";
import { Suspense } from "react";
import Link from "next/link";
import { Star } from "lucide-react";
import { Card } from "@pylonsync/example-ui/card";
import { Badge } from "@pylonsync/example-ui/badge";
import { Button } from "@pylonsync/example-ui/button";
import { searchProducts } from "@/lib/pylon-server";
import { gradient, initials } from "@/lib/util";
import { SearchBox } from "@/components/search-box";
import { FacetGroups } from "@/components/facet-groups";
import { SortSelect } from "@/components/sort-select";
import { Pager } from "@/components/pager";
import { ActiveFilters } from "@/components/active-filters";
import { AddToCartButton } from "@/components/add-to-cart-button";
import type { Product } from "@/lib/types";

const FACET_FIELDS = ["brand", "category", "color"];

const SORTS: Record<string, [string, "asc" | "desc"] | undefined> = {
  relevance: undefined,
  "price-asc": ["price", "asc"],
  "price-desc": ["price", "desc"],
  "rating-desc": ["rating", "desc"],
  newest: ["createdAt", "desc"],
};

type SearchParams = {
  q?: string;
  page?: string;
  sort?: string;
  brand?: string;
  category?: string;
  color?: string;
};

function pickFilters(params: SearchParams): Record<string, string> {
  const out: Record<string, string> = {};
  for (const facet of FACET_FIELDS) {
    const v = params[facet as keyof SearchParams];
    if (typeof v === "string" && v.length > 0) out[facet] = v;
  }
  return out;
}

export async function generateMetadata({
  searchParams,
}: {
  searchParams: Promise<SearchParams>;
}): Promise<Metadata> {
  const sp = await searchParams;
  const filters = pickFilters(sp);
  const facetParts = Object.entries(filters).map(
    ([k, v]) => `${v} ${k}`,
  );
  const baseTitle = sp.q
    ? `${sp.q} · Search`
    : facetParts.length > 0
    ? `${facetParts.join(", ")} · Catalog`
    : "Catalog";
  return {
    title: baseTitle,
    description: sp.q
      ? `Search results for "${sp.q}" across 10,000 products at Pylon Store.`
      : "Browse 10,000 products with live facets, full-text search, and instant filtering.",
  };
}

export default async function CatalogPage({
  searchParams,
}: {
  searchParams: Promise<SearchParams>;
}) {
  const sp = await searchParams;
  const query = sp.q?.trim() ?? "";
  const page = Number.parseInt(sp.page ?? "0", 10) || 0;
  const sortKey = sp.sort ?? "relevance";
  const filters = pickFilters(sp);
  const pageSize = 24;

  const search = await searchProducts({
    query,
    filters,
    facets: FACET_FIELDS,
    sort: SORTS[sortKey],
    page,
    pageSize,
  });

  const totalPages = Math.max(1, Math.ceil(search.total / pageSize));

  return (
    <>
      <div className="border-b bg-background">
        <div className="mx-auto flex max-w-[1400px] items-center gap-4 px-4 py-3 md:px-6">
          <SearchBox initialValue={query} />
          <span className="font-mono text-xs text-muted-foreground">
            {search.total.toLocaleString()} results · {search.took_ms}ms
          </span>
        </div>
      </div>

      <div className="mx-auto grid max-w-[1400px] flex-1 gap-8 px-4 py-6 md:grid-cols-[240px_1fr] md:px-6">
        <aside className="flex flex-col gap-6">
          <FacetGroups
            facetCounts={search.facet_counts}
            active={filters}
          />
          {(Object.keys(filters).length > 0 || query) && (
            <Button variant="outline" size="sm" asChild>
              <Link href="/">Clear all</Link>
            </Button>
          )}
        </aside>

        <main className="flex flex-col gap-4">
          <div className="flex flex-wrap items-center justify-between gap-3">
            <ActiveFilters filters={filters} query={query} />
            <Suspense>
              <SortSelect value={sortKey} />
            </Suspense>
          </div>

          {search.hits.length === 0 ? (
            <div className="rounded-lg border border-dashed p-12 text-center text-sm text-muted-foreground">
              No products match that filter. Try clearing some criteria.
            </div>
          ) : (
            <>
              <div className="grid grid-cols-2 gap-4 sm:grid-cols-3 lg:grid-cols-4 xl:grid-cols-5">
                {search.hits.map((p) => (
                  <ProductCard key={p.id} product={p} />
                ))}
              </div>
              <Pager page={page} totalPages={totalPages} />
            </>
          )}
        </main>
      </div>
    </>
  );
}

function ProductCard({ product }: { product: Product }) {
  return (
    <Card className="group flex flex-col overflow-hidden p-0 transition hover:-translate-y-0.5 hover:shadow-md">
      <Link
        href={`/p/${encodeURIComponent(product.id)}`}
        className="flex aspect-square items-center justify-center text-2xl font-semibold text-white/90"
        style={{ background: gradient(product.name, product.brand) }}
      >
        {initials(product.name)}
      </Link>
      <div className="flex flex-1 flex-col gap-1 p-3">
        <Link
          href={`/p/${encodeURIComponent(product.id)}`}
          className="text-[10px] font-semibold uppercase tracking-wider text-muted-foreground"
        >
          {product.brand}
        </Link>
        <Link
          href={`/p/${encodeURIComponent(product.id)}`}
          className="line-clamp-2 min-h-[34px] text-sm font-medium leading-snug hover:underline"
        >
          {product.name}
        </Link>
        <div className="mt-1 flex items-center justify-between text-sm">
          <span className="font-semibold">${product.price.toFixed(2)}</span>
          <span className="flex items-center gap-1 text-xs text-muted-foreground">
            <Star className="size-3 fill-current" />
            {product.rating.toFixed(1)}
          </span>
        </div>
        <AddToCartButton product={product} className="mt-2" />
      </div>
    </Card>
  );
}
