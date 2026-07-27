import React, { useState } from "react";
import { Plus, Trash2 } from "lucide-react";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import {
  lineTotalCents,
  money,
  parseAmount,
  parseQuantity,
  quantity,
  type LineItem,
} from "@/lib/billing";

export interface LineDraft {
  description: string;
  quantityMilli: number;
  unitPriceCents: number;
}

/**
 * The billable lines, and the row that adds one.
 *
 * Editing is deliberately limited to add and remove: an invoice that has been
 * sent is a document someone is paying against, and silently mutating a line
 * after the fact is how the copy in their inbox stops matching yours. Correct a
 * sent invoice by voiding it and issuing another.
 */
export function LineItems({
  items,
  editable,
  onAdd,
  onRemove,
}: {
  items: LineItem[];
  /** False once the invoice leaves draft. */
  editable: boolean;
  onAdd: (draft: LineDraft) => void | Promise<void>;
  onRemove: (id: string) => void | Promise<void>;
}) {
  const [description, setDescription] = useState("");
  const [qty, setQty] = useState("1");
  const [price, setPrice] = useState("");
  const [busy, setBusy] = useState(false);

  const parsedQty = parseQuantity(qty);
  const parsedPrice = parseAmount(price);
  const canAdd =
    description.trim().length > 0 &&
    parsedQty !== null &&
    parsedQty > 0 &&
    parsedPrice !== null &&
    !busy;

  async function add(event: React.FormEvent) {
    event.preventDefault();
    if (!canAdd) return;
    setBusy(true);
    try {
      await onAdd({
        description: description.trim(),
        quantityMilli: parsedQty as number,
        unitPriceCents: parsedPrice as number,
      });
      setDescription("");
      setQty("1");
      setPrice("");
    } finally {
      setBusy(false);
    }
  }

  const ordered = [...items].sort(
    (a, b) =>
      (Number((a as { position?: number }).position) || 0) -
      (Number((b as { position?: number }).position) || 0),
  );

  return (
    <div className="rounded-lg border border-border bg-card">
      <table className="w-full border-collapse text-[13px]">
        <thead>
          <tr className="hairline">
            <th scope="col" className="h-8 px-3 text-left text-[11px] font-medium text-muted-foreground">
              Description
            </th>
            <th scope="col" className="h-8 w-20 px-3 text-right text-[11px] font-medium text-muted-foreground">
              Qty
            </th>
            <th scope="col" className="h-8 w-28 px-3 text-right text-[11px] font-medium text-muted-foreground">
              Unit
            </th>
            <th scope="col" className="h-8 w-28 px-3 text-right text-[11px] font-medium text-muted-foreground">
              Amount
            </th>
            {editable ? <th className="w-10" /> : null}
          </tr>
        </thead>
        <tbody>
          {ordered.length === 0 ? (
            <tr>
              <td
                colSpan={editable ? 5 : 4}
                className="px-3 py-6 text-center text-[12px] text-muted-foreground"
              >
                No lines yet.
              </td>
            </tr>
          ) : (
            ordered.map((item) => (
              <tr key={item.id} className="border-b border-border/60 last:border-0">
                <td className="px-3 py-2">{item.description}</td>
                <td className="tabular px-3 py-2 text-right text-muted-foreground">
                  {quantity(item.quantityMilli)}
                </td>
                <td className="tabular px-3 py-2 text-right text-muted-foreground">
                  {money(item.unitPriceCents)}
                </td>
                <td className="tabular px-3 py-2 text-right">
                  {money(lineTotalCents(item))}
                </td>
                {editable ? (
                  <td className="px-2 py-2 text-right">
                    <Button
                      variant="ghost"
                      size="icon"
                      aria-label={`Remove ${item.description}`}
                      onClick={() => onRemove(item.id)}
                      className="size-7 text-muted-foreground hover:text-destructive"
                    >
                      <Trash2 />
                    </Button>
                  </td>
                ) : null}
              </tr>
            ))
          )}
        </tbody>
      </table>

      {editable ? (
        <form onSubmit={add} className="flex items-end gap-2 border-t border-border p-2">
          <Input
            aria-label="Description"
            placeholder="Senior engineering"
            value={description}
            onChange={(event) => setDescription(event.target.value)}
            className="h-8 flex-1"
          />
          <Input
            aria-label="Quantity"
            inputMode="decimal"
            value={qty}
            onChange={(event) => setQty(event.target.value)}
            className="h-8 w-20 text-right"
          />
          <Input
            aria-label="Unit price"
            inputMode="decimal"
            placeholder="165.00"
            value={price}
            onChange={(event) => setPrice(event.target.value)}
            className="h-8 w-28 text-right"
          />
          <Button type="submit" size="sm" disabled={!canAdd} className="h-8">
            <Plus />
            Add
          </Button>
        </form>
      ) : null}
    </div>
  );
}
