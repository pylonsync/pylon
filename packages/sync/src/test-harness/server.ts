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

export interface TestServerOptions {
  /** Override visibility per-entity (tenant scoping, RLS, etc.). */
  visible?: VisibilityFilter;
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
  private nextSeq = 0;

  constructor(opts: TestServerOptions = {}) {
    this.visible = opts.visible ?? defaultVisibilityFilter;
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

  // ---- Entity data --------------------------------------------------------

  /** Bulk-set rows for an entity. Use in test setup before the
   *  engine starts so the change log doesn't fill with seed events.
   *  The seq stays at 0 — clients pulling from cursor 0 will pick
   *  these up via the snapshot path. */
  seed(entity: string, rows: Row[]): void {
    const map = new Map<string, Row>();
    for (const r of rows) {
      const id = (r as { id?: string }).id;
      if (typeof id === "string") map.set(id, r);
    }
    this.rows.set(entity, map);
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
    this.appendLog({
      seq: this.bumpSeq(),
      entity,
      row_id: id,
      kind: "delete",
      data: { id } as Row,
      timestamp: new Date().toISOString(),
    });
    // prev_data isn't carried on the wire ChangeEvent type today —
    // the server has it for policy re-evaluation but it's not part
    // of the client-facing event. If tests need it later, extend the
    // type and thread it through. For now, prev is computed for the
    // visibility filter via the in-memory map snapshot.
    void prev;
  }

  // ---- Read paths the engine calls ----------------------------------------

  /** /api/entities/<entity>/cursor — policy-filtered list. */
  listEntityRows(entity: string, token: string | undefined): Row[] {
    const auth = this.authContextFor(token);
    const all = Array.from(this.rows.get(entity)?.values() ?? []);
    return this.visible(entity, all, auth);
  }

  /** /api/sync/pull — every visible change since `since`. */
  pull(token: string | undefined, since: number): {
    changes: ChangeEvent[];
    cursor: SyncCursor;
    has_more: boolean;
  } {
    const auth = this.authContextFor(token);
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
      // visible. For deletes, deliver the tombstone unconditionally
      // (the row used to be visible to this caller).
      if (ev.kind !== "delete" && !visibleIds.has(ev.row_id)) continue;
      changes.push(ev);
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

  /** Push a change event to every subscriber for the broadcasting
   *  user. Used by tests that want to simulate a live WS event
   *  separate from a mutation (e.g., reactive-result, custom envelopes). */
  pushToUser(userId: string, msg: Record<string, unknown>): void {
    this.broadcastToUser(userId, msg);
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

  private appendLog(ev: ChangeEvent): void {
    this.log.push(ev);
    // Broadcast to subscribers of every user that can see this row.
    for (const [userId] of this.sessions.entries()) {
      const session = this.sessions.get(userId);
      if (!session) continue;
    }
    // Per-row visibility for the broadcast: we deliver to every
    // subscribed user; their auth context decides what they receive.
    // Simpler than tracking per-user routes — tests aren't measuring
    // server fanout cost.
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
      const filtered = this.visible(ev.entity, [ev.data as Row], auth);
      if (ev.kind === "delete" || filtered.length > 0) {
        // Engine expects a flat ChangeEvent on WS — it sniffs
        // `msg.seq && msg.entity && msg.kind` to route. Forward the
        // event verbatim instead of wrapping in a {type,event}
        // envelope.
        for (const sub of subs) {
          sub(ev as unknown as Record<string, unknown>);
        }
      }
    }
  }
}
