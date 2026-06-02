// In-memory "server" for the sync test harness. Holds the canonical
// row set + session state and produces the JSON responses the engine
// expects from /api/auth/me, /api/sync/pull, /api/entities/<E>/cursor,
// and /api/sync/push.
//
// Intentionally tiny — every interesting case is expressed in test
// code (`server.insert(...)`, `server.setTenant(...)`) rather than
// behind layers of stubs. The cost: no policy DSL, no SQL query
// language. We model "this row is visible to this caller" as
// `visibleRows(entity, auth)` — the test seeds rows and decides what
// the caller can see for each scenario.

import type { ChangeEvent, Row, SyncCursor } from "../types";

export interface ServerSession {
  token: string;
  userId: string | null;
  tenantId: string | null;
  isAdmin: boolean;
  roles: string[];
}

export interface AuthContext {
  userId: string | null;
  tenantId: string | null;
  isAdmin: boolean;
  roles: string[];
}

export interface VisibilityFilter {
  /** Return the subset of `rows` the given auth context can see for
   *  this entity. Default: every row, so tests that don't care about
   *  policy just pass. Tests that DO care can pass a custom filter to
   *  exercise the "session changed mid-fetch" path. */
  (entity: string, rows: Row[], auth: AuthContext): Row[];
}

export const defaultVisibilityFilter: VisibilityFilter = (_e, rows) => rows;

/** Hook fired right before `/api/entities/<E>/cursor` serializes its
 *  response. Lets a scenario flip server state (e.g., `setTenant`) so
 *  the engine sees a different session signature on apply than on
 *  fetch — the canonical race the reconcile session-guard pins.
 *  Receives the same auth context the visibility filter uses.
 *  Returning a Promise makes the fetch await — useful for landing a
 *  session refresh in flight before the response serializes. */
export type BeforeListEntityHook = (
  entity: string,
  auth: AuthContext,
) => void | Promise<void>;

/** Hook fired right before `/api/sync/pull` serializes its response.
 *  Same role as `beforeListEntityRows` but on the pull path. */
export type BeforePullHook = (
  auth: AuthContext,
  since: number,
) => void | Promise<void>;

/** Field projector — strips fields from a row before it leaves the
 *  server. Models the `serverOnly` projection in production: the
 *  field exists in the canonical row but is never wired to a client. */
export type FieldProjector = (entity: string, row: Row) => Row;

export const identityProjector: FieldProjector = (_e, row) => row;

export interface TestServerOptions {
  /** Override visibility per-entity (tenant scoping, RLS, etc.). */
  visible?: VisibilityFilter;
  /** Mid-fetch hook for the entity-cursor path. */
  beforeListEntityRows?: BeforeListEntityHook;
  /** Mid-fetch hook for the sync-pull path. */
  beforePull?: BeforePullHook;
  /** Strip fields from rows on the way to the wire (mimics serverOnly). */
  projectRow?: FieldProjector;
}

/** Subscribers attached to WS connections — receive every change
 *  event we append to the log, plus session-changed envelopes. */
export type ServerSubscriber = (msg: Record<string, unknown>) => void;

export class TestServer {
  private sessions = new Map<string, ServerSession>();
  private rows = new Map<string, Map<string, Row>>(); // entity → id → row
  private log: ChangeEvent[] = [];
  private subscribers = new Map<string, Set<ServerSubscriber>>(); // userId → subs
  private visible: VisibilityFilter;
  private beforeListEntityHook?: BeforeListEntityHook;
  private beforePullHook?: BeforePullHook;
  private project: FieldProjector;
  private nextSeq = 0;
  /** When set, the next pull() returns this status instead of normal.
   *  Used to simulate 410 RESYNC_REQUIRED and similar transient errors. */
  private nextPullStatus: number | null = null;
  /** When true, every DELTA pull (since > 0) 410s, but a snapshot pull
   *  (since = 0) succeeds. Simulates a horizontally-scaled deployment
   *  where a client bounces between instances whose in-memory change
   *  logs diverge (no shared persistent log) — every cursor is "stale"
   *  on the instance it lands on. This is the condition that drove the
   *  280GB egress storm; the test asserts the client backs off instead
   *  of re-snapshotting forever. */
  force410OnDelta = false;
  /** Count of snapshot pulls served (since = 0). The egress storm was a
   *  runaway count here; the regression test bounds it. */
  snapshotPullCount = 0;
  /** Captured outbound WS messages from clients — tests assert against
   *  this to verify `reactive-subscribe`, `crdt-subscribe`, etc., were
   *  actually sent over the wire. */
  readonly receivedWsMessages: Array<{ userId: string | null; msg: unknown }> = [];

