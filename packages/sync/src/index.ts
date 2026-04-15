// ---------------------------------------------------------------------------
// agentdb sync client — local-first sync engine
// ---------------------------------------------------------------------------

export { IndexedDBPersistence, persistChange } from "./persistence";

export interface ChangeEvent {
  seq: number;
  entity: string;
  row_id: string;
  kind: "insert" | "update" | "delete";
  data?: Record<string, unknown>;
  timestamp: string;
}

export interface SyncCursor {
  last_seq: number;
}

export interface PullResponse {
  changes: ChangeEvent[];
  cursor: SyncCursor;
  has_more: boolean;
}

export interface PushResponse {
  applied: number;
  errors: string[];
  cursor: SyncCursor;
}

export interface ClientChange {
  entity: string;
  row_id: string;
  kind: "insert" | "update" | "delete";
  data?: Record<string, unknown>;
}

// ---------------------------------------------------------------------------
// Local store — in-memory replica of server state
// ---------------------------------------------------------------------------

export type Row = Record<string, unknown>;

export class LocalStore {
  private tables: Map<string, Map<string, Row>> = new Map();
  private listeners: Set<() => void> = new Set();

  /** Get all rows for an entity. */
  list(entity: string): Row[] {
    const table = this.tables.get(entity);
    if (!table) return [];
    return Array.from(table.values());
  }

  /** Get a row by ID. */
  get(entity: string, id: string): Row | null {
    return this.tables.get(entity)?.get(id) ?? null;
  }

  /** Apply a change event to the local store. */
  applyChange(change: ChangeEvent): void {
    if (!this.tables.has(change.entity)) {
      this.tables.set(change.entity, new Map());
    }
    const table = this.tables.get(change.entity)!;

    switch (change.kind) {
      case "insert":
        if (change.data) {
          table.set(change.row_id, { id: change.row_id, ...change.data });
        }
        break;
      case "update":
        if (change.data) {
          const existing = table.get(change.row_id) ?? { id: change.row_id };
          table.set(change.row_id, { ...existing, ...change.data });
        }
        break;
      case "delete":
        table.delete(change.row_id);
        break;
    }
  }

  /** Apply multiple changes. */
  applyChanges(changes: ChangeEvent[]): void {
    for (const change of changes) {
      this.applyChange(change);
    }
    this.notify();

    // Persist changes if persistence callback is set.
    if (this._persistFn) {
      for (const change of changes) {
        this._persistFn(change);
      }
    }
  }

  /** Set a persistence callback for auto-saving changes. */
  _persistFn: ((change: ChangeEvent) => void) | null = null;

  /** Subscribe to store changes. Returns unsubscribe function. */
  subscribe(listener: () => void): () => void {
    this.listeners.add(listener);
    return () => this.listeners.delete(listener);
  }

  notify(): void {
    for (const listener of this.listeners) {
      listener();
    }
  }

  /** Apply an optimistic insert. Returns a temporary ID. */
  optimisticInsert(entity: string, data: Row): string {
    const tempId = `_pending_${Date.now()}_${Math.random().toString(36).slice(2)}`;
    if (!this.tables.has(entity)) {
      this.tables.set(entity, new Map());
    }
    this.tables.get(entity)!.set(tempId, { id: tempId, ...data });
    this.notify();
    return tempId;
  }

  /** Apply an optimistic update. */
  optimisticUpdate(entity: string, id: string, data: Partial<Row>): void {
    const table = this.tables.get(entity);
    if (!table) return;
    const existing = table.get(id);
    if (existing) {
      table.set(id, { ...existing, ...data });
      this.notify();
    }
  }

  /** Apply an optimistic delete. */
  optimisticDelete(entity: string, id: string): void {
    this.tables.get(entity)?.delete(id);
    this.notify();
  }
}

// ---------------------------------------------------------------------------
// Pending mutation queue — offline-safe write queue
// ---------------------------------------------------------------------------

export interface PendingMutation {
  id: string;
  change: ClientChange;
  status: "pending" | "applied" | "failed";
  error?: string;
}

