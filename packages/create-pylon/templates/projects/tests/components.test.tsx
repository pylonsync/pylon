import { afterEach, describe, expect, test } from "bun:test";
// This project uses the classic JSX transform, so .tsx tests import React.
import React from "react";
import { cleanup, fireEvent, render, screen, within } from "@testing-library/react";
import { TaskBoard } from "../components/task-board";
import { TimeDialog } from "../components/time-dialog";
import { BudgetBar } from "../components/budget-bar";
import type { Task } from "../lib/work";

afterEach(cleanup);

// Tier 2: components. No mocking needed — they take data as props and report
// changes through callbacks, because the container owns `db`.

const tasks: Task[] = [
  { id: "t1", projectId: "p1", title: "Dispatcher board UI", status: "doing" },
  { id: "t2", projectId: "p1", title: "Weekly report", status: "todo" },
];

const noName = () => null;
const noMinutes = () => 0;

describe("TaskBoard", () => {
  test("renders every column", () => {
    render(
      <TaskBoard
        tasks={tasks}
        assigneeName={noName}
        minutesFor={noMinutes}
        onMove={() => {}}
        onOpen={() => {}}
      />,
    );
    for (const label of ["To do", "In progress", "Review", "Done"]) {
      expect(screen.getByLabelText(label)).toBeDefined();
    }
  });

  test("dropping on another column reports the move", () => {
    const moves: Array<[string, string]> = [];
    render(
      <TaskBoard
        tasks={tasks}
        assigneeName={noName}
        minutesFor={noMinutes}
        onMove={(id, status) => moves.push([id, status])}
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
    fireEvent.dragStart(screen.getByText("Weekly report"), { dataTransfer });
    fireEvent.drop(screen.getByLabelText("Done"), { dataTransfer });
    expect(moves).toEqual([["t2", "done"]]);
  });

  test("dropping back on the same column writes nothing", () => {
    const moves: unknown[] = [];
    render(
      <TaskBoard
        tasks={tasks}
        assigneeName={noName}
        minutesFor={noMinutes}
        onMove={() => moves.push(1)}
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
    fireEvent.dragStart(screen.getByText("Weekly report"), { dataTransfer });
    fireEvent.drop(screen.getByLabelText("To do"), { dataTransfer });
    expect(moves).toEqual([]);
  });

  test("shows a column's logged total", () => {
    render(
      <TaskBoard
        tasks={tasks}
        assigneeName={noName}
        minutesFor={(id) => (id === "t1" ? 600 : 0)}
        onMove={() => {}}
        onOpen={() => {}}
      />,
    );
    expect(within(screen.getByLabelText("In progress")).getAllByText("10h").length)
      .toBeGreaterThan(0);
  });

  test("flags a card that has blown its estimate", () => {
    // Showing "3h / 8h" on every card is noise; an overrun is worth seeing.
    render(
      <TaskBoard
        tasks={[{ ...tasks[0], estimateMinutes: 120 }]}
        assigneeName={noName}
        minutesFor={() => 300}
        onMove={() => {}}
        onOpen={() => {}}
      />,
    );
    expect(screen.getByText("5h / 2h")).toBeDefined();
  });

  test("opening a card reports the task id", () => {
    const opened: string[] = [];
    render(
      <TaskBoard
        tasks={tasks}
        assigneeName={noName}
        minutesFor={noMinutes}
        onMove={() => {}}
        onOpen={(id) => opened.push(id)}
      />,
    );
    fireEvent.click(screen.getByText("Dispatcher board UI"));
    expect(opened).toEqual(["t1"]);
  });
});

describe("TimeDialog", () => {
  test("accepts the formats people actually type", () => {
    const logged: number[] = [];
    render(
      <TimeDialog
        open
        taskTitle="Dispatcher board UI"
        onOpenChange={() => {}}
        onLog={(minutes) => logged.push(minutes)}
      />,
    );
    fireEvent.change(screen.getByLabelText("Time"), { target: { value: "1h30" } });
    fireEvent.click(screen.getByRole("button", { name: /log time/i }));
    expect(logged).toEqual([90]);
  });

  test("previews what it parsed, so a typo is visible before submitting", () => {
    render(
      <TimeDialog open taskTitle="X" onOpenChange={() => {}} onLog={() => {}} />,
    );
    fireEvent.change(screen.getByLabelText("Time"), { target: { value: "90" } });
    expect(screen.getByText("1h 30m")).toBeDefined();
  });

  test("refuses an unparseable value", () => {
    const logged: number[] = [];
    render(
      <TimeDialog open taskTitle="X" onOpenChange={() => {}} onLog={(m) => logged.push(m)} />,
    );
    fireEvent.change(screen.getByLabelText("Time"), { target: { value: "soon" } });
    fireEvent.click(screen.getByRole("button", { name: /log time/i }));
    expect(logged).toEqual([]);
    expect(screen.getByText(/try 90/i)).toBeDefined();
  });

  test("refuses more than a day", () => {
    // A typo on a timesheet becomes a typo on an invoice.
    const logged: number[] = [];
    render(
      <TimeDialog open taskTitle="X" onOpenChange={() => {}} onLog={(m) => logged.push(m)} />,
    );
    fireEvent.change(screen.getByLabelText("Time"), { target: { value: "30h" } });
    fireEvent.click(screen.getByRole("button", { name: /log time/i }));
    expect(logged).toEqual([]);
  });

  test("renders nothing when closed", () => {
    const { container } = render(
      <TimeDialog open={false} taskTitle="X" onOpenChange={() => {}} onLog={() => {}} />,
    );
    expect(container.firstChild).toBeNull();
  });
});

describe("BudgetBar", () => {
  test("calls out an overrun with the amount", () => {
    render(<BudgetBar budgetMinutes={2400} loggedMinutes={2760} />);
    expect(screen.getByText(/over budget by 6h/i)).toBeDefined();
  });

  test("shows billable value when there's a rate", () => {
    render(
      <BudgetBar budgetMinutes={2400} loggedMinutes={600} hourlyRateCents={16_500} />,
    );
    expect(screen.getByText(/\$1,650\.00 billable/)).toBeDefined();
  });

  test("says so when there's no budget rather than drawing an empty bar", () => {
    render(<BudgetBar budgetMinutes={0} loggedMinutes={600} />);
    expect(screen.getByText("No budget set")).toBeDefined();
  });
});
