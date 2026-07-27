// Demo data for a brand-new stock workspace.
//
// An empty product list shows nothing about levels, reorder flags, or the
// ledger. The first sign-in seeds a realistic shelf — including one line out of
// stock and one below its reorder point — so those states are visible rather
// than theoretical.
//
// The movements are the interesting part: each product\'s level is the sum of
// its history, so the seed writes a plausible sequence of receipts and sales
// rather than a starting quantity.

export interface SeedProduct {
  key: string;
  sku: string;
  name: string;
  category: string;
  cost: number;
  price: number;
  reorderPoint: number;
}

export interface SeedMovement {
  product: string;
  delta: number;
  reason: string;
  note?: string;
  /** Days ago. */
  age: number;
}

export const SEED_PRODUCTS: SeedProduct[] = [
  { key: "beans", sku: "CFE-001", name: "House Blend, 1kg", category: "Coffee", cost: 9.4, price: 18, reorderPoint: 12 },
  { key: "decaf", sku: "CFE-002", name: "Decaf Single Origin, 1kg", category: "Coffee", cost: 11.2, price: 22, reorderPoint: 6 },
  { key: "cups", sku: "PKG-010", name: "12oz Cups (sleeve of 50)", category: "Packaging", cost: 4.15, price: 0, reorderPoint: 20 },
  { key: "lids", sku: "PKG-011", name: "12oz Lids (sleeve of 50)", category: "Packaging", cost: 2.8, price: 0, reorderPoint: 20 },
  { key: "grinder", sku: "EQP-100", name: "Burr Grinder", category: "Equipment", cost: 148, price: 249, reorderPoint: 2 },
  { key: "filters", sku: "PKG-020", name: "Paper Filters (box of 100)", category: "Packaging", cost: 3.5, price: 7, reorderPoint: 10 },
];

export const SEED_MOVEMENTS: SeedMovement[] = [
  // Healthy stock.
  { product: "beans", delta: 60, reason: "received", note: "PO-4471", age: 21 },
  { product: "beans", delta: -8, reason: "sold", age: 14 },
  { product: "beans", delta: -11, reason: "sold", age: 7 },
  { product: "beans", delta: -6, reason: "sold", age: 2 },

  // Below its reorder point of 6 — ends at 4.
  { product: "decaf", delta: 24, reason: "received", note: "PO-4471", age: 21 },
  { product: "decaf", delta: -9, reason: "sold", age: 12 },
  { product: "decaf", delta: -8, reason: "sold", age: 5 },
  { product: "decaf", delta: -3, reason: "sold", age: 1 },

  { product: "cups", delta: 100, reason: "received", age: 30 },
  { product: "cups", delta: -34, reason: "sold", age: 10 },

  // Out of stock — ends at 0.
  { product: "lids", delta: 40, reason: "received", age: 30 },
  { product: "lids", delta: -38, reason: "sold", age: 9 },
  { product: "lids", delta: -2, reason: "damaged", note: "Crushed in transit", age: 4 },

  { product: "grinder", delta: 6, reason: "received", note: "PO-4502", age: 40 },
  { product: "grinder", delta: -2, reason: "sold", age: 18 },
  { product: "grinder", delta: 1, reason: "returned", note: "Customer changed mind", age: 6 },

  { product: "filters", delta: 48, reason: "received", age: 25 },
  { product: "filters", delta: -12, reason: "sold", age: 8 },
  { product: "filters", delta: -2, reason: "count", note: "Cycle count adjustment", age: 3 },
];

export interface ShapedSeed {
  products: Array<{ key: string; row: Record<string, unknown> }>;
  movements: Array<{ product: string; row: Record<string, unknown> }>;
}

export function shapeSeed(now: number = Date.now()): ShapedSeed {
  const daysAgo = (days: number) => new Date(now - days * 86_400_000).toISOString();

  return {
    products: SEED_PRODUCTS.map((p) => ({
      key: p.key,
      row: {
        sku: p.sku,
        name: p.name,
        category: p.category,
        unitCostCents: Math.round(p.cost * 100),
        unitPriceCents: Math.round(p.price * 100),
        reorderPoint: p.reorderPoint,
        archived: false,
        createdAt: daysAgo(60),
      },
    })),
    movements: SEED_MOVEMENTS.map((m) => ({
      product: m.product,
      row: {
        delta: m.delta,
        reason: m.reason,
        note: m.note ?? null,
        createdAt: daysAgo(m.age),
      },
    })),
  };
}
