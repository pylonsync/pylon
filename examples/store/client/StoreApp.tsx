/**
 * Pylon Store — faceted search showcase.
 *
 * Demonstrates `db.useSearch` end to end:
 *   - Free-text search with BM25 ranking
 *   - Live facet counts (brand / category / color) that update on
 *     every filter change + on background catalog mutations
 *   - Sort by price / rating / createdAt
 *   - Price-range facets via equality filters
 *   - Pagination
 *
 * The whole surface is ~300 lines; no external search library, no
 * index server. The `entity("Product", ..., { search: {...} })` line
 * in app.ts is the complete config.
 */

import React, { useEffect, useMemo, useState } from "react";
import { init, db, callFn, configureClient, storageKey } from "@pylonsync/react";

const BASE_URL = import.meta.env.VITE_PYLON_URL ?? "http://localhost:4321";
const WS_URL =
  import.meta.env.VITE_PYLON_WS_URL ??
  (BASE_URL.startsWith("https://")
    ? `${BASE_URL.replace(/^https:/, "wss:").replace(/\/$/, "")}:4322`
    : undefined);

init({ baseUrl: BASE_URL, appName: "store", wsUrl: WS_URL });
configureClient({ baseUrl: BASE_URL, appName: "store" });

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

type Product = {
  id: string;
  name: string;
  description: string;
  brand: string;
  category: string;
  color: string;
  price: number;
  rating: number;
  stock: number;
  imageUrl?: string;
  createdAt: string;
};

type SortOption = {
  label: string;
  value: [string, "asc" | "desc"] | undefined;
};

const SORTS: SortOption[] = [
  { label: "Relevance", value: undefined },
  { label: "Price: low to high", value: ["price", "asc"] },
  { label: "Price: high to low", value: ["price", "desc"] },
  { label: "Rating", value: ["rating", "desc"] },
  { label: "Newest", value: ["createdAt", "desc"] },
];

const FACET_LABELS: Record<string, string> = {
  brand: "Brand",
  category: "Category",
  color: "Color",
};

// ---------------------------------------------------------------------------
// App
// ---------------------------------------------------------------------------

export function StoreApp() {
  const [query, setQuery] = useState("");
  const [debouncedQuery, setDebouncedQuery] = useState("");
  const [filters, setFilters] = useState<Record<string, string>>({});
  const [sortIdx, setSortIdx] = useState(0);
  const [page, setPage] = useState(0);
  const pageSize = 24;

  // Debounce the text input by 200ms so fast typing doesn't fire a
  // server query per keystroke. Facet toggles, sort changes, and
  // pagination go through without debounce — those feel snappier
  // when they respond immediately.
  useEffect(() => {
    const t = setTimeout(() => setDebouncedQuery(query.trim()), 200);
    return () => clearTimeout(t);
  }, [query]);

  // Reset to page 0 whenever the query or filters change — otherwise
  // a deep page would render empty for a narrow new query.
  useEffect(() => {
    setPage(0);
  }, [debouncedQuery, filters, sortIdx]);

  const sort = SORTS[sortIdx].value;

  const search = db.useSearch<Product>("Product", {
    query: debouncedQuery,
    filters,
    facets: ["brand", "category", "color"],
    sort,
    page,
    pageSize,
  });

  // Seed the catalog once on first load. Idempotent — returns quickly
  // if already populated. We need a guest session first because
  // seedCatalog requires `auth.userId`; without this, fresh visitors
  // would hit an empty catalog until they sign in somewhere else.
  useEffect(() => {
    (async () => {
      let token =
        typeof window !== "undefined"
          ? window.localStorage.getItem(storageKey("token"))
          : null;
      if (!token) {
        try {
          const res = await fetch(`${BASE_URL}/api/auth/guest`, {
            method: "POST",
          });
          const body = await res.json();
          token = body.token as string;
          window.localStorage.setItem(storageKey("token"), token);
          configureClient({ baseUrl: BASE_URL, appName: "store" });
        } catch {
          return; // offline / backend not up yet — search UI handles empty gracefully
        }
      }
      callFn("seedCatalog", { count: 10_000 }).catch(() => {});
    })();
  }, []);

  const toggleFilter = (facet: string, value: string) => {
    setFilters((f) => {
      const next = { ...f };
      if (next[facet] === value) delete next[facet];
      else next[facet] = value;
      return next;
    });
  };

  const clearFilters = () => {
    setFilters({});
    setQuery("");
  };

  const totalPages = Math.max(1, Math.ceil(search.total / pageSize));

  return (
    <div className="store">
      <header className="store-header">
        <div className="store-brand">
          <BrandMark />
          <span>Pylon Store</span>
        </div>
        <div className="store-search">
          <input
            type="text"
            value={query}
            onChange={(e) => setQuery(e.target.value)}
            placeholder="Search 10,000 products…"
            className="store-search-input"
          />
          {search.tookMs > 0 && (
            <span className="store-search-meta">
              {search.total.toLocaleString()} results · {search.tookMs}ms
            </span>
          )}
        </div>
      </header>

      <div className="store-body">
        <aside className="store-sidebar">
          <FacetGroups
            facetCounts={search.facetCounts}
            active={filters}
            onToggle={toggleFilter}
          />
          {(Object.keys(filters).length > 0 || query) && (
            <button className="store-clear" onClick={clearFilters}>
              Clear all
            </button>
          )}
        </aside>

        <main className="store-main">
          <div className="store-toolbar">
            <span className="store-active-filters">
              {Object.entries(filters).map(([facet, value]) => (
                <button
                  key={`${facet}:${value}`}
                  className="store-chip"
                  onClick={() => toggleFilter(facet, value)}
                >
                  {FACET_LABELS[facet]}: {value} ✕
                </button>
              ))}
            </span>
            <label className="store-sort">
              <span>Sort</span>
              <select
                value={sortIdx}
                onChange={(e) => setSortIdx(Number(e.target.value))}
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
            <div className="store-error">Search failed: {search.error.message}</div>
          ) : search.hits.length === 0 ? (
            <div className="store-empty">
              No products match that filter. Try clearing some criteria.
            </div>
          ) : (
            <>
              <div className="store-grid">
                {search.hits.map((p) => (
                  <ProductCard key={p.id} product={p} />
                ))}
              </div>

              <div className="store-pager">
                <button
                  disabled={page === 0}
                  onClick={() => setPage((p) => p - 1)}
                >
                  ← Prev
                </button>
                <span>
                  Page {page + 1} of {totalPages}
                </span>
                <button
                  disabled={page + 1 >= totalPages}
                  onClick={() => setPage((p) => p + 1)}
                >
                  Next →
                </button>
              </div>
            </>
          )}
        </main>
      </div>
    </div>
  );
}

