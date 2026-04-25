"use client";

import Link from "next/link";
import { useEffect, useMemo, useState } from "react";
import { useRouter } from "next/navigation";
import { db } from "@pylonsync/react";
import { ArrowLeft, Loader2 } from "lucide-react";
import { Button } from "@pylonsync/example-ui/button";
import { Card, CardContent, CardHeader, CardTitle } from "@pylonsync/example-ui/card";
import { Separator } from "@pylonsync/example-ui/separator";
import { useCart } from "@/components/cart-store";
import { AddressList } from "@/app/account/client";
import { useAuth } from "@/lib/pylon-client";
import type { Address } from "@/lib/types";
import { gradient, initials } from "@/lib/util";

export default function CheckoutClient() {
  const router = useRouter();
  const { user, isAuthenticated } = useAuth();
  const cart = useCart();
  const userId = user?.id ?? "";

  const placeOrder = db.useMutation<
    { addressId: string },
    { orderId: string; subtotal: number; trackingNumber: string }
  >("placeOrder");

  const addresses = db.useQuery<Address>("Address", {
    where: userId ? { userId } : undefined,
  });

  const defaultId = useMemo(
    () =>
      addresses.data.find((a) => a.isDefault)?.id ??
      addresses.data[0]?.id ??
      "",
    [addresses.data],
  );
  const [selectedId, setSelectedId] = useState<string>(defaultId);
  useEffect(() => {
    if (!selectedId && defaultId) setSelectedId(defaultId);
  }, [defaultId, selectedId]);

  if (!isAuthenticated) {
    return (
      <main className="mx-auto max-w-md p-12 text-center">
        <p className="mb-4 text-sm text-muted-foreground">
          Log in to place an order.
        </p>
        <Button asChild>
          <Link href="/">Back to catalog</Link>
        </Button>
      </main>
    );
  }

  const handlePlace = async () => {
    if (!selectedId) return;
    try {
      const result = await placeOrder.mutate({ addressId: selectedId });
      router.push(`/orders/${encodeURIComponent(result.orderId)}`);
    } catch {}
  };

  const empty = cart.items.length === 0;

  return (
    <main className="mx-auto grid max-w-5xl gap-6 px-4 py-8 md:grid-cols-[1.2fr_1fr] md:px-6">
      <div className="flex flex-col gap-4">
        <Button variant="ghost" size="sm" className="self-start" asChild>
          <Link href="/">
            <ArrowLeft className="size-4" />
            Back to catalog
          </Link>
        </Button>

        <Card>
          <CardHeader>
            <CardTitle>Shipping address</CardTitle>
          </CardHeader>
          <CardContent>
            <AddressList
              userId={userId}
              addresses={addresses.data}
              loading={addresses.loading}
              selectable
              selectedId={selectedId}
              onSelect={setSelectedId}
            />
          </CardContent>
        </Card>

        <Card>
          <CardHeader>
            <CardTitle>Payment</CardTitle>
          </CardHeader>
          <CardContent className="text-sm text-muted-foreground">
            This demo skips real payment processing. Clicking{" "}
            <span className="font-medium text-foreground">Place order</span>{" "}
            creates the order, snapshots the address, clears your cart, and
            kicks off the shipping timeline.
          </CardContent>
        </Card>
      </div>

      <Card className="h-fit md:sticky md:top-20">
        <CardHeader>
          <CardTitle>Order summary</CardTitle>
        </CardHeader>
        <CardContent className="flex flex-col gap-3">
          {empty ? (
            <div className="text-sm text-muted-foreground">
              Your cart is empty.
            </div>
          ) : (
            <>
              <ul className="flex flex-col gap-2">
                {cart.items.map((it) => (
                  <li key={it.id} className="flex items-center gap-3">
                    <div
                      className="flex size-10 shrink-0 items-center justify-center rounded text-xs font-semibold text-white/90"
                      style={{
                        background: gradient(it.productName, it.productBrand),
                      }}
                    >
                      {initials(it.productName)}
                    </div>
                    <div className="flex-1 truncate text-sm">
                      <div className="truncate">{it.productName}</div>
                      <div className="text-xs text-muted-foreground">
                        Qty {it.quantity} · ${it.productPrice.toFixed(2)} each
                      </div>
                    </div>
                    <div className="text-sm font-medium">
                      ${(it.productPrice * it.quantity).toFixed(2)}
                    </div>
                  </li>
                ))}
              </ul>
              <Separator />
              <div className="flex items-center justify-between text-base font-semibold">
                <span>Total</span>
                <span>${cart.total.toFixed(2)}</span>
              </div>
            </>
          )}

          {placeOrder.error && (
            <div className="rounded-md border border-destructive/30 bg-destructive/5 px-3 py-2 text-xs text-destructive">
              {placeOrder.error.message}
            </div>
          )}

          <Button
            disabled={empty || !selectedId || placeOrder.loading}
            onClick={handlePlace}
          >
            {placeOrder.loading && <Loader2 className="size-4 animate-spin" />}
            Place order
          </Button>
        </CardContent>
      </Card>
    </main>
  );
}
