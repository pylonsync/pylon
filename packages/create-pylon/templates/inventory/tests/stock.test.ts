import { describe, expect, test } from "bun:test";
import {
  REASONS,
  isValidReason,
  money,
  needsReorder,
  onHand,
  onHandByProduct,
  parseAmount,
  parseCount,
  reasonAllows,
  signedQuantity,
  stockState,
  summarize,
  type Movement,
  type Product,
} from "../lib/stock";

const product = (over: Partial<Product> = {}): Product => ({
  id: "p1",
  sku: "SKU-1",
  name: "Thing",
  unitCostCents: 1000,
  reorderPoint: 5,
  ...over,
});

const move = (over: Partial<Movement> = {}): Movement => ({
  id: "m1",
  productId: "p1",
  delta: 10,
  reason: "received",
  ...over,
});

describe("onHand", () => {
  test("is the sum of the ledger, not a stored number", () => {
    expect(
      onHand("p1", [
        move({ id: "a", delta: 60 }),
        move({ id: "b", delta: -8 }),
        move({ id: "c", delta: -11 }),
      ]),
    ).toBe(41);
  });

  test("two concurrent receipts both count", () => {
    // The bug this design exists to prevent: with a mutable column, both
    // readers see 10, both write 15, and five units vanish.
    expect(
      onHand("p1", [
        move({ id: "start", delta: 10 }),
        move({ id: "a", delta: 5 }),
        move({ id: "b", delta: 5 }),
      ]),
    ).toBe(20);
  });

  test("ignores other products", () => {
    expect(onHand("p1", [move({ productId: "p2", delta: 999 })])).toBe(0);
  });

  test("no movements is zero", () => {
    expect(onHand("p1", [])).toBe(0);
  });

  test("a corrupt delta doesn't poison the total", () => {
    expect(
      onHand("p1", [move({ delta: 5 }), move({ id: "x", delta: NaN as unknown as number })]),
    ).toBe(5);
  });
});

describe("onHandByProduct", () => {
  test("matches the per-product version for every product", () => {
    const movements = [
      move({ id: "a", productId: "p1", delta: 10 }),
      move({ id: "b", productId: "p1", delta: -3 }),
      move({ id: "c", productId: "p2", delta: 4 }),
    ];
    const map = onHandByProduct(movements);
    expect(map.get("p1")).toBe(onHand("p1", movements));
    expect(map.get("p2")).toBe(onHand("p2", movements));
  });

  test("an empty ledger is an empty map, not zeroes for phantom products", () => {
    expect(onHandByProduct([]).size).toBe(0);
  });
});

describe("stockState", () => {
  test("zero or less is out, whatever the reorder point", () => {
    expect(stockState(0, 5)).toBe("out");
    expect(stockState(0, null)).toBe("out");
    expect(stockState(-2, 5)).toBe("out");
  });

  test("at or below the reorder point is low", () => {
    expect(stockState(5, 5)).toBe("low");
    expect(stockState(4, 5)).toBe("low");
  });

  test("above it is fine", () => {
    expect(stockState(6, 5)).toBe("ok");
  });

  test("no reorder point means only zero is notable", () => {
    expect(stockState(1, 0)).toBe("ok");
    expect(stockState(1, null)).toBe("ok");
  });
});

describe("reasonAllows", () => {
  test("a sale can only remove stock", () => {
    // A "Sold" that ADDS is a mis-click; letting it through makes the ledger
    // untrustworthy, which is the only thing the ledger has going for it.
    expect(reasonAllows("sold", -3)).toBe(true);
    expect(reasonAllows("sold", 3)).toBe(false);
  });

  test("a receipt can only add", () => {
    expect(reasonAllows("received", 5)).toBe(true);
    expect(reasonAllows("received", -5)).toBe(false);
  });

  test("a stock count goes either way", () => {
    expect(reasonAllows("count", 4)).toBe(true);
    expect(reasonAllows("count", -4)).toBe(true);
  });

  test("zero and unknown reasons are refused", () => {
    expect(reasonAllows("received", 0)).toBe(false);
    expect(reasonAllows("nonsense", 5)).toBe(false);
  });

  test("every reason declares a direction", () => {
    expect(REASONS.every((r) => ["in", "out", "either"].includes(r.direction))).toBe(true);
    expect(isValidReason("damaged")).toBe(true);
    expect(isValidReason("Damaged")).toBe(false);
  });
});

