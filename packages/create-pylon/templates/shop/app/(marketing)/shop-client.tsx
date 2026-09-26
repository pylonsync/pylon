"use client";

import React, { useEffect, useMemo, useRef, useState } from "react";
import { db, callFn, useRouter } from "@pylonsync/react";
import { EnsureGuest } from "@pylonsync/client";
import { Minus, Plus, X } from "lucide-react";
import { siteConfig, productBySlug } from "@/lib/site.config";
import {
  formatPrice,
  freeShippingRemaining,
  shippingCents,
  stockStatus,
  LOW_STOCK_AT,
  type CheckoutResult,
} from "@/lib/shop";
import { cart, countItems, useCart, type CartLines } from "@/lib/cart-store";
import { publishShelf } from "@/lib/shelf-store";

// The storefront, and the realtime part of this template. The grid subscribes
// to the public Product entity with `db.useQuery`, so every card's stock
// ("Only 2 left") ticks down, and flips to "Sold out", for everyone with the
// page open when someone checks out. Shopping is a normal cart: add items, then
// check out (name + email). The `checkout` action holds stock under a
// per-product lock (so the cart can't oversell) and, when Stripe is configured,
// hands the browser off to a hosted Stripe Checkout page; otherwise it records
// a reserved order and the shopper lands on /success.
//
// Cart contents live in lib/cart-store.ts so the nav's cart button (a separate
// client island) shows the same count and can open the panel rendered here.
// Shelf totals go to lib/shelf-store.ts for the hero's live count.
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

type CartEntry = { product: ProductRow; qty: number };

export function ShopGrid() {
  return (
    <EnsureGuest fallback={<GridSkeleton />}>
      <Shop />
    </EnsureGuest>
  );
}

function Shop() {
  const { data: products, loading } = db.useQuery<ProductRow>("Product");
  const { lines, open } = useCart();

  // Seed the catalog from config on first visit (idempotent server-side).
  useEffect(() => {
    void callFn("seedProducts", {});
  }, []);

  useEffect(() => {
    if (products.length > 0) publishShelf(products.map((p) => p.stock), LOW_STOCK_AT);
  }, [products]);

  const bySlug = useMemo(() => new Map(products.map((p) => [p.slug, p])), [products]);
  const ordered = useMemo(() => {
    const idx = new Map(siteConfig.products.items.map((p, i) => [p.slug, i]));
    return [...products].sort((a, b) => (idx.get(a.slug) ?? 99) - (idx.get(b.slug) ?? 99));
  }, [products]);

  function setQty(slug: string, qty: number) {
    const stock = bySlug.get(slug)?.stock ?? 0;
    const clamped = Math.max(0, Math.min(qty, stock));
    cart.setLines((c: CartLines) => {
      const next = { ...c };
      if (clamped <= 0) delete next[slug];
      else next[slug] = clamped;
      return next;
    });
  }

  // Cart entries that still reference a real product, clamped to live stock.
  const entries = Object.entries(lines)
    .map(([slug, qty]) => {
      const p = bySlug.get(slug);
      return p ? { product: p, qty: Math.min(qty, p.stock) } : null;
    })
    .filter((x): x is CartEntry => !!x && x.qty > 0);

  if (loading && products.length === 0) return <GridSkeleton />;

  return (
    <>
      {/* The first product is featured: two columns wide on tablet, and two
          columns by two rows on desktop, beside a 2x2 block of the rest. */}
      <div className={GRID}>
        {ordered.map((p, i) => (
          <ProductCard
            key={p.id}
            featured={i === 0}
            product={p}
            qty={lines[p.slug] ?? 0}
            onAdd={() => setQty(p.slug, (lines[p.slug] ?? 0) + 1)}
            onSet={(q) => setQty(p.slug, q)}
          />
        ))}
      </div>

      {open ? <CartPanel entries={entries} onClose={cart.close} onSetQty={setQty} /> : null}
    </>
  );
}

const GRID = "grid gap-x-6 gap-y-14 sm:grid-cols-2 lg:grid-cols-4 lg:gap-x-8";
const FEATURED_CELL = "sm:col-span-2 lg:row-span-2";

