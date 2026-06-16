"use client";

import React, { useEffect, useState } from "react";
import { db, callFn } from "@pylonsync/react";
import { useAuth } from "@pylonsync/client";
import { formatPrice, type OrderRow, type OwnerOrdersResult } from "@/lib/shop";

interface ProductRow {
  id: string;
  slug: string;
  name: string;
  stock: number;
  priceCents: number;
}

// The owner's live dashboard. Stock rides the public Product entity
// (`db.useQuery`), so the stock table updates live as orders land; that same
// change also triggers a re-fetch of the owner-gated `ordersForOwner`, so new
// orders appear without a refresh. Customer PII only travels through that gated
// call.
export function ShopDashboard({ userEmail }: { userEmail: string }) {
  const { data: products } = db.useQuery<ProductRow>("Product");
  // A signature of stock state — changes whenever any product's stock moves,
  // which is exactly when a new order landed or one was cancelled.
  const liveKey = products.reduce((s, p) => s + p.stock, 0) + ":" + products.length;

  const [orders, setOrders] = useState<OrderRow[] | null>(null);
  const [denied, setDenied] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [busyId, setBusyId] = useState<string | null>(null);

  async function load() {
    try {
      const r = await callFn<OwnerOrdersResult>("ordersForOwner", {});
      if (!r.authorized) setDenied(true);
      else {
        setOrders(r.orders);
        setDenied(false);
        setError(null);
      }
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    }
  }

  useEffect(() => {
    void load();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [liveKey]);

  async function orderAction(id: string, fn: "fulfillOrder" | "cancelOrder") {
    setBusyId(id);
    try {
      await callFn(fn, { orderId: id });
      await load();
    } finally {
      setBusyId(null);
    }
  }

  async function restock(slug: string) {
    setBusyId(slug);
    try {
      await callFn("restockProduct", { slug, add: 10 });
    } finally {
      setBusyId(null);
    }
  }

  if (denied) return <OwnerOnly email={userEmail} />;
  if (error) {
    return <div className="rounded-xl border border-red-200 bg-red-50 px-5 py-4 text-sm text-red-700">{error}</div>;
  }
  if (!orders) return <Skeleton />;

  const active = orders.filter((o) => o.status !== "cancelled");
  // A sale = paid / reserved / fulfilled. "pending" is an unpaid Stripe checkout
  // still in flight (stock held), so it doesn't count toward sold/revenue yet.
  const sales = active.filter((o) => o.status !== "pending");
  const toFulfill = sales.filter((o) => o.status === "paid" || o.status === "reserved").length;
  const unitsSold = sales.reduce((s, o) => s + o.qty, 0);
  const revenue = sales.reduce((s, o) => s + o.qty * o.unitPriceCents, 0);

  return (
    <div className="space-y-8">
      <div>
        <h1 className="text-xl font-semibold tracking-tight">Orders</h1>
        <p className="mt-1 text-sm text-zinc-500">
          Live — orders land here the moment they happen; cancelling returns the units to stock on
          the site instantly.
        </p>
      </div>

      <div className="grid gap-4 sm:grid-cols-3">
        <Stat label="To fulfill" value={String(toFulfill)} />
        <Stat label="Units sold" value={String(unitsSold)} />
        <Stat label="Revenue" value={formatPrice(revenue)} />
      </div>

      {/* Stock */}
      <div className="rounded-xl border border-zinc-200 bg-white">
        <div className="border-b border-zinc-100 px-4 py-3 text-sm font-semibold text-zinc-900">Stock</div>
        <ul className="divide-y divide-zinc-100">
          {products
            .slice()
            .sort((a, b) => a.name.localeCompare(b.name))
            .map((p) => (
              <li key={p.id} className="flex items-center justify-between gap-3 px-4 py-2.5">
                <span className="truncate text-sm text-zinc-800">{p.name}</span>
                <div className="flex items-center gap-3">
                  <span
                    className={
                      "text-[13px] font-medium tabular-nums " +
                      (p.stock <= 0 ? "text-zinc-400" : p.stock <= 3 ? "text-red-600" : "text-zinc-700")
                    }
                  >
                    {p.stock <= 0 ? "Sold out" : `${p.stock} in stock`}
                  </span>
                  <button
                    type="button"
                    disabled={busyId === p.slug}
                    onClick={() => restock(p.slug)}
                    className="rounded-md border border-zinc-300 px-2.5 py-1 text-[12px] font-medium text-zinc-600 transition-colors hover:bg-zinc-50 disabled:opacity-50"
                  >
                    +10
                  </button>
                </div>
              </li>
            ))}
        </ul>
      </div>

      {/* Orders */}
      <div className="rounded-xl border border-zinc-200 bg-white">
        <div className="border-b border-zinc-100 px-4 py-3 text-sm font-semibold text-zinc-900">
          Orders <span className="font-normal text-zinc-400">({active.length})</span>
        </div>
        {active.length === 0 ? (
          <p className="p-8 text-center text-sm text-zinc-500">No orders yet — share your shop.</p>
        ) : (
          <ul className="divide-y divide-zinc-100">
            {active.map((o) => (
              <li key={o.id} className="flex flex-wrap items-center gap-x-4 gap-y-2 px-4 py-3">
                <div className="min-w-0 flex-1">
                  <div className="flex items-center gap-2">
                    <span className="text-[14px] font-medium text-zinc-900">
                      {o.qty}× {o.productName}
                    </span>
                    <StatusBadge status={o.status} />
                  </div>
                  <div className="truncate text-[12.5px] text-zinc-500">
                    {o.customerName} · {o.customerEmail} · {formatPrice(o.qty * o.unitPriceCents)}
                  </div>
                </div>
                <div className="flex items-center gap-2">
                  {o.status === "reserved" || o.status === "paid" ? (
                    <button
                      type="button"
                      disabled={busyId === o.id}
                      onClick={() => orderAction(o.id, "fulfillOrder")}
                      className="rounded-md bg-zinc-900 px-3 py-1.5 text-[12.5px] font-medium text-white transition-colors hover:bg-zinc-700 disabled:opacity-50"
                    >
                      {busyId === o.id ? "…" : "Mark fulfilled"}
                    </button>
                  ) : null}
                  <button
                    type="button"
                    disabled={busyId === o.id}
                    onClick={() => orderAction(o.id, "cancelOrder")}
                    className="rounded-md border border-zinc-300 px-3 py-1.5 text-[12.5px] font-medium text-zinc-600 transition-colors hover:border-red-300 hover:text-red-600 disabled:opacity-50"
                  >
                    Cancel
                  </button>
                </div>
              </li>
            ))}
          </ul>
        )}
      </div>
    </div>
  );
}

function Stat({ label, value }: { label: string; value: string }) {
  return (
    <div className="rounded-xl border border-zinc-200 bg-white p-4">
      <div className="text-[11px] font-medium uppercase tracking-wide text-zinc-400">{label}</div>
      <div className="mt-1 text-2xl font-semibold tabular-nums text-zinc-900">{value}</div>
    </div>
  );
}

function StatusBadge({ status }: { status: string }) {
  const tone =
    status === "fulfilled"
      ? "bg-green-50 text-green-700"
      : status === "paid"
        ? "bg-emerald-50 text-emerald-700"
        : status === "pending"
          ? "bg-zinc-100 text-zinc-500"
          : status === "cancelled"
            ? "bg-zinc-100 text-zinc-400"
            : "bg-amber-50 text-amber-700"; // reserved
  const label = status === "pending" ? "awaiting payment" : status;
  return <span className={"rounded-full px-2 py-0.5 text-[10px] font-medium capitalize " + tone}>{label}</span>;
}

function OwnerOnly({ email }: { email: string }) {
  return (
    <div className="rounded-xl border border-dashed border-zinc-300 px-6 py-12 text-center">
      <h1 className="text-lg font-semibold">This dashboard is owner-only</h1>
      <p className="mx-auto mt-2 max-w-md text-sm text-zinc-500">
        You&apos;re signed in as <span className="font-medium text-zinc-700">{email || "this account"}</span>.
        Only the shop owner can see orders. Set{" "}
        <code className="rounded bg-zinc-100 px-1.5 py-0.5 text-[12px]">PYLON_OWNER_EMAIL={email || "you@yourshop.com"}</code>{" "}
        in your <code className="rounded bg-zinc-100 px-1.5 py-0.5 text-[12px]">.env</code>, restart, and reload —
        or sign in with the owner account.
      </p>
    </div>
  );
}

export function UserMenu({ email }: { email: string }) {
  const { signOut } = useAuth();
  const initial = (email.trim()[0] || "?").toUpperCase();
  async function onSignOut() {
    await signOut();
    window.location.assign("/");
  }
  return (
    <details className="group relative">
      <summary className="flex size-8 cursor-pointer select-none list-none items-center justify-center rounded-full bg-zinc-900 text-[12px] font-semibold text-white marker:hidden [&::-webkit-details-marker]:hidden">
        {initial}
      </summary>
      <div className="absolute right-0 top-full z-40 mt-2 w-56 overflow-hidden rounded-xl border border-zinc-200 bg-white py-1 shadow-[0_16px_48px_-16px_rgba(0,0,0,0.25)]">
        <div className="border-b border-zinc-100 px-3 py-2">
          <div className="truncate text-[13px] font-medium text-zinc-900">{email || "Signed in"}</div>
        </div>
        <button
          type="button"
          onClick={onSignOut}
          className="flex w-full items-center px-3 py-2 text-left text-[13px] text-zinc-700 transition-colors hover:bg-zinc-50"
        >
          Sign out
        </button>
      </div>
    </details>
  );
}

function Skeleton() {
  return (
    <div className="space-y-8">
      <div className="h-6 w-28 animate-pulse rounded bg-zinc-100" />
      <div className="grid gap-4 sm:grid-cols-3">
        {[0, 1, 2].map((i) => (
          <div key={i} className="h-20 animate-pulse rounded-xl bg-zinc-100" />
        ))}
      </div>
      <div className="h-40 animate-pulse rounded-xl bg-zinc-100" />
      <div className="h-48 animate-pulse rounded-xl bg-zinc-100" />
    </div>
  );
}
