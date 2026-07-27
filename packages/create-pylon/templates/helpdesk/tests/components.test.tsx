import { afterEach, describe, expect, test } from "bun:test";
// This project uses the classic JSX transform, so .tsx tests import React.
import React from "react";
import { cleanup, fireEvent, render, screen, within } from "@testing-library/react";
import { TicketList } from "../components/ticket-list";
import { Thread } from "../components/thread";
import { SlaIndicator } from "../components/sla-indicator";
import type { Ticket } from "../lib/tickets";

afterEach(cleanup);

// Tier 2: components. No mocking needed — they take data as props and report
// changes through callbacks, because the container owns `db`.

const NOW = Date.parse("2026-07-27T12:00:00Z");
const hoursAgo = (h: number) => new Date(NOW - h * 3_600_000).toISOString();
const noName = () => null;

const tickets: Ticket[] = [
  {
    id: "breached",
    subject: "Export is timing out",
    status: "open",
    priority: "urgent",
    createdAt: hoursAgo(3),
  },
  {
    id: "calm",
    subject: "Mobile layout is off",
    status: "open",
    priority: "low",
    createdAt: hoursAgo(1),
    firstRespondedAt: hoursAgo(1),
  },
];

describe("TicketList", () => {
  test("puts the breached ticket at the top", () => {
    // The queue's whole job is to answer "what next" — newest-first would bury
    // the overdue one.
    render(
      <TicketList
        tickets={tickets}
        customerName={noName}
        assigneeName={noName}
        now={NOW}
        onSelect={() => {}}
      />,
    );
    const rows = screen.getAllByRole("button");
    expect(within(rows[0]).getByText("Export is timing out")).toBeDefined();
  });

  test("marks a ticket with no agent as unassigned", () => {
    render(
      <TicketList
        tickets={tickets}
        customerName={noName}
        assigneeName={noName}
        now={NOW}
        onSelect={() => {}}
      />,
    );
    expect(screen.getAllByText("Unassigned").length).toBe(2);
  });

  test("selecting a row reports its id", () => {
    const picked: string[] = [];
    render(
      <TicketList
        tickets={tickets}
        customerName={noName}
        assigneeName={noName}
        now={NOW}
        onSelect={(id) => picked.push(id)}
      />,
    );
    fireEvent.click(screen.getByText("Mobile layout is off"));
    expect(picked).toEqual(["calm"]);
  });
});

describe("SlaIndicator", () => {
  test("shows how far past the window a breach is", () => {
    const { container } = render(
      <SlaIndicator ticket={tickets[0]} now={NOW} />,
    );
    expect(container.textContent).toContain("over");
  });

  test("renders nothing once answered", () => {
    // A badge on every row is a badge that means nothing.
    const { container } = render(<SlaIndicator ticket={tickets[1]} now={NOW} />);
    expect(container.firstChild).toBeNull();
  });

  test("renders nothing for a closed ticket", () => {
    const { container } = render(
      <SlaIndicator
        ticket={{ ...tickets[0], status: "closed" }}
        now={NOW}
      />,
    );
    expect(container.firstChild).toBeNull();
  });
});

describe("Thread", () => {
  const messages = [
    { id: "m1", body: "It broke", fromCustomer: true, createdAt: hoursAgo(3) },
    { id: "m2", body: "Looking now", fromCustomer: false, createdAt: hoursAgo(2) },
    { id: "m3", body: "Their MX is wrong", fromCustomer: false, internal: true, createdAt: hoursAgo(1) },
  ];

  test("renders oldest first", () => {
    render(
      <Thread
        messages={messages}
        authorName={() => "Agent"}
        customerName="Dana"
        now={NOW}
        onSend={() => {}}
      />,
    );
    const articles = screen.getAllByRole("article");
    expect(within(articles[0]).getByText("It broke")).toBeDefined();
  });

  test("marks an internal note so it can't be mistaken for a reply", () => {
    render(
      <Thread
        messages={messages}
        authorName={() => "Agent"}
        customerName="Dana"
        now={NOW}
        onSend={() => {}}
      />,
    );
    // Scoped to the message: the composer's toggle carries the same label, so a
    // bare getByText would match twice and throw.
    const notes = screen
      .getAllByRole("article")
      .filter((article) => within(article).queryByText("Internal note"));
    expect(notes).toHaveLength(1);
    expect(within(notes[0]).getByText("Their MX is wrong")).toBeDefined();
  });

  test("sends a public reply by default", () => {
    const sent: Array<[string, boolean]> = [];
    render(
      <Thread
        messages={[]}
        authorName={() => "Agent"}
        customerName="Dana"
        now={NOW}
        onSend={(body, internal) => sent.push([body, internal])}
      />,
    );
    fireEvent.change(screen.getByLabelText("Reply"), {
      target: { value: "On it" },
    });
    fireEvent.click(screen.getByRole("button", { name: /send reply/i }));
    expect(sent).toEqual([["On it", false]]);
  });

  test("the internal toggle switches the composer and the flag", () => {
    // Sending an internal note to the customer is the expensive mistake here,
    // so the mode has to be visible AND actually carried through.
    const sent: Array<[string, boolean]> = [];
    render(
      <Thread
        messages={[]}
        authorName={() => "Agent"}
        customerName="Dana"
        now={NOW}
        onSend={(body, internal) => sent.push([body, internal])}
      />,
    );
    fireEvent.click(screen.getByLabelText("Internal note"));
    fireEvent.change(screen.getByLabelText("Internal note", { selector: "textarea" }), {
      target: { value: "MX is broken" },
    });
    fireEvent.click(screen.getByRole("button", { name: /add note/i }));
    expect(sent).toEqual([["MX is broken", true]]);
  });

  test("an empty reply is not sent", () => {
    const sent: unknown[] = [];
    render(
      <Thread
        messages={[]}
        authorName={() => "Agent"}
        customerName="Dana"
        now={NOW}
        onSend={() => sent.push(1)}
      />,
    );
    fireEvent.change(screen.getByLabelText("Reply"), { target: { value: "   " } });
    fireEvent.click(screen.getByRole("button", { name: /send reply/i }));
    expect(sent).toEqual([]);
  });
});
