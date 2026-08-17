export { defineRoute } from "@pylonsync/sdk";
export type { RouteMode, AppManifest } from "@pylonsync/sdk";

// SSR primitives — Next.js-style <Link> and <Image>. Both render
// progressively (work without JS) and enhance on the client.
export { Link } from "./Link";
export type { LinkProps } from "./Link";
export { dynamic } from "./dynamic";
export type { DynamicOptions } from "./dynamic";
export { Image } from "./Image";
export type { ImageProps } from "./Image";
export { Form } from "./Form";
export type { FormProps } from "./Form";

// Dynamic image generation for the `opengraph-image.tsx` / `twitter-image.tsx`
// file convention (Next.js `next/og` parity). `ImageResponse` is a thin
// holder — the SSR runtime does the Satori→resvg render server-side.
export { ImageResponse, isImageResponse } from "./image-response";
export type {
  ImageResponseOptions,
  ImageResponseFont,
} from "./image-response";

// SSR page-author types — the contract every `app/**/page.tsx` is handed
// in props, plus `metadata` / `generateMetadata`. Type-only.
export type {
  PageProps,
  PageAuth,
  ServerData,
  SsrResponse,
  SsrCookieOptions,
  Metadata,
  GenerateMetadata,
  Sitemap,
  SitemapEntry,
  Robots,
  RobotsRule,
  RouteSegmentConfig,
  ErrorBoundaryProps,
  NotFoundProps,
  FormFields,
  FormDb,
  FormRequest,
  RouteHandler,
  RawRouteHandler,
  RawResponse,
} from "./ssr";

// Client navigation hooks for SSR pages (Next-style).
export {
  useRouter,
  useSearchParams,
  usePathname,
  useParams,
  useRouteSeed,
  useRouteData,
  redirect,
  notFound,
  NotFoundError,
} from "./useRouter";
export type { PylonRouter } from "./useRouter";

import {
  defaultStorage,
  pylonFetch,
  pylonFetchRaw,
  type Storage as PylonStorage,
} from "@pylonsync/sync";
import { peekActiveEngine } from "./engine-registry";

