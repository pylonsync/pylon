/**
 * Catalog — faceted full-text search across the Product table.
 *
 * Wraps `db.useSearch` and renders three blocks:
 *   - search input + result meta in a sticky strip below the header
 *   - sidebar with live facet counts (brand / category / color)
 *   - main grid with sort, pagination, and product cards
 *
 * Loading state shows skeleton cards on the initial load only — for
 * subsequent searches the previous results stay on screen so the UI
 * doesn't flicker between every keystroke.
 */
import { useEffect, useState } from "react";
import { db } from "@pylonsync/react";
import { Card } from "@/components/ui/card";
import { Input } from "@/components/ui/input";
import { Button } from "@/components/ui/button";
import { Badge } from "@/components/ui/badge";
import { Star, X } from "lucide-react";
import type { Product } from "./lib/types";
import { gradient, initials, navigate } from "./lib/util";

type SortOption = {
  label: string;
  value: [string, "asc" | "desc"] | undefined;
};

const SORTS: SortOption[] = [
  { label: "Relevance", value: undefined },
  { label: "Price: low to high", value: ["price", "asc"] },
  { label: "Price: high to low", value: ["price", "desc"] },
  { label: "Highest rated", value: ["rating", "desc"] },
  { label: "Newest", value: ["createdAt", "desc"] },
];

const FACET_LABELS: Record<string, string> = {
  brand: "Brand",
  category: "Category",
  color: "Color",
};

const FACET_ORDER = ["category", "brand", "color"];

export function Catalog({
  onAddToCart,
}: {
  onAddToCart: (p: Product) => void;
}) {
  const [query, setQuery] = useState("");
  const [debouncedQuery, setDebouncedQuery] = useState("");
  const [filters, setFilters] = useState<Record<string, string>>({});
  const [sortIdx, setSortIdx] = useState(0);
  const [page, setPage] = useState(0);
  const pageSize = 24;

  useEffect(() => {
    const t = setTimeout(() => setDebouncedQuery(query.trim()), 200);
    return () => clearTimeout(t);
  }, [query]);

  useEffect(() => {
    setPage(0);
  }, [debouncedQuery, filters, sortIdx]);

  const search = db.useSearch<Product>("Product", {
    query: debouncedQuery,
    filters,
    facets: ["brand", "category", "color"],
    sort: SORTS[sortIdx].value,
    page,
    pageSize,
  });

  const toggleFilter = (facet: string, value: string) => {
    setFilters((f) => {
      const next = { ...f };
      if (next[facet] === value) delete next[facet];
      else next[facet] = value;
      return next;
    });
  };

  const clearAll = () => {
    setFilters({});
    setQuery("");
  };

  const totalPages = Math.max(1, Math.ceil(search.total / pageSize));
  const hasActive = Object.keys(filters).length > 0 || query.length > 0;

  return (
    <>
      <div className="border-b bg-background">
        <div className="mx-auto flex max-w-[1400px] items-center gap-4 px-4 py-3 md:px-6">
          <Input
            value={query}
            onChange={(e) => setQuery(e.target.value)}
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
              {Object.entries(filters).map(([facet, value]) => (
                <Badge
                  key={`${facet}:${value}`}
                  variant="secondary"
                  className="cursor-pointer gap-1 capitalize"
                  onClick={() => toggleFilter(facet, value)}
                >
                  {FACET_LABELS[facet]}: {value}
                  <X className="size-3" />
                </Badge>
              ))}
            </div>
            <label className="flex items-center gap-2 text-xs text-muted-foreground">
              Sort
              <select
                value={sortIdx}
                onChange={(e) => setSortIdx(Number(e.target.value))}
                className="h-8 rounded-md border bg-background px-2 text-xs text-foreground"
              >
                {SORTS.map((s, i) => (
                  <option key={s.label} value={i}>
                    {s.label}
                  </option>
                ))}
              </select>
            </label>
          </div>

          {search.loading && search.hits.length === 0 ? (
            <SkeletonGrid count={12} />
          ) : search.error ? (
            <EmptyState>
              Search failed: {search.error.message}
            </EmptyState>
          ) : search.hits.length === 0 ? (
            <EmptyState>
              No products match that filter. Try clearing some criteria.
            </EmptyState>
          ) : (
            <>
              <div className="grid grid-cols-2 gap-4 sm:grid-cols-3 lg:grid-cols-4 xl:grid-cols-5">
                {search.hits.map((p) => (
                  <ProductCard
                    key={p.id}
                    product={p}
                    onAddToCart={onAddToCart}
                  />
                ))}
              </div>

              <Pager
                page={page}
                totalPages={totalPages}
                onPrev={() => setPage((p) => Math.max(0, p - 1))}
                onNext={() => setPage((p) => Math.min(totalPages - 1, p + 1))}
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
        const entries = Object.entries(counts).sort((a, b) => b[1] - a[1]);
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
    <Card
      className="group cursor-pointer overflow-hidden p-0 transition hover:-translate-y-0.5 hover:shadow-md"
      onClick={() => navigate(`#/p/${encodeURIComponent(product.id)}`)}
      role="link"
      tabIndex={0}
    >
      <div
        className="flex aspect-square items-center justify-center text-2xl font-semibold text-white/90"
        style={{ background: gradient(product.name, product.brand) }}
      >
        {initials(product.name)}
      </div>
      <div className="flex flex-col gap-1 p-3">
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
        <Button
          variant="outline"
          size="sm"
          className="mt-2"
          onClick={(e) => {
            e.stopPropagation();
            onAddToCart(product);
          }}
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