export class MutationQueue {
  private queue: PendingMutation[] = [];

  add(change: ClientChange): string {
    const id = `mut_${Date.now()}_${Math.random().toString(36).slice(2)}`;
    this.queue.push({ id, change, status: "pending" });
    return id;
  }

  pending(): PendingMutation[] {
    return this.queue.filter((m) => m.status === "pending");
  }

  markApplied(id: string): void {
    const m = this.queue.find((m) => m.id === id);
    if (m) m.status = "applied";
  }

  markFailed(id: string, error: string): void {
    const m = this.queue.find((m) => m.id === id);
    if (m) {
      m.status = "failed";
      m.error = error;
    }
  }

  clear(): void {
    this.queue = this.queue.filter((m) => m.status === "pending");
  }
}

// ---------------------------------------------------------------------------
// Sync engine — coordinates pull, push, local store, mutation queue
// ---------------------------------------------------------------------------

export type TransportType = "websocket" | "sse" | "poll";

export interface SyncEngineConfig {
  baseUrl: string;
  /** Transport type. Default: "websocket". Falls back to polling if connection fails. */
  transport?: TransportType;
  /** WebSocket URL. Default: derived from baseUrl (ws://). */
  wsUrl?: string;
  /** Poll interval in ms (only used when transport is "poll"). Default 1000. */
  pollInterval?: number;
  /** Reconnect delay in ms. Default 1000. */
  reconnectDelay?: number;
  /** Auth token for requests. */
  token?: string;
  /** Enable IndexedDB persistence. Data survives page refresh. Default: true in browser. */
  persist?: boolean;
  /** App name for IndexedDB database naming. Default: "default". */
  appName?: string;
}

export class SyncEngine {
  private config: SyncEngineConfig;
  private cursor: SyncCursor = { last_seq: 0 };
  private running = false;
  private ws: WebSocket | null = null;
  private reconnectTimer: ReturnType<typeof setTimeout> | null = null;
  private persistence: import("./persistence").IndexedDBPersistence | null = null;

  readonly store: LocalStore;
  readonly mutations: MutationQueue;

  /** Presence state for this client. */
  private presenceData: Record<string, unknown> = {};

  constructor(config: SyncEngineConfig) {
    this.config = config;
    this.store = new LocalStore();
    this.mutations = new MutationQueue();
  }

  /**
   * Hydrate the local store with server-rendered data.
   * Call this before start() to avoid a redundant initial pull.
   * Typically used for SSR: server fetches data + cursor, passes to client.
   */
  hydrate(data: HydrationData): void {
    for (const [entity, rows] of Object.entries(data.entities)) {
      for (const row of rows) {
        const id = (row as Record<string, unknown>).id as string;
        if (id) {
          this.store.applyChange({
            seq: 0,
            entity,
            row_id: id,
            kind: "insert",
            data: row as Record<string, unknown>,
            timestamp: "",
          });
        }
      }
    }
    if (data.cursor) {
      this.cursor = data.cursor;
    }
  }

  /** Start the sync engine. Loads persisted data, pulls updates, then connects for real-time. */
  async start(): Promise<void> {
    if (this.running) return;
    this.running = true;

    // Load persisted data if available.
    const shouldPersist = this.config.persist !== false && typeof indexedDB !== "undefined";
    if (shouldPersist) {
      try {
        const { IndexedDBPersistence, persistChange } = await import("./persistence");
        this.persistence = new IndexedDBPersistence(this.config.appName);
        await this.persistence.open();

        // Load cached data into the store.
        const cached = await this.persistence.loadAllEntities();
        for (const [entity, rows] of Object.entries(cached)) {
          for (const row of rows) {
            const id = (row as Record<string, unknown>).id as string;
            if (id) {
              this.store.applyChange({ seq: 0, entity, row_id: id, kind: "insert", data: row, timestamp: "" });
            }
          }
        }

        // Load cursor.
        const savedCursor = await this.persistence.loadCursor();
        if (savedCursor) {
          this.cursor = savedCursor;
        }

        // Auto-save changes to IndexedDB.
        const persistence = this.persistence;
        this.store._persistFn = (change: ChangeEvent) => {
          import("./persistence").then(({ persistChange }) => {
            if (persistence) persistChange(persistence, change);
          });
        };
      } catch {
        // IndexedDB not available — continue without persistence.
      }
    }

    // Pull from server, then connect real-time transport.
    await this.pull();

    // Save cursor after pull.
    if (this.persistence) {
      await this.persistence.saveCursor(this.cursor);
    }

    const transport = this.config.transport ?? "websocket";
    if (transport === "websocket") {
      this.connectWs();
    } else if (transport === "sse") {
      this.connectSse();
    } else if (transport === "poll") {
      this.startPolling();
    }
  }

