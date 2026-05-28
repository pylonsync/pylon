// Fetch + WebSocket mocks that route engine traffic to a TestServer.
//
// We intentionally don't reimplement the wire format here — every
// path the engine actually calls lives in `handle()` and produces
// exactly the JSON shape the engine parses. New engine paths require
// a new branch here (which is the point: forces the harness to stay
// honest about what the engine sends).

import type { TestServer } from "./server";

export interface TransportHandle {
  /** Token the engine is currently sending as Authorization: Bearer */
  currentToken: () => string | undefined;
  /** Set / clear the active token (mirrors localStorage flips). */
  setToken: (token: string | undefined) => void;
  /** Number of `fetch` calls observed — assert against this in
   *  tests that need to confirm "nothing more was requested." */
  fetchCount: () => number;
  /** Number of WS connections opened so far. */
  wsConnectCount: () => number;
  /** Tear down the global stubs. */
  restore: () => void;
}

interface MockResponse {
  status: number;
  body: unknown;
  headers?: Record<string, string>;
}

export function installTransport(server: TestServer): TransportHandle {
  let token: string | undefined;
  let fetchCount = 0;
  let wsConnectCount = 0;

  const originalFetch = globalThis.fetch;
  const originalWS = (globalThis as { WebSocket?: unknown }).WebSocket;

  globalThis.fetch = (async (
    input: RequestInfo | URL,
    init?: RequestInit,
  ): Promise<Response> => {
    fetchCount += 1;
    const url = typeof input === "string" ? input : input.toString();
    const method = (init?.method ?? "GET").toUpperCase();
    const authHeader = readAuthHeader(init);
    // If the request didn't include Authorization but our test rig
    // has a "current token", forward it the way the engine actually
    // does. The engine reads from localStorage; we shortcut here
    // because the engine's currentToken() comes from the storage
    // adapter we'd otherwise need to mock.
    const effectiveToken = authHeader ?? token;

    const result = handle(server, method, url, effectiveToken, init);
    const json = result.body == null ? "" : JSON.stringify(result.body);
    return new Response(json, {
      status: result.status,
      headers: {
        "content-type": "application/json",
        ...(result.headers ?? {}),
      },
    });
  }) as typeof fetch;

  // Minimal WebSocket stub. The engine calls `new WebSocket(url)`,
  // attaches onopen/onmessage/onclose, and posts session-changed +
  // change events. We hand it a controllable EventTarget the
  // TestServer subscribes to.
  class MockWebSocket extends EventTarget {
    readyState = 0;
    url: string;
    onopen: ((ev: Event) => void) | null = null;
    onclose: ((ev: Event) => void) | null = null;
    onmessage: ((ev: MessageEvent) => void) | null = null;
    onerror: ((ev: Event) => void) | null = null;
    private unsubscribe: (() => void) | null = null;

    constructor(url: string) {
      super();
      this.url = url;
      wsConnectCount += 1;
      // Resolve user_id from the engine's current token so we can
      // route this connection to the right subscriber bucket.
      const auth = server.authContextFor(effectiveTokenForWs(token));
      if (auth.userId) {
        this.unsubscribe = server.subscribe(auth.userId, (msg) => {
          const event = new MessageEvent("message", {
            data: JSON.stringify(msg),
          });
          this.dispatchEvent(event);
          this.onmessage?.(event);
        });
      }
      // Fire onopen on the next microtask so listeners attached
      // synchronously after construction still see it.
      queueMicrotask(() => {
        this.readyState = 1;
        const ev = new Event("open");
        this.dispatchEvent(ev);
        this.onopen?.(ev);
      });
    }

    send(_data: string): void {
      // Engine sends subscription messages over WS for reactive
      // queries. The harness ignores them unless a scenario tests
      // reactive specifically — most don't.
    }

    close(): void {
      this.readyState = 3;
      this.unsubscribe?.();
      this.unsubscribe = null;
      const ev = new Event("close");
      this.dispatchEvent(ev);
      this.onclose?.(ev);
    }
  }
  (globalThis as { WebSocket?: unknown }).WebSocket = MockWebSocket;

  function effectiveTokenForWs(t: string | undefined): string | undefined {
    return t;
  }

  return {
    currentToken: () => token,
    setToken: (t) => {
      token = t;
    },
    fetchCount: () => fetchCount,
    wsConnectCount: () => wsConnectCount,
    restore: () => {
      globalThis.fetch = originalFetch;
      if (originalWS) {
        (globalThis as { WebSocket?: unknown }).WebSocket = originalWS;
      } else {
        delete (globalThis as { WebSocket?: unknown }).WebSocket;
      }
    },
  };
}

function readAuthHeader(init: RequestInit | undefined): string | undefined {
  if (!init?.headers) return undefined;
  const h = init.headers as Record<string, string> | Headers;
  const raw =
    h instanceof Headers ? h.get("Authorization") : (h.Authorization ?? h.authorization);
  if (!raw) return undefined;
  return raw.startsWith("Bearer ") ? raw.slice("Bearer ".length) : raw;
}

function handle(
  server: TestServer,
  method: string,
  url: string,
  token: string | undefined,
  _init: RequestInit | undefined,
): MockResponse {
  // /api/auth/me — cheap auth context probe.
  if (url.endsWith("/api/auth/me") && method === "GET") {
    const ctx = server.authContextFor(token);
    return { status: 200, body: ctx };
  }

  // /api/sync/pull?since=N — incremental change pull.
  if (url.includes("/api/sync/pull") && method === "GET") {
    const since = Number(new URL(url, "http://test").searchParams.get("since") ?? "0");
    const resp = server.pull(token, since);
    return { status: 200, body: resp };
  }

  // /api/entities/<E>/cursor — policy-filtered list for reconcile.
  const cursorMatch = url.match(/\/api\/entities\/([^/?]+)\/cursor/);
  if (cursorMatch && method === "GET") {
    const entity = cursorMatch[1]!;
    const rows = server.listEntityRows(entity, token);
    return {
      status: 200,
      body: { data: rows, next_cursor: null, has_more: false },
    };
  }

  // /api/sync/push — not exercised by current scenarios.
  if (url.endsWith("/api/sync/push") && method === "POST") {
    return { status: 200, body: { ops: [] } };
  }

  // Anything else: 404 with a clear error so test failures point
  // at the exact missing route rather than a generic JSON parse.
  return {
    status: 404,
    body: { error: { code: "TEST_HARNESS_UNHANDLED", path: url, method } },
  };
}