// React hooks — high-level ergonomic shape
export {
  useQuery,
  useQueryOne,
  useReactiveQuery,
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
  UseReactiveQueryReturn,
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
export { useRoom, useRoomMessages } from "./useRoom";
export type {
  RoomPeer,
  RoomSnapshot,
  UseRoomOptions,
  UseRoomReturn,
} from "./useRoom";
export type { RoomMessage } from "@pylonsync/sync";

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

export { useSyncStatus } from "./useSyncStatus";
export type { SyncConnectionStatus } from "./useSyncStatus";

// One-liner API
export { db, init, getSync, setSessionToken } from "./db";

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

/** Current effective base URL. Used by hooks (useRoom, useShard) and the
 *  @pylonsync/client auth helpers (createOrg, passwordRegister, createInvite,
 *  …) that share the client config but don't have access to the module-private
 *  state.
 *
 *  When NOT explicitly configured, default to the page origin in a browser
 *  instead of the `http://localhost:4321` dev constant. A unified SSR/embedded
 *  app serves its API same-origin, so the static default was a footgun: every
 *  auth/org call fired at `localhost:4321` — broken on any non-4321 dev port
 *  AND in production (it would hit the engineer's dev port, not the app's
 *  domain). `init()`/`createSyncEngine` already resolve `window.location.origin`
 *  for the sync engine; this brings the auth helpers to the same origin so the
 *  two never disagree. An explicit `configureClient({ baseUrl })` still wins
 *  (separate-origin API setups), and SSR/node (no `window`) keeps the dev
 *  default (server-side calls use same-process paths anyway). */
export function getBaseUrl(): string {
  if (
    !_baseUrlConfigured &&
    typeof window !== "undefined" &&
    window.location?.origin
  ) {
    return window.location.origin;
  }
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

/**
 * Build the transport config for the React free helpers — base URL,
 * token getter, and cookie credentials are all centralized in
 * `pylonFetch`. Cookie-auth apps work because the transport always
 * sets `credentials: "include"`; bearer-auth apps work because
 * `getToken` returns the cached session token.
 */
function transportConfig(): import("@pylonsync/sync").TransportConfig {
  return {
    baseUrl: getBaseUrl(),
    getToken: () => currentAuthToken() ?? undefined,
  };
}

async function apiRequest(
  method: string,
  path: string,
  body?: unknown,
): Promise<unknown> {
  assertBaseUrlSafeForEnv();
  return pylonFetch(transportConfig(), path, {
    method: method as "GET" | "POST" | "PUT" | "PATCH" | "DELETE",
    json: body,
  });
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
): Promise<{ user_id: string | null; avatar_url?: string | null }> {
  return pylonFetch<{ user_id: string | null; avatar_url?: string | null }>(
    { baseUrl: getBaseUrl(), token },
    "/api/auth/me",
  );
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
  try {
    return await pylonFetch<{
      token: string;
      user_id: string;
      expires_at: number;
    }>({ baseUrl: getBaseUrl(), token }, "/api/auth/refresh", { method: "POST" });
  } catch {
    return null;
  }
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
 * Read the auth token from the configured storage adapter (default:
 * localStorage on the web). React Native and other non-browser hosts
 * inject their own adapter via `setReactStorage` so `callFn` and the
 * other free helpers send the right token without each call site
 * threading it explicitly.
 */
function currentAuthToken(): string | undefined {
  return _storage.get(storageKey("token")) ?? undefined;
}

let _storage: PylonStorage = defaultStorage();

/**
 * Swap the storage adapter used by the React free helpers (`callFn`,
 * `useSession`, `getAuthToken`, etc). React Native's `init()` calls this
 * with an AsyncStorage-backed adapter so token reads/writes go through
 * the same backend as the sync engine.
 */
export function setReactStorage(storage: PylonStorage): void {
  _storage = storage;
}

/** Current storage adapter used by the React layer. Exposed for adapters. */
export function getReactStorage(): PylonStorage {
  return _storage;
}

export async function callFn<T = unknown>(
  name: string,
  args: Record<string, unknown> = {},
  options: { token?: string } = {}
): Promise<T> {
  // Route through the engine when the app has one. `SyncEngine.fn`
  // observes the `X-Pylon-Change-Seq` response header and fires a
  // one-shot pull when the call produced events the replica hasn't
  // seen — without it the caller's own write doesn't appear until
  // something else refreshes, which reads as "I added it and nothing
  // happened". `db.fn` and `useMutation` have always done this; the
  // free helper silently didn't, so the same operation behaved
  // differently depending on which import you reached for.
  //
  // Two cases deliberately keep the bare fetch:
  //   - no engine: apps that never call `init()` must not have one
  //     constructed (and a WebSocket opened) by a plain POST.
  //   - an explicit `options.token`: that's a call as some other
  //     identity, and pulling its results into this replica would mix
  //     two users' data.
  const engine = options.token ? null : peekActiveEngine();
  if (engine) {
    return engine.fn<T>(name, args);
  }
  return pylonFetch<T>(
    {
      baseUrl: getBaseUrl(),
      getToken: () => options.token ?? currentAuthToken() ?? undefined,
    },
    `/api/fn/${name}`,
    { method: "POST", json: args },
  );
}

/** Options for {@link streamFn}. */
export interface StreamFnOptions {
  token?: string;
  /**
   * Auto-resume on abrupt network failure (default true). The server
   * buffers every fn stream server-side; when the connection drops
   * mid-stream, the generator silently reconnects to
   * `GET /api/fn-streams/<id>` from its last seen cursor and keeps
   * yielding — no chunk is duplicated or lost, and the final result
   * still arrives even if the reconnect lands after the handler
   * finished. Set false to surface disconnects as errors instead.
   */
  resume?: boolean;
  /** Reconnect attempts per silent gap (default 3). The budget resets
   *  whenever a resumed connection makes progress. */
  maxResumes?: number;
  /**
   * Called with the server-assigned stream id as soon as the response
   * upgrades to SSE. Persist it (e.g. in your row for the agent run)
   * to resume from a different tab or a full page reload via
   * {@link resumeStream}.
   */
  onStreamId?: (streamId: string) => void;
}

interface SseCursor {
  lastSeq: number;
  /** Set once the terminal result/error frame was seen. */
  terminal: boolean;
  finalResult: unknown;
}

/** Parse one SSE response body, yielding data frames and tracking the
 *  cursor. Multi-line payloads rejoin `data:` lines with '\n' per the
 *  SSE spec (the server splits them the same way). */
async function* consumeSseBody(
  res: Response,
  cursor: SseCursor,
): AsyncGenerator<string, void, unknown> {
  if (!res.body) throw new Error(`Stream failed: HTTP ${res.status}`);
  const reader = res.body.getReader();
  const decoder = new TextDecoder();
  let buffer = "";
  try {
  while (true) {
    const { done, value } = await reader.read();
    if (done) break;
    buffer += decoder.decode(value, { stream: true });
    const events = buffer.split("\n\n");
    buffer = events.pop() || "";

    for (const evt of events) {
      if (!evt.trim() || evt.startsWith(":")) continue; // heartbeat
      let eventType = "message";
      const dataLines: string[] = [];
      for (const line of evt.split("\n")) {
        if (line.startsWith("event: ")) eventType = line.slice(7);
        else if (line.startsWith("data: ")) dataLines.push(line.slice(6));
        else if (line.startsWith("data:")) dataLines.push(line.slice(5));
        else if (line.startsWith("id: ")) {
          const seq = Number.parseInt(line.slice(4), 10);
          if (Number.isFinite(seq)) cursor.lastSeq = seq;
        }
      }
      // Frames without any data line (the `retry:` prelude) carry
      // nothing to deliver.
      if (dataLines.length === 0) continue;
      const data = dataLines.join("\n");
      if (eventType === "result") {
        cursor.terminal = true;
        try {
          cursor.finalResult = JSON.parse(data);
        } catch {
          cursor.finalResult = data;
        }
      } else if (eventType === "error") {
        cursor.terminal = true;
        let message = "Function error";
        try {
          const err = JSON.parse(data) as { message?: string };
          if (err.message) message = err.message;
        } catch {
          // non-JSON error payload — keep the generic message
        }
        throw new Error(message);
      } else {
        yield data;
      }
    }
  }
  } finally {
    // Runs on normal completion AND when the consumer breaks out of
    // its for-await (generator return). Without the cancel, the fetch
    // body stays locked and the connection pins a browser socket (and
    // a server stream slot) until page unload.
    reader.cancel().catch(() => {});
  }
}

/**
 * Stream a server-side function's output as Server-Sent Events.
 *
 * Streams are resumable: the server buffers every frame under a stream
 * id (see `X-Pylon-Stream-Id`), so a dropped connection reconnects and
 * catches up transparently — including receiving the final result
 * after the handler already finished. Disable with `resume: false`.
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
  options: StreamFnOptions = {}
): AsyncGenerator<string, unknown, unknown> {
  const transport = {
    baseUrl: getBaseUrl(),
    getToken: () => options.token ?? currentAuthToken() ?? undefined,
  };
  // Streaming response — use pylonFetchRaw so we can read .body
  // ourselves. URL + auth + credentials are centralized in the
  // transport.
  const res = await pylonFetchRaw(transport, `/api/fn/${name}`, {
    method: "POST",
    json: args,
    accept: "text/event-stream",
  });
  // The server only upgrades to SSE once the handler actually calls
  // ctx.stream. A function that completes without streaming answers
  // with plain JSON (Content-Type: application/json) — the raw return
  // value on 200, `{"error":{code,message}}` on 400 — so SSE-capable
  // clients that hit a non-streaming function still get a result.
  const contentType = res.headers.get("content-type") ?? "";
  if (!contentType.includes("text/event-stream")) {
    const text = await res.text();
    if (!res.ok) {
      let message = `Stream failed: HTTP ${res.status}`;
      try {
        const err = JSON.parse(text) as { error?: { message?: string } };
        if (err.error?.message) message = err.error.message;
      } catch {
        // non-JSON error body — keep the HTTP status message
      }
      throw new Error(message);
    }
    try {
      return JSON.parse(text) as unknown;
    } catch {
      return text;
    }
  }
  if (!res.ok) {
    throw new Error(`Stream failed: HTTP ${res.status}`);
  }

  const streamId = res.headers.get("x-pylon-stream-id");
  if (streamId && options.onStreamId) options.onStreamId(streamId);

  const cursor: SseCursor = { lastSeq: 0, terminal: false, finalResult: undefined };
  const resume = options.resume !== false && Boolean(streamId);
  let attemptsLeft = options.maxResumes ?? 3;

  // First pass over the live connection, then (on silent disconnect)
  // resume passes over /api/fn-streams until the terminal frame.
  let body: AsyncGenerator<string, void, unknown> = consumeSseBody(res, cursor);
  for (;;) {
    let progressed = false;
    try {
      for await (const chunk of body) {
        progressed = true;
        yield chunk;
      }
    } catch (e) {
      if (cursor.terminal) throw e; // server-sent error frame — real
      if (!resume || attemptsLeft <= 0) throw e; // network error, no budget
    }
    if (cursor.terminal) return cursor.finalResult;
    // Connection ended without a terminal frame — silent disconnect.
    if (!resume || attemptsLeft <= 0) {
      throw new Error("Stream disconnected before completing");
    }
    if (progressed) attemptsLeft = options.maxResumes ?? 3;
    attemptsLeft -= 1;
    const resumed = await pylonFetchRaw(
      transport,
      `/api/fn-streams/${streamId}?since=${cursor.lastSeq}`,
      { method: "GET", accept: "text/event-stream" },
    );
    if (resumed.status === 503 && attemptsLeft > 0) {
      // STREAM_OVERLOADED is transient by definition (Retry-After: 1) —
      // failing here would kill exactly the stream resume exists to save.
      await resumed.text().catch(() => {});
      await new Promise((r) => setTimeout(r, 1000));
      continue;
    }
    if (!resumed.ok) {
      const text = await resumed.text().catch(() => "");
      throw new Error(
        `Stream resume failed: HTTP ${resumed.status}${text ? ` — ${text}` : ""}`,
      );
    }
    body = consumeSseBody(resumed, cursor);
  }
}

/**
 * Attach to a buffered fn stream by id — from another tab, a page
 * reload, or any time after the original `streamFn` call went away.
 * Replays from `since` (default 0 = everything, including the final
 * result if the handler already finished), then live-tails.
 *
 * Get the id from `streamFn`'s `onStreamId` callback and persist it
 * wherever the run's state lives. Completed streams stay resumable for
 * an hour (PYLON_STREAM_RETAIN_SECS).
 */
export async function* resumeStream(
  streamId: string,
  options: { token?: string; since?: number } = {}
): AsyncGenerator<string, unknown, unknown> {
  const transport = {
    baseUrl: getBaseUrl(),
    getToken: () => options.token ?? currentAuthToken() ?? undefined,
  };
  const cursor: SseCursor = {
    lastSeq: options.since ?? 0,
    terminal: false,
    finalResult: undefined,
  };
  let attemptsLeft = 3;
  for (;;) {
    const res = await pylonFetchRaw(
      transport,
      `/api/fn-streams/${streamId}?since=${cursor.lastSeq}`,
      { method: "GET", accept: "text/event-stream" },
    );
    if (res.status === 503 && attemptsLeft > 0) {
      attemptsLeft -= 1;
      await res.text().catch(() => {});
      await new Promise((r) => setTimeout(r, 1000));
      continue;
    }
    if (!res.ok) {
      const text = await res.text().catch(() => "");
      let message = `Stream resume failed: HTTP ${res.status}`;
      try {
        const err = JSON.parse(text) as { error?: { message?: string } };
        if (err.error?.message) message = err.error.message;
      } catch {
        // keep the HTTP message
      }
      throw new Error(message);
    }
    let progressed = false;
    try {
      for await (const chunk of consumeSseBody(res, cursor)) {
        progressed = true;
        yield chunk;
      }
    } catch (e) {
      if (cursor.terminal || attemptsLeft <= 0) throw e;
    }
    if (cursor.terminal) return cursor.finalResult;
    if (progressed) attemptsLeft = 3;
    attemptsLeft -= 1;
    if (attemptsLeft < 0) {
      throw new Error("Stream disconnected before completing");
    }
  }
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
    // Newer TS lib types refuse `Uint8Array<ArrayBufferLike>` as BodyInit
    // directly even though every runtime accepts it. Hand fetch the
    // underlying ArrayBuffer slice to sidestep the type narrowing.
    const u8 = input as Uint8Array;
    body = u8.buffer.slice(u8.byteOffset, u8.byteOffset + u8.byteLength) as ArrayBuffer;
  }

  filename ??= "upload";
  contentType ??= "application/octet-stream";

  return pylonFetch<UploadedFile>(
    {
      baseUrl: getBaseUrl(),
      getToken: () => options.token ?? currentAuthToken() ?? undefined,
    },
    "/api/files/upload",
    {
      method: "POST",
      body,
      headers: {
        "Content-Type": contentType,
        "X-Filename": filename,
      },
    },
  );
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

  return pylonFetch<UploadedFile>(
    {
      baseUrl: getBaseUrl(),
      getToken: () => options.token ?? currentAuthToken() ?? undefined,
    },
    "/api/files/upload",
    { method: "POST", body: form },
  );
}