describe("summarize", () => {
  const products = [
    product({ id: "ok", sku: "A", reorderPoint: 5, unitCostCents: 1000 }),
    product({ id: "low", sku: "B", reorderPoint: 5, unitCostCents: 500 }),
    product({ id: "out", sku: "C", reorderPoint: 5, unitCostCents: 200 }),
    product({ id: "gone", sku: "D", archived: true, unitCostCents: 9999 }),
  ];
  const movements = [
    move({ id: "1", productId: "ok", delta: 20 }),
    move({ id: "2", productId: "low", delta: 3 }),
    move({ id: "3", productId: "gone", delta: 50 }),
  ];

  test("counts active SKUs and units", () => {
    const s = summarize(products, movements);
    expect(s.skuCount).toBe(3);
    expect(s.unitCount).toBe(23);
  });

  test("values stock at COST", () => {
    // 20 x $10.00 + 3 x $5.00
    expect(summarize(products, movements).valuationCents).toBe(21_500);
  });

  test("archived lines are excluded entirely", () => {
    // Their history stays for auditing, but they aren't stock you hold.
    const s = summarize(products, movements);
    expect(s.unitCount).not.toBe(73);
    expect(s.valuationCents).toBeLessThan(500_000);
  });

  test("flags low and out separately", () => {
    const s = summarize(products, movements);
    expect(s.lowCount).toBe(1);
    expect(s.outCount).toBe(1);
  });

  test("negative stock doesn't credit the valuation", () => {
    // It's a data error to surface, not value to subtract.
    const s = summarize(
      [product({ id: "bad", unitCostCents: 1000, reorderPoint: 0 })],
      [move({ productId: "bad", delta: -4 })],
    );
    expect(s.valuationCents).toBe(0);
    expect(s.unitCount).toBe(0);
  });

  test("an empty shelf is zeroes", () => {
    expect(summarize([], [])).toEqual({
      skuCount: 0,
      unitCount: 0,
      valuationCents: 0,
      lowCount: 0,
      outCount: 0,
    });
  });
});

describe("needsReorder", () => {
  test("lists low and out, most urgent first", () => {
    const products = [
      product({ id: "ok", reorderPoint: 2 }),
      product({ id: "low", reorderPoint: 5 }),
      product({ id: "out", reorderPoint: 5 }),
    ];
    const movements = [
      move({ id: "1", productId: "ok", delta: 20 }),
      move({ id: "2", productId: "low", delta: 3 }),
    ];
    expect(needsReorder(products, movements).map((r) => r.product.id)).toEqual([
      "out",
      "low",
    ]);
  });

  test("excludes archived products", () => {
    expect(
      needsReorder([product({ id: "gone", archived: true, reorderPoint: 5 })], []),
    ).toEqual([]);
  });
});

describe("formatting", () => {
  test("money is exact", () => {
    expect(money(21_500)).toBe("$215.00");
    expect(money(5)).toBe("$0.05");
    expect(money(null)).toBe("$0.00");
  });

  test("the sign is always shown on a delta", () => {
    expect(signedQuantity(5)).toBe("+5");
    expect(signedQuantity(-3)).toBe("−3");
    expect(signedQuantity(0)).toBe("0");
  });
});

describe("parsing", () => {
  test("amounts become cents", () => {
    expect(parseAmount("9.40")).toBe(940);
    expect(parseAmount("$1,234")).toBe(123_400);
    expect(parseAmount("abc")).toBeNull();
    expect(parseAmount("")).toBeNull();
  });

  test("counts are whole units only", () => {
    // Half a physical thing is a unit-of-measure mistake, not a quantity.
    expect(parseCount("12")).toBe(12);
    expect(parseCount("-3")).toBe(-3);
    expect(parseCount("1.5")).toBeNull();
    expect(parseCount("abc")).toBeNull();
    expect(parseCount("")).toBeNull();
  });
});
