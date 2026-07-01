"use client";
/**
 * Catalog — faceted full-text search across the Product table.
 *
 * All filter state lives in the URL (?q=…&category=…&brand=…&color=…&
 * featured=true&sort=…). That makes any filtered view shareable, gives the
 * back button real history, and means the page pre-filters on load straight
 * from the query string. `db.useSearch` (server-side FTS + live facet counts)
 * is driven entirely off those params; product cards link to the real SSR
 * detail route at /p/<slug>.
 */
import { useEffect, useState } from "react";
import { db, Link, useRouter, useSearchParams } from "@pylonsync/react";
import { Card } from "@pylonsync/example-ui/card";
import { Input } from "@pylonsync/example-ui/input";
import { Button } from "@pylonsync/example-ui/button";
import { Badge } from "@pylonsync/example-ui/badge";
import { Star, X } from "lucide-react";
import type { Product } from "./lib/types";
import { gradient, initials, productPath } from "./lib/util";
import { useCart } from "./lib/cart";

type SortOption = {
  key: string;
  label: string;
  value: [string, "asc" | "desc"] | undefined;
};

const SORTS: SortOption[] = [
  { key: "relevance", label: "Relevance", value: undefined },
  { key: "best-selling", label: "Best selling", value: ["salesCount", "desc"] },
  { key: "price-asc", label: "Price: low to high", value: ["price", "asc"] },
  { key: "price-desc", label: "Price: high to low", value: ["price", "desc"] },
  { key: "rating", label: "Highest rated", value: ["rating", "desc"] },
  { key: "newest", label: "Newest", value: ["createdAt", "desc"] },
];

const FACET_LABELS: Record<string, string> = {
  brand: "Brand",
  category: "Category",
  color: "Color",
  priceBucket: "Price",
  ratingTier: "Rating",
};

// Facet keys that map 1:1 to a URL query param + a db.useSearch filter.
// Price + Rating sit high so they're visible without scrolling past the long
// category/brand/color lists.
const FACET_ORDER = ["category", "priceBucket", "ratingTier", "brand", "color"];

// Bucket facets render in a fixed logical order (low→high), not by count.
const BUCKET_ORDER: Record<string, string[]> = {
  priceBucket: ["Under $25", "$25 – $50", "$50 – $100", "$100 – $200", "$200 & up"],
  ratingTier: ["4.5★ & up", "4 – 4.5★", "3.5 – 4★", "Under 3.5★"],
};

const PAGE_SIZE = 24;

