import React, { Suspense, use } from "react";
import {
  type Metadata,
  type PageProps,
  type ServerData,
} from "@pylonsync/react";
import { Search } from "lucide-react";
import { cn } from "@/lib/utils";
import { ActivityRail, ActivityStrip } from "../client/ActivityFeed";
import { LiveGrid } from "../client/LiveGrid";
import { SeedOnEmpty } from "../client/SeedOnEmpty";
import type { Listing, Offer } from "../client/market";
import {
  CATEGORIES,
  browseListings,
  categoryLabel,
  isCategory,
} from "../lib/catalog";

export const metadata: Metadata = {
  title: "Reprise | Pre-owned furniture, audio, cameras, and clothing",
  description:
    "Buy and sell pre-owned furniture, lighting, audio, cameras, watches, and clothing from independent sellers. Make offers and see them answered live.",
};

const SORTS = [
  { id: "latest", label: "Newest" },
  { id: "price-low", label: "Price: low" },
  { id: "price-high", label: "Price: high" },
];

function browseHref(category: string, query: string, sort: string): string {
  const params = new URLSearchParams();
  if (category !== "all") params.set("category", category);
  if (query) params.set("q", query);
  if (sort !== "latest") params.set("sort", sort);
  const suffix = params.toString();
  return suffix ? `/?${suffix}` : "/";
}

function Catalog({
  serverData,
  category,
  query,
  sort,
}: {
  serverData: ServerData;
  category: string;
  query: string;
  sort: string;
}) {
  const listingsPromise = serverData.query<Listing>("Listing", {
    status: "active",
  });
  const offersPromise = serverData.query<Offer>("Offer", { status: "pending" });
  const active = use(listingsPromise);
  const pending = use(offersPromise);

  const matches = browseListings(active, { category, query, sort });
  const inQuery = query ? browseListings(active, { query }) : active;
  const countFor = (id: string) =>
    id === "all" ? inQuery.length : inQuery.filter((l) => l.category === id).length;
  const sellers = new Set(active.map((l) => l.sellerName)).size;

  const heading = query
    ? `Results for “${query}”`
    : category !== "all"
      ? categoryLabel(category)
      : "Fresh listings";

  return (
    <>
      <div className="flex flex-col gap-4 pb-5 pt-6 sm:pt-8 lg:flex-row lg:items-end lg:justify-between">
        <div className="min-w-0">
          <h1 className="font-display text-[40px] leading-[1.02] tracking-[-0.01em] sm:text-[52px]">
            {heading}
          </h1>
          <p className="mt-2 text-sm text-muted-foreground">
            {matches.length} {matches.length === 1 ? "item" : "items"} for sale
            {category === "all" && !query
              ? ` from ${sellers} independent ${sellers === 1 ? "seller" : "sellers"}`
              : ""}
            . Offers and sold status update live.
          </p>
        </div>
        <nav
          aria-label="Sort listings"
          className="flex shrink-0 items-center gap-1 rounded-full bg-muted p-1 text-[13px]"
        >
          {SORTS.map((item) => (
            <a
              key={item.id}
              href={browseHref(category, query, item.id)}
              aria-current={sort === item.id ? "true" : undefined}
              className={cn(
                "inline-flex min-h-8 items-center rounded-full px-3 font-medium transition-[background-color,color,box-shadow] duration-200",
                sort === item.id
                  ? "bg-card text-foreground shadow-[var(--shadow-border)]"
                  : "text-muted-foreground hover:text-foreground",
              )}
            >
              {item.label}
            </a>
          ))}
        </nav>
      </div>

      <nav
        aria-label="Listing categories"
        className="hide-scrollbar -mx-4 overflow-x-auto px-4 sm:-mx-8 sm:px-8"
      >
        <ul className="flex w-max gap-1.5 border-b border-border/70 pb-4">
          {[{ id: "all", label: "All" }, ...CATEGORIES].map((item) => {
            const count = countFor(item.id);
            if (item.id !== "all" && count === 0 && category !== item.id) {
              return null;
            }
            const current = category === item.id;
            return (
              <li key={item.id}>
                <a
                  href={browseHref(item.id, query, sort)}
                  aria-current={current ? "page" : undefined}
                  className={cn(
                    "inline-flex min-h-9 items-center gap-1.5 rounded-full px-3.5 text-[13px] font-medium transition-[background-color,color,box-shadow] duration-200",
                    current
                      ? "bg-foreground text-background"
                      : "bg-card text-foreground shadow-[var(--shadow-border)] hover:shadow-[var(--shadow-border-hover)]",
                  )}
                >
                  {item.label}
                  <span
                    className={cn(
                      "tabular-nums",
                      current ? "text-background/60" : "text-muted-foreground",
                    )}
                  >
                    {count}
                  </span>
                </a>
              </li>
            );
          })}
        </ul>
      </nav>

      <div className="pt-4 xl:hidden">
        <ActivityStrip />
      </div>

      <div className="grid gap-8 pt-6 xl:grid-cols-[minmax(0,1fr)_320px]">
        <section aria-label="Listings" className="min-w-0">
          {active.length === 0 ? (
            <EmptyState
              title="Adding sample listings"
              body="The catalog appears in a moment."
            >
              <SeedOnEmpty count={active.length} />
            </EmptyState>
          ) : matches.length === 0 ? (
            <EmptyState
              title="Nothing matches that yet"
              body="Try another category or a broader search."
            >
              <a
                href="/"
                className="mt-4 inline-flex min-h-10 items-center rounded-full bg-primary px-4 text-sm font-medium text-primary-foreground"
              >
                Clear filters
              </a>
            </EmptyState>
          ) : (
            <LiveGrid
              initial={active}
              initialOffers={pending}
              category={category}
              query={query}
              sort={sort}
            />
          )}
        </section>
        <aside className="hidden xl:block">
          <div className="sticky top-20">
            <ActivityRail />
          </div>
        </aside>
      </div>
    </>
  );
}

