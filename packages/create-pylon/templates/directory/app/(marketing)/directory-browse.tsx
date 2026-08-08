"use client";

import React, { useEffect, useMemo, useState } from "react";
import { db, callFn } from "@pylonsync/react";
import { EnsureGuest } from "@pylonsync/client";
import { siteConfig } from "@/lib/site.config";
import { parseTags, type ListingRow } from "@/lib/directory";

// The browse experience — the FTS showcase + the realtime heart of the template.
// `db.useSearch("Listing", …)` is a LIVE faceted full-text search: it re-runs
// the moment the box changes AND whenever the Listing table is written, so the
// vote counts tick up across every open tab the instant anyone upvotes (the
// `upvote` mutation is the write). No search service, no polling.
//
// Wrapped in <EnsureGuest> so the sync connection (which makes useSearch live)
// is established for anonymous visitors, and so seedListings can run on first
// load. The guest session holds no PII; Listings carry none.

const SORTS: { label: string; value: [string, "asc" | "desc"] }[] = [
  { label: "Top voted", value: ["votes", "desc"] },
  { label: "Newest", value: ["createdAt", "desc"] },
];

export function DirectoryBrowse() {
  return (
    <EnsureGuest fallback={<BrowseSkeleton />}>
      <BrowseInner />
    </EnsureGuest>
  );
}

function BrowseInner() {
  const [query, setQuery] = useState("");
  const [debounced, setDebounced] = useState("");
  const [category, setCategory] = useState<string | null>(null);
  const [sortIdx, setSortIdx] = useState(0);

  // Seed the directory from config on first visit (idempotent server-side).
  useEffect(() => {
    void callFn("seedListings", {});
  }, []);

  // Debounce the query so we're not firing a search on every keystroke.
  useEffect(() => {
    const t = setTimeout(() => setDebounced(query.trim()), 180);
    return () => clearTimeout(t);
  }, [query]);

  const search = db.useSearch<ListingRow>("Listing", {
    query: debounced,
    filters: category ? { category } : {},
    facets: ["category"],
    sort: SORTS[sortIdx].value,
    pageSize: 100,
  });

  const categoryCounts = search.facetCounts.category ?? {};
  const categories = useMemo(
    () => Object.keys(categoryCounts).sort((a, b) => categoryCounts[b] - categoryCounts[a]),
    [categoryCounts],
  );

  return (
    <div>
      {/* Search bar */}
      <div className="flex flex-col gap-3 sm:flex-row sm:items-center">
        <div className="relative flex-1">
          <SearchIcon />
          <input
            value={query}
            onChange={(e) => setQuery(e.target.value)}
            placeholder={siteConfig.hero.searchPlaceholder}
            aria-label="Search the directory"
            className="h-11 w-full rounded-full border border-zinc-300 bg-white pl-10 pr-4 text-[15px] text-zinc-900 outline-none transition placeholder:text-zinc-400 focus:border-brand focus:ring-2 focus:ring-brand/20"
          />
        </div>
        <div className="flex items-center gap-1 rounded-full border border-zinc-200 bg-white p-1">
          {SORTS.map((s, i) => (
            <button
              key={s.label}
              type="button"
              onClick={() => setSortIdx(i)}
              className={
                "rounded-full px-3 py-1.5 text-[13px] font-medium transition-colors " +
                (i === sortIdx ? "bg-brand text-white" : "text-zinc-600 hover:text-zinc-900")
              }
            >
              {s.label}
            </button>
          ))}
        </div>
      </div>

      <div className="mt-6 grid gap-8 lg:grid-cols-[200px_1fr]">
        {/* Facet sidebar — live counts per category from the search */}
        <aside className="hidden lg:block">
          <h3 className="font-mono text-[11px] uppercase tracking-[0.14em] text-zinc-400">Category</h3>
          <ul className="mt-3 space-y-0.5">
            <FacetItem
              label="All"
              count={search.total}
              active={category === null}
              onClick={() => setCategory(null)}
            />
            {categories.map((c) => (
              <FacetItem
                key={c}
                label={c}
                count={categoryCounts[c]}
                active={category === c}
                onClick={() => setCategory(category === c ? null : c)}
              />
            ))}
          </ul>
        </aside>

        {/* Results */}
        <div>
          <div className="mb-3 flex items-center justify-between text-[13px] text-zinc-500">
            <span>
              <span className="font-medium text-zinc-900">{search.total}</span>{" "}
              {search.total === 1 ? "tool" : "tools"}
              {category ? <> in {category}</> : null}
              {debounced ? <> matching “{debounced}”</> : null}
            </span>
            {category ? (
              <button type="button" onClick={() => setCategory(null)} className="text-brand hover:underline">
                Clear filter
              </button>
            ) : null}
          </div>

          {search.loading && search.hits.length === 0 ? (
            <BrowseSkeleton bare />
          ) : search.hits.length === 0 ? (
            <p className="rounded-xl border border-dashed border-zinc-300 px-6 py-12 text-center text-sm text-zinc-500">
              No tools match. Try a different search — or{" "}
              <a href="/submit" className="font-medium text-brand hover:underline">submit one</a>.
            </p>
          ) : (
            <ul className="space-y-3">
              {search.hits.map((l) => (
                <ListingCard key={l.id} listing={l} />
              ))}
            </ul>
          )}
        </div>
      </div>
    </div>
  );
}