function ProductCard({
  featured,
  product,
  qty,
  onAdd,
  onSet,
}: {
  featured?: boolean;
  product: ProductRow;
  qty: number;
  onAdd: () => void;
  onSet: (q: number) => void;
}) {
  const soldOut = product.stock <= 0;
  const size = productBySlug(product.slug)?.size;
  return (
    <article className={"group flex flex-col " + (featured ? FEATURED_CELL : "")}>
      <div
        className={
          "relative overflow-hidden bg-stone " +
          (featured ? "aspect-[4/5] sm:aspect-[5/4] lg:aspect-auto lg:min-h-[480px] lg:flex-1" : "aspect-[4/5]")
        }
      >
        <ProductPhoto
          image={product.image}
          name={product.name}
          className={
            "size-full object-cover transition-[transform,opacity,filter] duration-[1400ms] ease-out-quint group-hover:scale-[1.035] " +
            (soldOut ? "opacity-60 grayscale-[0.6]" : "")
          }
        />
        {soldOut ? (
          <span className="absolute top-4 left-4 rounded-full bg-warm-white/90 px-3 py-1 text-[12px] font-medium text-ink">
            Sold out
          </span>
        ) : null}
      </div>

      <div className="mt-5 flex items-baseline justify-between gap-4">
        <h3
          className={
            "font-display leading-tight tracking-[-0.01em] text-ink " +
            (featured ? "text-[2rem] sm:text-[2.25rem]" : "text-[1.375rem]")
          }
        >
          {product.name}
        </h3>
        <span className="shrink-0 text-[15px] font-medium tabular-nums text-ink">
          {formatPrice(product.priceCents)}
        </span>
      </div>
      {product.description ? (
        <p className="mt-1.5 max-w-[38ch] text-[14px] leading-relaxed text-ink-2">{product.description}</p>
      ) : null}
      {size ? <p className="mt-1 text-[13px] text-ink-2/75">{size}</p> : null}

      <div className="mt-auto flex min-h-15 items-center justify-between gap-4 pt-5">
        <StockLine stock={product.stock} />
        {qty > 0 ? (
          <Stepper
            qty={qty}
            max={product.stock}
            onDec={() => onSet(qty - 1)}
            onInc={onAdd}
            label={product.name}
          />
        ) : soldOut ? null : (
          <button
            type="button"
            onClick={onAdd}
            className="inline-flex h-10 shrink-0 items-center justify-center gap-1.5 rounded-full border border-ink/25 px-5 text-[13.5px] font-medium text-ink transition-[background-color,color,border-color,transform] duration-300 ease-out-quint hover:border-ink hover:bg-ink hover:text-warm-white active:scale-[0.97]"
          >
            <Plus className="size-3.5" strokeWidth={1.5} />
            Add to cart
          </button>
        )}
      </div>
    </article>
  );
}

// A product photo from lib/site.config.ts. If the file is missing, the frame
// shows the product name on the stone background instead of a broken image.
function ProductPhoto({ image, name, className }: { image: string; name: string; className?: string }) {
  const [failed, setFailed] = useState(false);
  if (failed || !image) {
    return (
      <div className="grid size-full place-items-center p-6 text-center">
        <span className="font-display text-[1.75rem] font-light text-ink/40">{name}</span>
      </div>
    );
  }
  // eslint-disable-next-line @next/next/no-img-element
  return <img src={image} alt={name} loading="lazy" onError={() => setFailed(true)} className={className} />;
}

