/**
 * Live auction room — auctioneer's stage on the left, current lot
 * spotlight in the center, lot order on the right. The auctioneer
 * (auction.creatorId) gets controls to advance lots; everyone else
 * can bid on the open lot.
 */
import { useMemo } from "react";
import { db, callFn } from "@pylonsync/react";
import {
  ArrowLeft,
  CheckCircle2,
  CircleDot,
  Clock,
  Gavel,
  Radio,
  XCircle,
} from "lucide-react";
import { Button } from "@pylonsync/example-ui/button";
import { Card, CardContent, CardHeader, CardTitle } from "@pylonsync/example-ui/card";
import { Badge } from "@pylonsync/example-ui/badge";
import { Separator } from "@pylonsync/example-ui/separator";
import { Input } from "@pylonsync/example-ui/input";
import { cn } from "@pylonsync/example-ui/utils";
import { useEffect, useState } from "react";
import type { Auction, Bid, Lot } from "./lib/types";
import { formatCents, navigate, timeLeft } from "./lib/util";
import { useAuth } from "./lib/auth";
import { useTick } from "./hooks/useTick";

export function LiveRoom({ id }: { id: string }) {
  const { data: auction, loading } = db.useQueryOne<Auction>("Auction", id);
  const { data: lots } = db.useQuery<Lot>("Lot", {
    where: { auctionId: id },
    orderBy: { position: "asc" },
  });
  useTick(500);
  const { user } = useAuth();
  const isAuctioneer = !!user && !!auction && auction.creatorId === user.id;

  const sortedLots = useMemo(
    () => [...(lots ?? [])].sort((a, b) => a.position - b.position),
    [lots],
  );
  const currentLot = sortedLots.find(
    (l) => auction && l.id === auction.currentLotId,
  );
  const upcoming = sortedLots.filter((l) => l.status === "pending");
  const completed = sortedLots.filter(
    (l) => l.status === "sold" || l.status === "passed",
  );

  if (loading) {
    return (
      <div className="p-12 text-center text-muted-foreground">Loading live auction…</div>
    );
  }
  if (!auction) {
    return (
      <div className="mx-auto max-w-md p-8 text-center text-sm text-muted-foreground">
        Auction not found.
      </div>
    );
  }

  return (
    <div className="grid h-[calc(100vh-3.5rem)] grid-rows-[auto_1fr]">
      <header
        className="flex items-center gap-4 border-b px-6 py-3"
        style={{
          background: `linear-gradient(90deg, ${auction.bannerColor ?? "#ec4899"}30, transparent)`,
        }}
      >
        <Button
          variant="ghost"
          size="sm"
          onClick={() => navigate("#/")}
        >
          <ArrowLeft className="size-4" />
        </Button>
        <div className="flex items-center gap-2">
          <Radio
            className={cn(
              "size-4",
              auction.status === "running" ? "animate-pulse text-destructive" : "text-muted-foreground",
            )}
          />
          <h1 className="font-display text-xl font-semibold tracking-tight">
            {auction.title}
          </h1>
          <Badge variant="secondary" className="capitalize">
            {auction.status}
          </Badge>
        </div>
        <div className="ml-auto text-xs text-muted-foreground">
          {sortedLots.length} lots · {completed.length} resolved
        </div>
      </header>

      <div className="grid grid-cols-[260px_1fr_280px] overflow-hidden">
        <LotQueue
          lots={sortedLots}
          currentLotId={auction.currentLotId ?? null}
          isAuctioneer={isAuctioneer}
          auctionStatus={auction.status}
        />
        <Spotlight
          auction={auction}
          lot={currentLot}
          upcoming={upcoming}
          isAuctioneer={isAuctioneer}
        />
        <BidStream lots={sortedLots} currentLotId={auction.currentLotId ?? null} />
      </div>
    </div>
  );
}