  private pollTimer: ReturnType<typeof setInterval> | null = null;

  private startPolling(): void {
    const interval = this.config.pollInterval ?? 1000;
    this.pollTimer = setInterval(() => {
      this.push().then(() => this.pull());
    }, interval);
  }

  /** Stop the sync engine. */
  stop(): void {
    this.running = false;
    if (this.ws) {
      this.ws.close();
      this.ws = null;
    }
    if (this.reconnectTimer) {
      clearTimeout(this.reconnectTimer);
      this.reconnectTimer = null;
    }
    if (this.pollTimer) {
      clearInterval(this.pollTimer);
      this.pollTimer = null;
    }
  }

  /** Connect to the WebSocket server for real-time updates. */
  private connectWs(): void {
    if (!this.running) return;

    const wsUrl = this.config.wsUrl ?? this.deriveWsUrl();
    try {
      this.ws = new WebSocket(wsUrl);
    } catch {
      this.scheduleReconnect();
      return;
    }

    this.ws.onmessage = (event) => {
      try {
        const msg = JSON.parse(event.data as string);

        // Sync change event.
        if (msg.seq && msg.entity && msg.kind) {
          const change = msg as ChangeEvent;
          if (change.seq > this.cursor.last_seq) {
            this.store.applyChanges([change]);
            this.cursor = { last_seq: change.seq };
          }
          return;
        }

        // Presence event.
        if (msg.type === "presence") {
          this.store.notify();
          return;
        }
      } catch {
        // Ignore malformed messages.
      }
    };

    this.ws.onclose = () => {
      this.ws = null;
      this.scheduleReconnect();
    };

    this.ws.onerror = () => {
      // onclose will fire after this.
    };
  }

  private scheduleReconnect(): void {
    if (!this.running) return;
    const delay = this.config.reconnectDelay ?? 1000;
    this.reconnectTimer = setTimeout(() => {
      this.reconnectTimer = null;
      // Pull any missed changes, then reconnect.
      this.pull().then(() => this.connectWs());
    }, delay);
  }

  /** Connect via Server-Sent Events. */
  private connectSse(): void {
    if (!this.running) return;

    const base = this.config.baseUrl;
    const url = new URL(base);
    const port = parseInt(url.port || "4321", 10);
    const sseUrl = `http://${url.hostname}:${port + 2}/events`;

    try {
      const es = new EventSource(sseUrl);
      es.onmessage = (event) => {
        try {
          const msg = JSON.parse(event.data);
          if (msg.seq && msg.entity && msg.kind) {
            const change = msg as ChangeEvent;
            if (change.seq > this.cursor.last_seq) {
              this.store.applyChanges([change]);
              this.cursor = { last_seq: change.seq };
            }
          }
        } catch {
          // Ignore malformed events.
        }
      };
      es.onerror = () => {
        es.close();
        // Reconnect after delay.
        setTimeout(() => {
          if (this.running) {
            this.pull().then(() => this.connectSse());
          }
        }, this.config.reconnectDelay ?? 1000);
      };
    } catch {
      // EventSource not available — fall back to polling.
      this.startPolling();
    }
  }

  private deriveWsUrl(): string {
    const base = this.config.baseUrl;
    // http://localhost:4321 -> ws://localhost:4322 (port+1)
    const url = new URL(base);
    const port = parseInt(url.port || "4321", 10);
    return `ws://${url.hostname}:${port + 1}`;
  }

