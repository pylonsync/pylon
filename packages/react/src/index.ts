export { defineRoute } from "@pylonsync/sdk";
export type { RouteMode, AppManifest } from "@pylonsync/sdk";

// React hooks — high-level ergonomic shape
export {
  useQuery,
  useQueryOne,
  useMutation,
  useInfiniteQuery,
  usePaginatedQuery,
  useEntityMutation,
  useAction,
  useQueryRaw,
  useQueryOneRaw,
  useLiveList,
  useLiveRow,
  useInsert,
  useUpdate,
  useDelete,
  useFn,
  useAggregate,
  useSearch,
} from "./hooks";
export type {
  QueryOptions,
  QueryFilter,
  IncludeSpec,
  UseQueryReturn,
  UseQueryOneReturn,
  UseMutationReturn,
  UseInfiniteQueryReturn,
  UsePaginatedQueryReturn,
  PaginatedQueryStatus,
  UseFnReturn,
  AggregateSpec,
  UseAggregateReturn,
  SearchSpec,
  UseSearchReturn,
} from "./hooks";

// Room hook
export { useRoom } from "./useRoom";
export type {
  RoomPeer,
  RoomSnapshot,
  UseRoomOptions,
  UseRoomReturn,
} from "./useRoom";

// Shard hook for real-time sims (games, MMO, live docs, etc.)
export { useShard, connectShard } from "./useShard";
export type {
  UseShardOptions,
  UseShardReturn,
  ShardClient,
} from "./useShard";

// Session hook — server-resolved user + tenant identity
export { useSession } from "./useSession";
export type { UseSessionReturn, ResolvedSession } from "./useSession";

// One-liner API
export { db, init } from "./db";

// Typed client (consumes generated AppSchema)
export { createTypedDb } from "./typed";
export type { TypedDb, AgentDBSchema } from "./typed";

// Re-export sync engine for direct use.
export {
  SyncEngine,
  createSyncEngine,
  getServerData,
  LocalStore,
  MutationQueue,
} from "@pylonsync/sync";
export type {
  ChangeEvent,
  SyncCursor,
  PullResponse,
  HydrationData,
  Row,
} from "@pylonsync/sync";

// ---------------------------------------------------------------------------
// Client context
// ---------------------------------------------------------------------------

export interface AgentDBClientConfig {
  baseUrl?: string;
  /**
   * App identifier used to namespace all client-side storage keys —
   * localStorage (token, cached user, feature-flag toggles) and
   * IndexedDB (sync replica). Two apps served from the same browser
   * origin (different ports in dev, or the same domain in prod) must
   * pick different names or they'll see each other's sessions and
   * local replicas. Defaults to "default" for a single-app setup.
   */
  appName?: string;
}

let _baseUrl = "http://localhost:4321";
let _baseUrlConfigured = false;
let _appName = "default";

/** Current effective base URL. Used by hooks (useRoom, useShard) that share
 *  the client config but don't have access to the module-private state. */
export function getBaseUrl(): string {
  return _baseUrl;
}

/** Current app name. Used by sync engine + storage helpers to namespace keys. */
export function getAppName(): string {
  return _appName;
}

/**
 * Resolve the localStorage key for a conceptual slot (e.g. "token",
 * "user") into its actual storage key. When `appName` is "default" we
 * fall back to the legacy unprefixed key so older single-app setups
 * keep working without migration.
 */
export function storageKey(slot: string): string {
  if (_appName === "default") return `pylon_${slot}`;
  return `pylon:${_appName}:${slot}`;
}

export function configureClient(config: AgentDBClientConfig): void {
  if (config.baseUrl) {
    _baseUrl = config.baseUrl;
    _baseUrlConfigured = true;
    maybeWarnDowngrade(config.baseUrl);
  }
  if (config.appName) {
    _appName = config.appName;
  }
}

/**
 * Shout loudly if the configured baseUrl is http:// while the current page
 * is served over https://. That combination means auth/session traffic
 * ships in plaintext to a (possibly different) origin — either a misconfig
 * or a downgrade attack via stale config. Browsers typically also block
 * mixed-content requests silently, so the warning helps debugging.
 */
function maybeWarnDowngrade(baseUrl: string): void {
  try {
    if (typeof window === "undefined") return;
    const page = window.location?.protocol;
    if (page === "https:" && baseUrl.startsWith("http://")) {
      console.warn(
        `[pylon] configured baseUrl is http:// but page origin is https:// — auth traffic will be blocked or sent in plaintext: ${baseUrl}`,
      );
    }
  } catch {
    /* ignore */
  }
}