  constructor(opts: TestServerOptions = {}) {
    this.visible = opts.visible ?? defaultVisibilityFilter;
    this.beforeListEntityHook = opts.beforeListEntityRows;
    this.beforePullHook = opts.beforePull;
    this.project = opts.projectRow ?? identityProjector;
  }

  // ---- Session management -------------------------------------------------

  /** Mint a session and return its bearer token. */
  signIn(input: {
    userId: string | null;
    tenantId?: string | null;
    isAdmin?: boolean;
    roles?: string[];
  }): string {
    const token = `tok_${Math.random().toString(36).slice(2, 10)}`;
    this.sessions.set(token, {
      token,
      userId: input.userId,
      tenantId: input.tenantId ?? null,
      isAdmin: input.isAdmin ?? false,
      roles: input.roles ?? [],
    });
    return token;
  }

  /** Re-stamp the tenant on an existing token (analogue of
   *  /api/auth/select-org). Fires session-changed to subscribers so
   *  the client can refresh its resolved session. */
  setTenant(token: string, tenantId: string | null): void {
    const s = this.sessions.get(token);
    if (!s) return;
    s.tenantId = tenantId;
    // Mirror real Pylon: server pushes session-changed to every
    // subscriber for this user_id so each tab learns.
    if (s.userId) {
      this.broadcastToUser(s.userId, { type: "session-changed" });
    }
  }

  /** Mutate fields on an existing session in place. Used by scenarios
   *  that need to change the sessionSignature without triggering the
   *  engine's tenant-flip resetReplica (e.g., role changes). */
  mutateSession(
    token: string,
    patch: Partial<Omit<ServerSession, "token">>,
  ): void {
    const s = this.sessions.get(token);
    if (!s) return;
    Object.assign(s, patch);
    if (s.userId) {
      this.broadcastToUser(s.userId, { type: "session-changed" });
    }
  }

  /** Revoke a session (drop the token from the map). Used by signOut
   *  scenarios — subsequent /api/auth/me returns anonymous. */
  revoke(token: string): void {
    const s = this.sessions.get(token);
    this.sessions.delete(token);
    if (s?.userId) {
      this.broadcastToUser(s.userId, { type: "session-changed" });
    }
  }

  /** What /api/auth/me returns for a given token. */
  authContextFor(token: string | undefined): AuthContext {
    const s = token ? this.sessions.get(token) : undefined;
    if (!s) return { userId: null, tenantId: null, isAdmin: false, roles: [] };
    return {
      userId: s.userId,
      tenantId: s.tenantId,
      isAdmin: s.isAdmin,
      roles: s.roles,
    };
  }

  /** Inject a 410 into the next pull. Engine should resetReplica and
   *  re-pull from seq=0; the second pull responds normally. */
  primeNextPullStatus(status: number): void {
    this.nextPullStatus = status;
  }
  consumeNextPullStatus(): number | null {
    const s = this.nextPullStatus;
    this.nextPullStatus = null;
    return s;
  }

  // ---- Entity data --------------------------------------------------------