function LotQueue({
  lots,
  currentLotId,
  isAuctioneer,
  auctionStatus,
}: {
  lots: Lot[];
  currentLotId: string | null;
  isAuctioneer: boolean;
  auctionStatus: string;
}) {
  return (
    <aside className="flex flex-col overflow-hidden border-r bg-card/40">
      <div className="border-b px-4 py-3 text-[11px] font-semibold uppercase tracking-wider text-muted-foreground">
        Lot order
      </div>
      <div className="flex-1 overflow-y-auto">
        {lots.map((lot) => {
          const active = currentLotId === lot.id;
          const Icon =
            lot.status === "sold"
              ? CheckCircle2
              : lot.status === "passed"
              ? XCircle
              : active
              ? Radio
              : CircleDot;
          return (
            <div
              key={lot.id}
              className={cn(
                "flex items-center gap-3 border-b border-border/40 px-4 py-3 last:border-b-0",
                active && "bg-accent",
              )}
            >
              <Icon
                className={cn(
                  "size-4",
                  lot.status === "sold"
                    ? "text-emerald-400"
                    : lot.status === "passed"
                    ? "text-muted-foreground"
                    : active
                    ? "animate-pulse text-destructive"
                    : "text-muted-foreground",
                )}
              />
              <div className="min-w-0 flex-1">
                <div className="truncate text-sm font-medium">
                  Lot {lot.position + 1} · {lot.title}
                </div>
                <div className="text-xs text-muted-foreground">
                  {lot.status === "sold"
                    ? `Sold ${formatCents(lot.currentCents)}`
                    : lot.status === "passed"
                    ? "Passed"
                    : active
                    ? `Open · ${formatCents(lot.currentCents)}`
                    : `Start ${formatCents(lot.startingCents)}`}
                </div>
              </div>
              {isAuctioneer &&
                lot.status === "pending" &&
                auctionStatus !== "ended" && (
                  <Button
                    size="xs"
                    variant="outline"
                    onClick={() =>
                      callFn("openLot", { lotId: lot.id }).catch(() => {})
                    }
                  >
                    Open
                  </Button>
                )}
            </div>
          );
        })}
      </div>
    </aside>
  );
}

function Spotlight({
  auction,
  lot,
  upcoming,
  isAuctioneer,
}: {
  auction: Auction;
  lot: Lot | undefined;
  upcoming: Lot[];
  isAuctioneer: boolean;
}) {
  const { ms, label } = timeLeft(lot?.endsAt);
  const closing = ms > 0 && ms < 10_000;

  if (!lot) {
    return (
      <main className="flex flex-col items-center justify-center gap-4 p-8 text-center">
        <Radio className="size-12 text-muted-foreground" />
        <h2 className="font-display text-2xl font-semibold">
          {auction.status === "ended" ? "Auction concluded" : "Between lots"}
        </h2>
        <p className="max-w-md text-sm text-muted-foreground">
          {auction.status === "ended"
            ? "Thanks for joining. Check the queue for results."
            : isAuctioneer
            ? "Open the next lot when you're ready."
            : `Up next: ${upcoming[0]?.title ?? "—"}`}
        </p>
        {isAuctioneer && upcoming.length > 0 && (
          <Button
            size="lg"
            onClick={() =>
              callFn("openLot", { lotId: upcoming[0].id }).catch(() => {})
            }
          >
            <Gavel className="size-4" />
            Open Lot {upcoming[0].position + 1}
          </Button>
        )}
      </main>
    );
  }

  return (
    <main className="flex flex-col gap-6 overflow-y-auto p-8">
      <div
        className="flex aspect-[2/1] items-center justify-center rounded-2xl text-7xl font-bold text-white/90"
        style={{
          background: `linear-gradient(135deg, ${lot.imageColor ?? "#ec4899"}, ${lot.imageColor ?? "#ec4899"}cc)`,
        }}
      >
        {lot.position + 1}
      </div>

      <div className="grid gap-6 md:grid-cols-[1.4fr_1fr]">
        <div>
          <div className="text-xs font-semibold uppercase tracking-wider text-muted-foreground">
            Lot {lot.position + 1}
          </div>
          <h2 className="mt-1 font-display text-3xl font-semibold leading-tight">
            {lot.title}
          </h2>
          <p className="mt-2 max-w-xl text-sm leading-relaxed text-muted-foreground">
            {lot.description}
          </p>
        </div>

        <Card>
          <CardContent className="p-5">
            <div className="flex flex-wrap items-end gap-4">
              <div>
                <div className="text-[10px] font-semibold uppercase tracking-wider text-muted-foreground">
                  Current bid
                </div>
                <div className="mt-1 font-mono text-3xl font-bold tabular-nums">
                  {formatCents(lot.currentCents)}
                </div>
              </div>
              <div className="ml-auto">
                <div className="text-[10px] font-semibold uppercase tracking-wider text-muted-foreground">
                  Going in
                </div>
                <div
                  className={cn(
                    "mt-1 flex items-center gap-1 font-mono text-2xl font-bold tabular-nums",
                    closing && "animate-pulse text-destructive",
                  )}
                >
                  <Clock className="size-4" />
                  {label}
                </div>
              </div>
            </div>
            <Separator className="my-4" />
            <LiveBidPanel lot={lot} />
          </CardContent>
        </Card>
      </div>
    </main>
  );
}