/**
 * In non-localhost production builds, refuse to use the built-in
 * http://localhost:4321 default — that default existing silently was how
 * a forgotten `configureClient` call could ship user tokens in the clear
 * to a broken dev URL. Throws instead of downgrading; a `configureClient`
 * call with an explicit origin fixes it.
 *
 * Local development still gets the convenience default.
 */
function assertBaseUrlSafeForEnv(): void {
  if (_baseUrlConfigured) return;
  if (typeof window === "undefined") return;
  const host = window.location?.hostname ?? "";
  const isLocal =
    host === "" ||
    host === "localhost" ||
    host === "127.0.0.1" ||
    host.endsWith(".localhost");
  if (!isLocal) {
    throw new Error(
      "[pylon] configureClient({ baseUrl }) must be called before any " +
        "request when the app is not running on localhost. Using the " +
        "built-in http://localhost:4321 default in production would ship " +
        "user credentials to the wrong origin.",
    );
  }
}

async function apiRequest(
  method: string,
  path: string,
  body?: unknown
): Promise<unknown> {
  assertBaseUrlSafeForEnv();
  // Auto-attach the session token so `db.insert`, `fetchList`, etc. behave
  // as the signed-in user without every call site threading the header.
  // Safe: `currentAuthToken` is a no-op server-side.
  const headers: Record<string, string> = {};
  if (body) headers["Content-Type"] = "application/json";
  const token = currentAuthToken();
  if (token) headers["Authorization"] = `Bearer ${token}`;
  const res = await fetch(`${_baseUrl}${path}`, {
    method,
    headers,
    body: body ? JSON.stringify(body) : undefined,
  });
  if (!res.ok) {
    const err = (await res.json().catch(() => ({}))) as Record<string, unknown>;
    const errorObj = err?.error as Record<string, unknown> | undefined;
    throw new Error((errorObj?.message as string) ?? `HTTP ${res.status}`);
  }
  return res.json();
}

// ---------------------------------------------------------------------------
// Direct data access (non-synced, for server components / one-shot reads)
// ---------------------------------------------------------------------------

export async function fetchList(entity: string): Promise<Record<string, unknown>[]> {
  return apiRequest("GET", `/api/entities/${entity}`) as Promise<Record<string, unknown>[]>;
}

export async function fetchById(
  entity: string,
  id: string
): Promise<Record<string, unknown> | null> {
  try {
    return (await apiRequest("GET", `/api/entities/${entity}/${id}`)) as Record<string, unknown>;
  } catch {
    return null;
  }
}

export async function insert(
  entity: string,
  data: Record<string, unknown>
): Promise<{ id: string }> {
  return apiRequest("POST", `/api/entities/${entity}`, data) as Promise<{ id: string }>;
}

export async function update(
  entity: string,
  id: string,
  data: Record<string, unknown>
): Promise<{ updated: boolean }> {
  return apiRequest("PATCH", `/api/entities/${entity}/${id}`, data) as Promise<{
    updated: boolean;
  }>;
}

export async function remove(
  entity: string,
  id: string
): Promise<{ deleted: boolean }> {
  return apiRequest("DELETE", `/api/entities/${entity}/${id}`) as Promise<{
    deleted: boolean;
  }>;
}

// ---------------------------------------------------------------------------
// Auth helpers
// ---------------------------------------------------------------------------

export async function createSession(
  userId: string
): Promise<{ token: string; user_id: string }> {
  return apiRequest("POST", "/api/auth/session", {
    user_id: userId,
  }) as Promise<{ token: string; user_id: string }>;
}

export async function getAuthContext(
  token?: string
): Promise<{ user_id: string | null }> {
  const headers: Record<string, string> = {};
  if (token) {
    headers["Authorization"] = `Bearer ${token}`;
  }
  const res = await fetch(`${_baseUrl}/api/auth/me`, { headers });
  return res.json() as Promise<{ user_id: string | null }>;
}

/**
 * Exchange a current session token for a new one with a fresh 30-day expiry.
 * The old token is revoked server-side. Call this before expiry to keep
 * long-lived sessions alive without forcing a re-login.
 *
 * Returns `null` if the old token is already expired or invalid — the
 * caller should treat that as "log back in."
 */
