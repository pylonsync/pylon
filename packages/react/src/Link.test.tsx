// A component that renders through <Link> (a button with a `render` prop)
// passes its own ref. The anchor reaches that ref, and Link still warms the
// destination and registers the seed for the same anchor.

import { afterEach, expect, test } from "bun:test";
import { cleanup, render } from "@testing-library/react";
import { createRef } from "react";

import { Link } from "./Link";

type PylonWindow = {
  __pylon?: { prefetch: (href: string, opts?: { document?: boolean }) => void };
  __pylonLinkSeeds?: WeakMap<Element, unknown>;
};

afterEach(() => {
  cleanup();
  delete (window as PylonWindow).__pylon;
  delete (window as PylonWindow).__pylonLinkSeeds;
});

function recordPrefetches() {
  const hrefs: string[] = [];
  (window as PylonWindow).__pylon = { prefetch: (href) => void hrefs.push(href) };
  return hrefs;
}

const idle = () => new Promise((resolve) => setTimeout(resolve, 300));

test("an object ref from the caller receives the anchor, and the link still prefetches", async () => {
  const prefetched = recordPrefetches();
  const ref = createRef<HTMLAnchorElement>();

  const { container, unmount } = render(
    <Link href="/sell" ref={ref} seed={{ id: 1 }}>
      Sell
    </Link>,
  );

  const anchor = container.querySelector("a");
  expect(anchor).not.toBeNull();
  expect(ref.current).toBe(anchor);
  expect((window as PylonWindow).__pylonLinkSeeds?.get(anchor!)).toEqual({ id: 1 });

  await idle();
  expect(prefetched).toContain("/sell");

  unmount();
  expect(ref.current).toBeNull();
});

test("a callback ref from the caller receives the anchor and runs its cleanup", async () => {
  const prefetched = recordPrefetches();
  const calls: string[] = [];

  const { unmount } = render(
    <Link
      href="/account"
      ref={(el) => {
        calls.push(el ? "attach" : "null");
        return () => {
          calls.push("cleanup");
        };
      }}
    >
      Account
    </Link>,
  );

  expect(calls).toEqual(["attach"]);
  await idle();
  expect(prefetched).toContain("/account");

  unmount();
  expect(calls).toEqual(["attach", "cleanup"]);
});
