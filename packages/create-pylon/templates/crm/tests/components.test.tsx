import { afterEach, describe, expect, test } from "bun:test";
// This project uses the classic JSX transform, so .tsx tests import React.
import React from "react";
import { cleanup, fireEvent, render, screen, within } from "@testing-library/react";
import { PipelineBoard } from "../components/pipeline-board";
import { DataTable } from "../components/data-table";
import { CommandPalette } from "../components/command-palette";
import { EmptyState } from "../components/empty-state";
import type { Deal } from "../lib/pipeline";

afterEach(cleanup);

// Tier 2: components. No mocking needed — they take data as props and report
// changes through callbacks, because the container owns `db`.

const deals: Deal[] = [
  { id: "d1", title: "Fleet dispatch rollout", value: 48_000, stage: "proposal" },
  { id: "d2", title: "Class booking system", value: 7_400, stage: "lead" },
];

const noName = () => null;

describe("PipelineBoard", () => {
  test("renders a column per board stage", () => {
    render(
      <PipelineBoard
        deals={deals}
        companyName={noName}
        ownerName={noName}
        onMove={() => {}}
        onOpen={() => {}}
      />,
    );
    for (const label of ["Lead", "Qualified", "Proposal", "Won"]) {
      expect(screen.getByLabelText(label)).toBeDefined();
    }
    // Lost is real in the model but deliberately not a column.
    expect(screen.queryByLabelText("Lost")).toBeNull();
  });

  test("shows each column's total in its header", () => {
    render(
      <PipelineBoard
        deals={deals}
        companyName={noName}
        ownerName={noName}
        onMove={() => {}}
        onOpen={() => {}}
      />,
    );
    // Scoped to the column, because the same figure also appears on the card —
    // a bare getByText would match twice and throw.
    const proposal = screen.getByLabelText("Proposal");
    expect(within(proposal).getAllByText("$48K").length).toBeGreaterThan(0);

    const lead = screen.getByLabelText("Lead");
    expect(within(lead).getAllByText("$7,400").length).toBeGreaterThan(0);

    // An empty column still totals, rather than showing nothing.
    const won = screen.getByLabelText("Won");
    expect(within(won).getByText("$0")).toBeDefined();
  });

  test("opening a card reports the deal id", () => {
    const opened: string[] = [];
    render(
      <PipelineBoard
        deals={deals}
        companyName={noName}
        ownerName={noName}
        onMove={() => {}}
        onOpen={(id) => opened.push(id)}
      />,
    );
    fireEvent.click(screen.getByText("Fleet dispatch rollout"));
    expect(opened).toEqual(["d1"]);
  });

  test("dropping on another column reports the move", () => {
    const moves: Array<[string, string]> = [];
    render(
      <PipelineBoard
        deals={deals}
        companyName={noName}
        ownerName={noName}
        onMove={(id, stage) => moves.push([id, stage])}
        onOpen={() => {}}
      />,
    );
    const data = new Map<string, string>();
    const dataTransfer = {
      setData: (k: string, v: string) => data.set(k, v),
      getData: (k: string) => data.get(k) ?? "",
      effectAllowed: "",
      dropEffect: "",
    };
    fireEvent.dragStart(screen.getByText("Class booking system"), { dataTransfer });
    fireEvent.drop(screen.getByLabelText("Won"), { dataTransfer });
    expect(moves).toEqual([["d2", "won"]]);
  });

  test("dropping a card back on its own column writes nothing", () => {
    // A jittery drag shouldn't produce a history entry saying it moved.
    const moves: Array<[string, string]> = [];
    render(
      <PipelineBoard
        deals={deals}
        companyName={noName}
        ownerName={noName}
        onMove={(id, stage) => moves.push([id, stage])}
        onOpen={() => {}}
      />,
    );
    const data = new Map<string, string>();
    const dataTransfer = {
      setData: (k: string, v: string) => data.set(k, v),
      getData: (k: string) => data.get(k) ?? "",
      effectAllowed: "",
      dropEffect: "",
    };
    fireEvent.dragStart(screen.getByText("Class booking system"), { dataTransfer });
    fireEvent.drop(screen.getByLabelText("Lead"), { dataTransfer });
    expect(moves).toEqual([]);
  });
});

