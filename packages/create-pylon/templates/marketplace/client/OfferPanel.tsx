"use client";

import React, { useRef, useState } from "react";
import { callFn, db } from "@pylonsync/react";
import { Check, X } from "lucide-react";
import { cn } from "@/lib/utils";
import { Spinner } from "@/components/ui/spinner";
import { percentOfAsk, suggestedOffers } from "../lib/catalog";
import { LoginCard } from "./LoginCard";
import { MarketProvider, useIdentity } from "./MarketProvider";
import { avatarColor, initials, money, timeAgo, type Offer } from "./market";

interface Props {
  listingId: string;
  sellerId: string;
  sellerName: string;
  title: string;
  price: number;
  status: "active" | "sold";
  initialOffers?: Offer[];
}

// Buy box + live offers for one listing. One live query (every offer on this
// listing) drives all of it: the buyer's own offer status, the public offer
// list, and the seller's accept/decline queue. An offer sent from any tab
// shows up here without a refresh.
function Panel(props: Props) {
  const { listingId, sellerId } = props;
  const identity = useIdentity();
  const isSeller = !!identity && identity.userId === sellerId;

  const { data, loading } = db.useQuery<Offer>("Offer", {
    where: { listingId },
    orderBy: { createdAt: "desc" },
  });
  const offers =
    loading && (data ?? []).length === 0
      ? [...(props.initialOffers ?? [])].sort((a, b) =>
          b.createdAt.localeCompare(a.createdAt),
        )
      : (data ?? []);
  const isSold =
    props.status === "sold" || offers.some((offer) => offer.status === "accepted");

  if (isSeller) {
    return <SellerView offers={offers} price={props.price} />;
  }

  const myOffer = identity
    ? offers.find((o) => o.buyerId === identity.userId)
    : undefined;

  return (
    <div className="flex flex-col gap-8">
      <BuyBox {...props} myOffer={myOffer} isSold={isSold} />
      <OfferHistory offers={offers} price={props.price} />
    </div>
  );
}

// Offer keys seen on the first settled render; later keys arrived live.
function useArrivals(offers: Offer[]): (id: string) => boolean {
  const seen = useRef<Set<string> | null>(null);
  if (seen.current === null) seen.current = new Set(offers.map((o) => o.id));
  return (id) => !seen.current!.has(id);
}

function StatusTag({ status }: { status: Offer["status"] }) {
  return (
    <span
      className={cn(
        "rounded-full px-2 py-0.5 text-[11px] font-semibold capitalize",
        status === "pending" && "bg-signal/12 text-signal-foreground",
        status === "accepted" && "bg-success/14 text-success-foreground",
        status === "declined" && "bg-muted text-muted-foreground",
      )}
    >
      {status}
    </span>
  );
}

function Avatar({ name, className }: { name: string; className?: string }) {
  return (
    <span
      aria-hidden="true"
      className={cn(
        "grid size-9 shrink-0 place-items-center rounded-full text-xs font-semibold text-white",
        className,
      )}
      style={{ background: avatarColor(name) }}
    >
      {initials(name)}
    </span>
  );
}

