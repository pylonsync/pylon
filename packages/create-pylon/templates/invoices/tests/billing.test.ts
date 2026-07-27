import { describe, expect, test } from "bun:test";
import {
  daysOverdue,
  displayStatus,
  isOverdue,
  isValidStatus,
  lineTotalCents,
  money,
  nextNumber,
  parseAmount,
  parseQuantity,
  quantity,
  summarize,
  totals,
  type Invoice,
  type LineItem,
  type Payment,
} from "../lib/billing";

const NOW = Date.parse("2026-07-27T12:00:00Z");
const at = (days: number) => new Date(NOW + days * 86_400_000).toISOString();

const invoice = (over: Partial<Invoice> = {}): Invoice => ({
  id: "i1",
  number: "INV-2026-0001",
  status: "sent",
  taxRateBps: 0,
  dueDate: at(10),
  ...over,
});

const line = (over: Partial<LineItem> = {}): LineItem => ({
  id: "l1",
  invoiceId: "i1",
  description: "Work",
  quantityMilli: 1000,
  unitPriceCents: 10_000,
  ...over,
});

describe("lineTotalCents", () => {
  test("multiplies quantity by unit price", () => {
    expect(lineTotalCents({ quantityMilli: 32_000, unitPriceCents: 16_500 })).toBe(528_000);
  });

  test("handles fractional quantities exactly", () => {
    // 1.5 hours at $165.00 — the case a float would get wrong.
    expect(lineTotalCents({ quantityMilli: 1500, unitPriceCents: 16_500 })).toBe(24_750);
  });

  test("rounds to a whole cent", () => {
    // 0.333 × $10.00 = $3.33
    expect(lineTotalCents({ quantityMilli: 333, unitPriceCents: 1000 })).toBe(333);
  });

  test("garbage is zero, not NaN", () => {
    expect(
      lineTotalCents({ quantityMilli: NaN as unknown as number, unitPriceCents: 100 }),
    ).toBe(0);
  });
});

describe("totals", () => {
  test("sums lines belonging to THIS invoice only", () => {
    const t = totals(
      invoice(),
      [line({ id: "a" }), line({ id: "b", invoiceId: "other", unitPriceCents: 999_999 })],
      [],
    );
    expect(t.subtotalCents).toBe(10_000);
  });

  test("applies tax to the rounded subtotal", () => {
    // 8.75% of $124.00 = $10.85
    const t = totals(
      invoice({ taxRateBps: 875 }),
      [line({ unitPriceCents: 12_400 })],
      [],
    );
    expect(t.subtotalCents).toBe(12_400);
    expect(t.taxCents).toBe(1085);
    expect(t.totalCents).toBe(13_485);
  });

  test("balance is total minus payments", () => {
    const payments: Payment[] = [{ id: "p1", invoiceId: "i1", amountCents: 4000 }];
    const t = totals(invoice(), [line()], payments);
    expect(t.paidCents).toBe(4000);
    expect(t.balanceCents).toBe(6000);
  });

  test("an overpayment floors the balance at zero", () => {
    // A credit is a decision to handle deliberately, not a negative balance
    // that quietly offsets the next invoice.
    const payments: Payment[] = [{ id: "p1", invoiceId: "i1", amountCents: 99_999 }];
    expect(totals(invoice(), [line()], payments).balanceCents).toBe(0);
  });

  test("no lines is zero across the board", () => {
    expect(totals(invoice(), [], [])).toEqual({
      subtotalCents: 0,
      taxCents: 0,
      totalCents: 0,
      paidCents: 0,
      balanceCents: 0,
    });
  });
});

describe("isOverdue / displayStatus", () => {
  test("sent, past due, and unpaid is overdue", () => {
    expect(isOverdue(invoice({ dueDate: at(-1) }), 5000, NOW)).toBe(true);
  });

  test("due today is not yet overdue", () => {
    // The customer has until the end of the day they were given.
    expect(isOverdue(invoice({ dueDate: at(0) }), 5000, NOW)).toBe(false);
  });

  test("a paid balance is never overdue", () => {
    expect(isOverdue(invoice({ dueDate: at(-30) }), 0, NOW)).toBe(false);
  });

  test("drafts and voids are never overdue", () => {
    expect(isOverdue(invoice({ status: "draft", dueDate: at(-30) }), 5000, NOW)).toBe(false);
    expect(isOverdue(invoice({ status: "void", dueDate: at(-30) }), 5000, NOW)).toBe(false);
  });

  test("no due date means nothing to be late for", () => {
    expect(isOverdue(invoice({ dueDate: null }), 5000, NOW)).toBe(false);
  });

  test("displayStatus surfaces the derived states", () => {
    expect(displayStatus(invoice({ dueDate: at(-3) }), 5000, NOW)).toBe("overdue");
    // Fully paid reads as paid even before someone changes the field.
    expect(displayStatus(invoice(), 0, NOW)).toBe("paid");
    expect(displayStatus(invoice({ status: "draft" }), 100, NOW)).toBe("draft");
    expect(displayStatus(invoice({ status: "void" }), 100, NOW)).toBe("void");
  });

  test("daysOverdue counts whole days", () => {
    expect(daysOverdue(invoice({ dueDate: at(-7) }), NOW)).toBe(7);
    expect(daysOverdue(invoice({ dueDate: null }), NOW)).toBeNull();
  });
});

