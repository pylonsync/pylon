import React, { Suspense, use } from "react";
import {
  Link,
  type Metadata,
  type PageProps,
  type ServerData,
} from "@pylonsync/react";
import { Card } from "../ui/card";
import { Badge } from "../ui/badge";
import { LiveTicker } from "../client/LiveTicker";
import { SeedOnEmpty } from "../client/SeedOnEmpty";
import { CategoryIcon } from "./_components/CategoryIcon";
import { WatchButton } from "../client/WatchButton";
import { gradient, money, conditionLabel, type Listing } from "../client/market";

export const metadata: Metadata = {
  title: "Pylon Market — buy & sell locally, live",
  description:
    "A live local marketplace. Server-rendered listings for SEO, realtime offers over the sync engine — one Pylon binary, one port.",
};

const CATEGORIES = [
  "all", "furniture", "electronics", "cameras", "bikes", "audio", "kitchen",
  "instruments", "outdoor", "apparel",
];

// The grid suspends on the server-side read and streams in with real rows in
// the HTML (good for SEO + LCP). `serverData.query` runs through the same
// policy gate as a query function's ctx.db.
function Grid({
  serverData,
  category,
}: {
  serverData: ServerData;
  category: string;
}) {
  const active = use(serverData.query<Listing>("Listing", { status: "active" }));
  const sorted = [...active].sort((a, b) =>
    b.createdAt.localeCompare(a.createdAt),
  );
  const listings =
    category === "all"
      ? sorted
      : sorted.filter((l) => l.category === category);

  if (listings.length === 0) {
    return (
      <>
        <div className="rounded-xl border border-dashed p-12 text-center">
          <p className="text-sm text-muted-foreground">
            {active.length === 0
              ? "Setting up a few sample listings…"
              : "Nothing in this category yet."}
          </p>
          {category !== "all" ? (
            <Link href="/" className="mt-2 inline-block text-sm underline">
              See everything
            </Link>
          ) : null}
        </div>
        <SeedOnEmpty count={active.length} />
      </>
    );
  }

  return (
    <div className="grid grid-cols-2 gap-4 sm:grid-cols-3 lg:grid-cols-4">
      {listings.map((l) => (
        <Link key={l.id} href={`/listing/${l.slug || l.id}`} className="group">
          <Card className="flex flex-col overflow-hidden p-0 transition group-hover:-translate-y-0.5 group-hover:shadow-md">
            <div
              className="relative flex aspect-square items-center justify-center text-white/90"
              style={{ background: gradient(l.seed || l.id) }}
            >
              <CategoryIcon category={l.category} className="size-14" />
              <WatchButton
                listingId={l.id}
                listingTitle={l.title}
                className="absolute right-2 top-2 size-8"
              />
              {l.status === "sold" ? (
                <span className="absolute inset-0 grid place-items-center bg-black/55 text-sm font-bold uppercase tracking-wide">
                  Sold
                </span>
              ) : null}
            </div>
            <div className="flex flex-1 flex-col gap-1 p-3">
              <span className="text-[10px] font-semibold uppercase tracking-wider text-muted-foreground">
                {l.category}
              </span>
              <span className="line-clamp-2 min-h-[34px] text-sm font-medium leading-snug">
                {l.title}
              </span>
              <div className="mt-1 flex items-center justify-between">
                <span className="font-semibold">{money(l.price)}</span>
                <Badge variant="outline" className="text-[10px]">
                  {conditionLabel(l.condition)}
                </Badge>
              </div>
            </div>
          </Card>
        </Link>
      ))}
    </div>
  );
}

export default function BrowsePage({ searchParams, serverData }: PageProps) {
  const category =
    typeof searchParams.category === "string" ? searchParams.category : "all";

  return (
    <div className="space-y-6">
      <section className="space-y-1">
        <h1 className="text-3xl font-semibold tracking-tight">
          The local marketplace that's actually live
        </h1>
        <p className="text-muted-foreground">
          Listings are server-rendered for search engines; offers are realtime.
          Open this in two tabs — list in one, watch it appear in the other.
        </p>
      </section>

      {/* Realtime client island */}
      <LiveTicker />

      <nav className="flex flex-wrap gap-2">
        {CATEGORIES.map((c) => (
          <Link
            key={c}
            href={c === "all" ? "/" : `/?category=${c}`}
            className={`rounded-full border px-3 py-1 text-sm transition hover:bg-muted ${
              category === c
                ? "border-foreground bg-foreground text-background hover:bg-foreground"
                : "text-muted-foreground"
            }`}
          >
            {c}
          </Link>
        ))}
      </nav>

      <Suspense
        fallback={
          <div className="grid grid-cols-2 gap-4 sm:grid-cols-3 lg:grid-cols-4">
            {Array.from({ length: 8 }).map((_, i) => (
              <div
                key={i}
                className="aspect-[3/4] animate-pulse rounded-xl bg-muted"
              />
            ))}
          </div>
        }
      >
        <Grid serverData={serverData} category={category} />
      </Suspense>
    </div>
  );
}