  /** Pull changes from the server. */
  async pull(): Promise<void> {
    try {
      const resp = await this.request<PullResponse>(
        "GET",
        `/api/sync/pull?since=${this.cursor.last_seq}`
      );
      if (resp.changes.length > 0) {
        this.store.applyChanges(resp.changes);
        this.cursor = resp.cursor;
      }
      // If there are more, pull again immediately.
      if (resp.has_more) {
        await this.pull();
      }
    } catch {
      // Silently fail — will retry on next poll.
    }
  }

  /** Push pending mutations to the server. */
  async push(): Promise<void> {
    const pending = this.mutations.pending();
    if (pending.length === 0) return;

    try {
      const resp = await this.request<PushResponse>("POST", "/api/sync/push", {
        changes: pending.map((m) => m.change),
      });

      // Mark mutations based on response.
      for (let i = 0; i < pending.length; i++) {
        if (i < resp.applied) {
          this.mutations.markApplied(pending[i].id);
        } else if (resp.errors[i - resp.applied]) {
          this.mutations.markFailed(pending[i].id, resp.errors[i - resp.applied]);
        }
      }

      this.mutations.clear();
    } catch {
      // Will retry on next tick.
    }
  }

  /** Insert a row with optimistic local update. */
  async insert(entity: string, data: Row): Promise<string> {
    const tempId = this.store.optimisticInsert(entity, data);
    this.mutations.add({
      entity,
      row_id: tempId,
      kind: "insert",
      data,
    });
    await this.push();
    return tempId;
  }

  /** Update a row with optimistic local update. */
  async update(entity: string, id: string, data: Partial<Row>): Promise<void> {
    this.store.optimisticUpdate(entity, id, data);
    this.mutations.add({
      entity,
      row_id: id,
      kind: "update",
      data: data as Row,
    });
    await this.push();
  }

  /** Delete a row with optimistic local update. */
  async delete(entity: string, id: string): Promise<void> {
    this.store.optimisticDelete(entity, id);
    this.mutations.add({
      entity,
      row_id: id,
      kind: "delete",
    });
    await this.push();
  }

  // -----------------------------------------------------------------------
  // Infinite scroll / cursor pagination
  // -----------------------------------------------------------------------

  /** Load a page of data from an entity with cursor-based pagination. */
  async loadPage(
    entity: string,
    options?: { limit?: number; offset?: number; order?: Record<string, "asc" | "desc"> }
  ): Promise<{ data: Row[]; total: number; hasMore: boolean }> {
    const limit = options?.limit ?? 20;
    const offset = options?.offset ?? 0;

    const filter: Record<string, unknown> = {
      $limit: limit,
      $offset: offset,
    };
    if (options?.order) {
      filter.$order = options.order;
    }

    const resp = await this.request<Row[]>(
      "POST",
      `/api/query/${entity}`,
      filter
    );

    const data = Array.isArray(resp) ? resp : [];
    return {
      data,
      total: data.length, // Server doesn't return total in filtered query
      hasMore: data.length === limit,
    };
  }

  /**
   * Create an infinite query that appends pages.
   * Returns an object with loadMore() and the current accumulated data.
   */
  createInfiniteQuery(entity: string, options?: { pageSize?: number; order?: Record<string, "asc" | "desc"> }) {
    const pageSize = options?.pageSize ?? 20;
    let allRows: Row[] = [];
    let offset = 0;
    let hasMore = true;
    let loading = false;

    const listeners = new Set<() => void>();

    const notify = () => {
      for (const fn of listeners) fn();
    };

    return {
      /** Load the next page. */
      loadMore: async () => {
        if (!hasMore || loading) return;
        loading = true;
        try {
          const page = await this.loadPage(entity, { limit: pageSize, offset, order: options?.order });
          allRows = [...allRows, ...page.data];
          offset += page.data.length;
          hasMore = page.hasMore;
          notify();
        } finally {
          loading = false;
        }
      },
      /** Get current accumulated rows. */
      get data() { return allRows; },
      /** Whether more pages are available. */
      get hasMore() { return hasMore; },
      /** Whether currently loading. */
      get loading() { return loading; },
      /** Subscribe to changes. */
      subscribe: (fn: () => void) => {
        listeners.add(fn);
        return () => listeners.delete(fn);
      },
      /** Reset and start over. */
      reset: () => {
        allRows = [];
        offset = 0;
        hasMore = true;
      },
    };
  }

