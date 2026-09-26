"use client";

import React from "react";
import { ShoppingBag } from "lucide-react";
import { cart, countItems, useCart } from "@/lib/cart-store";

// The nav cart button. Reads the shared cart store so the count matches the
// grid, and opens the cart panel rendered by the shop island. The count
// re-mounts on change, which replays its pop-in animation.
export function CartButton() {
  const { lines } = useCart();
  const count = countItems(lines);
  return (
    <button
      type="button"
      onClick={cart.open}
      aria-label={count === 1 ? "Cart, 1 item" : `Cart, ${count} items`}
      className="inline-flex h-10 items-center gap-2 rounded-full pr-1 pl-3 text-[14px] text-ink transition-colors duration-300 hover:bg-ink/[0.05]"
    >
      <ShoppingBag className="size-[18px]" strokeWidth={1.25} />
      <span className="hidden sm:inline">Cart</span>
      <span
        key={count}
        className={
          "inline-flex h-6 min-w-6 items-center justify-center rounded-full px-1.5 text-[12px] font-medium tabular-nums transition-colors duration-300 " +
          (count > 0 ? "animate-in zoom-in-50 bg-ink text-warm-white duration-300" : "bg-ink/[0.07] text-ink/60")
        }
      >
        {count}
      </span>
    </button>
  );
}
