"use client";

import Link from "next/link";
import { db } from "@pylonsync/react";
import { ArrowLeft, Check, Home, Package, Truck } from "lucide-react";
import { Button } from "@pylonsync/example-ui/button";
import { Card, CardContent, CardHeader, CardTitle } from "@pylonsync/example-ui/card";
import { Separator } from "@pylonsync/example-ui/separator";
import { cn } from "@pylonsync/example-ui/utils";
import type { Order, OrderItem, OrderStatus } from "@/lib/types";
import { STATUS_LABELS, STATUS_STEPS } from "@/lib/types";
import { gradient, initials } from "@/lib/util";
import { StatusBadge } from "@/app/account/client";

export default function OrderClient({ id }: { id: string }) {
  const { data: order, loading } = db.useQueryOne<Order>("Order", id);
  const items = db.useQuery<OrderItem>("OrderItem", { where: { orderId: id } });

  if (loading) {
    return (
      <main className="mx-auto max-w-3xl px-4 py-8 md:px-6">
        <Card className="p-8 text-sm text-muted-foreground">
          Loading order…
        </Card>
      </main>
    );
  }
  if (!order) {
    return (
      <main className="mx-auto max-w-3xl px-4 py-8 md:px-6">
        <Card className="p-8 text-center text-sm text-muted-foreground">
          Order not found.
        </Card>
      </main>
    );
  }

  const eta = new Date(order.estimatedDelivery);

  return (
    <main className="mx-auto flex max-w-3xl flex-col gap-6 px-4 py-8 md:px-6">
      <Button variant="ghost" size="sm" className="self-start" asChild>
        <Link href="/account">
          <ArrowLeft className="size-4" />
          Back to orders
        </Link>
      </Button>

      <Card>
        <CardHeader className="flex flex-row items-start justify-between gap-3">
          <div>
            <div className="text-xs text-muted-foreground">
              Order #{order.id.slice(-8).toUpperCase()}
            </div>
            <CardTitle className="mt-1 text-xl">
              {STATUS_LABELS[order.status]}
            </CardTitle>
            <div className="mt-1 text-sm text-muted-foreground">
              {order.status === "delivered"
                ? "Delivered"
                : `Estimated delivery ${eta.toLocaleString()}`}
            </div>
          </div>
          <StatusBadge status={order.status} />
        </CardHeader>
        <CardContent>
          <ShippingTimeline status={order.status} />
        </CardContent>
      </Card>

      <Card>
        <CardHeader>
          <CardTitle className="text-base">Items</CardTitle>
        </CardHeader>
        <CardContent className="flex flex-col gap-3">
          {items.data.map((it) => (
            <div key={it.id} className="flex items-center gap-3">
              <div
                className="flex size-12 shrink-0 items-center justify-center rounded-md text-xs font-semibold text-white/90"
                style={{
                  background: gradient(it.productName, it.productBrand),
                }}
              >
                {initials(it.productName)}
              </div>
              <div className="flex-1">
                <div className="text-sm font-medium">{it.productName}</div>
                <div className="text-xs text-muted-foreground">
                  {it.productBrand} · qty {it.quantity}
                </div>
              </div>
              <div className="text-sm font-medium">
                ${(it.unitPrice * it.quantity).toFixed(2)}
              </div>
            </div>
          ))}
          <Separator />
          <div className="flex items-center justify-between text-base font-semibold">
            <span>Total</span>
            <span>${order.subtotal.toFixed(2)}</span>
          </div>
        </CardContent>
      </Card>

      <Card>
        <CardHeader>
          <CardTitle className="text-base">Shipping</CardTitle>
        </CardHeader>
        <CardContent className="flex flex-col gap-1 text-sm text-muted-foreground">
          <div className="font-medium text-foreground">{order.shipName}</div>
          <div>{order.shipStreet}</div>
          <div>
            {order.shipCity}, {order.shipPostal} · {order.shipCountry}
          </div>
          <div className="mt-2 text-xs">
            Tracking{" "}
            <span className="font-mono text-foreground">
              {order.trackingNumber}
            </span>
          </div>
        </CardContent>
      </Card>
    </main>
  );
}

const STEP_ICONS: Record<
  OrderStatus,
  React.ComponentType<{ className?: string }>
> = {
  placed: Check,
  packed: Package,
  shipped: Truck,
  delivered: Home,
};

function ShippingTimeline({ status }: { status: OrderStatus }) {
  const currentIdx = STATUS_STEPS.indexOf(status);
  return (
    <ol className="grid grid-cols-4 gap-2">
      {STATUS_STEPS.map((step, i) => {
        const reached = i <= currentIdx;
        const active = i === currentIdx;
        const Icon = STEP_ICONS[step];
        return (
          <li
            key={step}
            className="flex flex-col items-center gap-2 text-center"
          >
            <div className="relative flex w-full items-center">
              {i > 0 && (
                <span
                  className={
                    "absolute left-0 right-1/2 top-1/2 h-0.5 -translate-y-1/2 " +
                    (i <= currentIdx ? "bg-primary" : "bg-border")
                  }
                />
              )}
              {i < STATUS_STEPS.length - 1 && (
                <span
                  className={
                    "absolute left-1/2 right-0 top-1/2 h-0.5 -translate-y-1/2 " +
                    (i < currentIdx ? "bg-primary" : "bg-border")
                  }
                />
              )}
              <span
                className={cn(
                  "relative z-10 mx-auto flex size-9 items-center justify-center rounded-full border-2 transition",
                  reached
                    ? "border-primary bg-primary text-primary-foreground"
                    : "border-border bg-background text-muted-foreground",
                  active && "shadow-md ring-4 ring-primary/15",
                )}
              >
                <Icon className="size-4" />
              </span>
            </div>
            <span
              className={
                "text-xs " +
                (reached
                  ? "font-medium text-foreground"
                  : "text-muted-foreground")
              }
            >
              {STATUS_LABELS[step]}
            </span>
          </li>
        );
      })}
    </ol>
  );
}