export async function refreshSession(
  token: string
): Promise<{ token: string; user_id: string; expires_at: number } | null> {
  const res = await fetch(`${_baseUrl}/api/auth/refresh`, {
    method: "POST",
    headers: { Authorization: `Bearer ${token}` },
  });
  if (!res.ok) return null;
  return res.json() as Promise<{
    token: string;
    user_id: string;
    expires_at: number;
  }>;
}

/**
 * Keep a session alive by automatically refreshing ~1 hour before expiry.
 *
 * ```ts
 * const session = await createSession("alice");
 * const stop = startSessionAutoRefresh(session, {
 *   onRefresh: (next) => localStorage.setItem("token", next.token),
 *   onExpired: () => redirect("/login"),
 * });
 * // later:
 * stop();
 * ```
 *
 * Returns a cleanup function that cancels the scheduled refresh. Call it
 * on logout or unmount — otherwise the timer leaks.
 *
 * Default refresh margin is 1 hour. Pass `{ marginSecs }` to tune.
 */
export function startSessionAutoRefresh(
  session: { token: string; expires_at: number },
  opts: {
    onRefresh: (next: { token: string; user_id: string; expires_at: number }) => void;
    onExpired?: () => void;
    marginSecs?: number;
  }
): () => void {
  const margin = opts.marginSecs ?? 3600;
  const now = Math.floor(Date.now() / 1000);
  const when = Math.max(0, session.expires_at - now - margin);
  // Cap JS setTimeout at 2^31-1 ms (~24.8d). For tokens with a longer
  // remaining life, schedule at the cap and let the next tick reschedule.
  const delay = Math.min(when * 1000, 2_147_483_000);
  let cancelled = false;
  const timer = setTimeout(async () => {
    if (cancelled) return;
    const next = await refreshSession(session.token);
    if (cancelled) return;
    if (next) {
      opts.onRefresh(next);
      // Chain: schedule the next refresh for the new token's expiry.
      startSessionAutoRefresh(next, opts);
    } else {
      opts.onExpired?.();
    }
  }, delay);
  return () => {
    cancelled = true;
    clearTimeout(timer);
  };
}

// ---------------------------------------------------------------------------
// TypeScript function calls (queries, mutations, actions)
// ---------------------------------------------------------------------------

/**
 * Call a server-side function defined in the `functions/` directory.
 *
 * @example
 * ```ts
 * const result = await callFn("placeBid", { lotId: "lot_1", amount: 150 });
 * ```
 */
/**
 * Read the auth token from the browser. Falls back to `localStorage`
 * under the conventional `pylon_token` key so components and hooks
 * that don't explicitly carry auth still send it. Production apps that
 * use httpOnly cookies pass `credentials: "include"` via `configureClient`
 * instead; see docs/ops/DEPLOY.md.
 */
function currentAuthToken(): string | undefined {
  if (typeof window === "undefined" || !window.localStorage) return undefined;
  return window.localStorage.getItem(storageKey("token")) ?? undefined;
}

export async function callFn<T = unknown>(
  name: string,
  args: Record<string, unknown> = {},
  options: { token?: string } = {}
): Promise<T> {
  const headers: Record<string, string> = { "Content-Type": "application/json" };
  const token = options.token ?? currentAuthToken();
  if (token) headers["Authorization"] = `Bearer ${token}`;
  const res = await fetch(`${_baseUrl}/api/fn/${name}`, {
    method: "POST",
    headers,
    body: JSON.stringify(args),
  });
  const json = (await res.json()) as unknown;
  if (!res.ok) {
    const err = (json as { error?: { code: string; message: string } }).error;
    throw new Error(err?.message || `HTTP ${res.status}`);
  }
  return json as T;
}

/**
 * Stream a server-side function's output as Server-Sent Events.
 *
 * @example
 * ```ts
 * for await (const chunk of streamFn("chat", { message: "hello" })) {
 *   console.log(chunk);
 * }
 * ```
 */
