import React, { useEffect, useState } from "react";
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
import { money, parseAmount } from "@/lib/billing";

const METHODS = ["bank", "card", "cash", "other"];

/**
 * Record a payment against an invoice.
 *
 * Defaults to the outstanding balance, because "paid in full" is what happens
 * most of the time and retyping the figure is how a typo gets in. Over-payment
 * is rejected here rather than silently producing a negative balance — a credit
 * is a decision, not a rounding outcome.
 */
export function PaymentDialog({
  open,
  balanceCents,
  onOpenChange,
  onRecord,
}: {
  open: boolean;
  balanceCents: number;
  onOpenChange: (open: boolean) => void;
  onRecord: (amountCents: number, method: string, reference: string) => void | Promise<void>;
}) {
  const [amount, setAmount] = useState("");
  const [method, setMethod] = useState("bank");
  const [reference, setReference] = useState("");
  const [busy, setBusy] = useState(false);

  useEffect(() => {
    if (!open) return;
    setAmount((balanceCents / 100).toFixed(2));
    setMethod("bank");
    setReference("");
    setBusy(false);
  }, [open, balanceCents]);

  const parsed = parseAmount(amount);
  const tooMuch = parsed !== null && parsed > balanceCents;
  const canSubmit = parsed !== null && parsed > 0 && !tooMuch && !busy;

  async function submit(event: React.FormEvent) {
    event.preventDefault();
    if (!canSubmit) return;
    setBusy(true);
    try {
      await onRecord(parsed as number, method, reference.trim());
      onOpenChange(false);
    } finally {
      setBusy(false);
    }
  }

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent>
        <DialogHeader>
          <DialogTitle>Record payment</DialogTitle>
        </DialogHeader>
        <form onSubmit={submit} className="grid gap-4">
          <div className="space-y-1.5">
            <Label htmlFor="payment-amount">Amount</Label>
            <Input
              id="payment-amount"
              autoFocus
              inputMode="decimal"
              value={amount}
              onChange={(event) => setAmount(event.target.value)}
            />
            <p className="text-[11px] text-muted-foreground">
              Balance due {money(balanceCents)}
            </p>
            {tooMuch ? (
              <p className="text-[11px] text-destructive">
                More than the balance — record the exact amount received.
              </p>
            ) : null}
            {parsed === null && amount.trim() ? (
              <p className="text-[11px] text-destructive">That isn't an amount.</p>
            ) : null}
          </div>

          <div className="grid grid-cols-2 gap-3">
            <div className="space-y-1.5">
              <Label htmlFor="payment-method">Method</Label>
              <Select
                id="payment-method"
                value={method}
                onChange={(event) => setMethod(event.target.value)}
              >
                {METHODS.map((m) => (
                  <option key={m} value={m}>
                    {m[0].toUpperCase() + m.slice(1)}
                  </option>
                ))}
              </Select>
            </div>
            <div className="space-y-1.5">
              <Label htmlFor="payment-reference">Reference</Label>
              <Input
                id="payment-reference"
                placeholder="Optional"
                value={reference}
                onChange={(event) => setReference(event.target.value)}
              />
            </div>
          </div>

          <DialogFooter>
            <Button type="button" variant="ghost" onClick={() => onOpenChange(false)}>
              Cancel
            </Button>
            <Button type="submit" disabled={!canSubmit}>
              {busy ? "Recording…" : "Record payment"}
            </Button>
          </DialogFooter>
        </form>
      </DialogContent>
    </Dialog>
  );
}
