"use client";

/**
 * Tiny client-side cart hook. Pylon's sync replica lives in
 * IndexedDB; this hook reads/writes `CartItem` rows and exposes a
 * stable surface for any cart UI piece (header badge, sheet, checkout).
 */
import { useCallback, useMemo } from "react";
import { db } from "@pylonsync/react";
import { useAuth } from "@/lib/pylon-client";
import type { CartItem, Product } from "@/lib/types";

export function useCart() {
  const { user } = useAuth();
  const userId = user?.id ?? "";

  const items = db.useQuery<CartItem>("CartItem", {
    where: userId ? { userId } : undefined,
    orderBy: { addedAt: "desc" },
  });
  const cart = db.useEntity("CartItem");

  const add = useCallback(
    (p: Product) => {
      if (!userId) return;
      const existing = items.data.find((i) => i.productId === p.id);
      if (existing) {
        cart.update(existing.id, { quantity: existing.quantity + 1 });
      } else {
        cart.insert({
          userId,
          productId: p.id,
          productName: p.name,
          productBrand: p.brand,
          productPrice: p.price,
          quantity: 1,
          addedAt: new Date().toISOString(),
        });
      }
    },
    [items.data, cart, userId],
  );

  const setQuantity = useCallback(
    (id: string, qty: number) => {
      if (qty <= 0) cart.remove(id);
      else cart.update(id, { quantity: qty });
    },
    [cart],
  );

  const remove = useCallback((id: string) => cart.remove(id), [cart]);
  const clear = useCallback(() => {
    items.data.forEach((i) => cart.remove(i.id));
  }, [items.data, cart]);

  const summary = useMemo(() => {
    const count = items.data.reduce((n, i) => n + i.quantity, 0);
    const total = items.data.reduce(
      (s, i) => s + i.productPrice * i.quantity,
      0,
    );
    return { count, total };
  }, [items.data]);

  return {
    items: items.data,
    loading: items.loading,
    count: summary.count,
    total: summary.total,
    add,
    setQuantity,
    remove,
    clear,
  };
}

export type UseCartReturn = ReturnType<typeof useCart>;