// ---------------------------------------------------------------------------
// Subcomponents
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
  const order = ["category", "brand", "color"];
  return (
    <div className="facets">
      {order.map((facet) => {
        const counts = facetCounts[facet];
        if (!counts) return null;
        const entries = Object.entries(counts).sort(
          (a, b) => b[1] - a[1],
        );
        return (
          <div className="facet-group" key={facet}>
            <h4 className="facet-title">{FACET_LABELS[facet]}</h4>
            <ul className="facet-list">
              {entries.slice(0, 10).map(([value, count]) => (
                <li key={value}>
                  <button
                    className={`facet-item ${
                      active[facet] === value ? "on" : ""
                    }`}
                    onClick={() => onToggle(facet, value)}
                  >
                    <span className="facet-value">{value}</span>
                    <span className="facet-count">{count}</span>
                  </button>
                </li>
              ))}
            </ul>
          </div>
        );
      })}
    </div>
  );
}

function ProductCard({ product }: { product: Product }) {
  return (
    <div className="product-card">
      <div
        className="product-thumb"
        style={{
          background: `linear-gradient(135deg, ${hashColor(product.name)}, ${hashColor(product.brand)})`,
        }}
      >
        <span className="product-thumb-initials">
          {product.name
            .split(" ")
            .slice(0, 2)
            .map((w) => w[0]?.toUpperCase())
            .join("")}
        </span>
      </div>
      <div className="product-meta">
        <div className="product-brand">{product.brand}</div>
        <div className="product-name">{product.name}</div>
        <div className="product-row">
          <span className="product-price">${product.price.toFixed(2)}</span>
          <span className="product-rating">★ {product.rating.toFixed(1)}</span>
        </div>
      </div>
    </div>
  );
}

function SkeletonGrid({ count }: { count: number }) {
  return (
    <div className="store-grid">
      {Array.from({ length: count }).map((_, i) => (
        <div key={i} className="product-card skeleton">
          <div className="product-thumb skeleton-thumb" />
          <div className="product-meta">
            <div className="skeleton-line short" />
            <div className="skeleton-line" />
            <div className="skeleton-line short" />
          </div>
        </div>
      ))}
    </div>
  );
}

function BrandMark() {
  return (
    <svg viewBox="0 0 48 64" width="18" height="24" fill="currentColor">
      <path d="M24 2 L10 20 L24 32 Z" />
      <path d="M24 2 L38 20 L24 32 Z" />
      <path d="M24 32 L18 48 L24 62 L30 48 Z" />
      <path d="M6 30 Q3 46 16 56 L18 50 Q10 44 11 32 Z" />
      <path d="M42 30 Q45 46 32 56 L30 50 Q38 44 37 32 Z" />
    </svg>
  );
}

function hashColor(s: string): string {
  let h = 0;
  for (let i = 0; i < s.length; i++) {
    h = (h * 31 + s.charCodeAt(i)) | 0;
  }
  const hue = Math.abs(h) % 360;
  return `hsl(${hue}, 50%, 55%)`;
}