function OfferHistory({ offers, price }: { offers: Offer[]; price: number }) {
  const isNew = useArrivals(offers);
  const pending = offers.filter((o) => o.status === "pending").length;

  return (
    <section aria-labelledby="offers-heading">
      <div className="flex items-baseline justify-between">
        <h2 id="offers-heading" className="text-base font-semibold">
          Offers
        </h2>
        <p className="text-[13px] text-muted-foreground">
          {pending} open · {offers.length} total
        </p>
      </div>
      {offers.length === 0 ? (
        <p className="mt-3 rounded-2xl bg-muted/60 px-4 py-5 text-sm text-muted-foreground">
          No offers yet. New offers appear here as buyers send them.
        </p>
      ) : (
        <ol aria-live="polite" className="mt-3 flex flex-col">
          {offers.slice(0, 6).map((o) => (
            <li
              key={o.id}
              className={cn(
                "flex items-center gap-3 rounded-xl border-b border-border/60 px-2 py-2.5 last:border-b-0",
                isNew(o.id) && "market-rise market-flash",
              )}
            >
              <Avatar name={o.buyerName} />
              <div className="min-w-0 flex-1">
                <p className="truncate text-sm">
                  <span className="font-medium">{o.buyerName}</span>{" "}
                  <span className="text-muted-foreground">
                    {o.message === "Bought at list price" ? "bought it" : "offered"}
                  </span>
                </p>
                <p className="text-xs text-muted-foreground">{timeAgo(o.createdAt)}</p>
              </div>
              <div className="text-right">
                <p className="text-sm font-semibold tabular-nums">{money(o.amount)}</p>
                <p className="text-[11px] tabular-nums text-muted-foreground">
                  {percentOfAsk(o.amount, price)}% of ask
                </p>
              </div>
              <StatusTag status={o.status} />
            </li>
          ))}
        </ol>
      )}
    </section>
  );
}

function SellerView({ offers, price }: { offers: Offer[]; price: number }) {
  const [busy, setBusy] = useState<string | null>(null);
  const [confirming, setConfirming] = useState<string | null>(null);
  const [err, setErr] = useState<string | null>(null);
  const isNew = useArrivals(offers);
  const pending = offers.filter((o) => o.status === "pending");

  async function respond(offerId: string, accept: boolean) {
    setBusy(offerId);
    setErr(null);
    try {
      // The direct function call stays reliable across dev-server
      // reconnects; the live query applies the offer + listing updates when
      // the server broadcasts them.
      await callFn("respondToOffer", { offerId, accept });
    } catch (e) {
      setErr((e as Error).message ?? "Could not answer the offer.");
    } finally {
      setBusy(null);
      setConfirming(null);
    }
  }

  return (
    <section
      aria-labelledby="seller-offers-heading"
      className="rounded-2xl bg-card p-4 shadow-[var(--shadow-border)] sm:p-5"
    >
      <div className="flex items-baseline justify-between">
        <h2 id="seller-offers-heading" className="text-base font-semibold">
          Offers on your listing
        </h2>
        <span className="text-[13px] text-muted-foreground">
          {pending.length} waiting
        </span>
      </div>
      <div aria-live="polite">
        {err ? (
          <p className="mt-3 rounded-xl bg-destructive/10 px-3 py-2 text-sm text-destructive">
            {err}
          </p>
        ) : null}
      </div>
      {offers.length === 0 ? (
        <p className="mt-3 text-sm text-muted-foreground">
          No offers yet. They appear here the moment a buyer sends one.
        </p>
      ) : (
        <ul className="mt-3 flex flex-col gap-2">
          {offers.map((o) => (
            <li
              key={o.id}
              className={cn(
                "flex flex-wrap items-center gap-3 rounded-xl bg-background/60 p-3 ring-1 ring-border/60",
                isNew(o.id) && "market-rise market-flash",
              )}
            >
              <Avatar name={o.buyerName} />
              <div className="min-w-0 flex-1">
                <p className="flex items-center gap-2">
                  <span className="text-lg font-semibold tabular-nums">
                    {money(o.amount)}
                  </span>
                  <span className="text-xs tabular-nums text-muted-foreground">
                    {percentOfAsk(o.amount, price)}% of ask
                  </span>
                </p>
                <p className="truncate text-[13px] text-muted-foreground">
                  {o.buyerName} · {timeAgo(o.createdAt)}
                  {o.message ? ` · “${o.message}”` : ""}
                </p>
              </div>
              {o.status === "pending" ? (
                <div className="flex shrink-0 gap-2">
                  {confirming === o.id ? (
                    <>
                      <button
                        type="button"
                        disabled={busy === o.id}
                        onClick={() => respond(o.id, true)}
                        className="inline-flex min-h-9 items-center gap-1.5 rounded-full bg-primary px-3.5 text-[13px] font-medium text-primary-foreground disabled:opacity-60"
                      >
                        {busy === o.id ? <Spinner /> : <Check className="size-3.5" />}
                        Sell for {money(o.amount)}
                      </button>
                      <button
                        type="button"
                        disabled={busy === o.id}
                        onClick={() => setConfirming(null)}
                        className="inline-flex min-h-9 items-center rounded-full px-3 text-[13px] font-medium text-muted-foreground hover:text-foreground"
                      >
                        Cancel
                      </button>
                    </>
                  ) : (
                    <>
                      <button
                        type="button"
                        disabled={busy === o.id}
                        onClick={() => setConfirming(o.id)}
                        className="inline-flex min-h-9 items-center rounded-full bg-primary px-3.5 text-[13px] font-medium text-primary-foreground transition-[scale] active:scale-[0.97] disabled:opacity-60"
                      >
                        Accept
                      </button>
                      <button
                        type="button"
                        disabled={busy === o.id}
                        onClick={() => respond(o.id, false)}
                        aria-label={`Decline ${money(o.amount)} offer from ${o.buyerName}`}
                        className="inline-flex min-h-9 items-center gap-1 rounded-full bg-card px-3 text-[13px] font-medium shadow-[var(--shadow-border)] transition-[scale] active:scale-[0.97] disabled:opacity-60"
                      >
                        <X className="size-3.5" />
                        Decline
                      </button>
                    </>
                  )}
                </div>
              ) : (
                <StatusTag status={o.status} />
              )}
            </li>
          ))}
        </ul>
      )}
    </section>
  );
}

