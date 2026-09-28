// useRouteData must hand React's `use()` the same promise on every render
// attempt. It called `use(Promise.resolve(loader()))`, which minted a new
// promise per render: React logged "A component was suspended by an
// uncached promise" on listing pages and the loader ran again on every
// retry.
//
// Mounted with react-dom/client directly, outside the act() environment:
// under act, React 19 does not retry a suspended render when its promise
// resolves in this test runtime, so every Suspense test would hang.

import { afterEach, beforeEach, describe, expect, test } from "bun:test";
import { Component, Suspense, type ReactNode } from "react";
import { createRoot, type Root } from "react-dom/client";

import { __routeDataCacheInternals, useRouteData } from "./useRouter";

let root: Root | null = null;
let host: HTMLElement | null = null;
let prevActEnv: unknown;

beforeEach(() => {
  prevActEnv = (globalThis as { IS_REACT_ACT_ENVIRONMENT?: unknown }).IS_REACT_ACT_ENVIRONMENT;
  (globalThis as { IS_REACT_ACT_ENVIRONMENT?: unknown }).IS_REACT_ACT_ENVIRONMENT = false;
  host = document.createElement("div");
  document.body.appendChild(host);
  root = createRoot(host);
});

afterEach(() => {
  root?.unmount();
  host?.remove();
  root = null;
  host = null;
  (globalThis as { IS_REACT_ACT_ENVIRONMENT?: unknown }).IS_REACT_ACT_ENVIRONMENT = prevActEnv;
});

function mount(node: ReactNode) {
  root!.render(<Suspense fallback={<p>loading</p>}>{node}</Suspense>);
}

async function until(text: string): Promise<void> {
  for (let i = 0; i < 100; i++) {
    if (host!.textContent === text) return;
    await new Promise((r) => setTimeout(r, 5));
  }
  throw new Error(`expected "${text}", saw "${host!.textContent}"`);
}

function captureConsoleError(): { messages: string[]; restore: () => void } {
  const original = console.error;
  const messages: string[] = [];
  console.error = (...args: unknown[]) => {
    messages.push(args.map(String).join(" "));
  };
  return { messages, restore: () => (console.error = original) };
}

describe("useRouteData", () => {
  test("suspends on one cached promise and runs the loader once", async () => {
    let calls = 0;
    const load = (slug: string) => {
      calls += 1;
      return new Promise<string>((r) => setTimeout(() => r(`item:${slug}`), 5));
    };
    function Item({ slug }: { slug: string }) {
      const v = useRouteData(() => load(slug), [slug]);
      return <p>{v}</p>;
    }
    const logs = captureConsoleError();
    try {
      mount(<Item slug="a" />);
      await until("item:a");
    } finally {
      logs.restore();
    }
    expect(calls).toBe(1);
    expect(logs.messages.some((m) => m.includes("uncached promise"))).toBe(false);
  });

  test("a synchronous loader value renders without a fallback", async () => {
    let calls = 0;
    function Item() {
      const v = useRouteData(() => {
        calls += 1;
        return "ready";
      }, ["sync"]);
      return <p>{v}</p>;
    }
    mount(<Item />);
    await until("ready");
    expect(calls).toBe(1);
  });

  test("new deps load again; two loaders with the same deps keep their own results", async () => {
    const seen: string[] = [];
    function Pair({ id }: { id: string }) {
      const a = useRouteData(async () => {
        seen.push(`a:${id}`);
        return `A-${id}`;
      }, [id]);
      const b = useRouteData(async () => {
        seen.push(`b:${id}`);
        return `B-${id}`;
      }, [id]);
      return <p>{`${a} ${b}`}</p>;
    }
    mount(<Pair id="1" />);
    await until("A-1 B-1");
    mount(<Pair id="2" />);
    await until("A-2 B-2");
    expect(seen.filter((s) => s === "a:1")).toHaveLength(1);
    expect(seen.filter((s) => s === "a:2")).toHaveLength(1);
    expect(seen.filter((s) => s === "b:2")).toHaveLength(1);
  });
});

describe("useRouteData promise cache", () => {
  test("a server render shares a promise only within one request", () => {
    __routeDataCacheInternals.clear();
    const g = globalThis as { window?: unknown };
    const saved = g.window;
    delete g.window;
    try {
      let calls = 0;
      const loader = () => {
        calls += 1;
        return Promise.resolve(`user-${calls}`);
      };
      const serverData = () => ({
        get: () => {}, list: () => {}, queryGraph: () => {}, paginate: () => {},
      });
      const requestA = serverData();
      const requestB = serverData();
      const a1 = __routeDataCacheInternals.lookup("src", [requestA, "slug"], loader);
      const a2 = __routeDataCacheInternals.lookup("src", [requestA, "slug"], loader);
      const b = __routeDataCacheInternals.lookup("src", [requestB, "slug"], loader);
      expect(a1).toBe(a2);
      expect(b).not.toBe(a1);
      expect(calls).toBe(2);
      // A module-level object (an API client) is not a request scope.
      const api = { me: () => {} };
      const m1 = __routeDataCacheInternals.lookup("src", [api, "me"], loader);
      const m2 = __routeDataCacheInternals.lookup("src", [api, "me"], loader);
      expect(m1).not.toBe(m2);
      // No object at all: never cached on the server.
      const p1 = __routeDataCacheInternals.lookup("src", ["slug"], loader);
      const p2 = __routeDataCacheInternals.lookup("src", ["slug"], loader);
      expect(p1).not.toBe(p2);
      expect(__routeDataCacheInternals.size()).toBe(0);
    } finally {
      g.window = saved;
    }
  });

  test("a rejected load stays long enough for the error boundary, then leaves", async () => {
    __routeDataCacheInternals.clear();
    let calls = 0;
    const loader = () => {
      calls += 1;
      return Promise.reject(new Error("boom"));
    };
    const first = __routeDataCacheInternals.lookup("src-err", ["x"], loader);
    await first.promise.catch(() => {});
    // React's re-render after the rejection sees the same promise.
    expect(__routeDataCacheInternals.lookup("src-err", ["x"], loader)).toBe(first);
    expect(calls).toBe(1);
    await new Promise((r) => setTimeout(r, 1_100));
    __routeDataCacheInternals.lookup("src-err", ["x"], loader).promise.catch(() => {});
    expect(calls).toBe(2);
    __routeDataCacheInternals.clear();
  });

  test("a failing loader renders the error boundary without a retry loop", async () => {
    let calls = 0;
    class Boundary extends Component<{ children: ReactNode }, { error: string | null }> {
      state = { error: null as string | null };
      static getDerivedStateFromError(e: Error) {
        return { error: e.message };
      }
      render() {
        return this.state.error ? <p>{`error:${this.state.error}`}</p> : this.props.children;
      }
    }
    function Item() {
      const v = useRouteData(() => {
        calls += 1;
        return new Promise<string>((_, reject) => setTimeout(() => reject(new Error("boom")), 5));
      }, ["failing"]);
      return <p>{v}</p>;
    }
    const logs = captureConsoleError();
    try {
      root!.render(
        <Boundary>
          <Suspense fallback={<p>loading</p>}>
            <Item />
          </Suspense>
        </Boundary>,
      );
      await until("error:boom");
    } finally {
      logs.restore();
    }
    expect(calls).toBe(1);
  });
});
