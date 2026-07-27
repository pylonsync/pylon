// Stock levels, valuation, and reorder state.
//
// THE CENTRAL DECISION: on-hand is never stored. It is the sum of an append-only
// ledger of movements. A mutable `quantity` column is the classic inventory bug
// — two people receive the same delivery, both read 10, both write 15, and you
// have lost five units with nothing to audit. Appending "+5" twice gives 20 and
// shows you exactly who did it.
//
// It also means every count can be explained. "Why do we have 3?" is answerable
// from the same rows that produced the number.
//
// Money is INTEGER CENTS. Quantities are whole units — you cannot hold half a
// physical thing, and allowing it hides unit-of-measure mistakes.
//
// Pure: no React, no `db`.

export interface Product {
  id: string;
  sku: string;
  name: string;
  category?: string | null;
  unitCostCents?: number | null;
  unitPriceCents?: number | null;
  /** Order more at or below this level. Null means never flag it. */
  reorderPoint?: number | null;
  archived?: boolean | null;
  createdAt?: string | null;
}

export interface Movement {
  id: string;
  productId: string;
  /** Signed: positive receives, negative issues. Never zero. */
  delta: number;
  reason: string;
  note?: string | null;
  createdAt?: string | null;
  actorId?: string | null;
}

export interface Reason {
  id: string;
  label: string;
  /** Which direction this reason is allowed to move stock. */
  direction: "in" | "out" | "either";
}

export const REASONS: Reason[] = [
  { id: "received", label: "Received", direction: "in" },
  { id: "sold", label: "Sold", direction: "out" },
  { id: "returned", label: "Customer return", direction: "in" },
  { id: "damaged", label: "Damaged", direction: "out" },
  { id: "count", label: "Stock count", direction: "either" },
];

export function reasonById(id: string): Reason | undefined {
  return REASONS.find((r) => r.id === id);
}

export function isValidReason(id: string): boolean {
  return REASONS.some((r) => r.id === id);
}

/**
 * Does this reason permit this direction? A "Sold" that adds stock is a
 * mis-click, and catching it here keeps the ledger meaningful — the whole point
 * of storing a reason is being able to trust it later.
 */
export function reasonAllows(reasonId: string, delta: number): boolean {
  const reason = reasonById(reasonId);
  if (!reason || delta === 0) return false;
  if (reason.direction === "either") return true;
  return reason.direction === "in" ? delta > 0 : delta < 0;
}

/** On-hand for one product: the sum of its movements. */
export function onHand(productId: string, movements: Movement[]): number {
  let total = 0;
  for (const movement of movements) {
    if (movement.productId !== productId) continue;
    const delta = Number(movement.delta);
    if (Number.isFinite(delta)) total += Math.trunc(delta);
  }
  return total;
}

/**
 * On-hand for every product in one pass.
 *
 * The per-product version is O(movements) each, so a list view calling it per
 * row is O(products × movements) — fine at ten products, not at a thousand.
 */
export function onHandByProduct(movements: Movement[]): Map<string, number> {
  const totals = new Map<string, number>();
  for (const movement of movements) {
    const delta = Number(movement.delta);
    if (!Number.isFinite(delta)) continue;
    totals.set(
      movement.productId,
      (totals.get(movement.productId) ?? 0) + Math.trunc(delta),
    );
  }
  return totals;
}

export type StockState = "out" | "low" | "ok";

/**
 * Out of stock, at/below the reorder point, or fine.
 *
 * Zero is `out` even when there's no reorder point set — you can't sell what
 * you don't have, and that's worth surfacing whether or not someone configured
 * a threshold.
 */
export function stockState(quantity: number, reorderPoint?: number | null): StockState {
  if (quantity <= 0) return "out";
  const point = Number(reorderPoint);
  if (Number.isFinite(point) && point > 0 && quantity <= point) return "low";
  return "ok";
}

export interface Summary {
  skuCount: number;
  unitCount: number;
  /** Stock at COST — what the shelves are worth to you, not to a customer. */
  valuationCents: number;
  lowCount: number;
  outCount: number;
}

export function summarize(products: Product[], movements: Movement[]): Summary {
  const byProduct = onHandByProduct(movements);
  const summary: Summary = {
    skuCount: 0,
    unitCount: 0,
    valuationCents: 0,
    lowCount: 0,
    outCount: 0,
  };

  for (const product of products) {
    if (product.archived) continue;
    summary.skuCount += 1;
    const quantity = byProduct.get(product.id) ?? 0;
    // Negative stock is a data error, not negative value — don't let it credit
    // the valuation and hide itself.
    summary.unitCount += Math.max(0, quantity);
    summary.valuationCents +=
      Math.max(0, quantity) * (Number(product.unitCostCents) || 0);

    const state = stockState(quantity, product.reorderPoint);
    if (state === "out") summary.outCount += 1;
    else if (state === "low") summary.lowCount += 1;
  }
  return summary;
}

/** Products at or below their reorder point, most urgent first. */
export function needsReorder(
  products: Product[],
  movements: Movement[],
): Array<{ product: Product; quantity: number; state: StockState }> {
  const byProduct = onHandByProduct(movements);
  return products
    .filter((product) => !product.archived)
    .map((product) => {
      const quantity = byProduct.get(product.id) ?? 0;
      return { product, quantity, state: stockState(quantity, product.reorderPoint) };
    })
    .filter((row) => row.state !== "ok")
    .sort((a, b) => a.quantity - b.quantity);
}

/** "$12.50" — exact, like every money figure in this app. */
export function money(cents: number | null | undefined): string {
  const value = Number(cents) || 0;
  const sign = value < 0 ? "-" : "";
  const abs = Math.abs(value);
  const dollars = Math.floor(abs / 100).toLocaleString("en-US");
  const rest = String(abs % 100).padStart(2, "0");
  return `${sign}$${dollars}.${rest}`;
}

/** "+5" / "−3" — the sign is the point, so it's always shown. */
export function signedQuantity(delta: number): string {
  const value = Math.trunc(Number(delta) || 0);
  return value > 0 ? `+${value}` : value < 0 ? `−${Math.abs(value)}` : "0";
}

/** Parse a typed amount into cents. Returns null on anything unreadable. */
export function parseAmount(input: string): number | null {
  const cleaned = input.replace(/[$,\s]/g, "");
  if (!cleaned || !/^-?\d*\.?\d*$/.test(cleaned) || cleaned === "." || cleaned === "-") {
    return null;
  }
  const value = Number(cleaned);
  if (!Number.isFinite(value)) return null;
  return Math.round(value * 100);
}

/** Parse a typed whole-unit count. Rejects fractions and nonsense. */
export function parseCount(input: string): number | null {
  const cleaned = input.replace(/[,\s]/g, "");
  if (!cleaned || !/^-?\d+$/.test(cleaned)) return null;
  const value = Number(cleaned);
  return Number.isFinite(value) ? value : null;
}