describe("DataTable", () => {
  const rows = [{ id: "1", name: "Acme" }];

  test("renders a cell per column", () => {
    render(
      <DataTable
        rows={rows}
        columns={[{ key: "name", header: "Company", cell: (r) => r.name }]}
      />,
    );
    expect(screen.getByRole("table")).toBeDefined();
    expect(screen.getByText("Acme")).toBeDefined();
  });

  test("shows the empty state instead of a bare header", () => {
    render(
      <DataTable
        rows={[]}
        columns={[{ key: "name", header: "Company", cell: () => null }]}
        empty={<EmptyState title="No companies yet" />}
      />,
    );
    expect(screen.queryByRole("table")).toBeNull();
    expect(screen.getByText("No companies yet")).toBeDefined();
  });

  test("clicking a row reports it", () => {
    const clicked: string[] = [];
    render(
      <DataTable
        rows={rows}
        columns={[{ key: "name", header: "Company", cell: (r) => r.name }]}
        onRowClick={(row) => clicked.push(row.id)}
      />,
    );
    fireEvent.click(screen.getByText("Acme"));
    expect(clicked).toEqual(["1"]);
  });
});

describe("CommandPalette", () => {
  const items = [
    { id: "1", type: "deal" as const, title: "Fleet dispatch", href: "/d/1" },
    { id: "2", type: "company" as const, title: "Acme Inc", href: "/c/2" },
  ];

  test("renders nothing when closed", () => {
    const { container } = render(
      <CommandPalette open={false} items={items} onClose={() => {}} onSelect={() => {}} />,
    );
    expect(container.firstChild).toBeNull();
  });

  test("filters as you type", () => {
    render(
      <CommandPalette open items={items} onClose={() => {}} onSelect={() => {}} />,
    );
    fireEvent.change(screen.getByLabelText(/search deals/i), {
      target: { value: "acme" },
    });
    expect(screen.getByText("Acme Inc")).toBeDefined();
    expect(screen.queryByText("Fleet dispatch")).toBeNull();
  });

  test("Enter selects the highlighted row and closes", () => {
    const selected: string[] = [];
    let closed = false;
    render(
      <CommandPalette
        open
        items={items}
        onClose={() => {
          closed = true;
        }}
        onSelect={(item) => selected.push(item.id)}
      />,
    );
    const input = screen.getByLabelText(/search deals/i);
    fireEvent.change(input, { target: { value: "fleet" } });
    fireEvent.keyDown(input, { key: "Enter" });
    expect(selected).toEqual(["1"]);
    expect(closed).toBe(true);
  });

  test("Escape closes without selecting", () => {
    const selected: string[] = [];
    let closed = false;
    render(
      <CommandPalette
        open
        items={items}
        onClose={() => {
          closed = true;
        }}
        onSelect={(item) => selected.push(item.id)}
      />,
    );
    fireEvent.keyDown(screen.getByLabelText(/search deals/i), { key: "Escape" });
    expect(closed).toBe(true);
    expect(selected).toEqual([]);
  });

  test("actions show only with an empty query", () => {
    render(
      <CommandPalette
        open
        items={items}
        actions={[{ id: "new", label: "New deal", run: () => {} }]}
        onClose={() => {}}
        onSelect={() => {}}
      />,
    );
    expect(screen.getByText("New deal")).toBeDefined();
    fireEvent.change(screen.getByLabelText(/search deals/i), {
      target: { value: "acme" },
    });
    expect(screen.queryByText("New deal")).toBeNull();
  });
});