  /** Bulk-seed rows for an entity AND emit insert events into the
   *  change log so the engine discovers them via /api/sync/pull at
   *  since=0 — same shape production clients see when they boot
   *  against a populated server. Without logging, the rows would only
   *  be discoverable via reconcile, which doesn't fire on start. */
  seed(entity: string, rows: Row[]): void {
    let map = this.rows.get(entity);
    if (!map) {
      map = new Map();
      this.rows.set(entity, map);
    }
    for (const r of rows) {
      const id = (r as { id?: string }).id;
      if (typeof id !== "string") continue;
      map.set(id, r);
      this.log.push({
        seq: this.bumpSeq(),
        entity,
        row_id: id,
        kind: "insert",
        data: r,
        timestamp: new Date().toISOString(),
      });
    }
  }

  insert(entity: string, row: Row): void {
    const id = (row as { id?: string }).id;
    if (typeof id !== "string") throw new Error("insert needs row.id");
    let map = this.rows.get(entity);
    if (!map) {
      map = new Map();
      this.rows.set(entity, map);
    }
    map.set(id, row);
    this.appendLog({
      seq: this.bumpSeq(),
      entity,
      row_id: id,
      kind: "insert",
      data: row,
      timestamp: new Date().toISOString(),
    });
  }

  update(entity: string, id: string, patch: Partial<Row>): void {
    const map = this.rows.get(entity);
    if (!map) return;
    const prev = map.get(id);
    if (!prev) return;
    const next = { ...prev, ...patch } as Row;
    map.set(id, next);
    this.appendLog({
      seq: this.bumpSeq(),
      entity,
      row_id: id,
      kind: "update",
      data: next,
      timestamp: new Date().toISOString(),
    });
  }

  delete(entity: string, id: string): void {
    const map = this.rows.get(entity);
    if (!map) return;
    const prev = map.get(id);
    if (!prev) return;
    map.delete(id);
    this.appendLog(
      {
        seq: this.bumpSeq(),
        entity,
        row_id: id,
        kind: "delete",
        data: { id } as Row,
        timestamp: new Date().toISOString(),
      },
      prev,
    );
  }

  /** Raw seq bump for tests that want to inject events directly. */
  nextSeqValue(): number {
    return this.bumpSeq();
  }

  // ---- Read paths the engine calls ----------------------------------------

  /** /api/entities/<entity>/cursor — policy-filtered list.
   *  Async so the `beforeListEntityRows` hook can await state changes
   *  (e.g., land a session refresh mid-fetch). Auth is re-read AFTER
   *  the hook so a hook that flips roles / tenant takes effect on
   *  the response the engine sees, matching what a real server does
   *  when the session mutates mid-request. */
  async listEntityRows(entity: string, token: string | undefined): Promise<Row[]> {
    if (this.beforeListEntityHook) {
      const earlyAuth = this.authContextFor(token);
      await this.beforeListEntityHook(entity, earlyAuth);
    }
    const auth = this.authContextFor(token);
    const all = Array.from(this.rows.get(entity)?.values() ?? []);
    return this.visible(entity, all, auth).map((r) => this.project(entity, r));
  }

  /** /api/sync/pull — every visible change since `since`. */
  async pull(token: string | undefined, since: number): Promise<{
    changes: ChangeEvent[];
    cursor: SyncCursor;
    has_more: boolean;
  }> {
    const auth = this.authContextFor(token);
    if (this.beforePullHook) await this.beforePullHook(auth, since);
    if (since === 0) this.snapshotPullCount += 1;
    const visibleSet = (entity: string) => {
      const filtered = this.visible(
        entity,
        Array.from(this.rows.get(entity)?.values() ?? []),
        auth,
      );
      return new Set(
        filtered.map((r) => (r as { id?: string }).id).filter(Boolean) as string[],
      );
    };
    const changes: ChangeEvent[] = [];
    for (const ev of this.log) {
      if (ev.seq <= since) continue;
      const visibleIds = visibleSet(ev.entity);
      // For inserts / updates, only deliver if the row is currently
      // visible. For deletes, deliver the tombstone only when the
      // caller could see the row before deletion — otherwise we leak
      // existence of rows the caller never had access to.
      if (ev.kind === "delete") {
        // Re-evaluate visibility against the pre-delete data, captured
        // on the ChangeEvent at append time. The current `rows` map
        // no longer has the row, so we can't recompute from there.
        const prev = (ev as ChangeEvent & { prev_data?: Row }).prev_data;
        if (prev) {
          const visiblePrev = this.visible(ev.entity, [prev], auth);
          if (visiblePrev.length === 0) continue;
        }
        changes.push(ev);
      } else {
        if (!visibleIds.has(ev.row_id)) continue;
        // Project the row on the way out so serverOnly-style fields
        // never reach the wire. Same path the WS broadcast applies.
        changes.push({
          ...ev,
          data: this.project(ev.entity, ev.data as Row),
        });
      }
    }
    return {
      changes,
      cursor: { last_seq: this.nextSeq },
      has_more: false,
    };
  }