function BuyBox({
  listingId,
  title,
  sellerId,
  sellerName,
  price,
  myOffer,
  isSold,
}: Props & { myOffer?: Offer; isSold: boolean }) {
  const identity = useIdentity();
  const userId = identity?.userId ?? "";
  const name = identity?.name ?? "you";
  const [mode, setMode] = useState<"idle" | "offer" | "buy" | "signin">("idle");
  const [amount, setAmount] = useState(String(suggestedOffers(price)[1] ?? price));
  const [message, setMessage] = useState("");
  const [err, setErr] = useState<string | null>(null);

  // db.useMutation paints the Offer into the local store the moment the
  // buyer submits (the `optimistic` row), so "Your offer" renders before the
  // server answers. makeOffer reuses the same id (threaded as
  // _optimisticId), so the server row replaces it in place; on failure the
  // engine rolls it back.
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

  // Buy now: the optimistic row is an accepted offer at the asking price;
  // the server marks the listing sold and declines the other offers.
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
          amount: price,
          message: "Bought at list price",
          status: "accepted",
          createdAt: ctx.now,
        },
      }),
    },
  );

  function start(next: "offer" | "buy") {
    setErr(null);
    setMode(identity ? next : "signin");
  }

  async function buy() {
    setErr(null);
    try {
      await buyNow.mutate({ listingId, buyerName: name });
    } catch (e) {
      setErr((e as Error).message ?? "Could not complete the purchase.");
    }
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
      await makeOffer.mutate({ listingId, amount: value, message, buyerName: name });
      setMode("idle");
    } catch (e) {
      setErr((e as Error).message ?? "Could not send the offer.");
    }
  }

  if (myOffer && !(myOffer.status === "declined" && mode === "offer")) {
    const bought = myOffer.message === "Bought at list price";
    return (
      <div className="rounded-2xl bg-card p-5 shadow-[var(--shadow-border)]">
        <div className="flex items-center justify-between gap-3">
          <p className="text-sm font-medium">
            {bought ? "You bought this item" : "Your offer"}
          </p>
          <StatusTag status={myOffer.status} />
        </div>
        <p className="mt-2 text-[32px] font-semibold leading-none tabular-nums tracking-[-0.02em]">
          {money(myOffer.amount)}
        </p>
        <p className="mt-3 text-sm text-muted-foreground">
          {myOffer.status === "pending"
            ? `Sent to ${sellerName}. Their answer appears here as soon as they respond.`
            : myOffer.status === "accepted"
              ? `${bought ? "Purchase confirmed." : `${sellerName} accepted.`} Arrange payment and pickup with the seller.`
              : `${sellerName} declined this offer.`}
        </p>
        {myOffer.status === "declined" && !isSold ? (
          <button
            type="button"
            onClick={() => setMode("offer")}
            className="mt-4 inline-flex min-h-10 items-center rounded-full bg-card px-4 text-sm font-medium shadow-[var(--shadow-border)]"
          >
            Make a new offer
          </button>
        ) : null}
      </div>
    );
  }

  if (isSold) {
    return (
      <div className="rounded-2xl bg-card p-5 shadow-[var(--shadow-border)]">
        <p className="font-display text-2xl">This item sold</p>
        <p className="mt-1 text-sm text-muted-foreground">
          The seller accepted an offer. Other listings are on the home page.
        </p>
      </div>
    );
  }

  if (mode === "signin") {
    return (
      <LoginCard
        title="Sign in to buy or make an offer"
        blurb="The demo account is filled in. Select Log in to continue."
        onDone={() => setMode("idle")}
        className="max-w-none"
      />
    );
  }

  const suggestions = suggestedOffers(price);
  const offerValue = Number.parseFloat(amount) || 0;

  return (
    <div className="rounded-2xl bg-card p-4 shadow-[var(--shadow-border)] sm:p-5">
      {mode === "buy" ? (
        <div className="market-rise">
          <p className="text-sm font-medium">Buy {title} for {money(price)}?</p>
          <p className="mt-1 text-sm text-muted-foreground">
            This pays the asking price and marks the listing sold.
          </p>
          <div className="mt-4 flex gap-2">
            <button
              type="button"
              onClick={buy}
              disabled={buyNow.loading}
              className="inline-flex min-h-12 flex-1 items-center justify-center gap-2 rounded-full bg-primary px-5 text-[15px] font-medium text-primary-foreground transition-[scale] active:scale-[0.98] disabled:opacity-60"
            >
              {buyNow.loading ? <Spinner /> : null}
              {buyNow.loading ? "Buying" : `Confirm purchase`}
            </button>
            <button
              type="button"
              onClick={() => setMode("idle")}
              disabled={buyNow.loading}
              className="inline-flex min-h-12 items-center rounded-full px-5 text-sm font-medium text-muted-foreground hover:text-foreground"
            >
              Cancel
            </button>
          </div>
        </div>
      ) : (
        <div className="flex flex-col gap-2 sm:flex-row">
          <button
            type="button"
            onClick={() => start("buy")}
            className="inline-flex min-h-12 flex-1 items-center justify-center rounded-full bg-primary px-5 text-[15px] font-medium text-primary-foreground transition-[background-color,scale] duration-150 hover:bg-primary/88 active:scale-[0.98]"
          >
            Buy now
          </button>
          <button
            type="button"
            onClick={() => start("offer")}
            aria-expanded={mode === "offer"}
            className={cn(
              "inline-flex min-h-12 flex-1 items-center justify-center rounded-full px-5 text-[15px] font-medium transition-[background-color,box-shadow,scale] duration-150 active:scale-[0.98]",
              mode === "offer"
                ? "bg-muted"
                : "bg-card shadow-[var(--shadow-border)] hover:shadow-[var(--shadow-border-hover)]",
            )}
          >
            Make an offer
          </button>
        </div>
      )}

      {mode === "offer" ? (
        <form onSubmit={submit} className="market-rise mt-5 flex flex-col gap-4">
          <div>
            <label htmlFor="offer-amount" className="text-sm font-medium">
              Your offer
            </label>
            <div className="mt-2 flex items-center rounded-xl bg-background px-4 ring-1 ring-input focus-within:ring-2 focus-within:ring-ring">
              <span className="text-2xl font-semibold text-muted-foreground">$</span>
              <input
                id="offer-amount"
                name="offerAmount"
                type="number"
                inputMode="decimal"
                autoComplete="off"
                min="1"
                step="1"
                value={amount}
                onChange={(e) => setAmount(e.target.value)}
                className="h-14 w-full bg-transparent pl-1 text-2xl font-semibold tabular-nums outline-none focus-visible:outline-none"
              />
              <span className="shrink-0 text-xs tabular-nums text-muted-foreground">
                {percentOfAsk(offerValue, price)}% of {money(price)}
              </span>
            </div>
            {suggestions.length > 0 ? (
              <div className="mt-2 flex flex-wrap gap-1.5">
                {suggestions.map((value) => (
                  <button
                    key={value}
                    type="button"
                    onClick={() => setAmount(String(value))}
                    aria-pressed={offerValue === value}
                    className={cn(
                      "inline-flex min-h-8 items-center rounded-full px-3 text-[13px] font-medium tabular-nums transition-colors",
                      offerValue === value
                        ? "bg-foreground text-background"
                        : "bg-muted text-foreground hover:bg-accent",
                    )}
                  >
                    {money(value)}
                  </button>
                ))}
              </div>
            ) : null}
          </div>
          <div>
            <label htmlFor="offer-note" className="text-sm font-medium">
              Note to {sellerName}{" "}
              <span className="font-normal text-muted-foreground">(optional)</span>
            </label>
            <textarea
              id="offer-note"
              name="offerNote"
              autoComplete="off"
              rows={2}
              value={message}
              onChange={(e) => setMessage(e.target.value)}
              placeholder="Pickup time, questions about condition"
              className="mt-2 w-full resize-none rounded-xl bg-background px-4 py-3 text-sm ring-1 ring-input outline-none placeholder:text-muted-foreground focus:ring-2 focus:ring-ring focus-visible:outline-none"
            />
          </div>
          <button
            type="submit"
            disabled={makeOffer.loading}
            className="inline-flex min-h-12 items-center justify-center gap-2 rounded-full bg-signal px-5 text-[15px] font-medium text-white transition-[filter,scale] duration-150 hover:brightness-95 active:scale-[0.98] disabled:opacity-60"
          >
            {makeOffer.loading ? <Spinner /> : null}
            {makeOffer.loading ? "Sending" : `Send offer of ${money(offerValue)}`}
          </button>
          <p className="text-center text-xs text-muted-foreground">
            Sending as <span className="font-medium text-foreground">{name}</span>.
            The seller can accept or decline.
          </p>
        </form>
      ) : null}

      <div aria-live="polite">
        {err ? (
          <p className="mt-3 rounded-xl bg-destructive/10 px-3 py-2 text-sm text-destructive">
            {err}
          </p>
        ) : null}
      </div>
    </div>
  );
}

export function OfferPanel(props: Props) {
  return (
    <MarketProvider
      fallback={
        <div className="flex flex-col gap-8">
          <div className="rounded-2xl bg-card p-4 shadow-[var(--shadow-border)] sm:p-5">
            <div className="flex flex-col gap-2 sm:flex-row">
              <div className="h-12 flex-1 rounded-full bg-muted" />
              <div className="h-12 flex-1 rounded-full bg-muted" />
            </div>
          </div>
          <StaticHistory {...props} />
        </div>
      }
    >
      <Panel {...props} />
    </MarketProvider>
  );
}

// Server-rendered offer list shown until the sync engine connects.
function StaticHistory({ initialOffers = [], price }: Props) {
  const offers = [...initialOffers].sort((a, b) =>
    b.createdAt.localeCompare(a.createdAt),
  );
  return <OfferHistory offers={offers} price={price} />;
}
