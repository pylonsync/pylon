/**
 * Account — bid history + watchlist for the current user.
 */
import { useMemo } from "react";
import { db } from "@pylonsync/react";
import { ArrowRight, Trophy, Wallet } from "lucide-react";
import { Card, CardContent, CardHeader, CardTitle } from "@pylonsync/example-ui/card";
import { Badge } from "@pylonsync/example-ui/badge";
import type { Bid, Lot } from "./lib/types";
import { formatCents, navigate } from "./lib/util";
import { useAuth } from "./lib/auth";

export function Account() {
  const { user } = useAuth();
  const userId = user?.id ?? "";
  const { data: bids } = db.useQuery<Bid>("Bid", {
    where: userId ? { bidderId: userId } : undefined,
  });
  const { data: lots } = db.useQuery<Lot>("Lot");

  const lotById = useMemo(
    () => new Map((lots ?? []).map((l) => [l.id, l])),
    [lots],
  );

  // Group bids by lot so the user sees one entry per lot they touched
  // with their highest bid.
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

  return (
    <div className="mx-auto max-w-4xl space-y-8 px-4 py-10 md:px-6">
      <header className="flex flex-wrap items-baseline justify-between gap-4">
        <h1 className="font-display text-3xl font-semibold tracking-tight">
          Your account
        </h1>
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
        icon={<Trophy className="size-4 text-amber-400" />}
        empty="You haven't won any lots yet."
      >
        {wins.map(({ lot, topBid }) => (
          <Row
            key={lot.id}
            lot={lot}
            topBid={topBid}
            isWinner
            onClick={() => navigate(`#/lot/${encodeURIComponent(lot.id)}`)}
          />
        ))}
      </Section>

      <Section
        title="All bids"
        empty="You haven't bid yet."
      >
        {myLots.map(({ lot, topBid }) => (
          <Row
            key={lot.id}
            lot={lot}
            topBid={topBid}
            isWinner={lot.status === "sold" && lot.winnerId === userId}
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
        <CardTitle className="flex items-center gap-2 text-sm">
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

function Row({
  lot,
  topBid,
  isWinner,
  onClick,
}: {
  lot: Lot;
  topBid: Bid;
  isWinner: boolean;
  onClick: () => void;
}) {
  return (
    <button
      onClick={onClick}
      className="flex w-full items-center gap-3 border-b border-border/40 px-6 py-3 text-left text-sm transition-colors last:border-b-0 hover:bg-accent/40"
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
        <div className="truncate font-medium">{lot.title}</div>
        <div className="text-xs text-muted-foreground">
          Your top bid: {formatCents(topBid.amountCents)} · Status:{" "}
          {lot.status}
        </div>
      </div>
      {isWinner && (
        <Badge variant="success">
          <Trophy className="size-3" />
          Won
        </Badge>
      )}
      <ArrowRight className="size-4 text-muted-foreground" />
    </button>
  );
}
