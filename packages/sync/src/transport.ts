// ---------------------------------------------------------------------------
// Shared client HTTP transport.
//
// One helper that every browser-side Pylon call funnels through:
// `pylonFetch(config, path, init)`. Consistently handles:
//
//  - base URL resolution (config.baseUrl, falling back to
//    window.location.origin on the browser when omitted)
//  - bearer token attachment (config.token or token-getter callback)
//  - `credentials: "include"` for cookie-auth apps
//  - request-body JSON serialization (when `init.json` is set)
//  - response JSON parsing with structured error envelope:
//      { error: { code, message } }
//  - non-JSON error bodies (HTML proxy pages, empty 204s) → synthetic
//    Error with status + code
//  - X-Pylon-Change-Seq response header surfaced via the optional
//    `onChangeSeq` callback (so callers can trigger catch-up pulls
//    when a mutation's seq exceeds their local cursor)
//
// Streaming + raw-binary callers (file uploads, SSE) bypass the
// JSON-parse step but reuse the URL/auth/credentials logic via
// `buildRequest`.
// ---------------------------------------------------------------------------

/**
 * What every caller of `pylonFetch` must supply. The SyncEngine
 * passes its own config; the React free helpers pass a lightweight
 * shim built from `getBaseUrl()` + `currentAuthToken()`.
 */
export interface TransportConfig {
  baseUrl?: string;
  /** Static bearer token. Use `getToken` if the token can change. */
  token?: string;
  /** Lazy bearer-token resolver — called per request. Use this when
   *  the token can be rotated mid-session (refresh, session-changed).
   *  Returning null/undefined means "no auth header"; cookie-auth
   *  apps rely on `credentials: "include"` instead. */
  getToken?: () => string | null | undefined;
  /** Invoked with the value of the X-Pylon-Change-Seq response
   *  header when the server sets one. Returning a Promise pauses no
   *  one — the transport doesn't await it. */
  onChangeSeq?: (seq: number) => void;
}

/**
 * Per-call init shape. Mirrors the standard `RequestInit` but
 * normalizes the two body shapes (JSON-encode-this vs raw-pass-
 * through) into separate fields so callers don't have to remember
 * to set Content-Type.
 */
export interface PylonRequestInit {
  method?: "GET" | "POST" | "PUT" | "PATCH" | "DELETE";
  /** When set, JSON-stringified into the body and Content-Type
   *  defaults to application/json. */
  json?: unknown;
  /** Raw body — passed through to fetch unchanged. Use this for file
   *  uploads (Blob, ArrayBuffer, FormData). */
  body?: BodyInit;
  /** Extra headers. Caller-supplied keys win against the transport's
   *  defaults. */
  headers?: Record<string, string>;
  /** Override the request's Accept header (useful for SSE streams). */
  accept?: string;
  /** AbortController signal for cancellation. */
  signal?: AbortSignal;
}

/**
 * Resolve the base URL. Browser-side fallback to
 * `window.location.origin` matches the SDK contract: when
 * `configureClient` was called with no baseUrl, calls go to the
 * current document's origin so dev + prod work without env
 * gymnastics.
 */
export function resolveBaseUrl(config: TransportConfig): string {
  if (config.baseUrl) return config.baseUrl.replace(/\/+$/, "");
  if (typeof window !== "undefined" && window.location?.origin) {
    return window.location.origin;
  }
  return "";
}

/**
 * Assemble fetch URL + RequestInit. Exposed for streaming / raw-
 * response callers (SSE, file upload) that need to handle the
 * Response themselves but want the transport's auth + credentials +
 * URL logic.
 */
