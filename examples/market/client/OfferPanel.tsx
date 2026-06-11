"use client";

import React, { useState } from "react";
import { db } from "@pylonsync/react";
import { Button } from "@pylonsync/example-ui/button";
import { Input } from "@pylonsync/example-ui/input";
import { Textarea } from "@pylonsync/example-ui/textarea";
import { Badge } from "@pylonsync/example-ui/badge";
import { MarketProvider, useIdentity } from "./MarketProvider";
import { money, timeAgo, type Offer } from "./market";

interface Props {
  listingId: string;
  sellerId: string;
  sellerName: string;
  title: string;
  price: number;
  status: "active" | "sold";
}

const statusBadge: Record<string, string> = {
  pending: "bg-amber-100 text-amber-800 border-amber-200",
  accepted: "bg-emerald-100 text-emerald-800 border-emerald-200",
  declined: "bg-zinc-100 text-zinc-500 border-zinc-200",
};

function Panel(props: Props) {
  const { listingId, sellerId, price } = props;
  const { userId } = useIdentity();
  const isSeller = userId === sellerId;
  const isSold = props.status === "sold";

  // The live query — every offer on this listing, newest first. When a buyer
  // in another tab makes an offer, it appears here instantly.
  const { data } = db.useQuery<Offer>("Offer", {
    where: { listingId },
    orderBy: { createdAt: "desc" },
  });
  const offers = data ?? [];
  const myOffer = offers.find((o) => o.buyerId === userId);

  if (isSeller) {
    return <SellerView offers={offers} title={props.title} />;
  }
  return (
    <BuyerView
      {...props}
      myOffer={myOffer}
      isSold={isSold}
      suggestedPrice={price}
    />
  );
}

function SellerView({ offers, title }: { offers: Offer[]; title: string }) {
  const [busy, setBusy] = useState<string | null>(null);
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
    try {
      await respondMutation.mutate({ offerId, accept });
    } catch (e) {
      console.error("respondToOffer failed", e);
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
      {offers.length === 0 ? (
        <p className="rounded-lg border border-dashed p-6 text-center text-sm text-muted-foreground">
          No offers yet. They'll show up here the moment a buyer makes one —
          live, no refresh.
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
                  <span className="text-lg font-semibold">
                    {money(o.amount)}
                  </span>
                  <span
                    className={`rounded-full border px-2 py-0.5 text-[10px] font-medium uppercase ${statusBadge[o.status]}`}
                  >
                    {o.status}
                  </span>
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
  const { userId, name } = useIdentity();
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

  // `myOffer` now includes the optimistic ghost, so this flips the instant
  // the offer is made.
  if (myOffer) {
    return (
      <div className="space-y-2 rounded-lg border bg-card p-4">
        <h2 className="font-semibold">Your offer</h2>
        <div className="flex items-center gap-2">
          <span className="text-2xl font-semibold">{money(myOffer.amount)}</span>
          <span
            className={`rounded-full border px-2 py-0.5 text-xs font-medium uppercase ${statusBadge[myOffer.status]}`}
          >
            {myOffer.status}
          </span>
        </div>
        <p className="text-sm text-muted-foreground">
          {myOffer.status === "pending"
            ? `Sent to ${sellerName} — you'll see their answer here live.`
            : myOffer.status === "accepted"
              ? "🎉 Accepted! Arrange pickup with the seller."
              : "This offer was declined."}
        </p>
      </div>
    );
  }

  if (isSold) {
    return (
      <div className="rounded-lg border bg-card p-4 text-sm text-muted-foreground">
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
    <form onSubmit={submit} className="space-y-3 rounded-lg border bg-card p-4">
      <h2 className="font-semibold">Make an offer</h2>
      <div className="flex items-center gap-2">
        <span className="text-muted-foreground">$</span>
        <Input
          type="number"
          min="1"
          step="1"
          value={amount}
          onChange={(e) => setAmount(e.target.value)}
          className="w-32"
          aria-label="Offer amount"
        />
      </div>
      <Textarea
        placeholder="Add a note (optional)…"
        value={message}
        onChange={(e) => setMessage(e.target.value)}
        rows={2}
      />
      {err ? <p className="text-sm text-destructive">{err}</p> : null}
      <Button type="submit" disabled={makeOffer.loading} className="w-full">
        {makeOffer.loading
          ? "Sending…"
          : `Offer ${money(Number.parseFloat(amount) || 0)}`}
      </Button>
      <p className="text-center text-xs text-muted-foreground">
        You're bidding as <span className="font-medium">{name}</span>
      </p>
    </form>
  );
}

export function OfferPanel(props: Props) {
  return (
    <MarketProvider>
      <Panel {...props} />
    </MarketProvider>
  );
}
