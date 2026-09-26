import { afterEach, expect, mock, test } from "bun:test";
import React from "react";
import { cleanup, render, screen } from "@testing-library/react";

// The live count reads the WaitlistStat row. Mock the data boundary so the test
// controls what the "server" says.
let statRows: { id: string; count: number; updatedAt: string }[] = [];
let loading = false;
mock.module("@pylonsync/react", () => ({
  db: { useQuery: () => ({ data: statRows, loading }) },
  callFn: async () => ({ seeded: false }),
}));
mock.module("@pylonsync/client", () => ({
  EnsureGuest: ({ children }: { children: React.ReactNode }) => <>{children}</>,
}));
const { LiveCount } = await import("../app/(marketing)/waitlist-hero");

afterEach(() => {
  cleanup();
  statRows = [];
  loading = false;
});

test("shows the table's count, with no vanity baseline", () => {
  statRows = [{ id: "w", count: 42, updatedAt: "" }];
  render(<LiveCount importedCount={0} label="people on the list" labelOne="person on the list" />);
  expect(screen.getByTestId("live-count").textContent).toBe("42");
});

test("shows nothing, not a placeholder number, before the first sync", () => {
  loading = true;
  render(<LiveCount importedCount={0} label="people on the list" labelOne="person on the list" />);
  expect(screen.queryByTestId("live-count")).toBeNull();
});

test("uses the singular label for one signup", () => {
  statRows = [{ id: "w", count: 1, updatedAt: "" }];
  const { container } = render(
    <LiveCount importedCount={0} label="people on the list" labelOne="person on the list" />,
  );
  expect(container.textContent).toContain("1person on the list");
});

test("shows nothing while the list is empty", () => {
  statRows = [{ id: "w", count: 0, updatedAt: "" }];
  render(<LiveCount importedCount={0} label="people on the list" labelOne="person on the list" />);
  expect(screen.queryByTestId("live-count")).toBeNull();
});
