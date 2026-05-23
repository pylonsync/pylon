// ---------------------------------------------------------------------------
// Wire + public types for the sync package. Pure type-only file —
// no runtime imports.
// ---------------------------------------------------------------------------

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

/**
 * Server-resolved auth/session state. Shape mirrors what `/api/auth/me`
 * returns (which is `AuthContext` from the Rust side, with camelCase
 * normalization on the way out).
 *
 * `userId=null` means anonymous. `tenantId=null` means the user hasn't
 * selected an org yet (or the backend is single-tenant).
 */
export interface ResolvedSession {
  userId: string | null;
  tenantId: string | null;
  isAdmin: boolean;
  roles: string[];
}

/**
 * Per-op result on /api/sync/push. Formal shape:
 *
 *   { op_id, row_id, entity, kind, status, seq?, error? }
 *
 * Status semantics:
 *   - `applied`   — first-time write committed at `seq`.
 *   - `replayed`  — same op_id arrived again after a confirmed apply;
 *                   `seq` is the cached value of the original write so
 *                   the client can adopt it on its optimistic ghost
 *                   without waiting for the WS rebroadcast.
 *   - `pending`   — a concurrent push carrying this op_id is still
 *                   in flight on the server. No `seq` yet; the
 *                   client should retry after a short backoff.
 *   - `error`     — the write was attempted and rejected; `error`
 *                   carries the server's code + message.
 *
 * Legacy `deduped` is treated as `replayed` for compatibility — older
 * servers (≤ 0.3.190) returned `deduped` for both InFlight and
 * Replayed cases.
 */
export interface PushOpResult {
  op_id?: string | null;
  entity?: string;
  row_id?: string;
  kind?: "insert" | "update" | "delete";
  status: "applied" | "replayed" | "pending" | "error" | "deduped";
  seq?: number;
  error?: { code: string; message: string } | string;
}

export interface PushResponse {
  applied: number;
  deduped: number;
  errors: string[];
  results?: PushOpResult[];
  cursor: SyncCursor;
}

export interface ClientChange {
  entity: string;
  row_id: string;
  kind: "insert" | "update" | "delete";
  data?: Record<string, unknown>;
  /**
   * Client-minted idempotency key. The server tracks recently-seen op_ids
   * and returns a no-op success for replays. Supply this on every retry of
   * the same logical mutation — the `MutationQueue` does so automatically.
   */
  op_id?: string;
}

/**
 * Reactive subscription spec — what the server needs to replay a
 * subscription if the client reconnects. Cached client-side so the
 * `ws.onopen` reconnect sweep can re-register every active sub
 * without the React hooks having to know about reconnect lifecycle.
 */
export interface ReactiveSpec {
  fn_name: string;
  args: unknown;
}

/**
 * Push message routed to a reactive subscription handler. `result`
 * fires on initial run + every time the server's re-run produces a
 * value whose hash differs from the last push. `error` fires when
 * the server can't execute the handler (function not registered,
 * reactive runtime unavailable, runtime error in user code).
 */
export type ReactiveMessage =
  | { kind: "result"; result: unknown }
  | { kind: "error"; code: string; message: string };

export type Row = Record<string, unknown>;

export type TransportType = "websocket" | "sse" | "poll";

/**
 * Coarse connection state for UI consumers.
 *
 * - `connecting`   — engine is starting up; first WS handshake hasn't
 *                    completed yet. Apps typically render their initial
 *                    skeleton during this state.
 * - `connected`    — WS is open and we've stayed open long enough to
 *                    consider it stable (5s on the wire). Live queries
 *                    are receiving real-time updates.
 * - `reconnecting` — WS dropped (network blip, autostop) and the
 *                    engine is backing off + retrying.
 * - `offline`      — engine has been stopped via `engine.stop()` or
 *                    was never started. No retries pending.
 */
export type SyncConnectionStatus =
  | "connecting"
  | "connected"
  | "reconnecting"
  | "offline";
