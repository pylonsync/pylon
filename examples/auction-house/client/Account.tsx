"use client";

/**
 * Account — bid history, watchlist, and balance for the current user.
 */
import { useMemo } from "react";
import { db } from "@pylonsync/react";
import { ArrowRight, Bookmark, BookmarkCheck, Trophy, Wallet } from "lucide-react";
import { Card, CardContent, CardHeader, CardTitle } from "@pylonsync/example-ui/card";
import { Badge } from "@pylonsync/example-ui/badge";
import { Skeleton } from "@pylonsync/example-ui/skeleton";
import type { Bid, Lot, Watch } from "./lib/types";
import { formatCents, navigate } from "./lib/util";
import { useAuth } from "./lib/auth";

export function Account() {
  const { user, loading: authLoading } = useAuth();
  const userId = user?.id ?? "";

  const { data: bids, loading: bidsLoading } = db.useQuery<Bid>("Bid", {
    where: userId ? { bidderId: userId } : undefined,
  });
  const { data: watches } = db.useQuery<Watch>("Watch", {
    where: userId ? { userId } : undefined,
  });
  const { data: lots } = db.useQuery<Lot>("Lot");

  const lotById = useMemo(
    () => new Map((lots ?? []).map((l) => [l.id, l])),
    [lots],
  );

  // Group bids by lot — one entry per lot with the user's highest bid.
  const myLots = useMemo(() => {
    const byLot = new Map<string, { lot: Lot; topBid: Bid }>();
    for (const b of bids ?? []) {
      const lot = lotById.get(b.lotId);
      if (!lot) continue;
      const existing = byLot.get(b.lotId);
      if (!existing || b.amountCents > existing.topBid.amountCents) {
        byLot.set(b.lotId, { lot, topBid: b });
      }
    }
    return Array.from(byLot.values()).sort(
      (a, b) => +new Date(b.topBid.createdAt) - +new Date(a.topBid.createdAt),
    );
  }, [bids, lotById]);

  const wins = myLots.filter(
    (m) => m.lot.status === "sold" && m.lot.winnerId === userId,
  );

  // Watched lots — lots the user bookmarked.
  const watchedLots = useMemo(() => {
    return (watches ?? [])
      .map((w) => ({ watch: w, lot: lotById.get(w.lotId) }))
      .filter((x): x is { watch: Watch; lot: Lot } => !!x.lot)
      .sort((a, b) => +new Date(b.watch.addedAt) - +new Date(a.watch.addedAt));
  }, [watches, lotById]);

  if (authLoading || bidsLoading) {
    return (
      <div className="mx-auto max-w-4xl space-y-8 px-4 py-10 md:px-6">
        <Skeleton className="h-9 w-48" />
        <div className="space-y-2">
          {[1, 2, 3].map((i) => (
            <Skeleton key={i} className="h-16 w-full rounded-lg" />
          ))}
        </div>
      </div>
    );
  }

  return (
    <div className="mx-auto max-w-4xl space-y-8 px-4 py-10 md:px-6">
      <header className="flex flex-wrap items-baseline justify-between gap-4">
        <h1 className="text-3xl font-semibold tracking-tight">Your account</h1>
        <div className="flex items-center gap-2 rounded-md border bg-card px-3 py-2">
          <Wallet className="size-4 text-muted-foreground" />
          <span className="text-xs text-muted-foreground">Balance</span>
          <span className="font-mono font-semibold tabular-nums">
            {formatCents(user?.balanceCents ?? 0)}
          </span>
        </div>
      </header>

      <Section
        title="Wins"
        icon={<Trophy className="size-4 text-primary" />}
        empty="You haven't won any lots yet."
      >
        {wins.map(({ lot, topBid }) => (
          <LotRow
            key={lot.id}
            lot={lot}
            sublabel={`Winning bid: ${formatCents(topBid.amountCents)}`}
            badge={<Badge variant="success"><Trophy className="size-3" /> Won</Badge>}
            onClick={() => navigate(`#/lot/${encodeURIComponent(lot.id)}`)}
          />
        ))}
      </Section>

      <Section
        title="Watchlist"
        icon={<BookmarkCheck className="size-4 text-primary" />}
        empty="No watched lots yet — click the bookmark on any lot to follow it."
      >
        {watchedLots.map(({ lot, watch }) => (
          <LotRow
            key={watch.id}
            lot={lot}
            sublabel={`Current bid: ${formatCents(lot.currentCents)} · ${lot.bidCount} bid${lot.bidCount === 1 ? "" : "s"}`}
            badge={
              lot.status === "sold" ? (
                <Badge variant="secondary">Sold</Badge>
              ) : lot.status === "running" ? (
                <Badge variant="outline">Open</Badge>
              ) : null
            }
            onRemoveWatch={watch.id}
            onClick={() => navigate(`#/lot/${encodeURIComponent(lot.id)}`)}
          />
        ))}
      </Section>

      <Section
        title="All bids"
        empty="You haven't bid yet."
      >
        {myLots.map(({ lot, topBid }) => (
          <LotRow
            key={lot.id}
            lot={lot}
            sublabel={`Your top bid: ${formatCents(topBid.amountCents)} · Status: ${lot.status}`}
            badge={
              lot.status === "sold" && lot.winnerId === userId ? (
                <Badge variant="success"><Trophy className="size-3" /> Won</Badge>
              ) : null
            }
            onClick={() => navigate(`#/lot/${encodeURIComponent(lot.id)}`)}
          />
        ))}
      </Section>
    </div>
  );
}

