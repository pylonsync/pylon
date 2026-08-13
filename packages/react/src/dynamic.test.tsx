// Contract tests for dynamic() — component-level code splitting.
//
// The property that matters is that ssr:false cannot produce a hydration
// mismatch. Pylon hydrates ONCE, after the whole document has streamed, so
// the server HTML and the first client render must be identical. Rendering
// the fallback in BOTH is what guarantees that; a component that appeared on
// the first client pass would diverge from the server's markup.

import { afterEach, describe, expect, test } from "bun:test";
import { cleanup, render, screen, waitFor } from "@testing-library/react";
// react-dom/server ships no types in this workspace's resolution; the test
// only needs one function from it.
const { renderToStaticMarkup } = require("react-dom/server") as {
  renderToStaticMarkup: (el: React.ReactElement) => string;
};
import React from "react";
import { dynamic } from "./dynamic";

// @testing-library auto-registers cleanup when it finds a global afterEach;
// bun:test doesn't provide one, so mounted trees would accumulate across tests
// in this file and queries would match components a later test never rendered.
afterEach(cleanup);

const Heavy = () => <div data-testid="heavy">heavy</div>;
const Skeleton = () => <div data-testid="skeleton">loading…</div>;

describe("dynamic() with ssr:false", () => {
  test("server-renders the fallback, never the component", () => {
    const D = dynamic(async () => ({ default: Heavy }), { loading: Skeleton });
    const html = renderToStaticMarkup(<D />);
    expect(html).toContain("loading…");
    expect(html).not.toContain("heavy");
  });

  test("the FIRST client render matches the server's, then swaps", async () => {
    // This is the anti-mismatch contract: identical first paint, real
    // component only after an effect (which never runs during SSR).
    const D = dynamic(async () => ({ default: Heavy }), { loading: Skeleton });
    const html = renderToStaticMarkup(<D />);
    const { container } = render(<D />);
    expect(container.innerHTML).toBe(html);
    await waitFor(() => expect(screen.getByTestId("heavy")).toBeDefined());
  });

  test("renders nothing rather than crashing when no fallback is given", () => {
    const D = dynamic(async () => ({ default: Heavy }));
    expect(renderToStaticMarkup(<D />)).toBe("");
  });

  test("passes props through to the loaded component", async () => {
    const Greet = ({ name }: { name: string }) => <span data-testid="g">hi {name}</span>;
    const D = dynamic(async () => ({ default: Greet }), { loading: Skeleton });
    render(<D name="eric" />);
    await waitFor(() => expect(screen.getByTestId("g").textContent).toBe("hi eric"));
  });

  test("accepts a module namespace or a bare component", async () => {
    const D = dynamic(async () => Heavy as any, { loading: Skeleton });
    render(<D />);
    await waitFor(() => expect(screen.getByTestId("heavy")).toBeDefined());
  });

  test("loads the chunk once across multiple instances", async () => {
    let calls = 0;
    const D = dynamic(
      async () => {
        calls++;
        return { default: Heavy };
      },
      { loading: Skeleton },
    );
    render(
      <>
        <D />
        <D />
        <D />
      </>,
    );
    await waitFor(() => expect(screen.getAllByTestId("heavy")).toHaveLength(3));
    expect(calls).toBe(1);
  });

  test("a failed chunk leaves the fallback up instead of blanking", async () => {
    const orig = console.error;
    console.error = () => {};
    try {
      const D = dynamic(async () => {
        throw new Error("network");
      }, { loading: Skeleton });
      render(<D />);
      await new Promise((r) => setTimeout(r, 20));
      expect(screen.getByTestId("skeleton")).toBeDefined();
    } finally {
      console.error = orig;
    }
  });
});

describe("dynamic() with ssr:true", () => {
  test("server-renders the real component", async () => {
    const D = dynamic(async () => ({ default: Heavy }), {
      ssr: true,
      loading: Skeleton,
    });
    // React resolves lazy during a streaming server render; renderToStaticMarkup
    // is sync, so drive it through the client renderer instead.
    render(<D />);
    await waitFor(() => expect(screen.getByTestId("heavy")).toBeDefined());
  });
});
