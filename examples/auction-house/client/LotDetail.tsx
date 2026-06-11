"use client";

/**
 * Lot detail — bidding interface for a timed lot. Live bid history,
 * countdown timer, bid form, and watchlist toggle. Closing-soon
 * state pulses red.
 */
import { useEffect, useMemo, useState } from "react";
import { db } from "@pylonsync/react";
import { ArrowLeft, Bookmark, BookmarkCheck, Clock, Gavel } from "lucide-react";
import { Button } from "@pylonsync/example-ui/button";
import { Card, CardContent, CardHeader, CardTitle } from "@pylonsync/example-ui/card";
import { Input } from "@pylonsync/example-ui/input";
import { Badge } from "@pylonsync/example-ui/badge";
import { Separator } from "@pylonsync/example-ui/separator";
import { Skeleton } from "@pylonsync/example-ui/skeleton";
import { cn } from "@pylonsync/example-ui/utils";
import type { Auction, Bid, Lot, Watch } from "./lib/types";
import { formatCents, navigate, timeLeft } from "./lib/util";
import { useAuth } from "./lib/auth";
import { useTick } from "./hooks/useTick";

export function LotDetail({
  id,
  onPromptAuth,
}: {
  id: string;
  onPromptAuth: () => void;
}) {
  const { data: lot, loading } = db.useQueryOne<Lot>("Lot", id);
  const { data: auction } = db.useQueryOne<Auction>(
    "Auction",
    lot?.auctionId ?? "",
  );
  const { data: bids } = db.useQuery<Bid>("Bid", { where: { lotId: id } });
  const { user, isAuthenticated } = useAuth();
  const { data: watches } = db.useQuery<Watch>("Watch", {
    where: user?.id ? { userId: user.id, lotId: id } : undefined,
  });
  useTick(1000);

  const isWatched = (watches ?? []).length > 0;
  const watchId = watches?.[0]?.id;

  const toggleWatch = async () => {
    if (!isAuthenticated) {
      onPromptAuth();
      return;
    }
    if (isWatched && watchId) {
      await db.delete("Watch", watchId);
    } else if (user?.id) {
      await db.insert("Watch", {
        userId: user.id,
        lotId: id,
        addedAt: new Date().toISOString(),
      });
    }
  };

  if (loading) {
    return (
      <div className="mx-auto max-w-5xl px-4 py-8 md:px-6">
        <Skeleton className="mb-4 h-8 w-32" />
        <div className="grid gap-6 md:grid-cols-[1.2fr_1fr]">
          <Skeleton className="aspect-square rounded-xl" />
          <div className="flex flex-col gap-4">
            <Skeleton className="h-6 w-3/4" />
            <Skeleton className="h-4 w-full" />
            <Skeleton className="h-32 w-full rounded-lg" />
          </div>
        </div>
      </div>
    );
  }
  if (!lot || !auction) {
    return (
      <div className="mx-auto max-w-md p-8 text-center text-sm text-muted-foreground">
        Lot not found.
      </div>
    );
  }

  return (
    <div className="mx-auto max-w-5xl px-4 py-8 md:px-6">
      <div className="mb-4 flex items-center gap-2">
        <Button
          variant="ghost"
          size="sm"
          onClick={() => navigate(`#/a/${encodeURIComponent(lot.auctionId)}`)}
        >
          <ArrowLeft className="size-4" />
          Back to {auction.title}
        </Button>
        <div className="flex-1" />
        <Button
          variant="ghost"
          size="sm"
          onClick={toggleWatch}
          className={cn(isWatched && "text-primary")}
          title={isWatched ? "Remove from watchlist" : "Add to watchlist"}
        >
          {isWatched ? (
            <BookmarkCheck className="size-4" />
          ) : (
            <Bookmark className="size-4" />
          )}
          {isWatched ? "Watching" : "Watch"}
        </Button>
      </div>

      <div className="grid gap-6 md:grid-cols-[1.2fr_1fr]">
        <div
          className="flex aspect-square items-center justify-center rounded-xl text-7xl font-bold text-white/90"
          style={{
            background: `linear-gradient(135deg, ${lot.imageColor ?? "#6366f1"}, ${lot.imageColor ?? "#6366f1"}cc)`,
          }}
        >
          {lot.position + 1}
        </div>

        <div className="flex flex-col gap-4">
          <div>
            <div className="text-xs font-semibold uppercase tracking-wider text-muted-foreground">
              Lot {lot.position + 1} · {auction.title}
            </div>
            <h1 className="mt-1 text-2xl font-semibold leading-tight">
              {lot.title}
            </h1>
            <p className="mt-2 text-sm leading-relaxed text-muted-foreground">
              {lot.description}
            </p>
          </div>

          <Card>
            <CardContent className="p-5">
              <CurrentBid lot={lot} />
              <Separator className="my-4" />
              <BidPanel
                lot={lot}
                auction={auction}
                onPromptAuth={onPromptAuth}
              />
            </CardContent>
          </Card>

          <BidHistory bids={bids ?? []} />
        </div>
      </div>
    </div>
  );
}