function Section({
  title,
  icon,
  empty,
  children,
}: {
  title: string;
  icon?: React.ReactNode;
  empty: string;
  children: React.ReactNode;
}) {
  const hasChildren = Array.isArray(children) ? children.length > 0 : !!children;
  return (
    <Card>
      <CardHeader className="pb-2">
        <CardTitle className="flex items-center gap-2 text-sm font-medium">
          {icon}
          {title}
        </CardTitle>
      </CardHeader>
      <CardContent className="space-y-1 px-0 pb-0">
        {hasChildren ? (
          children
        ) : (
          <div className="px-6 py-6 text-center text-sm text-muted-foreground">
            {empty}
          </div>
        )}
      </CardContent>
    </Card>
  );
}

function LotRow({
  lot,
  sublabel,
  badge,
  onRemoveWatch,
  onClick,
}: {
  lot: Lot;
  sublabel: string;
  badge?: React.ReactNode;
  onRemoveWatch?: string;
  onClick: () => void;
}) {
  return (
    <div className="flex w-full items-center gap-3 border-b border-border/40 px-6 py-3 last:border-b-0">
      <button
        onClick={onClick}
        className="flex min-w-0 flex-1 items-center gap-3 text-left"
      >
        <div
          className="flex size-10 shrink-0 items-center justify-center rounded-md text-xs font-semibold text-white/90"
          style={{
            background: `linear-gradient(135deg, ${lot.imageColor ?? "#6366f1"}, ${lot.imageColor ?? "#6366f1"}cc)`,
          }}
        >
          {lot.position + 1}
        </div>
        <div className="min-w-0 flex-1">
          <div className="truncate text-sm font-medium">{lot.title}</div>
          <div className="text-xs text-muted-foreground">{sublabel}</div>
        </div>
      </button>
      <div className="flex shrink-0 items-center gap-2">
        {badge}
        {onRemoveWatch && (
          <button
            onClick={() => db.delete("Watch", onRemoveWatch)}
            className="text-muted-foreground transition-colors hover:text-destructive"
            title="Remove from watchlist"
          >
            <Bookmark className="size-4" />
          </button>
        )}
        <button onClick={onClick} className="text-muted-foreground">
          <ArrowRight className="size-4" />
        </button>
      </div>
    </div>
  );
}
