import { afterEach, expect, mock, test } from "bun:test";
import React from "react";
import { cleanup, render, screen } from "@testing-library/react";

// The live count reads the SubscriberCount row. Mock the data boundary so the
// test controls what the "server" says.
let rows: { id: string; count: number }[] = [];
let loading = false;
mock.module("@pylonsync/react", () => ({
  db: { useQuery: () => ({ data: rows, loading }) },
  callFn: async () => ({ seeded: false }),
}));
mock.module("@pylonsync/client", () => ({
  EnsureGuest: ({ children }: { children: React.ReactNode }) => <>{children}</>,
}));
const { LiveCounter } = await import("../app/(marketing)/newsletter-signup");

afterEach(() => {
  cleanup();
  rows = [];
  loading = false;
});

test("shows the table's count, with no vanity baseline", () => {
  rows = [{ id: "c", count: 42 }];
  render(<LiveCounter importedCount={0} label="designers subscribed" labelOne="designer subscribed" />);
  expect(screen.getByTestId("live-count").textContent).toBe("42 designers subscribed");
});

test("shows nothing before the first sync or while the list is empty", () => {
  loading = true;
  const { unmount } = render(<LiveCounter importedCount={0} label="designers subscribed" labelOne="designer subscribed" />);
  expect(screen.queryByTestId("live-count")).toBeNull();
  unmount();
  loading = false;
  rows = [{ id: "c", count: 0 }];
  render(<LiveCounter importedCount={0} label="designers subscribed" labelOne="designer subscribed" />);
  expect(screen.queryByTestId("live-count")).toBeNull();
});

test("uses the singular label for one subscriber", () => {
  rows = [{ id: "c", count: 1 }];
  render(<LiveCounter importedCount={0} label="designers subscribed" labelOne="designer subscribed" />);
  expect(screen.getByTestId("live-count").textContent).toBe("1 designer subscribed");
});
