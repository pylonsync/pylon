"use client";

import React, { useState } from "react";
import { callFn, db } from "@pylonsync/react";
import { Alert, AlertDescription, AlertTitle } from "@/components/ui/alert";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import {
  Card,
  CardContent,
  CardHeader,
  CardTitle,
} from "@/components/ui/card";
import {
  Empty,
  EmptyDescription,
  EmptyHeader,
  EmptyTitle,
} from "@/components/ui/empty";
import {
  Field,
  FieldGroup,
  FieldLabel,
  FieldSeparator,
} from "@/components/ui/field";
import { Input } from "@/components/ui/input";
import { Spinner } from "@/components/ui/spinner";
import { Textarea } from "@/components/ui/textarea";
import { AuthGate, MarketProvider, useIdentity } from "./MarketProvider";
import { money, timeAgo, type Offer } from "./market";

interface Props {
  listingId: string;
  sellerId: string;
  sellerName: string;
  title: string;
  price: number;
  status: "active" | "sold";
  initialOffers?: Offer[];
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

  // The live query — every offer on this listing, newest first. Reads are
  // public, so this runs for signed-out visitors too; it just lights up the
  // moment a buyer in another tab makes an offer.
  const { data } = db.useQuery<Offer>("Offer", {
    orderBy: { createdAt: "desc" },
  });
  const offers = Array.from(
    new Map(
      [...(props.initialOffers ?? []), ...(data ?? [])].map((offer) => [
        offer.id,
        offer,
      ]),
    ).values(),
  ).filter((offer) => offer.listingId === listingId);
  const isSold =
    props.status === "sold" || offers.some((offer) => offer.status === "accepted");
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
  const [confirming, setConfirming] = useState<string | null>(null);
  const [err, setErr] = useState<string | null>(null);
  const pending = offers.filter((o) => o.status === "pending");

  async function respond(offerId: string, accept: boolean) {
    setBusy(offerId);
    setErr(null);
    try {
      // Use the direct function transport for seller decisions. It remains
      // reliable across dev-server reconnects, while the live query applies
      // the atomic offer + listing updates as soon as the server broadcasts.
      await callFn("respondToOffer", { offerId, accept });
    } catch (e) {
      setErr((e as Error).message ?? "Could not respond to offer.");
    } finally {
      setBusy(null);
      setConfirming(null);
    }
  }

