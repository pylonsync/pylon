/**
 * Account page — orders list + addresses CRUD.
 *
 * Two columns at desktop, stacked on mobile. Both data sources are
 * `db.useQuery`, so any new order or address shows up live without a
 * refresh — useful when an order's status advances from packed →
 * shipped → delivered while the user is looking at the page.
 */
import { useState } from "react";
import { db } from "@pylonsync/react";
import { ArrowRight, MapPin, Plus, Star, Trash2 } from "lucide-react";
import { Button } from "@/components/ui/button";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Badge } from "@/components/ui/badge";
import {
  Dialog,
  DialogContent,
  DialogHeader,
  DialogTitle,
  DialogDescription,
} from "@/components/ui/dialog";
import type { Address, Order } from "./lib/types";
import { STATUS_LABELS } from "./lib/types";
import { useAuth } from "./lib/auth";
import { navigate } from "./lib/util";

export function AccountPage() {
  const { user } = useAuth();
  const userId = user?.id ?? "";

  const orders = db.useQuery<Order>("Order", {
    where: userId ? { userId } : undefined,
    orderBy: { placedAt: "desc" },
  });
  const addresses = db.useQuery<Address>("Address", {
    where: userId ? { userId } : undefined,
  });

  return (
    <main className="mx-auto grid max-w-5xl gap-6 px-4 py-8 md:grid-cols-[1.4fr_1fr] md:px-6">
      <section className="flex flex-col gap-4">
        <h2 className="text-lg font-semibold">Orders</h2>
        {orders.loading ? (
          <Card className="p-6 text-sm text-muted-foreground">Loading…</Card>
        ) : orders.data.length === 0 ? (
          <Card className="p-6 text-center text-sm text-muted-foreground">
            You haven&rsquo;t placed any orders yet.
            <div className="mt-3">
              <Button size="sm" onClick={() => navigate("#/")}>
                Browse the catalog
              </Button>
            </div>
          </Card>
        ) : (
          orders.data.map((o) => <OrderRow key={o.id} order={o} />)
        )}
      </section>

      <section className="flex flex-col gap-4">
        <div className="flex items-center justify-between">
          <h2 className="text-lg font-semibold">Shipping addresses</h2>
        </div>
        <AddressList addresses={addresses.data} loading={addresses.loading} />
      </section>
    </main>
  );
}

// ---------------------------------------------------------------------------
// Orders
// ---------------------------------------------------------------------------

function OrderRow({ order }: { order: Order }) {
  const placed = new Date(order.placedAt).toLocaleString();
  return (
    <Card
      className="cursor-pointer transition hover:border-primary/30"
      onClick={() => navigate(`#/orders/${encodeURIComponent(order.id)}`)}
    >
      <CardContent className="flex items-center gap-4 p-4">
        <div className="flex flex-1 flex-col gap-1">
          <div className="flex items-center gap-2 text-sm">
            <span className="font-mono text-xs text-muted-foreground">
              #{order.id.slice(-8).toUpperCase()}
            </span>
            <StatusBadge status={order.status} />
          </div>
          <div className="text-sm text-muted-foreground">
            {order.itemCount} item{order.itemCount === 1 ? "" : "s"} · placed{" "}
            {placed}
          </div>
        </div>
        <div className="text-right">
          <div className="text-base font-semibold">
            ${order.subtotal.toFixed(2)}
          </div>
          <div className="text-xs text-muted-foreground">
            Tracking · {order.trackingNumber}
          </div>
        </div>
        <ArrowRight className="size-4 text-muted-foreground" />
      </CardContent>
    </Card>
  );
}

export function StatusBadge({ status }: { status: Order["status"] }) {
  const variant: Record<Order["status"], "default" | "secondary" | "outline"> =
    {
      placed: "secondary",
      packed: "secondary",
      shipped: "default",
      delivered: "outline",
    };
  return (
    <Badge variant={variant[status] ?? "secondary"} className="capitalize">
      {STATUS_LABELS[status] ?? status}
    </Badge>
  );
}

// ---------------------------------------------------------------------------
// Addresses
// ---------------------------------------------------------------------------

