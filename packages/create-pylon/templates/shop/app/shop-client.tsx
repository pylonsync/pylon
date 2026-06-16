"use client";

import React, { useEffect, useMemo, useState } from "react";
import { db, callFn } from "@pylonsync/react";
import { EnsureGuest } from "@pylonsync/client";
import { siteConfig } from "@/lib/site.config";
import { formatPrice, type CheckoutResult } from "@/lib/shop";

// The storefront — the realtime heart of this template. The grid subscribes to
// the public Product entity with `db.useQuery`, so every card's stock ("3 left")
// ticks down — and flips to "Sold out" — for everyone with the page open the
// instant someone checks out. Shopping is a normal cart: add items, then check
// out (name + email). The `checkout` action holds stock under a per-product lock
// (so the cart can't oversell) and, when Stripe is configured, hands the browser
// off to a hosted Stripe Checkout page; otherwise it records a reserved order.
//
// Wrapped in <EnsureGuest> so the sync connection is established for anonymous
// visitors. The guest session holds no PII and can't read the Order table; the
// grid only ever reads the public catalog + stock.

interface ProductRow {
  id: string;
  slug: string;
  name: string;
  priceCents: number;
  description?: string | null;
  image: string;
  stock: number;
}

export function ShopGrid() {
  return (
    <EnsureGuest fallback={<GridSkeleton />}>
      <Shop />
    </EnsureGuest>
  );
}

function Shop() {
  const { data: products, loading } = db.useQuery<ProductRow>("Product");
  const [cart, setCart] = useState<Record<string, number>>({}); // slug → qty
  const [checkingOut, setCheckingOut] = useState(false);
  const [placed, setPlaced] = useState(false);

  // Seed the catalog from config on first visit (idempotent server-side).
  useEffect(() => {
    void callFn("seedProducts", {});
  }, []);

  const bySlug = useMemo(() => new Map(products.map((p) => [p.slug, p])), [products]);
  const ordered = useMemo(() => {
    const idx = new Map(siteConfig.products.items.map((p, i) => [p.slug, i]));
    return [...products].sort((a, b) => (idx.get(a.slug) ?? 99) - (idx.get(b.slug) ?? 99));
  }, [products]);

  function setQty(slug: string, qty: number) {
    const stock = bySlug.get(slug)?.stock ?? 0;
    const clamped = Math.max(0, Math.min(qty, stock));
    setCart((c) => {
      const next = { ...c };
      if (clamped <= 0) delete next[slug];
      else next[slug] = clamped;
      return next;
    });
  }

  // Cart entries that still reference a real product, clamped to live stock.
  const entries = Object.entries(cart)
    .map(([slug, qty]) => {
      const p = bySlug.get(slug);
      return p ? { product: p, qty: Math.min(qty, p.stock) } : null;
    })
    .filter((x): x is { product: ProductRow; qty: number } => !!x && x.qty > 0);
  const itemCount = entries.reduce((s, e) => s + e.qty, 0);
  const total = entries.reduce((s, e) => s + e.product.priceCents * e.qty, 0);

  if (loading && products.length === 0) return <GridSkeleton />;

  return (
    <>
      <div className="grid gap-5 sm:grid-cols-2 lg:grid-cols-3">
        {ordered.map((p) => (
          <ProductCard
            key={p.id}
            product={p}
            qty={cart[p.slug] ?? 0}
            onAdd={() => setQty(p.slug, (cart[p.slug] ?? 0) + 1)}
            onSet={(q) => setQty(p.slug, q)}
          />
        ))}
      </div>

      {/* Sticky cart bar */}
      {itemCount > 0 ? (
        <div className="pointer-events-none fixed inset-x-0 bottom-0 z-40 px-4 pb-4">
          <div className="pointer-events-auto mx-auto flex max-w-md items-center justify-between gap-4 rounded-full border border-zinc-200 bg-white px-5 py-3 shadow-[0_16px_48px_-16px_rgba(0,0,0,0.3)]">
            <span className="text-[14px] font-medium text-zinc-900">
              {itemCount} {itemCount === 1 ? "item" : "items"}
              <span className="ml-2 text-zinc-400">·</span>
              <span className="ml-2 tabular-nums">{formatPrice(total)}</span>
            </span>
            <button
              type="button"
              onClick={() => {
                setPlaced(false);
                setCheckingOut(true);
              }}
              className="inline-flex h-9 items-center rounded-full bg-brand px-5 text-[14px] font-medium text-white transition-opacity hover:opacity-90"
            >
              Checkout →
            </button>
          </div>
        </div>
      ) : null}

      {/* Checkout modal */}
      {checkingOut ? (
        <CheckoutModal
          entries={entries}
          total={total}
          placed={placed}
          onClose={() => setCheckingOut(false)}
          onSetQty={setQty}
          onPlaced={(succeededSlugs) => {
            setCart((c) => {
              const next = { ...c };
              succeededSlugs.forEach((s) => delete next[s]);
              return next;
            });
            setPlaced(true);
          }}
        />
      ) : null}
    </>
  );
}

