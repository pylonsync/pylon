import { afterEach, describe, expect, test } from "bun:test";
// This project uses the classic JSX transform, so .tsx tests import React.
import React from "react";
import { cleanup, fireEvent, render, screen, within } from "@testing-library/react";
import { LineItems } from "../components/line-items";
import { TotalsPanel } from "../components/totals-panel";
import { PaymentDialog } from "../components/payment-dialog";
import { StatusBadge } from "../components/status-badge";
import { totals, type LineItem } from "../lib/billing";

afterEach(cleanup);

// Tier 2: components. No mocking needed — they take data as props and report
// changes through callbacks, because the container owns `db`.

const items: LineItem[] = [
  {
    id: "l1",
    invoiceId: "i1",
    description: "Senior engineering",
    quantityMilli: 32_000,
    unitPriceCents: 16_500,
  },
];

describe("LineItems", () => {
  test("renders the line with its computed amount", () => {
    render(<LineItems items={items} editable onAdd={() => {}} onRemove={() => {}} />);
    expect(screen.getByText("Senior engineering")).toBeDefined();
    expect(screen.getByText("32")).toBeDefined();
    expect(screen.getByText("$5,280.00")).toBeDefined();
  });

  test("adding parses the typed quantity and price into integers", () => {
    const added: unknown[] = [];
    render(<LineItems items={[]} editable onAdd={(d) => added.push(d)} onRemove={() => {}} />);
    fireEvent.change(screen.getByLabelText("Description"), {
      target: { value: "Training" },
    });
    fireEvent.change(screen.getByLabelText("Quantity"), { target: { value: "1.5" } });
    fireEvent.change(screen.getByLabelText("Unit price"), { target: { value: "450.50" } });
    fireEvent.click(screen.getByRole("button", { name: /add/i }));
    expect(added).toEqual([
      { description: "Training", quantityMilli: 1500, unitPriceCents: 45_050 },
    ]);
  });

  test("an unreadable price can't be added", () => {
    // Silently billing $0.00 for a typo is the failure this prevents.
    const added: unknown[] = [];
    render(<LineItems items={[]} editable onAdd={(d) => added.push(d)} onRemove={() => {}} />);
    fireEvent.change(screen.getByLabelText("Description"), { target: { value: "X" } });
    fireEvent.change(screen.getByLabelText("Unit price"), { target: { value: "abc" } });
    fireEvent.click(screen.getByRole("button", { name: /add/i }));
    expect(added).toEqual([]);
  });

  test("a sent invoice hides the editing controls entirely", () => {
    // It's a document someone is paying against; editing desyncs it from their copy.
    render(
      <LineItems items={items} editable={false} onAdd={() => {}} onRemove={() => {}} />,
    );
    expect(screen.queryByLabelText("Description")).toBeNull();
    expect(screen.queryByRole("button", { name: /remove/i })).toBeNull();
  });

  test("removing reports the line id", () => {
    const removed: string[] = [];
    render(
      <LineItems items={items} editable onAdd={() => {}} onRemove={(id) => removed.push(id)} />,
    );
    fireEvent.click(screen.getByRole("button", { name: /remove senior engineering/i }));
    expect(removed).toEqual(["l1"]);
  });

  test("no lines shows a placeholder row, not an empty table", () => {
    render(<LineItems items={[]} editable onAdd={() => {}} onRemove={() => {}} />);
    expect(screen.getByText("No lines yet.")).toBeDefined();
  });
});

describe("TotalsPanel", () => {
  test("shows subtotal, tax, total and balance in reading order", () => {
    const t = totals(
      { id: "i1", number: "INV-1", status: "sent", taxRateBps: 875 },
      items,
      [{ id: "p", invoiceId: "i1", amountCents: 100_000 }],
    );
    const { container } = render(<TotalsPanel totals={t} taxRateBps={875} />);
    const text = container.textContent ?? "";
    expect(text).toContain("$5,280.00");
    expect(text).toContain("Tax (8.75%)");
    expect(text).toContain("Balance due");
  });

  test("hides the tax row when there's no tax", () => {
    const t = totals({ id: "i1", number: "INV-1", status: "sent" }, items, []);
    const { container } = render(<TotalsPanel totals={t} taxRateBps={0} />);
    expect(container.textContent).not.toContain("Tax");
  });
});

describe("PaymentDialog", () => {
  test("defaults to the outstanding balance", () => {
    // Retyping the figure is how a typo gets in.
    render(
      <PaymentDialog open balanceCents={52_800} onOpenChange={() => {}} onRecord={() => {}} />,
    );
    expect((screen.getByLabelText("Amount") as HTMLInputElement).value).toBe("528.00");
  });

  test("records the parsed amount in cents", () => {
    const recorded: unknown[] = [];
    render(
      <PaymentDialog
        open
        balanceCents={52_800}
        onOpenChange={() => {}}
        onRecord={(cents, method) => recorded.push([cents, method])}
      />,
    );
    fireEvent.change(screen.getByLabelText("Amount"), { target: { value: "250.25" } });
    fireEvent.click(screen.getByRole("button", { name: /record payment/i }));
    expect(recorded).toEqual([[25_025, "bank"]]);
  });

  test("refuses more than the balance", () => {
    const recorded: unknown[] = [];
    render(
      <PaymentDialog
        open
        balanceCents={1000}
        onOpenChange={() => {}}
        onRecord={(cents) => recorded.push(cents)}
      />,
    );
    fireEvent.change(screen.getByLabelText("Amount"), { target: { value: "50" } });
    fireEvent.click(screen.getByRole("button", { name: /record payment/i }));
    expect(recorded).toEqual([]);
    expect(screen.getByText(/more than the balance/i)).toBeDefined();
  });

  test("renders nothing when closed", () => {
    const { container } = render(
      <PaymentDialog
        open={false}
        balanceCents={1000}
        onOpenChange={() => {}}
        onRecord={() => {}}
      />,
    );
    expect(container.firstChild).toBeNull();
  });
});

describe("StatusBadge", () => {
  test("labels the derived overdue state", () => {
    render(<StatusBadge status="overdue" />);
    expect(screen.getByText("Overdue")).toBeDefined();
  });

  test("labels the stored ones", () => {
    const { container } = render(
      <>
        <StatusBadge status="draft" />
        <StatusBadge status="paid" />
      </>,
    );
    expect(within(container).getByText("Draft")).toBeDefined();
    expect(within(container).getByText("Paid")).toBeDefined();
  });
});
