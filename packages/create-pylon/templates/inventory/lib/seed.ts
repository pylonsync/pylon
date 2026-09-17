// Demo data for a brand-new stock workspace.
//
// The first sign-in seeds a working shelf: three dozen products across six
// categories and three months of ledger. Several lines end below their reorder
// point and a few are out of stock, so those states are visible rather than
// theoretical.
//
// The movements are the interesting part: each product's level is the sum of
// its history, so the seed writes a sequence of receipts, sales, returns, and
// counts rather than a starting quantity. The sequence is generated from a
// per-product profile with a fixed-seed generator, so it is the same on every
// run.

export interface SeedProduct {
  key: string;
  sku: string;
  name: string;
  category: string;
  cost: number;
  price: number;
  reorderPoint: number;
  /** Units received per delivery. */
  lot: number;
  /** Deliveries in the last 90 days. */
  deliveries: number;
  /** Average units sold per week. */
  weeklySales: number;
  /** How the ledger should end. */
  ending: "healthy" | "low" | "out";
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
  { key: "beans", sku: "CFE-001", name: "House Blend, 1kg", category: "Coffee", cost: 9.4, price: 18, reorderPoint: 12, lot: 60, deliveries: 4, weeklySales: 11, ending: "healthy" },
  { key: "decaf", sku: "CFE-002", name: "Decaf Single Origin, 1kg", category: "Coffee", cost: 11.2, price: 22, reorderPoint: 6, lot: 24, deliveries: 3, weeklySales: 6, ending: "low" },
  { key: "espresso", sku: "CFE-003", name: "Espresso Roast, 1kg", category: "Coffee", cost: 10.1, price: 20, reorderPoint: 12, lot: 48, deliveries: 4, weeklySales: 12, ending: "healthy" },
  { key: "ethiopia", sku: "CFE-004", name: "Ethiopia Yirgacheffe, 250g", category: "Coffee", cost: 4.6, price: 12, reorderPoint: 10, lot: 40, deliveries: 3, weeklySales: 9, ending: "healthy" },
  { key: "colombia", sku: "CFE-005", name: "Colombia Huila, 250g", category: "Coffee", cost: 4.2, price: 11, reorderPoint: 10, lot: 40, deliveries: 3, weeklySales: 10, ending: "low" },
  { key: "coldbrew", sku: "CFE-006", name: "Cold Brew Concentrate, 1L", category: "Coffee", cost: 5.8, price: 14, reorderPoint: 8, lot: 30, deliveries: 4, weeklySales: 8, ending: "healthy" },
  { key: "drip-bags", sku: "CFE-007", name: "Drip Bags (box of 10)", category: "Coffee", cost: 3.9, price: 9, reorderPoint: 8, lot: 36, deliveries: 2, weeklySales: 7, ending: "out" },

  { key: "breakfast", sku: "TEA-001", name: "English Breakfast, 100g", category: "Tea", cost: 2.8, price: 8, reorderPoint: 6, lot: 24, deliveries: 3, weeklySales: 4, ending: "healthy" },
  { key: "earlgrey", sku: "TEA-002", name: "Earl Grey, 100g", category: "Tea", cost: 3.1, price: 8.5, reorderPoint: 6, lot: 24, deliveries: 2, weeklySales: 4, ending: "healthy" },
  { key: "sencha", sku: "TEA-003", name: "Sencha, 100g", category: "Tea", cost: 4.4, price: 11, reorderPoint: 4, lot: 18, deliveries: 2, weeklySales: 3, ending: "low" },
  { key: "chai", sku: "TEA-004", name: "Masala Chai, 100g", category: "Tea", cost: 3.3, price: 9, reorderPoint: 6, lot: 24, deliveries: 3, weeklySales: 5, ending: "healthy" },
  { key: "peppermint", sku: "TEA-005", name: "Peppermint, 50g", category: "Tea", cost: 2.1, price: 6.5, reorderPoint: 4, lot: 20, deliveries: 2, weeklySales: 3, ending: "healthy" },