export function Catalog() {
  const router = useRouter();
  const params = useSearchParams();
  const cart = useCart();

  // URL is the source of truth for every filter.
  const urlQuery = params.get("q") ?? "";
  const sortKey = params.get("sort") ?? "relevance";
  const featured = params.get("featured") === "true";
  const page = Math.max(0, Number(params.get("page") ?? "0") || 0);
  const filters: Record<string, string> = {};
  for (const facet of FACET_ORDER) {
    const v = params.get(facet);
    if (v) filters[facet] = v;
  }
  if (featured) filters.featured = "true";

  const sortIdx = Math.max(
    0,
    SORTS.findIndex((s) => s.key === sortKey),
  );

  // Local text state so typing is instant; debounced into the URL (?q=).
  const [text, setText] = useState(urlQuery);
  useEffect(() => {
    setText(urlQuery);
  }, [urlQuery]);
  useEffect(() => {
    const t = setTimeout(() => {
      if (text.trim() !== urlQuery)
        setParam({ q: text.trim() || null, page: null }, "replace");
    }, 200);
    return () => clearTimeout(t);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [text]);

  // Apply param updates (null = remove) and navigate. `replace` keeps typing
  // out of the history stack; facet/sort clicks use push so Back works.
  function setParam(
    updates: Record<string, string | null>,
    mode: "push" | "replace" = "push",
  ) {
    const next = new URLSearchParams(params.toString());
    for (const [k, v] of Object.entries(updates)) {
      if (v === null || v === "") next.delete(k);
      else next.set(k, v);
    }
    const qs = next.toString();
    const href = qs ? `/?${qs}` : "/";
    if (mode === "replace") router.replace(href);
    else router.push(href);
  }

  const search = db.useSearch<Product>("Product", {
    query: urlQuery,
    filters,
    facets: ["brand", "category", "color", "priceBucket", "ratingTier"],
    sort: SORTS[sortIdx].value,
    page,
    pageSize: PAGE_SIZE,
  });

  const toggleFilter = (facet: string, value: string) => {
    setParam({ [facet]: filters[facet] === value ? null : value, page: null });
  };

  const clearAll = () => {
    setText("");
    router.push("/");
  };

  const totalPages = Math.max(1, Math.ceil(search.total / PAGE_SIZE));
  const activeFilterEntries = Object.entries(filters);
  const hasActive = activeFilterEntries.length > 0 || urlQuery.length > 0;

  return (
    <>
      <div className="border-b bg-background">
        <div className="mx-auto flex max-w-[1400px] items-center gap-4 px-4 py-3 md:px-6">
          <Input
            value={text}
            onChange={(e) => setText(e.target.value)}
            placeholder="Search 10,000 products…"
            className="h-10 max-w-xl"
          />
          {search.tookMs > 0 && (
            <span className="font-mono text-xs text-muted-foreground">
              {search.total.toLocaleString()} results · {search.tookMs}ms
            </span>
          )}
        </div>
      </div>

      <div className="mx-auto grid max-w-[1400px] gap-8 px-4 py-6 md:grid-cols-[240px_1fr] md:px-6">
        <aside className="flex flex-col gap-6">
          <button
            type="button"
            onClick={() =>
              setParam({ featured: featured ? null : "true", page: null })
            }
            className={
              "flex items-center justify-between rounded-md border px-3 py-2 text-sm font-medium transition-colors " +
              (featured
                ? "border-primary bg-primary text-primary-foreground"
                : "hover:bg-accent hover:text-accent-foreground")
            }
          >
            <span>★ Featured only</span>
            {featured ? <X className="size-3.5" /> : null}
          </button>

          <FacetGroups
            facetCounts={search.facetCounts}
            active={filters}
            onToggle={toggleFilter}
          />
          {hasActive && (
            <Button variant="outline" size="sm" onClick={clearAll}>
              Clear all
            </Button>
          )}
        </aside>

        <main className="flex flex-col gap-4">
          <div className="flex flex-wrap items-center justify-between gap-3">
            <div className="flex flex-wrap gap-2">
              {activeFilterEntries.map(([facet, value]) => (
                <Badge
                  key={`${facet}:${value}`}
                  variant="secondary"
                  className="cursor-pointer gap-1 capitalize"
                  onClick={() =>
                    facet === "featured"
                      ? setParam({ featured: null, page: null })
                      : toggleFilter(facet, value)
                  }
                >
                  {facet === "featured"
                    ? "Featured"
                    : `${FACET_LABELS[facet]}: ${value}`}
                  <X className="size-3" />
                </Badge>
              ))}
            </div>
            <label className="flex items-center gap-2 text-xs text-muted-foreground">
              Sort
              <select
                value={sortKey}
                onChange={(e) =>
                  setParam({
                    sort: e.target.value === "relevance" ? null : e.target.value,
                    page: null,
                  })
                }
                className="h-8 rounded-md border bg-background px-2 text-xs text-foreground"
              >
                {SORTS.map((s) => (
                  <option key={s.key} value={s.key}>
                    {s.label}
                  </option>
                ))}
              </select>
            </label>
          </div>

          {search.loading && search.hits.length === 0 ? (
            <SkeletonGrid count={12} />
          ) : search.error ? (
            <EmptyState>Search failed: {search.error.message}</EmptyState>
          ) : search.hits.length === 0 ? (
            <EmptyState>
              No products match that filter. Try clearing some criteria.
            </EmptyState>
          ) : (
            <>
              <div className="grid grid-cols-2 gap-4 sm:grid-cols-3 lg:grid-cols-4 xl:grid-cols-5">
                {search.hits.map((p) => (
                  <ProductCard key={p.id} product={p} onAddToCart={cart.add} />
                ))}
              </div>

              <Pager
                page={page}
                totalPages={totalPages}
                onPrev={() => setParam({ page: String(Math.max(0, page - 1)) })}
                onNext={() =>
                  setParam({ page: String(Math.min(totalPages - 1, page + 1)) })
                }
              />
            </>
          )}
        </main>
      </div>
    </>
  );
}

// ---------------------------------------------------------------------------
// Facets
// ---------------------------------------------------------------------------

function FacetGroups({
  facetCounts,
  active,
  onToggle,
}: {
  facetCounts: Record<string, Record<string, number>>;
  active: Record<string, string>;
  onToggle: (facet: string, value: string) => void;
}) {
  return (
    <div className="flex flex-col gap-5">
      {FACET_ORDER.map((facet) => {
        const counts = facetCounts[facet];
        if (!counts) return null;
        const fixedOrder = BUCKET_ORDER[facet];
        const entries: [string, number][] = fixedOrder
          ? fixedOrder
              .filter((v) => counts[v] != null)
              .map((v) => [v, counts[v]])
          : Object.entries(counts).sort((a, b) => b[1] - a[1]);
        return (
          <div key={facet} className="flex flex-col gap-1.5">
            <h4 className="text-[11px] font-semibold uppercase tracking-wider text-muted-foreground">
              {FACET_LABELS[facet]}
            </h4>
            <ul className="flex flex-col gap-0.5">
              {entries.slice(0, 10).map(([value, count]) => {
                const on = active[facet] === value;
                return (
                  <li key={value}>
                    <button
                      type="button"
                      onClick={() => onToggle(facet, value)}
                      className={
                        "flex w-full items-center justify-between rounded-md px-2 py-1.5 text-sm capitalize transition-colors " +
                        (on
                          ? "bg-primary text-primary-foreground"
                          : "text-foreground/80 hover:bg-accent hover:text-accent-foreground")
                      }
                    >
                      <span>{value}</span>
                      <span
                        className={
                          "font-mono text-[11px] " +
                          (on ? "opacity-80" : "text-muted-foreground")
                        }
                      >
                        {count}
                      </span>
                    </button>
                  </li>
                );
              })}
            </ul>
          </div>
        );
      })}
    </div>
  );
}

// ---------------------------------------------------------------------------
// Product card
// ---------------------------------------------------------------------------

function ProductCard({
  product,
  onAddToCart,
}: {
  product: Product;
  onAddToCart: (p: Product) => void;
}) {
  return (
    <Card className="group overflow-hidden p-0 transition hover:-translate-y-0.5 hover:shadow-md">
      <Link href={productPath(product.slug)} seed={product} className="block">
        <div
          className="relative flex aspect-square items-center justify-center text-2xl font-semibold text-white/90"
          style={{ background: gradient(product.name, product.brand) }}
        >
          {initials(product.name)}
          {product.tags ? (
            <div className="absolute left-2 top-2 flex flex-wrap gap-1">
              {product.tags
                .split(",")
                .filter(Boolean)
                .map((t) => (
                  <Badge
                    key={t}
                    variant="secondary"
                    className="bg-white/90 text-[10px] text-foreground shadow-sm"
                  >
                    {t}
                  </Badge>
                ))}
            </div>
          ) : null}
        </div>
        <div className="flex flex-col gap-1 p-3 pb-0">
          <div className="text-[10px] font-semibold uppercase tracking-wider text-muted-foreground">
            {product.brand}
          </div>
          <div className="line-clamp-2 min-h-[34px] text-sm font-medium leading-snug">
            {product.name}
          </div>
          <div className="mt-1 flex items-center justify-between text-sm">
            <span className="font-semibold">${product.price.toFixed(2)}</span>
            <span className="flex items-center gap-1 text-xs text-muted-foreground">
              <Star className="size-3 fill-current" />
              {product.rating.toFixed(1)}
            </span>
          </div>
        </div>
      </Link>
      <div className="p-3 pt-2">
        <Button
          variant="outline"
          size="sm"
          className="w-full"
          onClick={() => onAddToCart(product)}
        >
          Add to cart
        </Button>
      </div>
    </Card>
  );
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

function Pager({
  page,
  totalPages,
  onPrev,
  onNext,
}: {
  page: number;
  totalPages: number;
  onPrev: () => void;
  onNext: () => void;
}) {
  return (
    <div className="mt-4 flex items-center justify-center gap-4 text-sm text-muted-foreground">
      <Button variant="outline" size="sm" disabled={page === 0} onClick={onPrev}>
        ← Previous
      </Button>
      <span>
        Page {page + 1} of {totalPages}
      </span>
      <Button
        variant="outline"
        size="sm"
        disabled={page + 1 >= totalPages}
        onClick={onNext}
      >
        Next →
      </Button>
    </div>
  );
}

function EmptyState({ children }: { children: React.ReactNode }) {
  return (
    <div className="rounded-lg border border-dashed p-12 text-center text-sm text-muted-foreground">
      {children}
    </div>
  );
}

function SkeletonGrid({ count }: { count: number }) {
  return (
    <div className="grid grid-cols-2 gap-4 sm:grid-cols-3 lg:grid-cols-4 xl:grid-cols-5">
      {Array.from({ length: count }).map((_, i) => (
        <Card key={i} className="overflow-hidden p-0">
          <div className="aspect-square animate-pulse bg-muted" />
          <div className="space-y-2 p-3">
            <div className="h-3 w-2/5 animate-pulse rounded bg-muted" />
            <div className="h-3 w-3/4 animate-pulse rounded bg-muted" />
            <div className="h-3 w-1/3 animate-pulse rounded bg-muted" />
          </div>
        </Card>
      ))}
    </div>
  );
}