  /** Get the current cursor position. */
  getCursor(): SyncCursor {
    return { ...this.cursor };
  }

  /** Whether the WebSocket is currently connected. */
  get connected(): boolean {
    return this.ws?.readyState === WebSocket.OPEN;
  }

  // -----------------------------------------------------------------------
  // Presence
  // -----------------------------------------------------------------------

  /** Set this client's presence data and broadcast it. */
  setPresence(data: Record<string, unknown>): void {
    this.presenceData = data;
    this.sendWs({
      type: "presence",
      event: "update",
      data: this.presenceData,
    });
  }

  /** Send a topic message to all connected clients. */
  publishTopic(topic: string, data: unknown): void {
    this.sendWs({
      type: "topic",
      topic,
      data,
    });
  }

  private sendWs(msg: unknown): void {
    if (this.ws && this.ws.readyState === WebSocket.OPEN) {
      this.ws.send(JSON.stringify(msg));
    }
  }

  private async request<T>(method: string, path: string, body?: unknown): Promise<T> {
    const headers: Record<string, string> = {};
    if (body) headers["Content-Type"] = "application/json";
    if (this.config.token) headers["Authorization"] = `Bearer ${this.config.token}`;

    const res = await fetch(`${this.config.baseUrl}${path}`, {
      method,
      headers,
      body: body ? JSON.stringify(body) : undefined,
    });

    if (!res.ok) {
      throw new Error(`Sync request failed: ${res.status}`);
    }

    return res.json() as Promise<T>;
  }
}

// ---------------------------------------------------------------------------
// SSR / Hydration types
// ---------------------------------------------------------------------------

/** Data shape for hydrating the client from server-rendered content. */
export interface HydrationData {
  /** Map of entity name -> rows fetched on the server. */
  entities: Record<string, Record<string, unknown>[]>;
  /** The sync cursor at the time of server fetch. */
  cursor?: SyncCursor;
}

/**
 * Server-side helper: fetch entities from the agentdb API and return
 * hydration data that can be passed to the client's SyncEngine.hydrate().
 *
 * Use this in Next.js server components, getServerSideProps, or route handlers.
 */
export async function getServerData(
  baseUrl: string,
  entities: string[],
  options?: { token?: string }
): Promise<HydrationData> {
  const headers: Record<string, string> = {};
  if (options?.token) {
    headers["Authorization"] = `Bearer ${options.token}`;
  }

  const entityData: Record<string, Record<string, unknown>[]> = {};

  for (const entity of entities) {
    try {
      const res = await fetch(`${baseUrl}/api/entities/${entity}`, { headers });
      if (res.ok) {
        entityData[entity] = (await res.json()) as Record<string, unknown>[];
      } else {
        entityData[entity] = [];
      }
    } catch {
      entityData[entity] = [];
    }
  }

  // Get current sync cursor.
  let cursor: SyncCursor = { last_seq: 0 };
  try {
    const res = await fetch(`${baseUrl}/api/sync/pull?since=0&limit=0`, { headers });
    if (res.ok) {
      const pull = (await res.json()) as PullResponse;
      cursor = pull.cursor;
    }
  } catch {
    // Use beginning cursor.
  }

  return { entities: entityData, cursor };
}

// ---------------------------------------------------------------------------
// Convenience factory
// ---------------------------------------------------------------------------

/** Create a sync engine connected to the agentdb dev server. */
export function createSyncEngine(
  baseUrl = "http://localhost:4321",
  options?: { token?: string; reconnectDelay?: number; wsUrl?: string }
): SyncEngine {
  return new SyncEngine({
    baseUrl,
    token: options?.token,
    reconnectDelay: options?.reconnectDelay,
    wsUrl: options?.wsUrl,
  });
}