  { key: "cups12", sku: "PKG-010", name: "12oz Cups (sleeve of 50)", category: "Packaging", cost: 4.15, price: 0, reorderPoint: 20, lot: 100, deliveries: 3, weeklySales: 22, ending: "healthy" },
  { key: "lids12", sku: "PKG-011", name: "12oz Lids (sleeve of 50)", category: "Packaging", cost: 2.8, price: 0, reorderPoint: 20, lot: 100, deliveries: 2, weeklySales: 22, ending: "out" },
  { key: "cups8", sku: "PKG-012", name: "8oz Cups (sleeve of 50)", category: "Packaging", cost: 3.6, price: 0, reorderPoint: 15, lot: 80, deliveries: 3, weeklySales: 14, ending: "healthy" },
  { key: "lids8", sku: "PKG-013", name: "8oz Lids (sleeve of 50)", category: "Packaging", cost: 2.4, price: 0, reorderPoint: 15, lot: 80, deliveries: 3, weeklySales: 14, ending: "low" },
  { key: "sleeves", sku: "PKG-014", name: "Cup Sleeves (box of 500)", category: "Packaging", cost: 11, price: 0, reorderPoint: 4, lot: 12, deliveries: 2, weeklySales: 2, ending: "healthy" },
  { key: "filters", sku: "PKG-020", name: "Paper Filters (box of 100)", category: "Packaging", cost: 3.5, price: 7, reorderPoint: 10, lot: 48, deliveries: 3, weeklySales: 9, ending: "healthy" },
  { key: "bags-1kg", sku: "PKG-021", name: "Retail Bags, 1kg (100)", category: "Packaging", cost: 18, price: 0, reorderPoint: 3, lot: 10, deliveries: 2, weeklySales: 1.5, ending: "healthy" },
  { key: "bags-250g", sku: "PKG-022", name: "Retail Bags, 250g (200)", category: "Packaging", cost: 22, price: 0, reorderPoint: 3, lot: 10, deliveries: 2, weeklySales: 2, ending: "low" },
  { key: "napkins", sku: "PKG-030", name: "Napkins (case of 2000)", category: "Packaging", cost: 19, price: 0, reorderPoint: 3, lot: 8, deliveries: 2, weeklySales: 1.2, ending: "healthy" },
  { key: "stirrers", sku: "PKG-031", name: "Wooden Stirrers (box of 1000)", category: "Packaging", cost: 6.5, price: 0, reorderPoint: 4, lot: 12, deliveries: 2, weeklySales: 1.5, ending: "healthy" },

  { key: "grinder", sku: "EQP-100", name: "Burr Grinder", category: "Equipment", cost: 148, price: 249, reorderPoint: 2, lot: 6, deliveries: 2, weeklySales: 0.5, ending: "healthy" },
  { key: "kettle", sku: "EQP-101", name: "Gooseneck Kettle", category: "Equipment", cost: 42, price: 79, reorderPoint: 3, lot: 10, deliveries: 2, weeklySales: 1, ending: "healthy" },
  { key: "v60", sku: "EQP-102", name: "Pour-over Dripper, 02", category: "Equipment", cost: 9, price: 24, reorderPoint: 5, lot: 20, deliveries: 2, weeklySales: 2.5, ending: "healthy" },
  { key: "aeropress", sku: "EQP-103", name: "AeroPress", category: "Equipment", cost: 22, price: 39, reorderPoint: 4, lot: 12, deliveries: 2, weeklySales: 2, ending: "low" },
  { key: "scale", sku: "EQP-104", name: "Brew Scale with Timer", category: "Equipment", cost: 31, price: 59, reorderPoint: 2, lot: 8, deliveries: 2, weeklySales: 1, ending: "healthy" },
  { key: "frenchpress", sku: "EQP-105", name: "French Press, 1L", category: "Equipment", cost: 17, price: 34, reorderPoint: 3, lot: 10, deliveries: 1, weeklySales: 1, ending: "out" },
  { key: "tamper", sku: "EQP-106", name: "Espresso Tamper, 58mm", category: "Equipment", cost: 19, price: 38, reorderPoint: 2, lot: 6, deliveries: 1, weeklySales: 0.5, ending: "healthy" },