function LiveBidPanel({ lot }: { lot: Lot }) {
  const { isAuthenticated, user } = useAuth();
  const placeBid = db.useMutation<{ lotId: string; amountCents: number }>(
    "placeBid",
  );
  const minNext =
    lot.bidCount === 0
      ? lot.startingCents
      : lot.currentCents + lot.minIncrementCents;
  const [amount, setAmount] = useState<string>(() => `${minNext / 100}`);

  useEffect(() => {
    setAmount(`${minNext / 100}`);
  }, [minNext]);

  const quickBid = (mult: number) => {
    const c = lot.currentCents + lot.minIncrementCents * mult;
    placeBid.mutate({ lotId: lot.id, amountCents: c }).catch(() => {});
  };

  if (!isAuthenticated) {
    return (
      <p className="text-sm text-muted-foreground">
        Log in to place a bid.
      </p>
    );
  }

  return (
    <div className="flex flex-col gap-2">
      <div className="grid grid-cols-3 gap-1.5">
        {[1, 2, 5].map((m) => (
          <Button
            key={m}
            variant="outline"
            size="sm"
            onClick={() => quickBid(m)}
            disabled={placeBid.loading}
          >
            +{formatCents(lot.minIncrementCents * m)}
          </Button>
        ))}
      </div>
      <div className="flex gap-2">
        <Input
          type="number"
          inputMode="decimal"
          value={amount}
          onChange={(e) => setAmount(e.target.value)}
          min={minNext / 100}
          step="0.01"
          className="font-mono"
        />
        <Button
          onClick={() =>
            placeBid
              .mutate({ lotId: lot.id, amountCents: Math.round(Number(amount) * 100) })
              .catch(() => {})
          }
          disabled={placeBid.loading}
        >
          <Gavel className="size-4" />
          Bid
        </Button>
      </div>
      {placeBid.error && (
        <div className="rounded-md border border-destructive/30 bg-destructive/5 px-3 py-2 text-xs text-destructive">
          {placeBid.error.message}
        </div>
      )}
      {user?.balanceCents != null && (
        <div className="text-xs text-muted-foreground">
          Balance: {formatCents(user.balanceCents)}
        </div>
      )}
    </div>
  );
}

function BidStream({
  lots,
  currentLotId,
}: {
  lots: Lot[];
  currentLotId: string | null;
}) {
  const auctionId = lots[0]?.auctionId;
  const { data: bids } = db.useQuery<Bid>(
    "Bid",
    auctionId ? { where: { auctionId } } : undefined,
  );
  const sorted = useMemo(
    () =>
      [...(bids ?? [])].sort(
        (a, b) => +new Date(b.createdAt) - +new Date(a.createdAt),
      ),
    [bids],
  );

  return (
    <aside className="flex flex-col overflow-hidden border-l bg-card/40">
      <div className="border-b px-4 py-3 text-[11px] font-semibold uppercase tracking-wider text-muted-foreground">
        Bid stream
      </div>
      <div className="flex-1 overflow-y-auto">
        {sorted.length === 0 ? (
          <div className="px-4 py-8 text-center text-xs text-muted-foreground">
            No bids yet.
          </div>
        ) : (
          sorted.slice(0, 60).map((b) => {
            const lot = lots.find((l) => l.id === b.lotId);
            const isCurrent = b.lotId === currentLotId;
            return (
              <div
                key={b.id}
                className={cn(
                  "flex items-baseline gap-2 border-b border-border/30 px-4 py-2 text-sm last:border-b-0",
                  isCurrent && "bg-primary/5",
                )}
              >
                <span className="font-medium">{b.bidderName}</span>
                <span className="ml-auto font-mono tabular-nums">
                  {formatCents(b.amountCents)}
                </span>
                <span className="w-10 text-right text-[10px] text-muted-foreground">
                  L{(lot?.position ?? 0) + 1}
                </span>
              </div>
            );
          })
        )}
      </div>
    </aside>
  );
}