function ProductCard({
  product,
  qty,
  onAdd,
  onSet,
}: {
  product: ProductRow;
  qty: number;
  onAdd: () => void;
  onSet: (q: number) => void;
}) {
  const soldOut = product.stock <= 0;
  return (
    <div className="flex flex-col overflow-hidden rounded-2xl border border-zinc-200 bg-white">
      <ProductImage image={product.image} soldOut={soldOut} />
      <div className="flex flex-1 flex-col p-5">
        <div className="flex items-start justify-between gap-3">
          <h3 className="text-[15px] font-semibold text-zinc-900">{product.name}</h3>
          <span className="shrink-0 text-[15px] font-semibold text-zinc-900">
            {formatPrice(product.priceCents)}
          </span>
        </div>
        {product.description ? (
          <p className="mt-1.5 text-[13px] leading-relaxed text-zinc-500">{product.description}</p>
        ) : null}

        <div className="mt-3">
          <StockBadge stock={product.stock} />
        </div>

        <div className="mt-4">
          {qty > 0 ? (
            <div className="flex items-center justify-between rounded-lg border border-zinc-200 px-1.5 py-1">
              <button
                type="button"
                onClick={() => onSet(qty - 1)}
                aria-label="Decrease quantity"
                className="flex size-7 items-center justify-center rounded-md text-zinc-600 hover:bg-zinc-100"
              >
                −
              </button>
              <span className="text-[14px] font-medium tabular-nums">{qty} in cart</span>
              <button
                type="button"
                onClick={onAdd}
                disabled={qty >= product.stock}
                aria-label="Increase quantity"
                className="flex size-7 items-center justify-center rounded-md text-zinc-600 hover:bg-zinc-100 disabled:opacity-30"
              >
                +
              </button>
            </div>
          ) : (
            <button
              type="button"
              disabled={soldOut}
              onClick={onAdd}
              className={
                "inline-flex h-10 w-full items-center justify-center rounded-lg text-sm font-medium transition-colors " +
                (soldOut
                  ? "cursor-not-allowed bg-zinc-100 text-zinc-400"
                  : "bg-brand text-white hover:opacity-90")
              }
            >
              {soldOut ? "Sold out" : "Add to cart"}
            </button>
          )}
        </div>
      </div>
    </div>
  );
}

function StockBadge({ stock }: { stock: number }) {
  if (stock <= 0) {
    return (
      <span className="inline-flex items-center gap-1.5 rounded-full bg-zinc-100 px-2.5 py-1 text-[12px] font-medium text-zinc-500">
        Sold out
      </span>
    );
  }
  const low = stock <= 3;
  return (
    <span
      className={
        "inline-flex items-center gap-1.5 rounded-full px-2.5 py-1 text-[12px] font-medium " +
        (low ? "bg-red-50 text-red-600" : "bg-green-50 text-green-700")
      }
    >
      <span className="relative flex size-1.5">
        <span className={"absolute inline-flex size-1.5 animate-ping rounded-full " + (low ? "bg-red-500/70" : "bg-green-500/60")} />
        <span className={"relative inline-flex size-1.5 rounded-full " + (low ? "bg-red-500" : "bg-green-600")} />
      </span>
      {low ? `Only ${stock} left` : `${stock} in stock`}
    </span>
  );
}

