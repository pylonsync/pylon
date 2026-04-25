/**
 * Homepage — featured + upcoming + ended auctions.
 *
 * Pulls the live `Auction` collection and groups by status. Each card
 * shows the auction's banner color, lot count, and status-aware CTA
 * (Bid now / Set reminder / View results).
 */
import { useMemo } from "react";
import { db } from "@pylonsync/react";
import { ArrowRight, Clock, Hammer, Radio } from "lucide-react";
import { Card } from "@pylonsync/example-ui/card";
import { Badge } from "@pylonsync/example-ui/badge";
import { Button } from "@pylonsync/example-ui/button";
import { cn } from "@pylonsync/example-ui/utils";
import type { Auction, Lot } from "./lib/types";
import { navigate, timeLeft } from "./lib/util";

export function AuctionList() {
  const { data: auctions, loading } = db.useQuery<Auction>("Auction", {
    orderBy: { startsAt: "asc" },
  });
  const { data: lots } = db.useQuery<Lot>("Lot");

  const lotsByAuction = useMemo(() => {
    const m = new Map<string, Lot[]>();
    for (const l of lots ?? []) {
      const arr = m.get(l.auctionId) ?? [];
      arr.push(l);
      m.set(l.auctionId, arr);
    }
    return m;
  }, [lots]);

  const groups = useMemo(() => {
    const running: Auction[] = [];
    const scheduled: Auction[] = [];
    const ended: Auction[] = [];
    for (const a of auctions ?? []) {
      if (a.status === "running") running.push(a);
      else if (a.status === "scheduled") scheduled.push(a);
      else if (a.status === "ended") ended.push(a);
    }
    return { running, scheduled, ended };
  }, [auctions]);

  if (loading) {
    return <div className="p-12 text-center text-muted-foreground">Loading…</div>;
  }

  if ((auctions ?? []).length === 0) {
    return (
      <div className="mx-auto max-w-3xl p-16 text-center">
        <Hammer className="mx-auto size-12 text-muted-foreground" />
        <h2 className="mt-4 text-xl font-semibold">No auctions yet</h2>
        <p className="mt-2 text-sm text-muted-foreground">
          The seed should populate sample auctions on first load. If this
          persists, sign in and host one yourself.
        </p>
      </div>
    );
  }

  return (
    <div className="mx-auto max-w-6xl space-y-12 px-4 py-10 md:px-6">
      {groups.running.length > 0 && (
        <Section title="Live now" subtitle="Bidding open across multiple lots.">
          <div className="grid gap-4 md:grid-cols-2">
            {groups.running.map((a) => (
              <AuctionCard
                key={a.id}
                auction={a}
                lots={lotsByAuction.get(a.id) ?? []}
              />
            ))}
          </div>
        </Section>
      )}
      {groups.scheduled.length > 0 && (
        <Section
          title="Upcoming"
          subtitle="Set a reminder — these will go live on schedule."
        >
          <div className="grid gap-4 md:grid-cols-3">
            {groups.scheduled.map((a) => (
              <AuctionCard
                key={a.id}
                auction={a}
                lots={lotsByAuction.get(a.id) ?? []}
              />
            ))}
          </div>
        </Section>
      )}
      {groups.ended.length > 0 && (
        <Section title="Recently ended">
          <div className="grid gap-4 md:grid-cols-3">
            {groups.ended.map((a) => (
              <AuctionCard
                key={a.id}
                auction={a}
                lots={lotsByAuction.get(a.id) ?? []}
              />
            ))}
          </div>
        </Section>
      )}
    </div>
  );
}

function Section({
  title,
  subtitle,
  children,
}: {
  title: string;
  subtitle?: string;
  children: React.ReactNode;
}) {
  return (
    <section>
      <div className="mb-4 flex items-baseline gap-3">
        <h2 className="font-display text-2xl font-semibold tracking-tight">
          {title}
        </h2>
        {subtitle && (
          <span className="text-sm text-muted-foreground">{subtitle}</span>
        )}
      </div>
      {children}
    </section>
  );
}

function AuctionCard({ auction, lots }: { auction: Auction; lots: Lot[] }) {
  const { label } = timeLeft(
    auction.status === "scheduled" ? auction.startsAt : auction.endsAt,
  );
  const soldCount = lots.filter((l) => l.status === "sold").length;
  const totalLots = lots.length;
  const goesTo =
    auction.kind === "live" && auction.status !== "ended"
      ? `#/a/${encodeURIComponent(auction.id)}/live`
      : `#/a/${encodeURIComponent(auction.id)}`;

  return (
    <Card
      onClick={() => navigate(goesTo)}
      className="group cursor-pointer overflow-hidden p-0 transition hover:-translate-y-0.5 hover:border-primary/40 hover:shadow-lg"
    >
      <div
        className="relative h-32 w-full"
        style={{
          background: `linear-gradient(135deg, ${auction.bannerColor ?? "#6366f1"}, ${auction.bannerColor ?? "#6366f1"}80)`,
        }}
      >
        <div className="absolute right-3 top-3">
          <KindBadge kind={auction.kind} status={auction.status} />
        </div>
      </div>
      <div className="p-5">
        <h3 className="font-display text-lg font-semibold leading-tight">
          {auction.title}
        </h3>
        <p className="mt-1 line-clamp-2 text-sm text-muted-foreground">
          {auction.description}
        </p>
        <div className="mt-4 flex items-center justify-between text-xs">
          <div className="flex items-center gap-1.5 text-muted-foreground">
            <Clock className="size-3.5" />
            <span>
              {auction.status === "scheduled"
                ? `Starts in ${label}`
                : auction.status === "running"
                ? `Ends in ${label}`
                : "Ended"}
            </span>
          </div>
          <div className="text-muted-foreground">
            {soldCount > 0
              ? `${soldCount}/${totalLots} sold`
              : `${totalLots} lot${totalLots === 1 ? "" : "s"}`}
          </div>
        </div>
        <div className="mt-4 flex items-center justify-between">
          <Badge variant="secondary" className="capitalize">
            {auction.status}
          </Badge>
          <span className="flex items-center gap-1 text-sm font-medium text-primary opacity-0 transition-opacity group-hover:opacity-100">
            View <ArrowRight className="size-3.5" />
          </span>
        </div>
      </div>
    </Card>
  );
}

function KindBadge({
  kind,
  status,
}: {
  kind: string;
  status: string;
}) {
  if (kind === "live") {
    const live = status === "running";
    return (
      <span
        className={cn(
          "inline-flex items-center gap-1.5 rounded-full px-2.5 py-1 text-[10px] font-semibold uppercase tracking-wider",
          live ? "bg-destructive text-destructive-foreground" : "bg-black/40 text-white",
        )}
      >
        {live && <span className="size-1.5 animate-pulse rounded-full bg-current" />}
        <Radio className="size-3" />
        Live
      </span>
    );
  }
  return (
    <span className="inline-flex items-center gap-1 rounded-full bg-black/40 px-2.5 py-1 text-[10px] font-semibold uppercase tracking-wider text-white">
      <Clock className="size-3" />
      Timed
    </span>
  );
}