export function buildRequest(
  config: TransportConfig,
  path: string,
  init: PylonRequestInit = {},
): { url: string; init: RequestInit } {
  const base = resolveBaseUrl(config);
  const url = `${base}${path}`;
  const headers: Record<string, string> = { ...(init.headers ?? {}) };
  // Auth: explicit getToken wins over static token. Either may be
  // null/undefined — cookie-auth apps rely solely on credentials.
  const token = config.getToken?.() ?? config.token;
  if (token) headers["Authorization"] = `Bearer ${token}`;
  if (init.accept) headers["Accept"] = init.accept;
  let body: BodyInit | undefined = init.body;
  if (init.json !== undefined) {
    if (!headers["Content-Type"]) headers["Content-Type"] = "application/json";
    body = JSON.stringify(init.json);
  }
  return {
    url,
    init: {
      method: init.method ?? (body !== undefined ? "POST" : "GET"),
      headers,
      // `credentials: "include"` is mandatory for cookie-auth apps.
      // Bearer-auth apps don't care (no cookie to send) so the cost
      // is the same. Browser CORS rules require the server to set
      // `Access-Control-Allow-Credentials: true` and a specific
      // origin (not `*`) for the cookie to be sent — Pylon's CORS
      // middleware does this by default.
      credentials: "include",
      body,
      signal: init.signal,
    },
  };
}

/** Error thrown by `pylonFetch` for non-2xx responses. Carries the
 *  status + structured error code + the parsed JSON body (if any). */
export class PylonHttpError extends Error {
  status: number;
  code?: string;
  body?: unknown;
  constructor(message: string, status: number, code?: string, body?: unknown) {
    super(message);
    this.name = "PylonHttpError";
    this.status = status;
    this.code = code;
    this.body = body;
  }
}

/**
 * The canonical client HTTP call. Returns the parsed JSON body on
 * 2xx. Throws `PylonHttpError` on non-2xx. Empty 204 bodies parse
 * as `null`.
 *
 * Surfaces `X-Pylon-Change-Seq` via `config.onChangeSeq` so the
 * SyncEngine can fire a catch-up pull when a mutation's seq is
 * ahead of its local cursor — matches the
 * `requestWithChangeSync` shape but lifts the logic out of the
 * engine so React free helpers (`callFn`, `apiRequest`) get it too.
 */
export async function pylonFetch<T = unknown>(
  config: TransportConfig,
  path: string,
  init: PylonRequestInit = {},
): Promise<T> {
  const { url, init: req } = buildRequest(config, path, init);
  const res = await fetch(url, req);
  const text = await res.text();
  let parsed: unknown = null;
  if (text) {
    try {
      parsed = JSON.parse(text);
    } catch {
      // Non-JSON body — proxy HTML error page, etc. Fall through;
      // the !res.ok branch synthesizes a useful error.
    }
  }
  // Surface change-seq for catch-up logic. Best-effort: a malformed
  // header doesn't fail the request.
  if (config.onChangeSeq) {
    const header = res.headers.get("x-pylon-change-seq");
    if (header) {
      const seq = Number.parseInt(header, 10);
      if (Number.isFinite(seq) && seq > 0) {
        config.onChangeSeq(seq);
      }
    }
  }
  if (!res.ok) {
    const err = parsed as { error?: { code?: string; message?: string } } | null;
    const message =
      err?.error?.message ??
      `${req.method ?? "GET"} ${path} failed: ${res.status}`;
    throw new PylonHttpError(message, res.status, err?.error?.code, parsed);
  }
  return parsed as T;
}

/**
 * Streaming variant — returns the raw `Response` so callers can read
 * `.body` (SSE), `.blob()` (file download), etc. Bypasses JSON
 * parsing but keeps the URL + auth + credentials logic.
 *
 * Caller is responsible for status checking and the
 * X-Pylon-Change-Seq callback (we don't read the response without
 * the caller's involvement).
 */
export async function pylonFetchRaw(
  config: TransportConfig,
  path: string,
  init: PylonRequestInit = {},
): Promise<Response> {
  const { url, init: req } = buildRequest(config, path, init);
  return fetch(url, req);
}