  { key: "mug", sku: "MRC-200", name: "Logo Mug", category: "Merch", cost: 5.5, price: 16, reorderPoint: 10, lot: 48, deliveries: 2, weeklySales: 6, ending: "healthy" },
  { key: "tumbler", sku: "MRC-201", name: "Insulated Tumbler, 16oz", category: "Merch", cost: 12, price: 32, reorderPoint: 6, lot: 24, deliveries: 2, weeklySales: 4, ending: "low" },
  { key: "tote", sku: "MRC-202", name: "Canvas Tote", category: "Merch", cost: 4.2, price: 14, reorderPoint: 8, lot: 36, deliveries: 1, weeklySales: 3, ending: "healthy" },
  { key: "tee", sku: "MRC-203", name: "Logo T-shirt", category: "Merch", cost: 8, price: 28, reorderPoint: 6, lot: 30, deliveries: 1, weeklySales: 2.5, ending: "healthy" },
  { key: "cap", sku: "MRC-204", name: "Corduroy Cap", category: "Merch", cost: 7, price: 26, reorderPoint: 4, lot: 20, deliveries: 1, weeklySales: 1.5, ending: "healthy" },

  { key: "oat", sku: "DRY-300", name: "Oat Milk, 1L (case of 6)", category: "Dairy & Alternatives", cost: 11.4, price: 0, reorderPoint: 8, lot: 30, deliveries: 6, weeklySales: 14, ending: "healthy" },
  { key: "whole", sku: "DRY-301", name: "Whole Milk, 2L (case of 4)", category: "Dairy & Alternatives", cost: 7.2, price: 0, reorderPoint: 10, lot: 40, deliveries: 8, weeklySales: 24, ending: "healthy" },
  { key: "almond", sku: "DRY-302", name: "Almond Milk, 1L (case of 6)", category: "Dairy & Alternatives", cost: 12.6, price: 0, reorderPoint: 4, lot: 12, deliveries: 4, weeklySales: 5, ending: "low" },
  { key: "syrup-vanilla", sku: "DRY-310", name: "Vanilla Syrup, 750ml", category: "Dairy & Alternatives", cost: 6.8, price: 0, reorderPoint: 4, lot: 12, deliveries: 3, weeklySales: 3, ending: "healthy" },
  { key: "syrup-caramel", sku: "DRY-311", name: "Caramel Syrup, 750ml", category: "Dairy & Alternatives", cost: 6.8, price: 0, reorderPoint: 4, lot: 12, deliveries: 3, weeklySales: 3, ending: "healthy" },
];

const LEDGER_DAYS = 90;

/** Small deterministic generator so the ledger is the same on every run. */
function generator(seed: number): () => number {
  let state = seed >>> 0 || 1;
  return () => {
    state ^= state << 13;
    state ^= state >>> 17;
    state ^= state << 5;
    return (state >>> 0) / 0x1_0000_0000;
  };
}

function hash(text: string): number {
  let h = 2166136261;
  for (let i = 0; i < text.length; i++) {
    h ^= text.charCodeAt(i);
    h = Math.imul(h, 16777619);
  }
  return h >>> 0;
}

/**
 * Build one product's ledger: deliveries spread over the window, sales every
 * few days between them, and the occasional return, breakage, or count. The
 * final sale is sized so the product ends where its profile says it should.
 */
