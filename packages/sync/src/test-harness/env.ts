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
import { TestServer, type TestServerOptions, type VisibilityFilter } from "./server";
import { installTransport, type TransportHandle } from "./transport";

export interface CreateTestEnvOptions {
  /** Override visibility per-entity (tenant scoping, RLS, etc.). */
  visible?: VisibilityFilter;
  /** Override the appName the engine uses for storage keys. */
  appName?: string;
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
  const server = new TestServer({ visible: opts.visible } as TestServerOptions);
  const transport = installTransport(server);
  const engine = new SyncEngine({
    baseUrl: "http://test.invalid",
    appName: opts.appName ?? "harness",
    persist: false,
    // Tight timings so scenarios don't have to sleep seconds. The
    // engine's reconcile debounce is also relaxed so back-to-back
    // visibility-change triggers don't get coalesced away in tests.
    reconcileMinIntervalMs: 0,
  });

  // The engine reads tokens via its storage adapter. With persist:false
  // there's still an in-memory default storage; we don't reach into
  // it here. Instead, the transport mock pulls the active token from
  // a shared variable set by signIn(). The Engine's
  // currentToken() returns undefined in tests (no storage write); the
  // mock fetch + mock WebSocket honor our `token` directly. This
  // shortcut keeps the engine code path unchanged while still
  // exercising the auth-context flow.

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
      return token;
    },
    selectOrg(tenantId) {
      if (!token) throw new Error("selectOrg requires a prior signIn()");
      server.setTenant(token, tenantId);
    },
    signOut() {
      token = undefined;
      transport.setToken(undefined);
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
