import { afterEach, expect, test } from "bun:test";
// This project uses the classic JSX transform, so .tsx tests import React.
import React from "react";
import { render, screen, cleanup } from "@testing-library/react";
import { Button } from "../components/ui/button";

afterEach(cleanup);

// A React component test. happy-dom + @testing-library/react are wired via
// bunfig.toml — render a component and assert on the DOM. For a component that
// reads Pylon data hooks (db.useQuery, callFn), mock the boundary first — see
// AGENTS.md → Testing (Tier 2).
test("Button renders its label", () => {
  render(<Button>Click me</Button>);
  expect(screen.getByRole("button", { name: "Click me" })).toBeDefined();
});