// Stock status with a colored dot. When the live count changes, the line
// flashes once so the change is visible.
function StockLine({ stock }: { stock: number }) {
  const status = stockStatus(stock);
  const flash = useChangeCount(stock);
  const dot =
    status.kind === "sold_out" ? "bg-ink/25" : status.kind === "low" ? "bg-brand" : "bg-[#5d7a4a]";
  return (
    <p
      key={flash}
      className={
        "flex items-center gap-2 rounded-full px-1 -mx-1 text-[13px] tabular-nums " +
        (flash > 0 ? "stock-tick " : "") +
        (status.kind === "low" ? "font-medium text-brand" : "text-ink-2")
      }
      aria-live="polite"
    >
      <span className="relative flex size-1.5">
        {status.kind === "low" ? <span className={`ping-soft absolute inset-0 rounded-full ${dot}`} /> : null}
        <span className={`relative size-1.5 rounded-full ${dot}`} />
      </span>
      {status.label}
    </p>
  );
}

// Counts how many times `value` has changed since mount (0 on first render).
function useChangeCount(value: number) {
  const prev = useRef(value);
  const [count, setCount] = useState(0);
  useEffect(() => {
    if (prev.current !== value) {
      prev.current = value;
      setCount((c) => c + 1);
    }
  }, [value]);
  return count;
}

function Stepper({
  qty,
  max,
  onDec,
  onInc,
  label,
  small,
}: {
  qty: number;
  max: number;
  onDec: () => void;
  onInc: () => void;
  label: string;
  small?: boolean;
}) {
  const btn =
    "flex items-center justify-center rounded-full text-ink/70 transition-colors duration-200 hover:bg-ink/[0.06] hover:text-ink disabled:opacity-30 disabled:hover:bg-transparent " +
    (small ? "size-7" : "size-8");
  return (
    <div
      className={
        "inline-flex shrink-0 items-center rounded-full border border-ink/20 " + (small ? "h-9 px-0.5" : "h-10 px-1")
      }
    >
      <button type="button" onClick={onDec} aria-label={`Remove one ${label}`} className={btn}>
        <Minus className="size-3.5" strokeWidth={1.5} />
      </button>
      <span className="min-w-7 text-center text-[13.5px] font-medium tabular-nums text-ink" aria-label={`${qty} in cart`}>
        {qty}
      </span>
      <button type="button" onClick={onInc} disabled={qty >= max} aria-label={`Add one ${label}`} className={btn}>
        <Plus className="size-3.5" strokeWidth={1.5} />
      </button>
    </div>
  );
}

