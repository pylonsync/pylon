// Fetch + WebSocket mocks that route engine traffic to a TestServer.
//
// We intentionally don't reimplement the wire format here — every
// path the engine actually calls lives in `handle()` and produces
// exactly the JSON shape the engine parses. New engine paths require
// a new branch here (which is the point: forces the harness to stay
// honest about what the engine sends).

import type { Row } from "../types";
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
  /** Close the most-recently-opened mock WS so the engine sees a
   *  disconnect and schedules reconnect via its backoff. Returns
   *  true if a socket was actually closed, false otherwise. */
  closeLatestWs: () => boolean;
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
  // Track open mock sockets so a test that needs to simulate a
  // server-side disconnect (cluster bounce, autostop) can grab the
  // latest connection and close it. The engine's reconnect loop
  // builds a fresh MockWebSocket on the next attempt; each shows up
  // here in order so the list maps 1:1 to actual connect attempts.
  const openSockets: MockWebSocket[] = [];

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

    const result = await handle(server, method, url, effectiveToken, init);
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
    static readonly CONNECTING = 0;
    static readonly OPEN = 1;
    static readonly CLOSING = 2;
    static readonly CLOSED = 3;
    readyState = 0;
    binaryType: "blob" | "arraybuffer" = "blob";
    url: string;
    protocol = "";
    onopen: ((ev: Event) => void) | null = null;
    onclose: ((ev: Event) => void) | null = null;
    onmessage: ((ev: MessageEvent) => void) | null = null;
    onerror: ((ev: Event) => void) | null = null;
    private unsubscribe: (() => void) | null = null;
    private boundToken: string | undefined;

    constructor(url: string, protocols?: string | string[]) {
      super();
      this.url = url;
      wsConnectCount += 1;
      openSockets.push(this);
      // The engine encodes the token as `bearer.<percent-encoded>` in
      // the WS subprotocol when one is set. Decode it so the harness
      // can route this connection to the right subscriber bucket
      // independently of any localStorage state.
      const protoToken = extractBearerFromProtocols(protocols);
      this.boundToken = protoToken ?? token;
      const auth = server.authContextFor(this.boundToken);
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

    send(data: string): void {
      // Capture outbound client messages so tests can assert that
      // `reactive-subscribe`, `crdt-subscribe`, etc., were actually
      // emitted by the engine. The real server would dispatch on
      // these; the harness just records them.
      try {
        const parsed = JSON.parse(data);
        server.recordClientWsMessage(this.boundToken, parsed);
      } catch {
        server.recordClientWsMessage(this.boundToken, data);
      }
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

  return {
    currentToken: () => token,
    setToken: (t) => {
      token = t;
    },
    fetchCount: () => fetchCount,
    wsConnectCount: () => wsConnectCount,
    closeLatestWs: () => {
      // Walk back from the most recent socket to find one that's
      // still open. Older closed sockets get skipped silently.
      for (let i = openSockets.length - 1; i >= 0; i--) {
        const s = openSockets[i];
        if (s.readyState !== 3) {
          s.close();
          return true;
        }
      }
      return false;
    },
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
  const headersInit = init.headers;
  let raw: string | null = null;
  if (headersInit instanceof Headers) {
    raw = headersInit.get("Authorization") ?? headersInit.get("authorization");
  } else if (Array.isArray(headersInit)) {
    for (const [k, v] of headersInit) {
      if (k.toLowerCase() === "authorization") {
        raw = v;
        break;
      }
    }
  } else {
    const h = headersInit as Record<string, string>;
    raw = h.Authorization ?? h.authorization ?? null;
  }
  if (!raw) return undefined;
  return raw.startsWith("Bearer ") ? raw.slice("Bearer ".length) : raw;
}

function extractBearerFromProtocols(
  protocols: string | string[] | undefined,
): string | undefined {
  if (!protocols) return undefined;
  const list = Array.isArray(protocols) ? protocols : [protocols];
  for (const p of list) {
    if (typeof p !== "string") continue;
    if (p.startsWith("bearer.")) {
      try {
        return decodeURIComponent(p.slice("bearer.".length));
      } catch {
        return p.slice("bearer.".length);
      }
    }
  }
  return undefined;
}

async function handle(
  server: TestServer,
  method: string,
  url: string,
  token: string | undefined,
  _init: RequestInit | undefined,
): Promise<MockResponse> {
  // /api/auth/me — cheap auth context probe. Engine expects snake_case
  // keys here (user_id / tenant_id / is_admin); the TestServer holds
  // them as camelCase internally so we re-key on the way out.
  if (url.endsWith("/api/auth/me") && method === "GET") {
    const ctx = server.authContextFor(token);
    return {
      status: 200,
      body: {
        user_id: ctx.userId,
        tenant_id: ctx.tenantId,
        is_admin: ctx.isAdmin,
        roles: ctx.roles,
      },
    };
  }

  // /api/sync/pull?since=N — incremental change pull.
  if (url.includes("/api/sync/pull") && method === "GET") {
    const primed = server.consumeNextPullStatus();
    if (primed) {
      return {
        status: primed,
        body: { error: { code: "RESYNC_REQUIRED" } },
      };
    }
    const since = Number(new URL(url, "http://test").searchParams.get("since") ?? "0");
    // Cluster-divergence sim: a delta pull lands on an instance that
    // can't serve our cursor → 410. A snapshot (since=0) still succeeds.
    if (since > 0 && server.force410OnDelta) {
      return {
        status: 410,
        body: { error: { code: "RESYNC_REQUIRED" } },
      };
    }
    const resp = await server.pull(token, since);
    return { status: 200, body: resp };
  }

  // /api/entities/<E>/cursor — policy-filtered list for reconcile.
  const cursorMatch = url.match(/\/api\/entities\/([^/?]+)\/cursor/);
  if (cursorMatch && method === "GET") {
    const entity = cursorMatch[1]!;
    const rows: Row[] = await server.listEntityRows(entity, token);
    return {
      status: 200,
      body: { data: rows, next_cursor: null, has_more: false },
    };
  }

  // /api/sync/push — accept ops from optimistic mutations.
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
