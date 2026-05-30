// createTestEnv() — the scenario harness for the JS sync engine.
//
// Composes TestServer + transport mocks + a SyncEngine so tests
// can read like a story:
//
//   const env = createTestEnv();
//   env.server.seed("Recording", [{ id: "r1", orgId: "org-a" }]);
//   env.signIn({ userId: "u1", tenantId: null });
//   await env.start();
//   expect(env.engine.store.list("Recording")).toHaveLength(1);
//   env.server.setTenant(env.token, "org-a");
//   await env.flush();
//   expect(env.engine.store.list("Recording")).toHaveLength(1);
//
// The harness owns three layered concerns:
//   1. TestServer  — canonical state + change log.
//   2. Transport   — fetch + WebSocket mocks routing to the server.
//   3. SyncEngine  — the real, unmodified engine, pointed at the
//                    mocks above.
//
// Everything is in-process. No real network, no IndexedDB (engine
// runs with persist:false). Time is real but flush() drains
// microtasks + the engine's internal sleeps so most scenarios feel
// synchronous.

import { SyncEngine } from "../index";
import type { Storage } from "../storage";
import { TestServer, type TestServerOptions, type VisibilityFilter } from "./server";
import { installTransport, type TransportHandle } from "./transport";

/** Bare in-memory storage adapter — same shape as the localStorage
 *  one but isolated per test so signIn() can plant a token the engine
 *  picks up via currentToken(). */
class MemoryStorage implements Storage {
  private map = new Map<string, string>();
  get(key: string): string | null {
    return this.map.get(key) ?? null;
  }
  set(key: string, value: string): void {
    this.map.set(key, value);
  }
  remove(key: string): void {
    this.map.delete(key);
  }
}

export interface CreateTestEnvOptions {
  /** Override visibility per-entity (tenant scoping, RLS, etc.). */
  visible?: VisibilityFilter;
  /** Override the appName the engine uses for storage keys. */
  appName?: string;
  /** Mid-fetch hook fired before /api/entities/<E>/cursor responds.
   *  Lets a scenario flip server state to drive the reconcile
   *  session-guard race. */
  beforeListEntityRows?: import("./server").BeforeListEntityHook;
  /** Mid-fetch hook fired before /api/sync/pull responds. */
  beforePull?: import("./server").BeforePullHook;
  /** Field projector — strips serverOnly-style fields before the
   *  row leaves the server. */
  projectRow?: import("./server").FieldProjector;
  /** Transport mode passed to the engine. Default "websocket". Use
   *  "poll" to disable the WS-onopen reconcile race in scenarios
   *  that only want to pin the in-start pipeline. */
  transport?: "websocket" | "poll" | "sse";
  /** Base delay (ms) for WS reconnect backoff. The default value used
   *  inside the harness (`undefined` → 1000) introduces seconds of
   *  wall-clock waiting on every reconnect scenario; pass a small
   *  value when the test specifically exercises the reconnect path. */
  reconnectDelay?: number;
}

export interface TestEnv {
  server: TestServer;
  engine: SyncEngine;
  transport: TransportHandle;
  /** Token of the most recent signIn() call. */
  token: string | undefined;
  /** Mint a session on the server AND tell the transport to send it
   *  as Authorization: Bearer on future requests. Returns the token. */
  signIn(input: {
    userId: string | null;
    tenantId?: string | null;
    isAdmin?: boolean;
    roles?: string[];
  }): string;
  /** Re-stamp tenant on the active token (mirrors select-org). */
  selectOrg(tenantId: string | null): void;
  /** Sign out: drops the active token. The engine still has its
   *  cached resolved session until something forces a refresh. */
  signOut(): void;
  /** Boot the engine and wait for the initial pull + hydration. */
  start(): Promise<void>;
  /** Drain pending microtasks + give the engine a moment to react
   *  to recent events (WS messages, session-changed envelopes, etc.).
   *  Most scenarios call this between mutations + assertions. */
  flush(ms?: number): Promise<void>;
  /** Tear down: stops the engine, restores global fetch/WebSocket. */
  dispose(): Promise<void>;
}

export function createTestEnv(opts: CreateTestEnvOptions = {}): TestEnv {
  const server = new TestServer({
    visible: opts.visible,
    beforeListEntityRows: opts.beforeListEntityRows,
    beforePull: opts.beforePull,
    projectRow: opts.projectRow,
  } as TestServerOptions);
  const transport = installTransport(server);
  // Per-scenario in-memory storage so signIn() can plant a token the
  // engine actually reads via currentToken(). Without this the engine
  // falls back to the default localStorage adapter (or no-op outside
  // a browser), which means every fetch goes anonymous and tenant-
  // scoped tests can't distinguish "right session" from "no session."
  const storage = new MemoryStorage();
  const appName = opts.appName ?? "harness";
  const tokenStorageKey =
    appName === "default" ? "pylon_token" : `pylon:${appName}:token`;
  const engine = new SyncEngine({
    baseUrl: "http://test.invalid",
    appName,
    persist: false,
    storage,
    transport: opts.transport ?? "websocket",
    reconnectDelay: opts.reconnectDelay,
    // Tight timings so scenarios don't have to sleep seconds. The
    // engine's reconcile debounce is also relaxed so back-to-back
    // visibility-change triggers don't get coalesced away in tests.
    reconcileMinIntervalMs: 0,
    // Disable multi-tab coordination by default — each scenario is a
    // single isolated engine, and the broker's 400ms settle window
    // would add seconds of wait across the suite. Scenarios that need
    // the multi-tab path opt in by creating two envs with a shared
    // appName + multiTab:true.
    multiTab: false,
  });

  let token: string | undefined;

  const env: TestEnv = {
    server,
    engine,
    transport,
    get token() {
      return token;
    },
    signIn(input) {
      token = server.signIn(input);
      transport.setToken(token);
      // Plant the token in the engine's storage adapter so
      // currentToken() returns it on the very next fetch — matches
      // what `pylon login` / cookie-set / persistSession() do in a
      // real browser.
      storage.set(tokenStorageKey, token);
      return token;
    },
    selectOrg(tenantId) {
      if (!token) throw new Error("selectOrg requires a prior signIn()");
      server.setTenant(token, tenantId);
    },
    signOut() {
      token = undefined;
      transport.setToken(undefined);
      storage.remove(tokenStorageKey);
    },
    async start() {
      await engine.start();
      await this.flush();
    },
    async flush(ms = 25) {
      // Two passes: first drain microtasks so any chain reactions
      // (WS receive → notify → re-pull) get a chance to execute,
      // then a small real-time sleep to let the engine's internal
      // timers fire (e.g., reconnect backoff). 25ms is plenty for
      // anything that isn't a deliberate test of long-running poll
      // cadence; bump it via the arg when a scenario needs more.
      for (let i = 0; i < 4; i++) {
        await Promise.resolve();
      }
      if (ms > 0) {
        await new Promise((r) => setTimeout(r, ms));
      }
      for (let i = 0; i < 4; i++) {
        await Promise.resolve();
      }
    },
    async dispose() {
      try {
        engine.stop?.();
      } catch {
        /* stop is best-effort */
      }
      transport.restore();
    },
  };

  return env;
}
