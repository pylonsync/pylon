/**
 * Auction detail — banner + lot grid for a timed auction. Each lot
 * card shows current bid, time remaining, and links to its dedicated
 * bid page. For live auctions, link to LiveRoom instead.
 */
import { useMemo } from "react";
import { db } from "@pylonsync/react";
import { ArrowLeft, ArrowRight, Clock, Radio } from "lucide-react";
import { Card } from "@pylonsync/example-ui/card";
import { Badge } from "@pylonsync/example-ui/badge";
import { Button } from "@pylonsync/example-ui/button";
import { cn } from "@pylonsync/example-ui/utils";
import type { Auction, Lot } from "./lib/types";
import { formatCents, navigate, timeLeft } from "./lib/util";
import { useTick } from "./hooks/useTick";

export function AuctionDetail({ id }: { id: string }) {
  const { data: auction, loading } = db.useQueryOne<Auction>("Auction", id);
  const { data: lots } = db.useQuery<Lot>("Lot", {
    where: { auctionId: id },
    orderBy: { position: "asc" },
  });
  useTick(1000);

  const sortedLots = useMemo(() => [...(lots ?? [])].sort((a, b) => a.position - b.position), [lots]);

  if (loading) {
    return <div className="p-12 text-center text-muted-foreground">Loading auction…</div>;
  }
  if (!auction) {
    return (
      <div className="mx-auto max-w-md p-8 text-center text-sm text-muted-foreground">
        Auction not found.
      </div>
    );
  }

  if (auction.kind === "live") {
    return (
      <div className="mx-auto max-w-3xl p-12 text-center">
        <Radio className="mx-auto size-10 text-destructive" />
        <h2 className="mt-3 font-display text-2xl font-semibold">
          {auction.title}
        </h2>
        <p className="mt-2 text-sm text-muted-foreground">
          Live auctions run in the live room. Join the room to follow the
          auctioneer in real time.
        </p>
        <Button
          className="mt-4"
          onClick={() =>
            navigate(`#/a/${encodeURIComponent(auction.id)}/live`)
          }
        >
          Enter live room
        </Button>
      </div>
    );
  }

  return (
    <div>
      <Banner auction={auction} />
      <div className="mx-auto max-w-6xl px-4 py-8 md:px-6">
        <Button
          variant="ghost"
          size="sm"
          className="mb-4"
          onClick={() => navigate("#/")}
        >
          <ArrowLeft className="size-4" />
          Back to auctions
        </Button>
        <div className="mb-6 flex flex-wrap items-baseline gap-3">
          <h1 className="font-display text-3xl font-semibold tracking-tight">
            {auction.title}
          </h1>
          <Badge variant="secondary" className="capitalize">
            {auction.status}
          </Badge>
        </div>
        <p className="mb-8 max-w-2xl text-sm text-muted-foreground">
          {auction.description}
        </p>
        <div className="grid gap-4 sm:grid-cols-2 lg:grid-cols-3">
          {sortedLots.map((lot) => (
            <LotCard key={lot.id} lot={lot} />
          ))}
        </div>
      </div>
    </div>
  );
}

function Banner({ auction }: { auction: Auction }) {
  return (
    <div
      className="h-32 w-full md:h-40"
      style={{
        background: `linear-gradient(135deg, ${auction.bannerColor ?? "#6366f1"}, ${auction.bannerColor ?? "#6366f1"}80)`,
      }}
    />
  );
}

function LotCard({ lot }: { lot: Lot }) {
  const { label: timeLabel, ms } = timeLeft(lot.endsAt);
  const closing = ms > 0 && ms < 30_000;
  const sold = lot.status === "sold";

  return (
    <Card
      onClick={() => navigate(`#/lot/${encodeURIComponent(lot.id)}`)}
      className="group cursor-pointer overflow-hidden p-0 transition hover:-translate-y-0.5 hover:border-primary/40"
    >
      <div
        className="flex aspect-[4/3] items-center justify-center text-2xl font-semibold text-white/90"
        style={{
          background: `linear-gradient(135deg, ${lot.imageColor ?? "#6366f1"}, ${lot.imageColor ?? "#6366f1"}cc)`,
        }}
      >
        {lot.position + 1}
      </div>
      <div className="p-4">
        <div className="text-[10px] font-semibold uppercase tracking-wider text-muted-foreground">
          Lot {lot.position + 1}
        </div>
        <div className="mt-0.5 line-clamp-2 text-sm font-medium leading-snug">
          {lot.title}
        </div>
        <div className="mt-3 flex items-baseline justify-between">
          <span className="font-mono text-base font-semibold">
            {formatCents(lot.currentCents)}
          </span>
          <span className="text-xs text-muted-foreground">
            {lot.bidCount} bid{lot.bidCount === 1 ? "" : "s"}
          </span>
        </div>
        <div className="mt-2 flex items-center justify-between text-xs">
          <span
            className={cn(
              "flex items-center gap-1",
              sold
                ? "text-muted-foreground"
                : closing
                ? "font-medium text-destructive"
                : "text-muted-foreground",
            )}
          >
            <Clock className="size-3" />
            {sold
              ? "Sold"
              : lot.status === "passed"
              ? "Passed"
              : timeLabel}
          </span>
          <Badge
            variant={
              sold
                ? "success"
                : lot.status === "passed"
                ? "secondary"
                : closing
                ? "destructive"
                : "outline"
            }
            className="capitalize"
          >
            {lot.status === "running" ? "Open" : lot.status}
          </Badge>
        </div>
      </div>
    </Card>
  );
}
