/**
 * Raw HTTP responses from webhook actions.
 *
 * An action called through `/api/webhooks/<name>` normally answers with its
 * return value as JSON and status 200. Returning `ctx.response(...)` sends
 * the given status, content type, headers, and body instead. Every other
 * route (`/api/fn/<name>`, `ctx.runAction`) returns the value as ordinary
 * JSON data.
 *
 * The host enforces the same rules (crates/router/src/raw_response.rs), so
 * a hand-built object cannot bypass them. The two header lists must stay in
 * sync.
 */

/** What `ctx.response` accepts. */
export interface RawResponseInit {
  /** HTTP status, 200-599. Default 200. */
  status?: number;
  /** Shorthand for the `Content-Type` header. Default `text/plain; charset=utf-8`. */
  contentType?: string;
  /** Extra response headers. Values must be visible ASCII. */
  headers?: Record<string, string>;
  /** Response body. Default empty. Must be empty for 204, 205, and 304. */
  body?: string;
}

/** The marked value an action returns. Build it with `ctx.response`. */
export interface RawResponse {
  readonly __pylonResponse: 1;
  readonly status: number;
  readonly headers: Readonly<Record<string, string>>;
  readonly body: string;
}

/** Header names the server sets itself. Compared lowercase. */
const RESERVED_HEADERS = new Set([
  "connection",
  "content-length",
  "keep-alive",
  "proxy-authenticate",
  "proxy-authorization",
  "proxy-connection",
  "te",
  "trailer",
  "transfer-encoding",
  "upgrade",
  "set-cookie",
  "permissions-policy",
  "referrer-policy",
  "x-content-type-options",
  "x-frame-options",
  "x-xss-protection",
]);
const RESERVED_HEADER_PREFIXES = ["access-control-"];

const TOKEN_RE = /^[!#$%&'*+\-.^_`|~0-9A-Za-z]+$/;
const VALUE_RE = /^[\t\x20-\x7e]*$/;

function checkHeader(name: string, value: unknown): void {
  if (!TOKEN_RE.test(name)) {
    throw new Error(`ctx.response: header name ${JSON.stringify(name)} is not a valid HTTP token`);
  }
  if (typeof value !== "string") {
    throw new Error(`ctx.response: header "${name}" must have a string value`);
  }
  if (!VALUE_RE.test(value)) {
    throw new Error(
      `ctx.response: header "${name}" has a value with control or non-ASCII characters`,
    );
  }
  const lower = name.toLowerCase();
  if (RESERVED_HEADERS.has(lower) || RESERVED_HEADER_PREFIXES.some((p) => lower.startsWith(p))) {
    throw new Error(`ctx.response: header "${name}" is set by the server`);
  }
}

/**
 * Build a raw HTTP response for a webhook action to return.
 *
 * ```ts
 * return ctx.response({ contentType: "text/xml", body: "<Response/>" });
 * ```
 */
export function response(init: RawResponseInit = {}): RawResponse {
  const status = init.status ?? 200;
  if (!Number.isInteger(status) || status < 200 || status > 599) {
    throw new Error(`ctx.response: status must be an integer from 200 to 599, got ${status}`);
  }
  const body = init.body ?? "";
  if (typeof body !== "string") {
    throw new Error("ctx.response: body must be a string");
  }
  if (body !== "" && (status === 204 || status === 205 || status === 304)) {
    throw new Error(`ctx.response: status ${status} cannot carry a body`);
  }
  const headers: Record<string, string> = {};
  for (const [name, value] of Object.entries(init.headers ?? {})) {
    checkHeader(name, value);
    headers[name] = value;
  }
  if (init.contentType !== undefined) {
    checkHeader("Content-Type", init.contentType);
    for (const name of Object.keys(headers)) {
      if (name.toLowerCase() === "content-type") delete headers[name];
    }
    headers["content-type"] = init.contentType;
  }
  return { __pylonResponse: 1, status, headers, body };
}