  return (
    <div className="flex flex-col gap-3">
      <div className="flex items-center justify-between">
        <h2 className="font-semibold">Offers on your listing</h2>
        <Badge variant="outline">{pending.length} pending</Badge>
      </div>
      <div aria-live="polite">
        {err ? (
          <Alert variant="destructive">
            <AlertDescription>{err}</AlertDescription>
          </Alert>
        ) : null}
      </div>
      {offers.length === 0 ? (
        <Empty className="border-0 bg-muted py-6">
          <EmptyHeader>
            <EmptyTitle className="text-base">No offers yet</EmptyTitle>
            <EmptyDescription>
              They will appear here as soon as a buyer sends one.
            </EmptyDescription>
          </EmptyHeader>
        </Empty>
      ) : (
        <ul className="flex flex-col gap-2">
          {offers.map((o) => (
            <li key={o.id}>
            <Card className="flex items-center justify-between gap-3 rounded-lg p-3">
              <div className="min-w-0">
                <div className="flex items-center gap-2">
                  <span className="text-lg font-semibold tabular-nums">
                    {money(o.amount)}
                  </span>
                  <Badge
                    variant={statusVariant[o.status] ?? "outline"}
                    className="capitalize"
                  >
                    {o.status}
                  </Badge>
                </div>
                <p className="truncate text-sm text-muted-foreground">
                  {o.buyerName} · {timeAgo(o.createdAt)}
                  {o.message ? ` · “${o.message}”` : ""}
                </p>
              </div>
              {o.status === "pending" ? (
                <div className="flex shrink-0 gap-2">
                  {confirming === o.id ? (
                    <>
                      <Button
                        size="sm"
                        disabled={busy === o.id}
                        onClick={() => respond(o.id, true)}
                      >
                        Confirm
                      </Button>
                      <Button
                        size="sm"
                        variant="ghost"
                        disabled={busy === o.id}
                        onClick={() => setConfirming(null)}
                      >
                        Cancel
                      </Button>
                    </>
                  ) : (
                    <Button
                      size="sm"
                      disabled={busy === o.id}
                      onClick={() => setConfirming(o.id)}
                    >
                      Accept
                    </Button>
                  )}
                  {confirming !== o.id ? (
                    <Button
                      size="sm"
                      variant="outline"
                      disabled={busy === o.id}
                      onClick={() => respond(o.id, false)}
                    >
                      Decline
                    </Button>
                  ) : null}
                </div>
              ) : null}
            </Card>
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
  const [confirmingBuy, setConfirmingBuy] = useState(false);
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
      <Card>
        <CardHeader className="pb-3">
          <CardTitle>Your offer</CardTitle>
        </CardHeader>
        <CardContent className="flex flex-col gap-2">
          <div className="flex items-center gap-2">
            <span className="text-2xl font-semibold tabular-nums">
              {money(myOffer.amount)}
            </span>
            <Badge
              variant={statusVariant[myOffer.status] ?? "outline"}
              className="capitalize"
            >
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
        </CardContent>
      </Card>
    );
  }

  if (isSold) {
    return (
      <Alert>
        <AlertTitle>This item has sold</AlertTitle>
        <AlertDescription>
          Browse other finds to discover something similar.
        </AlertDescription>
      </Alert>
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
    <Card>
      <CardContent className="flex flex-col gap-4 p-5">
      <div className="flex flex-col gap-2">
        {confirmingBuy ? (
          <Alert>
            <AlertTitle>
              Buy this item for {money(suggestedPrice)}?
            </AlertTitle>
            <AlertDescription>
              This accepts the asking price and marks the listing sold.
            </AlertDescription>
            <div className="mt-3 flex gap-2">
              <Button
                type="button"
                size="sm"
                onClick={buy}
                disabled={buyNow.loading}
                className="flex-1"
              >
                {buyNow.loading ? <Spinner data-icon="inline-start" /> : null}
                {buyNow.loading ? "Buying…" : "Confirm purchase"}
              </Button>
              <Button
                type="button"
                size="sm"
                variant="outline"
                onClick={() => setConfirmingBuy(false)}
                disabled={buyNow.loading}
              >
                Cancel
              </Button>
            </div>
          </Alert>
        ) : (
          <>
            <Button
              type="button"
              onClick={() => setConfirmingBuy(true)}
              className="w-full"
            >
              Buy now for {money(suggestedPrice)}
            </Button>
            <p className="text-center text-xs text-muted-foreground">
              Instant purchase at the asking price.
            </p>
          </>
        )}
      </div>

      <FieldSeparator>or make an offer</FieldSeparator>

      <form onSubmit={submit}>
        <FieldGroup className="gap-3">
          <Field>
            <FieldLabel htmlFor="offer-amount">Your offer ($)</FieldLabel>
            <Input
              id="offer-amount"
              name="offerAmount"
              type="number"
              inputMode="decimal"
              autoComplete="off"
              min="1"
              step="1"
              value={amount}
              onChange={(e) => setAmount(e.target.value)}
              className="max-w-32"
            />
          </Field>
          <Field>
            <FieldLabel htmlFor="offer-note">
            Note <span className="font-normal text-muted-foreground">(optional)</span>
            </FieldLabel>
          <Textarea
            id="offer-note"
            name="offerNote"
            autoComplete="off"
            placeholder="Share any useful details…"
            value={message}
            onChange={(e) => setMessage(e.target.value)}
            rows={2}
          />
          </Field>
        <div aria-live="polite">
          {err ? (
            <Alert variant="destructive">
              <AlertDescription>{err}</AlertDescription>
            </Alert>
          ) : null}
        </div>
        <Button
          type="submit"
          variant="outline"
          disabled={makeOffer.loading}
          className="w-full"
        >
          {makeOffer.loading ? <Spinner data-icon="inline-start" /> : null}
          {makeOffer.loading
            ? "Sending…"
            : `Offer ${money(Number.parseFloat(amount) || 0)}`}
        </Button>
        <p className="text-center text-xs text-muted-foreground">
          You're bidding as <span className="font-medium">{name}</span>
        </p>
        </FieldGroup>
      </form>
      </CardContent>
    </Card>
  );
}

export function OfferPanel(props: Props) {
  return (
    <MarketProvider>
      <Panel {...props} />
    </MarketProvider>
  );
}