// The cart, as a panel sliding in from the right. Opened from the nav cart
// button (components/cart-button.tsx) through the shared store.
function CartPanel({
  entries,
  onClose,
  onSetQty,
}: {
  entries: CartEntry[];
  onClose: () => void;
  onSetQty: (slug: string, qty: number) => void;
}) {
  const router = useRouter();
  const [name, setName] = useState("");
  const [email, setEmail] = useState("");
  const [status, setStatus] = useState<"idle" | "placing">("idle");
  const [error, setError] = useState<string | null>(null);

  const terms = siteConfig.checkout.shipping;
  const subtotal = entries.reduce((s, e) => s + e.product.priceCents * e.qty, 0);
  const shipping = shippingCents(subtotal, terms);
  const remaining = freeShippingRemaining(subtotal, terms);
  const count = countItems(Object.fromEntries(entries.map((e) => [e.product.slug, e.qty])));

  useEffect(() => {
    function onKey(e: KeyboardEvent) {
      if (e.key === "Escape") onClose();
    }
    window.addEventListener("keydown", onKey);
    const overflow = document.body.style.overflow;
    document.body.style.overflow = "hidden";
    return () => {
      window.removeEventListener("keydown", onKey);
      document.body.style.overflow = overflow;
    };
  }, [onClose]);

  async function submitCheckout(e: React.FormEvent) {
    e.preventDefault();
    if (status === "placing") return;
    if (!name.trim() || !email.trim()) {
      setError("Enter your name and email.");
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
      setError(err instanceof Error ? err.message : "Checkout failed. Try again.");
      return;
    }

    if (res.ok && res.mode === "stripe") {
      // Hand off to Stripe's hosted checkout. The button stays busy while the
      // browser navigates away. Stock is already held; the webhook settles it.
      window.location.href = res.checkoutUrl;
      return;
    }

    if (res.ok && res.mode === "reserved") {
      // No payment processor configured: the order is held for the owner.
      // Empty the cart and show the confirmation page.
      const sold = res.soldOut;
      cart.setLines(() => ({}));
      cart.close();
      const qs = new URLSearchParams({ order: "reserved" });
      if (sold.length > 0) qs.set("sold", sold.join(","));
      router.push(`/success?${qs.toString()}`);
      return;
    }

    setStatus("idle");
    if (res.reason === "sold_out") {
      setError(`Sold out before checkout: ${res.soldOut.join(", ") || "those items"}. Nothing was ordered.`);
    } else if (res.reason === "empty") {
      setError("Your cart is empty.");
    } else {
      setError("Could not place the order. Check your details.");
    }
  }

  return (
    <div className="fixed inset-0 z-50 flex justify-end">
      <button
        type="button"
        aria-label="Close cart"
        onClick={onClose}
        className="animate-in fade-in absolute inset-0 bg-espresso/35 duration-500"
      />
      <aside
        role="dialog"
        aria-modal="true"
        aria-label="Cart"
        className="animate-in slide-in-from-right relative flex h-full w-full max-w-[460px] flex-col bg-warm-white shadow-[-40px_0_80px_-40px_rgba(34,29,24,0.35)] duration-500 ease-out-quint"
      >
        <div className="flex h-20 shrink-0 items-center justify-between px-6 sm:px-8">
          <h2 className="font-display text-[1.75rem] font-light tracking-[-0.01em] text-ink">
            Cart <span className="text-ink/40 tabular-nums">{count}</span>
          </h2>
          <button
            type="button"
            onClick={onClose}
            aria-label="Close"
            className="flex size-10 items-center justify-center rounded-full text-ink/70 transition-colors hover:bg-ink/[0.06] hover:text-ink"
          >
            <X className="size-5" strokeWidth={1.25} />
          </button>
        </div>

        {entries.length === 0 ? (
          <div className="flex flex-1 flex-col items-center justify-center px-8 text-center">
            <p className="font-display text-[1.5rem] font-light text-ink">Your cart is empty.</p>
            <button
              type="button"
              onClick={onClose}
              className="mt-6 inline-flex h-11 items-center rounded-full border border-ink/25 px-6 text-[14px] font-medium text-ink transition-colors hover:border-ink"
            >
              Back to the shelf
            </button>
          </div>
        ) : (
          <>
            <div className="px-6 pb-5 sm:px-8">
              <p className="text-[13px] text-ink-2">
                {remaining > 0 ? (
                  <>
                    Add <span className="font-medium tabular-nums text-ink">{formatPrice(remaining)}</span> for free
                    shipping.
                  </>
                ) : (
                  "Your order ships free."
                )}
              </p>
              <div className="mt-2.5 h-[2px] overflow-hidden rounded-full bg-ink/10">
                <div
                  className="h-full rounded-full bg-ink transition-[width] duration-700 ease-out-quint"
                  style={{ width: `${Math.min(100, (subtotal / terms.freeOverCents) * 100)}%` }}
                />
              </div>
            </div>

            <ul className="flex-1 divide-y divide-ink/10 overflow-y-auto border-t border-ink/10 px-6 sm:px-8">
              {entries.map(({ product, qty }) => (
                <li key={product.slug} className="flex gap-4 py-5">
                  <div className="h-[100px] w-20 shrink-0 overflow-hidden bg-stone">
                    <ProductPhoto image={product.image} name="" className="size-full object-cover" />
                  </div>
                  <div className="flex min-w-0 flex-1 flex-col">
                    <div className="flex items-baseline justify-between gap-3">
                      <span className="font-display truncate text-[1.1875rem] text-ink">{product.name}</span>
                      <span className="shrink-0 text-[14px] font-medium tabular-nums text-ink">
                        {formatPrice(product.priceCents * qty)}
                      </span>
                    </div>
                    <span className="mt-0.5 text-[12.5px] text-ink-2">
                      {formatPrice(product.priceCents)} each
                    </span>
                    <div className="mt-auto flex items-center justify-between pt-3">
                      <Stepper
                        small
                        qty={qty}
                        max={product.stock}
                        onDec={() => onSetQty(product.slug, qty - 1)}
                        onInc={() => onSetQty(product.slug, qty + 1)}
                        label={product.name}
                      />
                      <button
                        type="button"
                        onClick={() => onSetQty(product.slug, 0)}
                        className="text-[12.5px] text-ink-2 underline decoration-ink/20 underline-offset-4 transition-colors hover:text-ink hover:decoration-ink"
                      >
                        Remove
                      </button>
                    </div>
                  </div>
                </li>
              ))}
            </ul>

            <form onSubmit={submitCheckout} className="shrink-0 border-t border-ink/10 bg-stone/40 px-6 py-6 sm:px-8">
              <dl className="space-y-1.5 text-[14px]">
                <div className="flex justify-between text-ink-2">
                  <dt>Subtotal</dt>
                  <dd className="tabular-nums">{formatPrice(subtotal)}</dd>
                </div>
                <div className="flex justify-between text-ink-2">
                  <dt>Shipping</dt>
                  <dd className="tabular-nums">{shipping === 0 ? "Free" : formatPrice(shipping)}</dd>
                </div>
                <div className="flex justify-between pt-1.5 text-[16px] font-medium text-ink">
                  <dt>Total</dt>
                  <dd className="tabular-nums">{formatPrice(subtotal + shipping)}</dd>
                </div>
              </dl>
              <div className="mt-5 grid gap-2.5 sm:grid-cols-2">
                <label className="block">
                  <span className="sr-only">Name</span>
                  <input
                    value={name}
                    onChange={(e) => setName(e.target.value)}
                    placeholder="Name"
                    autoComplete="name"
                    className={inputCls}
                  />
                </label>
                <label className="block">
                  <span className="sr-only">Email</span>
                  <input
                    type="email"
                    value={email}
                    onChange={(e) => setEmail(e.target.value)}
                    placeholder="Email"
                    autoComplete="email"
                    className={inputCls}
                  />
                </label>
              </div>
              {error ? <p className="mt-3 text-[13px] text-[#a1301f]">{error}</p> : null}
              <button
                type="submit"
                disabled={status === "placing"}
                className="mt-4 inline-flex h-12 w-full items-center justify-center rounded-full bg-ink text-[14.5px] font-medium text-warm-white transition-[background-color,transform] duration-300 ease-out-quint hover:bg-brand active:scale-[0.99] disabled:opacity-60"
              >
                {status === "placing" ? "Placing order…" : `Check out · ${formatPrice(subtotal + shipping)}`}
              </button>
            </form>
          </>
        )}
      </aside>
    </div>
  );
}