function CheckoutModal({
  entries,
  total,
  placed,
  onClose,
  onSetQty,
  onPlaced,
}: {
  entries: { product: ProductRow; qty: number }[];
  total: number;
  placed: boolean;
  onClose: () => void;
  onSetQty: (slug: string, qty: number) => void;
  onPlaced: (succeededSlugs: string[]) => void;
}) {
  const [name, setName] = useState("");
  const [email, setEmail] = useState("");
  const [status, setStatus] = useState<"idle" | "placing">("idle");
  const [error, setError] = useState<string | null>(null);
  const [soldOutNote, setSoldOutNote] = useState<string[]>([]);

  async function submitCheckout(e: React.FormEvent) {
    e.preventDefault();
    if (status === "placing") return;
    if (!name.trim() || !email.trim()) {
      setError("Name and email are required.");
      return;
    }
    setStatus("placing");
    setError(null);

    const origin = window.location.origin;
    let res: CheckoutResult;
    try {
      res = await callFn<CheckoutResult>("checkout", {
        items: entries.map((it) => ({ slug: it.product.slug, qty: it.qty })),
        customerName: name.trim(),
        customerEmail: email.trim(),
        successUrl: `${origin}/success`,
        cancelUrl: `${origin}/#shop`,
      });
    } catch (err) {
      setStatus("idle");
      setError(err instanceof Error ? err.message : "Checkout failed. Please try again.");
      return;
    }

    if (res.ok && res.mode === "stripe") {
      // Hand off to Stripe's hosted checkout — keep the button busy while the
      // browser navigates away. Stock is already held; the webhook settles it.
      window.location.href = res.checkoutUrl;
      return;
    }

    setStatus("idle");
    if (res.ok && res.mode === "reserved") {
      // No payment processor configured — the order is held for the owner.
      setSoldOutNote(res.soldOut);
      onPlaced(entries.map((it) => it.product.slug));
      return;
    }
    // res.ok === false
    if (res.reason === "sold_out") {
      setError(`Sold out before checkout: ${res.soldOut.join(", ") || "those items"}. Nothing was ordered.`);
    } else if (res.reason === "empty") {
      setError("Your cart is empty.");
    } else {
      setError("Could not place the order. Please check your details.");
    }
  }

  return (
    <div className="fixed inset-0 z-50 flex items-end justify-center bg-zinc-900/40 p-4 sm:items-center">
      <div className="w-full max-w-md rounded-2xl bg-white p-6 shadow-2xl">
        {placed && entries.length === 0 ? (
          <div className="text-center">
            <div className="mx-auto flex size-10 items-center justify-center rounded-full bg-brand text-white">✓</div>
            <p className="mt-3 text-[15px] font-semibold text-zinc-900">
              {siteConfig.checkout.confirmationMessage}
            </p>
            {soldOutNote.length > 0 ? (
              <p className="mt-2 text-[13px] text-amber-600">
                {soldOutNote.join(", ")} sold out before we could reserve {soldOutNote.length === 1 ? "it" : "them"}.
              </p>
            ) : null}
            <button
              type="button"
              onClick={onClose}
              className="mt-5 inline-flex h-10 items-center justify-center rounded-lg border border-zinc-300 px-5 text-sm font-medium text-zinc-700 hover:bg-zinc-50"
            >
              Keep shopping
            </button>
          </div>
        ) : (
          <>
            <div className="flex items-center justify-between">
              <h2 className="text-base font-semibold text-zinc-900">Your cart</h2>
              <button type="button" onClick={onClose} aria-label="Close" className="text-zinc-400 hover:text-zinc-700">
                ✕
              </button>
            </div>

            <ul className="mt-4 space-y-3">
              {entries.map(({ product, qty }) => (
                <li key={product.slug} className="flex items-center gap-3">
                  <span className="grid size-10 shrink-0 place-items-center rounded-lg bg-paper text-2xl">{product.image}</span>
                  <div className="min-w-0 flex-1">
                    <div className="truncate text-[14px] font-medium text-zinc-900">{product.name}</div>
                    <div className="text-[12.5px] text-zinc-500">{formatPrice(product.priceCents)} each</div>
                  </div>
                  <div className="flex items-center rounded-md border border-zinc-200">
                    <button type="button" onClick={() => onSetQty(product.slug, qty - 1)} className="px-2 py-1 text-zinc-500 hover:text-zinc-900">−</button>
                    <span className="min-w-5 text-center text-[13px] font-medium tabular-nums">{qty}</span>
                    <button type="button" onClick={() => onSetQty(product.slug, qty + 1)} disabled={qty >= product.stock} className="px-2 py-1 text-zinc-500 hover:text-zinc-900 disabled:opacity-30">+</button>
                  </div>
                  <span className="w-16 shrink-0 text-right text-[14px] font-medium tabular-nums">
                    {formatPrice(product.priceCents * qty)}
                  </span>
                </li>
              ))}
            </ul>

            {entries.length === 0 ? (
              <p className="mt-4 text-center text-sm text-zinc-500">Your cart is empty.</p>
            ) : (
              <form onSubmit={submitCheckout} className="mt-5 border-t border-zinc-100 pt-4">
                <div className="flex items-center justify-between text-[15px] font-semibold text-zinc-900">
                  <span>Total</span>
                  <span className="tabular-nums">{formatPrice(total)}</span>
                </div>
                <div className="mt-4 grid gap-2.5">
                  <input value={name} onChange={(e) => setName(e.target.value)} placeholder="Your name" aria-label="Your name" autoComplete="name" className={inputCls} />
                  <input type="email" value={email} onChange={(e) => setEmail(e.target.value)} placeholder="you@email.com" aria-label="Email" autoComplete="email" className={inputCls} />
                </div>
                {error ? <p className="mt-2 text-[13px] text-red-600">{error}</p> : null}
                <button
                  type="submit"
                  disabled={status === "placing"}
                  className="mt-4 inline-flex h-11 w-full items-center justify-center rounded-lg bg-brand text-sm font-medium text-white transition-opacity hover:opacity-90 disabled:opacity-60"
                >
                  {status === "placing" ? "Taking you to checkout…" : `Checkout · ${formatPrice(total)}`}
                </button>
                <p className="mt-2 text-center text-[12px] text-zinc-400">
                  Secure checkout. You&apos;ll confirm payment on the next step.
                </p>
              </form>
            )}
          </>
        )}
      </div>
    </div>
  );
}

const inputCls =
  "h-10 w-full rounded-lg border border-zinc-300 bg-white px-3 text-sm text-zinc-900 outline-none transition placeholder:text-zinc-400 focus:border-brand focus:ring-2 focus:ring-brand/20";

function GridSkeleton() {
  return (
    <div className="grid gap-5 sm:grid-cols-2 lg:grid-cols-3">
      {Array.from({ length: 6 }).map((_, i) => (
        <div key={i} className="overflow-hidden rounded-2xl border border-zinc-200 bg-white">
          <div className="aspect-[4/3] animate-pulse bg-zinc-100" />
          <div className="space-y-2 p-5">
            <div className="h-4 w-2/3 animate-pulse rounded bg-zinc-100" />
            <div className="h-3 w-full animate-pulse rounded bg-zinc-100" />
            <div className="h-10 w-full animate-pulse rounded-lg bg-zinc-100" />
          </div>
        </div>
      ))}
    </div>
  );
}