function CurrentBid({ lot }: { lot: Lot }) {
  const { ms, label } = timeLeft(lot.endsAt);
  const closing = ms > 0 && ms < 30_000;
  const sold = lot.status === "sold";
  const passed = lot.status === "passed";

  return (
    <div className="flex flex-wrap items-end gap-6">
      <div>
        <div className="text-[10px] font-semibold uppercase tracking-wider text-muted-foreground">
          Current bid
        </div>
        <div className="mt-1 font-mono text-3xl font-bold tabular-nums">
          {formatCents(lot.currentCents)}
        </div>
        <div className="mt-1 text-xs text-muted-foreground">
          {lot.bidCount} bid{lot.bidCount === 1 ? "" : "s"}
        </div>
      </div>
      <div className="ml-auto">
        <div className="text-[10px] font-semibold uppercase tracking-wider text-muted-foreground">
          {sold ? "Sold for" : passed ? "Closed" : "Time left"}
        </div>
        <div
          className={cn(
            "mt-1 flex items-center gap-1.5 font-mono text-xl font-semibold tabular-nums",
            closing && !sold && !passed && "animate-pulse text-destructive",
          )}
        >
          <Clock className="size-4" />
          {sold ? formatCents(lot.currentCents) : passed ? "—" : label}
        </div>
      </div>
    </div>
  );
}

function BidPanel({
  lot,
  auction,
  onPromptAuth,
}: {
  lot: Lot;
  auction: Auction;
  onPromptAuth: () => void;
}) {
  const { isAuthenticated, user } = useAuth();
  const placeBid = db.useMutation<{ lotId: string; amountCents: number }>(
    "placeBid",
  );
  const minNext = lot.bidCount === 0
    ? lot.startingCents
    : lot.currentCents + lot.minIncrementCents;
  const [amount, setAmount] = useState<string>(() => `${minNext / 100}`);

  useEffect(() => {
    setAmount(`${minNext / 100}`);
  }, [minNext]);

  if (lot.status !== "running" || auction.status !== "running") {
    return (
      <p className="text-sm text-muted-foreground">
        Bidding is closed for this lot.
      </p>
    );
  }

  const handleBid = async () => {
    if (!isAuthenticated) {
      onPromptAuth();
      return;
    }
    const cents = Math.round(Number(amount) * 100);
    if (!Number.isFinite(cents) || cents <= 0) return;
    await placeBid.mutate({ lotId: lot.id, amountCents: cents }).catch(() => {
      // error surfaces in placeBid.error below
    });
  };

  const quickBid = (mult: number) => {
    const c = lot.bidCount === 0
      ? lot.startingCents
      : lot.currentCents + lot.minIncrementCents * mult;
    if (!isAuthenticated) {
      onPromptAuth();
      return;
    }
    placeBid.mutate({ lotId: lot.id, amountCents: c }).catch(() => {});
  };

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
        <div className="relative flex-1">
          <span className="absolute left-3 top-1/2 -translate-y-1/2 text-muted-foreground">
            $
          </span>
          <Input
            type="number"
            inputMode="decimal"
            value={amount}
            onChange={(e) => setAmount(e.target.value)}
            min={minNext / 100}
            step="0.01"
            className="pl-7 font-mono"
          />
        </div>
        <Button onClick={handleBid} disabled={placeBid.loading}>
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

function BidHistory({ bids }: { bids: Bid[] }) {
  const sorted = useMemo(
    () => [...bids].sort((a, b) => +new Date(b.createdAt) - +new Date(a.createdAt)),
    [bids],
  );

  return (
    <Card>
      <CardHeader className="pb-2">
        <CardTitle className="text-sm font-medium">Bid history</CardTitle>
      </CardHeader>
      <CardContent className="px-0 pb-0">
        {sorted.length === 0 ? (
          <div className="px-6 py-8 text-center text-sm text-muted-foreground">
            No bids yet — yours could be the first.
          </div>
        ) : (
          <ul className="divide-y divide-border/40">
            {sorted.slice(0, 12).map((b, i) => (
              <li
                key={b.id}
                className={cn(
                  "flex items-center gap-3 px-6 py-2.5 text-sm",
                  i === 0 && "bg-primary/10",
                )}
              >
                <span className="font-medium">{b.bidderName}</span>
                <span className="ml-auto font-mono tabular-nums">
                  {formatCents(b.amountCents)}
                </span>
                <span className="w-12 text-right text-xs text-muted-foreground">
                  {relativeTime(b.createdAt)}
                </span>
              </li>
            ))}
          </ul>
        )}
      </CardContent>
    </Card>
  );
}

function relativeTime(iso: string) {
  const ms = Date.now() - new Date(iso).getTime();
  if (ms < 60_000) return `${Math.floor(ms / 1000)}s`;
  if (ms < 3_600_000) return `${Math.floor(ms / 60_000)}m`;
  return `${Math.floor(ms / 3_600_000)}h`;
}