export function AddressList({
  addresses,
  loading,
  selectable,
  selectedId,
  onSelect,
}: {
  addresses: Address[];
  loading: boolean;
  selectable?: boolean;
  selectedId?: string;
  onSelect?: (id: string) => void;
}) {
  const [editorOpen, setEditorOpen] = useState(false);
  const addr = db.useEntity("Address");
  const { user } = useAuth();
  const userId = user?.id ?? "";

  if (loading) {
    return <Card className="p-6 text-sm text-muted-foreground">Loading…</Card>;
  }

  return (
    <div className="flex flex-col gap-3">
      {addresses.length === 0 ? (
        <Card className="p-6 text-center text-sm text-muted-foreground">
          No addresses yet.
        </Card>
      ) : (
        addresses.map((a) => {
          const selected = selectable && selectedId === a.id;
          return (
            <Card
              key={a.id}
              className={
                "p-4 transition " +
                (selectable
                  ? "cursor-pointer " +
                    (selected
                      ? "border-primary ring-2 ring-primary/20"
                      : "hover:border-primary/30")
                  : "")
              }
              onClick={() => selectable && onSelect?.(a.id)}
            >
              <div className="flex items-start gap-3">
                <MapPin className="mt-0.5 size-4 text-muted-foreground" />
                <div className="flex-1">
                  <div className="flex items-center gap-2">
                    <span className="text-sm font-medium">{a.fullName}</span>
                    {a.isDefault && (
                      <Badge variant="secondary" className="gap-1">
                        <Star className="size-3" /> Default
                      </Badge>
                    )}
                  </div>
                  <div className="mt-0.5 text-sm text-muted-foreground">
                    {a.street}
                  </div>
                  <div className="text-sm text-muted-foreground">
                    {a.city}, {a.postal} · {a.country}
                  </div>
                </div>
                {!selectable && (
                  <div className="flex flex-col gap-1">
                    {!a.isDefault && (
                      <Button
                        variant="ghost"
                        size="sm"
                        onClick={(e) => {
                          e.stopPropagation();
                          // Demote any current default first.
                          addresses
                            .filter((x) => x.isDefault && x.id !== a.id)
                            .forEach((x) =>
                              addr.update(x.id, { isDefault: false }),
                            );
                          addr.update(a.id, { isDefault: true });
                        }}
                      >
                        Make default
                      </Button>
                    )}
                    <Button
                      variant="ghost"
                      size="sm"
                      className="text-muted-foreground hover:text-destructive"
                      onClick={(e) => {
                        e.stopPropagation();
                        addr.remove(a.id);
                      }}
                    >
                      <Trash2 className="size-3.5" />
                      Remove
                    </Button>
                  </div>
                )}
              </div>
            </Card>
          );
        })
      )}

      <Button variant="outline" onClick={() => setEditorOpen(true)}>
        <Plus className="size-4" />
        Add address
      </Button>

      <AddressEditor
        open={editorOpen}
        onClose={() => setEditorOpen(false)}
        onSave={(data) => {
          addr.insert({
            ...data,
            userId,
            isDefault: addresses.length === 0,
          });
        }}
      />
    </div>
  );
}

function AddressEditor({
  open,
  onClose,
  onSave,
}: {
  open: boolean;
  onClose: () => void;
  onSave: (data: Omit<Address, "id" | "userId" | "isDefault">) => void;
}) {
  const [form, setForm] = useState({
    fullName: "",
    street: "",
    city: "",
    postal: "",
    country: "United States",
  });

  return (
    <Dialog open={open} onOpenChange={(o) => !o && onClose()}>
      <DialogContent>
        <DialogHeader>
          <DialogTitle>Add a shipping address</DialogTitle>
          <DialogDescription>
            We&rsquo;ll snapshot it on every order so future edits don&rsquo;t
            change shipping history.
          </DialogDescription>
        </DialogHeader>
        <form
          className="grid gap-3"
          onSubmit={(e) => {
            e.preventDefault();
            onSave(form);
            setForm({
              fullName: "",
              street: "",
              city: "",
              postal: "",
              country: "United States",
            });
            onClose();
          }}
        >
          <Field label="Full name">
            <Input
              required
              value={form.fullName}
              onChange={(e) => setForm({ ...form, fullName: e.target.value })}
            />
          </Field>
          <Field label="Street">
            <Input
              required
              value={form.street}
              onChange={(e) => setForm({ ...form, street: e.target.value })}
            />
          </Field>
          <div className="grid grid-cols-[1fr_120px] gap-3">
            <Field label="City">
              <Input
                required
                value={form.city}
                onChange={(e) => setForm({ ...form, city: e.target.value })}
              />
            </Field>
            <Field label="ZIP / postal">
              <Input
                required
                value={form.postal}
                onChange={(e) => setForm({ ...form, postal: e.target.value })}
              />
            </Field>
          </div>
          <Field label="Country">
            <Input
              required
              value={form.country}
              onChange={(e) => setForm({ ...form, country: e.target.value })}
            />
          </Field>
          <Button type="submit" className="mt-2">
            Save address
          </Button>
        </form>
      </DialogContent>
    </Dialog>
  );
}

function Field({
  label,
  children,
}: {
  label: string;
  children: React.ReactNode;
}) {
  return (
    <div className="grid gap-1.5">
      <Label>{label}</Label>
      {children}
    </div>
  );
}
