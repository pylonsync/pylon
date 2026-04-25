"use client";

import Link from "next/link";
import { Minus, Plus, Trash2 } from "lucide-react";
import { Button } from "@pylonsync/example-ui/button";
import { Separator } from "@pylonsync/example-ui/separator";
import {
  Sheet,
  SheetClose,
  SheetContent,
  SheetDescription,
  SheetFooter,
  SheetHeader,
  SheetTitle,
} from "@pylonsync/example-ui/sheet";
import { gradient, initials } from "@/lib/util";
import { useCart } from "./cart-store";

export function CartSheet({
  open,
  onClose,
}: {
  open: boolean;
  onClose: () => void;
}) {
  const cart = useCart();

  return (
    <Sheet open={open} onOpenChange={(o) => !o && onClose()}>
      <SheetContent side="right">
        <SheetHeader>
          <SheetTitle>Your cart</SheetTitle>
          <SheetDescription>
            {cart.count === 0
              ? "Your cart is empty."
              : `${cart.count} item${cart.count === 1 ? "" : "s"}, syncing live across tabs.`}
          </SheetDescription>
        </SheetHeader>

        {cart.items.length === 0 ? (
          <div className="px-6 py-12 text-center text-sm text-muted-foreground">
            Add a product from the catalog to start a cart.
          </div>
        ) : (
          <ul className="flex-1 overflow-y-auto px-6 py-4">
            {cart.items.map((it) => (
              <li
                key={it.id}
                className="flex gap-3 border-b py-3 last:border-b-0"
              >
                <div
                  className="flex size-16 shrink-0 items-center justify-center rounded-md text-base font-semibold text-white/90"
                  style={{ background: gradient(it.productName, it.productBrand) }}
                >
                  {initials(it.productName)}
                </div>
                <div className="flex flex-1 flex-col gap-1">
                  <Link
                    href={`/p/${encodeURIComponent(it.productId)}`}
                    onClick={onClose}
                    className="text-left text-sm font-medium hover:underline"
                  >
                    {it.productName}
                  </Link>
                  <div className="text-xs text-muted-foreground">
                    {it.productBrand}
                  </div>
                  <div className="mt-auto flex items-center justify-between">
                    <div className="flex items-center gap-1">
                      <Button
                        variant="outline"
                        size="icon"
                        className="size-7"
                        onClick={() => cart.setQuantity(it.id, it.quantity - 1)}
                        aria-label="Decrease quantity"
                      >
                        <Minus className="size-3" />
                      </Button>
                      <span className="w-7 text-center font-mono text-sm">
                        {it.quantity}
                      </span>
                      <Button
                        variant="outline"
                        size="icon"
                        className="size-7"
                        onClick={() => cart.setQuantity(it.id, it.quantity + 1)}
                        aria-label="Increase quantity"
                      >
                        <Plus className="size-3" />
                      </Button>
                    </div>
                    <span className="text-sm font-medium">
                      ${(it.productPrice * it.quantity).toFixed(2)}
                    </span>
                  </div>
                </div>
                <Button
                  variant="ghost"
                  size="icon"
                  className="size-7 text-muted-foreground hover:text-destructive"
                  onClick={() => cart.remove(it.id)}
                  aria-label="Remove from cart"
                >
                  <Trash2 className="size-3.5" />
                </Button>
              </li>
            ))}
          </ul>
        )}

        {cart.items.length > 0 && (
          <SheetFooter>
            <Separator />
            <div className="flex items-center justify-between text-base font-medium">
              <span>Subtotal</span>
              <span>${cart.total.toFixed(2)}</span>
            </div>
            <SheetClose asChild>
              <Button asChild className="w-full">
                <Link href="/checkout">Checkout</Link>
              </Button>
            </SheetClose>
            <Button
              variant="ghost"
              className="text-muted-foreground"
              onClick={cart.clear}
            >
              Clear cart
            </Button>
          </SheetFooter>
        )}
      </SheetContent>
    </Sheet>
  );
}
