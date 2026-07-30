"use client";

import React, { useState } from "react";
import { db } from "@pylonsync/react";
import { Button } from "../ui/button";
import { Input } from "../ui/input";
import { Textarea } from "../ui/textarea";
import { Badge } from "../ui/badge";
import { AuthGate, MarketProvider, useIdentity } from "./MarketProvider";
import { money, timeAgo, type Offer } from "./market";

interface Props {
  listingId: string;
  sellerId: string;
  sellerName: string;
  title: string;
  price: number;
  status: "active" | "sold";
}

type BadgeVariant = "default" | "secondary" | "destructive" | "outline" | "success" | "warning";

const statusVariant: Record<string, BadgeVariant> = {
  pending: "warning",
  accepted: "success",
  declined: "outline",
};

function Panel(props: Props) {
  const { listingId, sellerId, price } = props;
  const identity = useIdentity();
  const isSeller = !!identity && identity.userId === sellerId;
  const isSold = props.status === "sold";

  // The live query — every offer on this listing, newest first. Reads are
  // public, so this runs for signed-out visitors too; it just lights up the
  // moment a buyer in another tab makes an offer.
  const { data } = db.useQuery<Offer>("Offer", {
    where: { listingId },
    orderBy: { createdAt: "desc" },
  });
  const offers = data ?? [];
  const myOffer = identity
    ? offers.find((o) => o.buyerId === identity.userId)
    : undefined;

  if (isSeller) {
    return <SellerView offers={offers} />;
  }
  // Making an offer needs a real account — gate it (prefilled demo login).
  return (
    <AuthGate
      title="Sign in to make an offer"
      blurb="Offers are tied to a real account so the seller knows who is bidding. The demo account is ready; just select Log in."
    >
      <BuyerView
        {...props}
        myOffer={myOffer}
        isSold={isSold}
        suggestedPrice={price}
      />
    </AuthGate>
  );
}

function SellerView({ offers }: { offers: Offer[] }) {
  const [busy, setBusy] = useState<string | null>(null);
  const [err, setErr] = useState<string | null>(null);
  const pending = offers.filter((o) => o.status === "pending");

  // Optimistic accept/decline: flip the offer's status in the local store
  // immediately so the seller's list updates the instant they click. The
  // server (respondToOffer) reconciles the rest — marking the listing sold and
  // declining the sibling offers — when its broadcast lands.
  const respondMutation = db.useMutation<{ offerId: string; accept: boolean }>(
    "respondToOffer",
    {
      optimistic: (args) => {
        const o = offers.find((x) => x.id === args.offerId);
        return o
          ? [
              {
                entity: "Offer",
                data: { ...o, status: args.accept ? "accepted" : "declined" },
              },
            ]
          : [];
      },
    },
  );

  async function respond(offerId: string, accept: boolean) {
    setBusy(offerId);
    setErr(null);
    try {
      await respondMutation.mutate({ offerId, accept });
    } catch (e) {
      setErr((e as Error).message ?? "Could not respond to offer.");
    } finally {
      setBusy(null);
    }
  }

  return (
    <div className="space-y-3">
      <div className="flex items-center justify-between">
        <h2 className="font-semibold">Offers on your listing</h2>
        <Badge variant="outline">{pending.length} pending</Badge>
      </div>
      {err ? <p className="text-sm text-destructive">{err}</p> : null}
      {offers.length === 0 ? (
        <p className="rounded-xl bg-muted p-6 text-center text-sm text-muted-foreground">
          No offers yet. They will appear here as soon as a buyer sends one.
        </p>
      ) : (
        <ul className="space-y-2">
          {offers.map((o) => (
            <li
              key={o.id}
              className="flex items-center justify-between gap-3 rounded-lg border bg-card p-3"
            >
              <div className="min-w-0">
                <div className="flex items-center gap-2">
                  <span className="text-lg font-semibold tabular-nums">
                    {money(o.amount)}
                  </span>
                  <Badge variant={statusVariant[o.status] ?? "outline"}>
                    {o.status}
                  </Badge>
                </div>
                <p className="truncate text-sm text-muted-foreground">
                  {o.buyerName} · {timeAgo(o.createdAt)}
                  {o.message ? ` · "${o.message}"` : ""}
                </p>
              </div>
              {o.status === "pending" ? (
                <div className="flex shrink-0 gap-2">
                  <Button
                    size="sm"
                    disabled={busy === o.id}
                    onClick={() => respond(o.id, true)}
                  >
                    Accept
                  </Button>
                  <Button
                    size="sm"
                    variant="outline"
                    disabled={busy === o.id}
                    onClick={() => respond(o.id, false)}
                  >
                    Decline
                  </Button>
                </div>
              ) : null}
            </li>
          ))}
        </ul>
      )}
    </div>
  );
}