describe("summarize", () => {
  const items: LineItem[] = [
    line({ id: "a", invoiceId: "late", unitPriceCents: 100_000 }),
    line({ id: "b", invoiceId: "open", unitPriceCents: 50_000 }),
    line({ id: "c", invoiceId: "done", unitPriceCents: 20_000 }),
    line({ id: "d", invoiceId: "draft", unitPriceCents: 30_000 }),
  ];
  const payments: Payment[] = [{ id: "p", invoiceId: "done", amountCents: 20_000 }];
  const invoices: Invoice[] = [
    invoice({ id: "late", dueDate: at(-5) }),
    invoice({ id: "open", dueDate: at(5) }),
    invoice({ id: "done", status: "paid" }),
    invoice({ id: "draft", status: "draft" }),
  ];

  test("outstanding covers every sent invoice, overdue only the late ones", () => {
    const s = summarize(invoices, items, payments, NOW);
    expect(s.outstandingCents).toBe(150_000);
    expect(s.overdueCents).toBe(100_000);
    expect(s.overdueCount).toBe(1);
  });

  test("collected counts payments regardless of status", () => {
    expect(summarize(invoices, items, payments, NOW).paidCents).toBe(20_000);
  });

  test("drafts are counted but never billed", () => {
    // A draft has real line items ($300) but isn't owed until it's sent —
    // including it would overstate what the business is waiting on.
    const s = summarize(invoices, items, payments, NOW);
    expect(s.draftCount).toBe(1);
    expect(s.outstandingCents).toBe(150_000);

    const withoutDraft = summarize(
      invoices.filter((i) => i.status !== "draft"),
      items,
      payments,
      NOW,
    );
    expect(withoutDraft.outstandingCents).toBe(s.outstandingCents);
  });

  test("an empty book is zeroes", () => {
    expect(summarize([], [], [], NOW)).toEqual({
      outstandingCents: 0,
      overdueCents: 0,
      paidCents: 0,
      draftCount: 0,
      overdueCount: 0,
    });
  });
});

describe("money", () => {
  test("shows exact amounts — an invoice never compacts", () => {
    expect(money(124_000)).toBe("$1,240.00");
    expect(money(5)).toBe("$0.05");
    expect(money(0)).toBe("$0.00");
    expect(money(1_234_567)).toBe("$12,345.67");
  });

  test("handles nothing and negatives", () => {
    expect(money(null)).toBe("$0.00");
    expect(money(-2500)).toBe("-$25.00");
  });
});

describe("quantity", () => {
  test("drops trailing zeroes", () => {
    expect(quantity(1000)).toBe("1");
    expect(quantity(1500)).toBe("1.5");
    expect(quantity(333)).toBe("0.333");
    expect(quantity(null)).toBe("0");
  });
});

describe("parseAmount", () => {
  test("reads what a person actually types", () => {
    expect(parseAmount("1,240.50")).toBe(124_050);
    expect(parseAmount("$1240.5")).toBe(124_050);
    expect(parseAmount("1240")).toBe(124_000);
    expect(parseAmount(" 12.34 ")).toBe(1234);
  });

  test("rejects nonsense rather than silently billing $0.00", () => {
    expect(parseAmount("")).toBeNull();
    expect(parseAmount("abc")).toBeNull();
    expect(parseAmount("1.2.3")).toBeNull();
    expect(parseAmount(".")).toBeNull();
  });
});

describe("parseQuantity", () => {
  test("reads decimals into thousandths", () => {
    expect(parseQuantity("1.5")).toBe(1500);
    expect(parseQuantity("32")).toBe(32_000);
  });

  test("rejects nonsense and negatives", () => {
    expect(parseQuantity("abc")).toBeNull();
    expect(parseQuantity("-1")).toBeNull();
    expect(parseQuantity("")).toBeNull();
  });
});

describe("nextNumber", () => {
  test("continues the year's series", () => {
    expect(nextNumber(["INV-2026-0001", "INV-2026-0007"], 2026)).toBe("INV-2026-0008");
  });

  test("starts a new year at one", () => {
    expect(nextNumber(["INV-2025-0042"], 2026)).toBe("INV-2026-0001");
  });

  test("ignores numbers it can't read", () => {
    expect(nextNumber(["INV-2026-abc", "", "INV-2026-0003"], 2026)).toBe("INV-2026-0004");
  });

  test("an empty book starts at one", () => {
    expect(nextNumber([], 2026)).toBe("INV-2026-0001");
  });

  test("pads so the series sorts lexically", () => {
    // The list view sorts on the string; unpadded numbers would put 10 before 9.
    expect(nextNumber(["INV-2026-0009"], 2026)).toBe("INV-2026-0010");
  });
});

describe("statuses", () => {
  test("overdue is derived, never a stored status", () => {
    expect(isValidStatus("overdue")).toBe(false);
    expect(isValidStatus("draft")).toBe(true);
    expect(isValidStatus("sent")).toBe(true);
    expect(isValidStatus("paid")).toBe(true);
    expect(isValidStatus("void")).toBe(true);
  });
});
