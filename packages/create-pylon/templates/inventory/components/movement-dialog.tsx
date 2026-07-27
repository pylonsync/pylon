import React, { useEffect, useMemo, useState } from "react";
import {
  Dialog,
  DialogContent,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Select } from "@/components/ui/select";
import { REASONS, parseCount, reasonById } from "@/lib/stock";

/**
 * Record a movement.
 *
 * The user types a POSITIVE count and picks a reason; the direction comes from
 * the reason. Asking someone to type "-3" for a sale is how you get a "+3" sale
 * and a shelf that disagrees with the system.
 *
 * "Stock count" is the exception: it sets the level TO a number rather than
 * adjusting by one, because that's what counting a shelf actually produces. The
 * delta is computed from the current level here so the ledger still only ever
 * receives deltas.
 */
export function MovementDialog({
  open,
  products,
  currentLevel,
  defaultProductId,
  onOpenChange,
  onRecord,
}: {
  open: boolean;
  products: Array<{ id: string; name: string; sku: string }>;
  /** On-hand for a product, so a stock count can be turned into a delta. */
  currentLevel: (productId: string) => number;
  defaultProductId?: string;
  onOpenChange: (open: boolean) => void;
  onRecord: (
    productId: string,
    delta: number,
    reason: string,
    note: string,
  ) => void | Promise<void>;
}) {
  const [productId, setProductId] = useState(defaultProductId ?? "");
  const [reason, setReason] = useState("received");
  const [count, setCount] = useState("");
  const [note, setNote] = useState("");
  const [busy, setBusy] = useState(false);

  useEffect(() => {
    if (!open) return;
    setProductId(defaultProductId ?? "");
    setReason("received");
    setCount("");
    setNote("");
    setBusy(false);
  }, [open, defaultProductId]);

  const isCount = reason === "count";
  const parsed = parseCount(count);
  const level = productId ? currentLevel(productId) : 0;

  // For a stock count the typed number is the NEW level, so the delta is the
  // difference. For everything else the reason supplies the sign.
  const delta = useMemo(() => {
    if (parsed === null) return null;
    if (isCount) return parsed - level;
    const direction = reasonById(reason)?.direction;
    const magnitude = Math.abs(parsed);
    if (magnitude === 0) return null;
    return direction === "out" ? -magnitude : magnitude;
  }, [parsed, isCount, level, reason]);

  const wouldGoNegative = !isCount && delta !== null && level + delta < 0;
  const canSubmit =
    productId !== "" && delta !== null && delta !== 0 && !wouldGoNegative && !busy;

  async function submit(event: React.FormEvent) {
    event.preventDefault();
    if (!canSubmit) return;
    setBusy(true);
    try {
      await onRecord(productId, delta as number, reason, note.trim());
      onOpenChange(false);
    } finally {
      setBusy(false);
    }
  }

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent>
        <DialogHeader>
          <DialogTitle>Record movement</DialogTitle>
        </DialogHeader>
        <form onSubmit={submit} className="grid gap-4">
          <div className="space-y-1.5">
            <Label htmlFor="movement-product">Product</Label>
            <Select
              id="movement-product"
              value={productId}
              onChange={(event) => setProductId(event.target.value)}
            >
              <option value="">—</option>
              {products.map((product) => (
                <option key={product.id} value={product.id}>
                  {product.sku} · {product.name}
                </option>
              ))}
            </Select>
          </div>

          <div className="grid grid-cols-2 gap-3">
            <div className="space-y-1.5">
              <Label htmlFor="movement-reason">Reason</Label>
              <Select
                id="movement-reason"
                value={reason}
                onChange={(event) => setReason(event.target.value)}
              >
                {REASONS.map((r) => (
                  <option key={r.id} value={r.id}>
                    {r.label}
                  </option>
                ))}
              </Select>
            </div>
            <div className="space-y-1.5">
              <Label htmlFor="movement-count">
                {isCount ? "Counted" : "Quantity"}
              </Label>
              <Input
                id="movement-count"
                autoFocus
                inputMode="numeric"
                placeholder={isCount ? String(level) : "10"}
                value={count}
                onChange={(event) => setCount(event.target.value)}
              />
            </div>
          </div>

          {productId ? (
            <p className="text-[11px] text-muted-foreground">
              On hand {level}
              {delta !== null && delta !== 0 ? ` → ${level + delta}` : ""}
            </p>
          ) : null}
          {wouldGoNegative ? (
            <p className="text-[11px] text-destructive">
              That would go below zero. Record a stock count if the shelf
              disagrees.
            </p>
          ) : null}

          <div className="space-y-1.5">
            <Label htmlFor="movement-note">Note</Label>
            <Input
              id="movement-note"
              placeholder="PO number, or why"
              value={note}
              onChange={(event) => setNote(event.target.value)}
            />
          </div>

          <DialogFooter>
            <Button type="button" variant="ghost" onClick={() => onOpenChange(false)}>
              Cancel
            </Button>
            <Button type="submit" disabled={!canSubmit}>
              {busy ? "Recording…" : "Record"}
            </Button>
          </DialogFooter>
        </form>
      </DialogContent>
    </Dialog>
  );
}