const inputCls =
  "h-11 w-full rounded-full border border-ink/15 bg-warm-white px-4 text-[14px] text-ink outline-none transition-[border-color,box-shadow] duration-300 placeholder:text-ink-2/70 focus:border-ink/50 focus:ring-4 focus:ring-ink/[0.06]";

function GridSkeleton() {
  return (
    <div className={GRID}>
      {Array.from({ length: 5 }).map((_, i) => (
        <div key={i} className={"flex flex-col " + (i === 0 ? FEATURED_CELL : "")}>
          <div
            className={
              "animate-pulse bg-stone " +
              (i === 0 ? "aspect-[4/5] sm:aspect-[5/4] lg:aspect-auto lg:min-h-[480px] lg:flex-1" : "aspect-[4/5]")
            }
          />
          <div className="mt-5 flex justify-between">
            <div className="h-6 w-1/2 animate-pulse rounded bg-ink/[0.06]" />
            <div className="h-5 w-12 animate-pulse rounded bg-ink/[0.06]" />
          </div>
          <div className="mt-3 h-4 w-3/4 animate-pulse rounded bg-ink/[0.06]" />
          <div className="mt-6 flex justify-between">
            <div className="h-4 w-20 animate-pulse rounded bg-ink/[0.06]" />
            <div className="h-10 w-32 animate-pulse rounded-full bg-ink/[0.06]" />
          </div>
        </div>
      ))}
    </div>
  );
}
