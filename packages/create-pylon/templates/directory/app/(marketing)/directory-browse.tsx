"use client";

import React, { useEffect, useMemo, useState } from "react";
import { db, callFn } from "@pylonsync/react";
import { EnsureGuest } from "@pylonsync/client";
import { ArrowUpRight, ChevronUp } from "lucide-react";
import { siteConfig } from "@/lib/site.config";
import { LINK } from "@/components/marketing";
import { parseTags, type ListingRow } from "@/lib/directory";

// The browse island: search box, category rail, and the list.
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

const MONO_SMALL = "font-mono text-[12px]";

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

  const facetLinks = (
    <>
      <FacetLink label="All" count={search.total} active={category === null} onClick={() => setCategory(null)} />
      {categories.map((c) => (
        <FacetLink
          key={c}
          label={c}
          count={categoryCounts[c]}
          active={category === c}
          onClick={() => setCategory(category === c ? null : c)}
        />
      ))}
    </>
  );

  return (
    <div>
      <input
        value={query}
        onChange={(e) => setQuery(e.target.value)}
        placeholder={siteConfig.intro.searchPlaceholder}
        aria-label="Search the directory"
        className="h-10 w-full border border-zinc-300 bg-white px-3 font-mono text-[13px] text-zinc-900 outline-none placeholder:text-zinc-400 focus:border-brand"
      />

      <div className="mt-6 grid gap-8 lg:grid-cols-[180px_1fr]">
        {/* Category rail. Live counts per category come from the search. On
            narrow screens the same links run in a horizontal row. */}
        <aside>
          <p className="border-b border-zinc-200 pb-2 text-[12px] font-medium uppercase tracking-wide text-zinc-500">
            Category
          </p>
          <ul className="mt-2 flex gap-x-4 gap-y-1 overflow-x-auto pb-1 lg:flex-col lg:overflow-visible">{facetLinks}</ul>
        </aside>

        {/* Results */}
        <div>
          <div className="flex items-center justify-between border-b border-zinc-200 pb-2 text-[13px] text-zinc-500">
            <span className={MONO_SMALL}>
              {search.total} {search.total === 1 ? "tool" : "tools"}
              {category ? <> in {category}</> : null}
              {debounced ? <> matching "{debounced}"</> : null}
            </span>
            <span className="flex items-center gap-3">
              {SORTS.map((s, i) => (
                <button
                  key={s.label}
                  type="button"
                  onClick={() => setSortIdx(i)}
                  aria-pressed={i === sortIdx}
                  className={i === sortIdx ? "font-medium text-zinc-900" : "text-zinc-500 hover:text-zinc-900"}
                >
                  {s.label}
                </button>
              ))}
            </span>
          </div>

          {search.loading && search.hits.length === 0 ? (
            <BrowseSkeleton bare />
          ) : search.hits.length === 0 ? (
            <p className="py-10 text-[14px] text-zinc-600">
              No tools match.{" "}
              {category ? (
                <button type="button" onClick={() => setCategory(null)} className={LINK}>
                  Clear the category filter
                </button>
              ) : (
                <a href="/submit" className={LINK}>
                  Submit one
                </a>
              )}
              .
            </p>
          ) : (
            <ul>
              {search.hits.map((l) => (
                <ListingRowItem key={l.id} listing={l} />
              ))}
            </ul>
          )}
        </div>
      </div>
    </div>
  );
}

function FacetLink({
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
    <li className="shrink-0">
      <button
        type="button"
        onClick={onClick}
        aria-pressed={active}
        className={
          "flex w-full items-baseline gap-2 py-0.5 text-left text-[14px] lg:justify-between " +
          (active ? "font-medium text-zinc-900" : "text-brand hover:underline")
        }
      >
        <span>{label}</span>
        <span className={`${MONO_SMALL} text-zinc-400`}>{count ?? 0}</span>
      </button>
    </li>
  );
}

function ListingRowItem({ listing }: { listing: ListingRow }) {
  const tags = parseTags(listing.tags);
  return (
    <li className="grid grid-cols-[44px_1fr_20px] items-start gap-3 border-b border-zinc-200 py-3">
      <VoteButton listingId={listing.id} votes={listing.votes} />
      <div className="min-w-0">
        <div className="flex flex-wrap items-baseline gap-x-2">
          <a href={listing.url} target="_blank" rel="noopener noreferrer" className="text-[14px] font-semibold text-zinc-900 hover:text-brand">
            {listing.name}
          </a>
          <span className={`${MONO_SMALL} text-zinc-400`}>{listing.category}</span>
          {listing.featured ? <span className={`${MONO_SMALL} text-brand`}>featured</span> : null}
        </div>
        <p className="mt-0.5 text-[14px] leading-snug text-zinc-600">{listing.tagline}</p>
        {tags.length > 0 ? (
          <p className={`${MONO_SMALL} mt-1 text-zinc-400`}>{tags.join("  ")}</p>
        ) : null}
      </div>
      <a
        href={listing.url}
        target="_blank"
        rel="noopener noreferrer"
        aria-label={`Open ${listing.name}`}
        className="mt-0.5 text-zinc-400 hover:text-brand"
      >
        <ArrowUpRight size={16} strokeWidth={1.75} aria-hidden />
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
      aria-label={voted ? "Upvoted" : "Upvote"}
      aria-pressed={voted}
      className={
        "flex w-11 flex-col items-center py-0.5 font-mono text-[13px] tabular-nums " +
        (voted ? "text-brand" : "text-zinc-500 hover:text-brand")
      }
    >
      <span>{votes}</span>
      <ChevronUp size={14} strokeWidth={2} aria-hidden />
    </button>
  );
}

function BrowseSkeleton({ bare }: { bare?: boolean }) {
  const list = (
    <ul>
      {Array.from({ length: 6 }).map((_, i) => (
        <li key={i} className="grid grid-cols-[44px_1fr_20px] gap-3 border-b border-zinc-200 py-3">
          <div className="mx-auto h-8 w-6 animate-pulse bg-zinc-100" />
          <div className="space-y-2 py-0.5">
            <div className="h-3.5 w-1/3 animate-pulse bg-zinc-100" />
            <div className="h-3 w-2/3 animate-pulse bg-zinc-100" />
          </div>
        </li>
      ))}
    </ul>
  );
  if (bare) return list;
  return (
    <div>
      <div className="h-10 w-full animate-pulse border border-zinc-200 bg-zinc-50" />
      <div className="mt-6 grid gap-8 lg:grid-cols-[180px_1fr]">
        <div className="hidden h-40 animate-pulse bg-zinc-50 lg:block" />
        {list}
      </div>
    </div>
  );
}