function ledgerFor(product: SeedProduct, purchaseOrder: (age: number) => string): SeedMovement[] {
  const rand = generator(hash(product.key));
  const out: SeedMovement[] = [];
  let onHand = 0;

  const deliveryAges: number[] = [];
  const gap = LEDGER_DAYS / product.deliveries;
  for (let i = 0; i < product.deliveries; i++) {
    deliveryAges.push(Math.round(LEDGER_DAYS - i * gap - rand() * gap * 0.4));
  }

  for (const age of deliveryAges) {
    out.push({ product: product.key, delta: product.lot, reason: "received", note: purchaseOrder(age), age });
    onHand += product.lot;
  }

  const salesPerEvent = Math.max(1, Math.round(product.weeklySales / 2));
  const eventEvery = product.weeklySales >= 1 ? 3.5 : 7 / Math.max(product.weeklySales, 0.25);
  const sales: SeedMovement[] = [];
  for (let age = LEDGER_DAYS - 2; age >= 1; age -= eventEvery) {
    const day = Math.round(age);
    const received = out.filter((m) => m.reason === "received" && m.age >= day).reduce((sum, m) => sum + m.delta, 0);
    const sold = sales.reduce((sum, m) => sum - m.delta, 0);
    const available = received - sold;
    if (available <= 0) continue;
    const jitter = 0.6 + rand() * 0.8;
    const qty = Math.min(available, Math.max(1, Math.round(salesPerEvent * jitter)));
    sales.push({ product: product.key, delta: -qty, reason: "sold", age: day });
  }
  out.push(...sales);
  onHand -= sales.reduce((sum, m) => sum - m.delta, 0);

  if (product.price > 0 && rand() < 0.35) {
    out.push({ product: product.key, delta: 1, reason: "returned", note: "Customer return", age: Math.round(5 + rand() * 30) });
    onHand += 1;
  }
  if (rand() < 0.3 && onHand > 2) {
    const broken = 1 + Math.round(rand());
    out.push({ product: product.key, delta: -broken, reason: "damaged", note: rand() < 0.5 ? "Crushed in transit" : "Dropped in the stockroom", age: Math.round(3 + rand() * 40) });
    onHand -= broken;
  }
  if (rand() < 0.4 && onHand > 3) {
    const adjust = rand() < 0.5 ? -1 : 1;
    out.push({ product: product.key, delta: adjust, reason: "count", note: "Cycle count adjustment", age: Math.round(2 + rand() * 20) });
    onHand += adjust;
  }

  // Returns, breakage, and counts land at random dates, so a sale recorded
  // before them may now overdraw. Walk the ledger in date order and trim any
  // outgoing movement to what was actually on the shelf.
  out.sort((a, b) => b.age - a.age);
  let running = 0;
  for (const m of out) {
    if (running + m.delta < 0) m.delta = -running;
    running += m.delta;
  }
  for (let i = out.length - 1; i >= 0; i--) if (out[i].delta === 0) out.splice(i, 1);
  onHand = running;

  const target =
    product.ending === "out" ? 0 : product.ending === "low" ? Math.max(1, Math.floor(product.reorderPoint * 0.5)) : Math.round(product.reorderPoint * (1.6 + rand() * 1.5));
  const diff = target - onHand;
  if (diff < 0) {
    out.push({ product: product.key, delta: diff, reason: "sold", age: 1 });
  } else if (diff > 0) {
    out.push({ product: product.key, delta: diff, reason: "received", note: purchaseOrder(1), age: 1 });
  }

  return out.sort((a, b) => b.age - a.age);
}

export const SEED_MOVEMENTS: SeedMovement[] = (() => {
  const orders = new Map<number, string>();
  let next = 4471;
  const purchaseOrder = (age: number) => {
    const week = Math.floor(age / 7);
    let po = orders.get(week);
    if (!po) {
      po = `PO-${next++}`;
      orders.set(week, po);
    }
    return po;
  };
  return SEED_PRODUCTS.flatMap((p) => ledgerFor(p, purchaseOrder));
})();

export interface ShapedSeed {
  products: Array<{ key: string; row: Record<string, unknown> }>;
  movements: Array<{ product: string; row: Record<string, unknown> }>;
}

export function shapeSeed(now: number = Date.now()): ShapedSeed {
  const daysAgo = (days: number) => new Date(now - days * 86_400_000).toISOString();

  return {
    products: SEED_PRODUCTS.map((p, index) => ({
      key: p.key,
      row: {
        sku: p.sku,
        name: p.name,
        category: p.category,
        unitCostCents: Math.round(p.cost * 100),
        unitPriceCents: Math.round(p.price * 100),
        reorderPoint: p.reorderPoint,
        archived: false,
        createdAt: daysAgo(120 - index),
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
