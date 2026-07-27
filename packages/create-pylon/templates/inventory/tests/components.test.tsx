import { afterEach, describe, expect, test } from "bun:test";
// This project uses the classic JSX transform, so .tsx tests import React.
import React from "react";
import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { StockLevel } from "../components/stock-level";
import { MovementDialog } from "../components/movement-dialog";

afterEach(cleanup);

// Tier 2: components. No mocking needed — they take data as props and report
// changes through callbacks, because the container owns `db`.

const products = [
  { id: "p1", sku: "CFE-001", name: "House Blend" },
  { id: "p2", sku: "PKG-011", name: "Lids" },
];

describe("StockLevel", () => {
  test("marks an empty shelf", () => {
    render(<StockLevel quantity={0} reorderPoint={5} />);
    expect(screen.getByText("out")).toBeDefined();
  });

  test("marks a level at the reorder point", () => {
    render(<StockLevel quantity={5} reorderPoint={5} />);
    expect(screen.getByText("low")).toBeDefined();
  });

  test("a healthy level is just the number", () => {
    const { container } = render(<StockLevel quantity={41} reorderPoint={5} />);
    expect(container.textContent).toBe("41");
  });
});

describe("MovementDialog", () => {
  const level = (id: string) => (id === "p1" ? 12 : 0);

  test("a sale is recorded as a negative delta from a positive input", () => {
    // Asking someone to type "-3" is how you get a "+3" sale.
    const recorded: unknown[] = [];
    render(
      <MovementDialog
        open
        products={products}
        currentLevel={level}
        onOpenChange={() => {}}
        onRecord={(id, delta, reason) => recorded.push([id, delta, reason])}
      />,
    );
    fireEvent.change(screen.getByLabelText("Product"), { target: { value: "p1" } });
    fireEvent.change(screen.getByLabelText("Reason"), { target: { value: "sold" } });
    fireEvent.change(screen.getByLabelText("Quantity"), { target: { value: "3" } });
    fireEvent.click(screen.getByRole("button", { name: /^record$/i }));
    expect(recorded).toEqual([["p1", -3, "sold"]]);
  });

  test("a receipt is positive", () => {
    const recorded: unknown[] = [];
    render(
      <MovementDialog
        open
        products={products}
        currentLevel={level}
        onOpenChange={() => {}}
        onRecord={(id, delta) => recorded.push(delta)}
      />,
    );
    fireEvent.change(screen.getByLabelText("Product"), { target: { value: "p1" } });
    fireEvent.change(screen.getByLabelText("Quantity"), { target: { value: "10" } });
    fireEvent.click(screen.getByRole("button", { name: /^record$/i }));
    expect(recorded).toEqual([10]);
  });

  test("a stock count sets the level, so it sends the difference", () => {
    // Counting a shelf produces "there are 9", not "adjust by -3".
    const recorded: unknown[] = [];
    render(
      <MovementDialog
        open
        products={products}
        currentLevel={level}
        onOpenChange={() => {}}
        onRecord={(id, delta) => recorded.push(delta)}
      />,
    );
    fireEvent.change(screen.getByLabelText("Product"), { target: { value: "p1" } });
    fireEvent.change(screen.getByLabelText("Reason"), { target: { value: "count" } });
    fireEvent.change(screen.getByLabelText("Counted"), { target: { value: "9" } });
    fireEvent.click(screen.getByRole("button", { name: /^record$/i }));
    expect(recorded).toEqual([-3]);
  });

  test("refuses to take stock below zero", () => {
    const recorded: unknown[] = [];
    render(
      <MovementDialog
        open
        products={products}
        currentLevel={level}
        onOpenChange={() => {}}
        onRecord={(id, delta) => recorded.push(delta)}
      />,
    );
    fireEvent.change(screen.getByLabelText("Product"), { target: { value: "p2" } });
    fireEvent.change(screen.getByLabelText("Reason"), { target: { value: "sold" } });
    fireEvent.change(screen.getByLabelText("Quantity"), { target: { value: "1" } });
    fireEvent.click(screen.getByRole("button", { name: /^record$/i }));
    expect(recorded).toEqual([]);
    expect(screen.getByText(/below zero/i)).toBeDefined();
  });

  test("shows what the level will become", () => {
    render(
      <MovementDialog
        open
        products={products}
        currentLevel={level}
        onOpenChange={() => {}}
        onRecord={() => {}}
      />,
    );
    fireEvent.change(screen.getByLabelText("Product"), { target: { value: "p1" } });
    fireEvent.change(screen.getByLabelText("Quantity"), { target: { value: "5" } });
    expect(screen.getByText(/On hand 12 → 17/)).toBeDefined();
  });

  test("nothing is recorded without a product", () => {
    const recorded: unknown[] = [];
    render(
      <MovementDialog
        open
        products={products}
        currentLevel={level}
        onOpenChange={() => {}}
        onRecord={() => recorded.push(1)}
      />,
    );
    fireEvent.change(screen.getByLabelText("Quantity"), { target: { value: "5" } });
    fireEvent.click(screen.getByRole("button", { name: /^record$/i }));
    expect(recorded).toEqual([]);
  });

  test("renders nothing when closed", () => {
    const { container } = render(
      <MovementDialog
        open={false}
        products={products}
        currentLevel={level}
        onOpenChange={() => {}}
        onRecord={() => {}}
      />,
    );
    expect(container.firstChild).toBeNull();
  });
});