function BuyerView({
  listingId,
  title,
  sellerId,
  sellerName,
  myOffer,
  isSold,
  suggestedPrice,
}: Props & { myOffer?: Offer; isSold: boolean; suggestedPrice: number }) {
  // Rendered inside <AuthGate>, so identity is non-null here.
  const identity = useIdentity();
  const userId = identity?.userId ?? "";
  const name = identity?.name ?? "you";
  const [amount, setAmount] = useState(String(suggestedPrice));
  const [message, setMessage] = useState("");
  const [err, setErr] = useState<string | null>(null);

  // Local-first optimism, baked in: db.useMutation paints the Offer into the
  // local store the instant you click (the `optimistic` ghost), so the live
  // query below renders "Your offer" immediately — no waiting on the server,
  // no hand-rolled state. The server's makeOffer reuses the same id (threaded
  // as _optimisticId), so its broadcast merges in place; on failure the engine
  // rolls the ghost back on its own.
  const makeOffer = db.useMutation<
    { listingId: string; amount: number; message: string; buyerName: string },
    { id: string }
  >("makeOffer", {
    optimistic: (args, ctx) => ({
      entity: "Offer",
      data: {
        id: ctx.id,
        listingId,
        listingTitle: title,
        sellerId,
        buyerId: userId,
        buyerName: name,
        amount: args.amount,
        message: args.message,
        status: "pending",
        createdAt: ctx.now,
      },
    }),
  });

  // Buy now: same optimistic pattern, but the ghost is an *accepted* offer at
  // the list price — the buyer sees "🎉 Accepted" instantly while the server
  // marks the listing sold + declines other bids.
  const buyNow = db.useMutation<{ listingId: string; buyerName: string }, { id: string }>(
    "buyNow",
    {
      optimistic: (_args, ctx) => ({
        entity: "Offer",
        data: {
          id: ctx.id,
          listingId,
          listingTitle: title,
          sellerId,
          buyerId: userId,
          buyerName: name,
          amount: suggestedPrice,
          message: "Bought at list price",
          status: "accepted",
          createdAt: ctx.now,
        },
      }),
    },
  );

  async function buy() {
    setErr(null);
    try {
      await buyNow.mutate({ listingId, buyerName: name });
    } catch (e) {
      setErr((e as Error).message ?? "Could not complete the purchase.");
    }
  }

  // `myOffer` now includes the optimistic ghost, so this flips the instant
  // the offer is made.
  if (myOffer) {
    return (
      <div className="space-y-2 rounded-xl bg-card p-4 shadow-[var(--shadow-border)]">
        <h2 className="font-semibold">Your offer</h2>
        <div className="flex items-center gap-2">
          <span className="text-2xl font-semibold tabular-nums">{money(myOffer.amount)}</span>
          <Badge variant={statusVariant[myOffer.status] ?? "outline"}>
            {myOffer.status}
          </Badge>
        </div>
        <p className="text-sm text-muted-foreground">
          {myOffer.status === "pending"
            ? `Sent to ${sellerName}. Their answer will appear here live.`
            : myOffer.status === "accepted"
              ? "Accepted. Confirm payment and delivery with the seller."
              : "This offer was declined."}
        </p>
      </div>
    );
  }

  if (isSold) {
    return (
      <div className="rounded-xl bg-card p-4 text-sm text-muted-foreground shadow-[var(--shadow-border)]">
        This item has sold.
      </div>
    );
  }

  async function submit(e: React.FormEvent) {
    e.preventDefault();
    const value = Number.parseFloat(amount);
    if (!Number.isFinite(value) || value <= 0) {
      setErr("Enter an offer amount.");
      return;
    }
    setErr(null);
    try {
      // The ghost is painted synchronously here; the view has already flipped
      // to "Your offer" by the time this awaits.
      await makeOffer.mutate({ listingId, amount: value, message, buyerName: name });
    } catch (e) {
      setErr((e as Error).message ?? "Could not send offer.");
    }
  }

  return (
    <div className="space-y-4 rounded-xl bg-card p-5 shadow-[var(--shadow-border)]">
      <div className="space-y-2">
        <Button
          type="button"
          onClick={buy}
          disabled={buyNow.loading}
          className="w-full"
        >
          {buyNow.loading ? "Buying…" : `Buy now for ${money(suggestedPrice)}`}
        </Button>
        <p className="text-center text-xs text-muted-foreground">
          Instant purchase at the asking price.
        </p>
      </div>

      <div className="flex items-center gap-3 text-xs uppercase tracking-wide text-muted-foreground">
        <span className="h-px flex-1 bg-border" />
        or make an offer
        <span className="h-px flex-1 bg-border" />
      </div>

      <form onSubmit={submit} className="space-y-3">
        <div className="space-y-1.5">
          <label htmlFor="offer-amount" className="text-sm font-medium">
            Your offer
          </label>
          <div className="flex items-center gap-2">
            <span className="text-muted-foreground">$</span>
            <Input
              id="offer-amount"
              type="number"
              min="1"
              step="1"
              value={amount}
              onChange={(e) => setAmount(e.target.value)}
              className="w-32"
            />
          </div>
        </div>
        <div className="space-y-1.5">
          <label htmlFor="offer-note" className="text-sm font-medium">
            Note <span className="font-normal text-muted-foreground">(optional)</span>
          </label>
          <Textarea
            id="offer-note"
            placeholder="Share any useful details"
            value={message}
            onChange={(e) => setMessage(e.target.value)}
            rows={2}
          />
        </div>
        {err ? <p className="text-sm text-destructive">{err}</p> : null}
        <Button
          type="submit"
          variant="outline"
          disabled={makeOffer.loading}
          className="w-full"
        >
          {makeOffer.loading
            ? "Sending…"
            : `Offer ${money(Number.parseFloat(amount) || 0)}`}
        </Button>
        <p className="text-center text-xs text-muted-foreground">
          You're bidding as <span className="font-medium">{name}</span>
        </p>
      </form>
    </div>
  );
}

export function OfferPanel(props: Props) {
  return (
    <MarketProvider>
      <Panel {...props} />
    </MarketProvider>
  );
}