export async function* streamFn(
  name: string,
  args: Record<string, unknown> = {},
  options: { token?: string } = {}
): AsyncGenerator<string, unknown, unknown> {
  const headers: Record<string, string> = {
    "Content-Type": "application/json",
    Accept: "text/event-stream",
  };
  if (options.token) headers["Authorization"] = `Bearer ${options.token}`;

  const res = await fetch(`${_baseUrl}/api/fn/${name}`, {
    method: "POST",
    headers,
    body: JSON.stringify(args),
  });
  if (!res.ok || !res.body) {
    throw new Error(`Stream failed: HTTP ${res.status}`);
  }

  const reader = res.body.getReader();
  const decoder = new TextDecoder();
  let buffer = "";
  let finalResult: unknown = undefined;

  while (true) {
    const { done, value } = await reader.read();
    if (done) break;
    buffer += decoder.decode(value, { stream: true });
    const events = buffer.split("\n\n");
    buffer = events.pop() || "";

    for (const evt of events) {
      if (!evt.trim()) continue;
      let eventType = "message";
      let data = "";
      for (const line of evt.split("\n")) {
        if (line.startsWith("event: ")) eventType = line.slice(7);
        else if (line.startsWith("data: ")) data += line.slice(6);
      }
      if (eventType === "result") {
        try {
          finalResult = JSON.parse(data);
        } catch {
          finalResult = data;
        }
      } else if (eventType === "error") {
        try {
          const err = JSON.parse(data) as { message?: string };
          throw new Error(err.message || "Function error");
        } catch (e) {
          throw e instanceof Error ? e : new Error(String(e));
        }
      } else {
        yield data;
      }
    }
  }

  return finalResult;
}

/**
 * List all server-side functions available.
 */
export async function listFns(): Promise<
  { name: string; fn_type: "query" | "mutation" | "action" }[]
> {
  return apiRequest("GET", "/api/fn") as Promise<
    { name: string; fn_type: "query" | "mutation" | "action" }[]
  >;
}

// ---------------------------------------------------------------------------
// File upload
// ---------------------------------------------------------------------------

export interface UploadedFile {
  id: string;
  url: string;
  size: number;
}

/**
 * Upload a file (File/Blob or raw bytes) to /api/files/upload.
 *
 * For File / Blob inputs this sends a single raw binary request with the
 * filename and content-type as headers (the server short-circuits on this
 * shape so uploads avoid being coerced through string-based handling).
 *
 * @example
 * ```ts
 * const uploaded = await uploadFile(fileFromInput);
 * console.log(uploaded.url, uploaded.id, uploaded.size);
 * ```
 */
export async function uploadFile(
  input: File | Blob | ArrayBuffer | Uint8Array,
  options: {
    filename?: string;
    contentType?: string;
    token?: string;
  } = {}
): Promise<UploadedFile> {
  let body: BodyInit;
  let filename = options.filename;
  let contentType = options.contentType;

  if (typeof File !== "undefined" && input instanceof File) {
    body = input;
    filename ??= input.name;
    contentType ??= input.type || "application/octet-stream";
  } else if (typeof Blob !== "undefined" && input instanceof Blob) {
    body = input;
    contentType ??= input.type || "application/octet-stream";
  } else if (input instanceof ArrayBuffer) {
    body = input;
  } else {
    body = input as Uint8Array;
  }

  filename ??= "upload";
  contentType ??= "application/octet-stream";

  const headers: Record<string, string> = {
    "Content-Type": contentType,
    "X-Filename": filename,
  };
  if (options.token) headers["Authorization"] = `Bearer ${options.token}`;

  const res = await fetch(`${_baseUrl}/api/files/upload`, {
    method: "POST",
    headers,
    body,
  });

  if (!res.ok) {
    const err = (await res.json().catch(() => ({}))) as {
      error?: { code: string; message: string };
    };
    throw new Error(err.error?.message || `Upload failed: HTTP ${res.status}`);
  }

  return (await res.json()) as UploadedFile;
}

/**
 * Upload via multipart/form-data. Useful when the app needs to pass extra
 * fields alongside the file (captions, categories, etc.), though only the
 * first file part is stored today.
 */
export async function uploadFileMultipart(
  file: File | Blob,
  fields: Record<string, string> = {},
  options: { token?: string } = {}
): Promise<UploadedFile> {
  const form = new FormData();
  for (const [k, v] of Object.entries(fields)) {
    form.append(k, v);
  }
  form.append("file", file);

  const headers: Record<string, string> = {};
  if (options.token) headers["Authorization"] = `Bearer ${options.token}`;

  const res = await fetch(`${_baseUrl}/api/files/upload`, {
    method: "POST",
    headers,
    body: form,
  });
  if (!res.ok) {
    const err = (await res.json().catch(() => ({}))) as {
      error?: { code: string; message: string };
    };
    throw new Error(err.error?.message || `Upload failed: HTTP ${res.status}`);
  }
  return (await res.json()) as UploadedFile;
}