function EmptyState({
  title,
  body,
  children,
}: {
  title: string;
  body: string;
  children?: React.ReactNode;
}) {
  return (
    <div className="flex min-h-80 flex-col items-center justify-center rounded-2xl bg-card px-6 py-16 text-center shadow-[var(--shadow-border)]">
      <h2 className="font-display text-3xl">{title}</h2>
      <p className="mt-2 text-sm text-muted-foreground">{body}</p>
      {children}
    </div>
  );
}

function CatalogSkeleton() {
  return (
    <div className="pt-8">
      <div className="h-12 w-72 rounded-lg bg-muted" />
      <div className="mt-3 h-4 w-96 max-w-full rounded bg-muted" />
      <div className="mt-8 grid grid-cols-2 gap-x-4 gap-y-8 md:grid-cols-3 xl:grid-cols-4">
        {Array.from({ length: 8 }).map((_, index) => (
          <div key={index}>
            <div className="aspect-[4/5] rounded-xl bg-muted" />
            <div className="mt-3 h-4 w-1/3 rounded bg-muted" />
            <div className="mt-2 h-3.5 w-3/4 rounded bg-muted" />
          </div>
        ))}
      </div>
    </div>
  );
}

export default function BrowsePage({ searchParams, serverData }: PageProps) {
  const category =
    typeof searchParams.category === "string" && isCategory(searchParams.category)
      ? searchParams.category
      : "all";
  const query = typeof searchParams.q === "string" ? searchParams.q.trim() : "";
  const sort = typeof searchParams.sort === "string" ? searchParams.sort : "latest";

  return (
    <>
      <form
        method="get"
        action="/"
        role="search"
        className="relative pt-4 md:hidden"
      >
        <label htmlFor="mobile-search" className="sr-only">
          Search listings
        </label>
        <Search
          aria-hidden="true"
          strokeWidth={1.75}
          className="pointer-events-none absolute left-3.5 top-[calc(50%+8px)] size-4 -translate-y-1/2 text-muted-foreground"
        />
        <input
          id="mobile-search"
          name="q"
          type="search"
          autoComplete="off"
          defaultValue={query}
          placeholder="Search listings"
          className="h-11 w-full rounded-full border-0 bg-card pl-10 pr-4 text-[16px] shadow-[var(--shadow-border)] outline-none placeholder:text-muted-foreground"
        />
      </form>
      <Suspense fallback={<CatalogSkeleton />}>
        <Catalog
          serverData={serverData}
          category={category}
          query={query}
          sort={sort}
        />
      </Suspense>
    </>
  );
}