  // ---- WS push ------------------------------------------------------------

  subscribe(userId: string, sub: ServerSubscriber): () => void {
    let set = this.subscribers.get(userId);
    if (!set) {
      set = new Set();
      this.subscribers.set(userId, set);
    }
    set.add(sub);
    return () => {
      set?.delete(sub);
    };
  }

  /** Push an arbitrary envelope to every subscriber for a user — used
   *  for `reactive-result`, `row-revoked`, `session-changed` etc. */
  pushToUser(userId: string, msg: Record<string, unknown>): void {
    this.broadcastToUser(userId, msg);
  }

  /** Record an outbound WS message from a client (subscribe / unsub /
   *  ping). Tests can assert against `receivedWsMessages` to verify
   *  the engine actually sent something over the wire. */
  recordClientWsMessage(token: string | undefined, msg: unknown): void {
    const auth = this.authContextFor(token);
    this.receivedWsMessages.push({ userId: auth.userId, msg });
  }

  private broadcastToUser(userId: string, msg: Record<string, unknown>): void {
    const subs = this.subscribers.get(userId);
    if (!subs) return;
    for (const sub of subs) sub(msg);
  }

  // ---- Internals ----------------------------------------------------------

  private bumpSeq(): number {
    this.nextSeq += 1;
    return this.nextSeq;
  }

  private appendLog(ev: ChangeEvent, prevForDelete?: Row): void {
    // Stash the pre-delete row on the event so pull() and WS broadcast
    // can evaluate visibility against the data the caller used to see.
    if (ev.kind === "delete" && prevForDelete) {
      (ev as ChangeEvent & { prev_data?: Row }).prev_data = prevForDelete;
    }
    this.log.push(ev);
    // Per-user fanout: each subscribed user gets the event ONLY when
    // their auth context can see the affected row. Inserts/updates use
    // the row's current data; deletes use prev_data (the row the user
    // used to see). Matches the real broadcast layer in pylon-cloud
    // which strips invisible deletes so a tenant boundary can't leak
    // "row X was deleted" to a user that never saw row X.
    const prev = (ev as ChangeEvent & { prev_data?: Row }).prev_data;
    const dataForVisibility = (ev.kind === "delete" ? prev : ev.data) as
      | Row
      | undefined;
    if (!dataForVisibility) return;
    for (const [userId, subs] of this.subscribers) {
      const session = Array.from(this.sessions.values()).find(
        (s) => s.userId === userId,
      );
      const auth = session
        ? {
            userId: session.userId,
            tenantId: session.tenantId,
            isAdmin: session.isAdmin,
            roles: session.roles,
          }
        : { userId: null, tenantId: null, isAdmin: false, roles: [] };
      const filtered = this.visible(ev.entity, [dataForVisibility], auth);
      if (filtered.length === 0) continue;
      // Engine expects a flat ChangeEvent on WS — it sniffs
      // `msg.seq && msg.entity && msg.kind` to route. Forward the
      // event verbatim, but strip serverOnly fields the same way the
      // wire path would.
      const projected =
        ev.kind === "delete"
          ? ev
          : ({ ...ev, data: this.project(ev.entity, ev.data as Row) } as ChangeEvent);
      for (const sub of subs) {
        sub(projected as unknown as Record<string, unknown>);
      }
    }
  }
}