function FacetItem({
  label,
  count,
  active,
  onClick,
}: {
  label: string;
  count: number;
  active: boolean;
  onClick: () => void;
}) {
  return (
    <li>
      <button
        type="button"
        onClick={onClick}
        className={
          "flex w-full items-center justify-between rounded-lg px-2.5 py-1.5 text-left text-[13.5px] transition-colors " +
          (active ? "bg-brand-soft font-medium text-brand" : "text-zinc-600 hover:bg-zinc-100")
        }
      >
        <span>{label}</span>
        <span className="tabular-nums text-[12px] text-zinc-400">{count ?? 0}</span>
      </button>
    </li>
  );
}

function ListingCard({ listing }: { listing: ListingRow }) {
  const tags = parseTags(listing.tags);
  return (
    <li className="flex items-start gap-4 rounded-2xl border border-zinc-200 bg-white p-4 transition-shadow hover:shadow-sm">
      <VoteButton listingId={listing.id} votes={listing.votes} />
      <div className="min-w-0 flex-1">
        <div className="flex flex-wrap items-center gap-2">
          <a
            href={listing.url}
            target="_blank"
            rel="noopener noreferrer"
            className="text-[15px] font-semibold text-zinc-900 hover:text-brand"
          >
            {listing.name}
          </a>
          {listing.featured ? (
            <span className="rounded-full bg-brand-soft px-2 py-0.5 text-[10px] font-medium uppercase tracking-wide text-brand">
              Featured
            </span>
          ) : null}
          <span className="rounded-full bg-zinc-100 px-2 py-0.5 text-[11px] font-medium text-zinc-500">
            {listing.category}
          </span>
        </div>
        <p className="mt-1 text-[14px] leading-relaxed text-zinc-600">{listing.tagline}</p>
        {tags.length > 0 ? (
          <div className="mt-2 flex flex-wrap gap-1.5">
            {tags.map((t) => (
              <span key={t} className="rounded bg-zinc-50 px-1.5 py-0.5 text-[11px] text-zinc-400">
                #{t}
              </span>
            ))}
          </div>
        ) : null}
      </div>
      <a
        href={listing.url}
        target="_blank"
        rel="noopener noreferrer"
        aria-label={`Visit ${listing.name}`}
        className="mt-1 shrink-0 text-zinc-300 transition-colors hover:text-brand"
      >
        <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round" aria-hidden>
          <path d="M7 17 17 7M7 7h10v10" />
        </svg>
      </a>
    </li>
  );
}

// The upvote control. Clicking calls the public `upvote` mutation; the count
// you see comes from the LIVE search, so it ticks up here and in every other
// open tab. localStorage remembers your votes so the arrow stays "voted".
function VoteButton({ listingId, votes }: { listingId: string; votes: number }) {
  const [voted, setVoted] = useState(false);

  useEffect(() => {
    try {
      setVoted(localStorage.getItem(`voted:${listingId}`) === "1");
    } catch {
      /* ignore */
    }
  }, [listingId]);

  async function onVote() {
    if (voted) return;
    setVoted(true);
    try {
      localStorage.setItem(`voted:${listingId}`, "1");
    } catch {
      /* ignore */
    }
    try {
      await callFn("upvote", { listingId });
    } catch {
      setVoted(false);
      try {
        localStorage.removeItem(`voted:${listingId}`);
      } catch {
        /* ignore */
      }
    }
  }

  return (
    <button
      type="button"
      onClick={onVote}
      disabled={voted}
      aria-label="Upvote"
      className={
        "flex h-14 w-12 shrink-0 flex-col items-center justify-center rounded-xl border text-center transition-colors " +
        (voted
          ? "border-brand bg-brand-soft text-brand"
          : "border-zinc-200 text-zinc-500 hover:border-brand hover:text-brand")
      }
    >
      <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2.5" strokeLinecap="round" strokeLinejoin="round" aria-hidden>
        <path d="m18 15-6-6-6 6" />
      </svg>
      <span className="text-[13px] font-semibold tabular-nums">{votes}</span>
    </button>
  );
}

function SearchIcon() {
  return (
    <svg
      className="pointer-events-none absolute left-3.5 top-1/2 size-4 -translate-y-1/2 text-zinc-400"
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      strokeWidth="2"
      strokeLinecap="round"
      strokeLinejoin="round"
      aria-hidden
    >
      <circle cx="11" cy="11" r="8" />
      <path d="m21 21-4.3-4.3" />
    </svg>
  );
}

function BrowseSkeleton({ bare }: { bare?: boolean }) {
  const list = (
    <ul className="space-y-3">
      {Array.from({ length: 6 }).map((_, i) => (
        <li key={i} className="flex items-start gap-4 rounded-2xl border border-zinc-200 bg-white p-4">
          <div className="h-14 w-12 animate-pulse rounded-xl bg-zinc-100" />
          <div className="flex-1 space-y-2 py-1">
            <div className="h-4 w-1/3 animate-pulse rounded bg-zinc-100" />
            <div className="h-3 w-2/3 animate-pulse rounded bg-zinc-100" />
          </div>
        </li>
      ))}
    </ul>
  );
  if (bare) return list;
  return (
    <div>
      <div className="h-11 w-full animate-pulse rounded-full bg-zinc-100" />
      <div className="mt-6 grid gap-8 lg:grid-cols-[200px_1fr]">
        <div className="hidden h-40 animate-pulse rounded-xl bg-zinc-100 lg:block" />
        {list}
      </div>
    </div>
  );
}
