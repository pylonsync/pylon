// SSR handler — invoked by runtime.ts when the host sends a
// "render_route" message. Dynamically imports the page module, calls
// react-dom/server.renderToReadableStream, base64-encodes chunks back
// over the NDJSON pipe.
//
// The whole module is loaded lazily (handleRenderRoute is awaited from
// the dispatch arm) so projects without SSR routes pay nothing — no
// react-dom dependency requirement, no startup cost.

// Bun runtime global. This module runs under Bun but is type-checked by
// consuming apps under node/DOM (no `Bun` global) — declare the surface used.
declare const Bun: {
  resolveSync?(specifier: string, from: string): string;
};

/**
 * Is the runtime in dev mode? MUST match the Rust host's `is_dev_mode()`
 * (crates/runtime/src/frontend.rs): `PYLON_DEV_MODE` is on ONLY for the exact
 * strings "1" or "true" (case-insensitive). A bare `if (process.env.PYLON_DEV_MODE)`
 * is WRONG — the string "false"/"0" is truthy in JS, so an explicit
 * `PYLON_DEV_MODE=false` on a PROD machine would wrongly enable dev behavior
 * (e.g. the live-reload `<script>` was being injected into prod pages, whose
 * EventSource then 404-retried `/_pylon/dev/live` forever).
 */
export function isDevMode(): boolean {
  const v = process.env.PYLON_DEV_MODE;
  return v === "1" || v?.toLowerCase() === "true";
}

/**
 * The message payload the host sends. Matches RenderRouteMessage in
 * crates/functions/src/protocol.rs.
 */
export interface RenderRouteMessage {
  type: "render_route";
  call_id: string;
  /**
   * Project-relative module path (e.g. "app/hello/page"). The
   * adapter joins cwd + this + the right extension (.tsx → .ts).
   */
  component: string;
  /**
   * Project-relative module paths for the layout chain, walked
   * root → leaf. Each layout's default export wraps the next as
   * `children`. Absent / empty when no layouts apply.
   *
   * Example:
   *   layouts: ["app/layout", "app/blog/layout"]
   *   component: "app/blog/[slug]/page"
   *
   * Resolves to:
   *   <RootLayout>
   *     <BlogLayout>
   *       <Page {...props} />
   *     </BlogLayout>
   *   </RootLayout>
   */
  layouts?: string[];
  /** The matched route pattern (e.g. `/blog/:slug`). */
  route_path: string;
  /** The incoming URL path (e.g. `/blog/hello-world`). */
  url: string;
  /** Dynamic-segment matches keyed by name (e.g. `{slug: "hello-world"}`). */
  params: Record<string, string>;
  /** Parsed query string. */
  search_params: Record<string, string>;
  /** Lowercased header names → values. */
  headers: Record<string, string>;
  /** Parsed cookies. */
  cookies: Record<string, string>;
  /** Pylon auth context. */
  auth: {
    user_id: string | null;
    is_admin: boolean;
    tenant_id: string | null;
    roles: string[];
  };
  /**
   * Whether the request carried a session cookie (presence, not identity) —
   * surfaced as the identity-free `props.session.exists` (Phase 0 auth
   * bucketing). Default false / absent for back-compat.
   */
  session_present?: boolean;
  /**
   * Initial HTTP status the response controller starts at (default 200).
   * The host sets this to 404 when dispatching a `not-found.tsx` render
   * for an unmatched URL, so the boundary streams at 404 without the
   * component having to call `response.setStatus`. A page can still
   * override it via `response.setStatus`.
   */
  initial_status?: number;
}

type Send = (msg: Record<string, unknown>) => void;

/**
 * Control-flow signal a page or layout throws to short-circuit the
 * render: `response.redirect(url)` / `response.notFound()`. The adapter
 * catches it and turns it into a 3xx + Location or a 404 instead of a
 * normal 200 body. Extends Error so React's stream rejects cleanly when
 * it's thrown during the shell render. (Throw it OUTSIDE an error
 * boundary — an enclosing boundary would swallow the signal.)
 */
export class PylonRouteControl extends Error {
  kind: "redirect" | "notFound";
  url?: string;
  redirectStatus?: number;
  constructor(kind: "redirect" | "notFound") {
    super(`__pylon_route_${kind}`);
    this.kind = kind;
  }
}

/**
 * Digest brand that `@pylonsync/react`'s `notFound()` stamps on the error it
 * throws. We recognize it here by string instead of importing the class so the
 * runtime stays decoupled from the React package (which doesn't depend on
 * functions). Keep in sync with `NotFoundError.digest` in
 * `packages/react/src/useRouter.ts`.
 */
const REACT_NOT_FOUND_DIGEST = "PYLON_NOT_FOUND";

/**
 * Normalize a thrown value into a route-control signal: either the framework's
 * own `PylonRouteControl` (`response.redirect()` / `response.notFound()`) or a
 * branded `NotFoundError` thrown by `@pylonsync/react`'s `notFound()` from a
 * page/layout render. Returns `null` for an ordinary error so the caller falls
 * through to its real error-handling path.
 */
export function asRouteControl(err: unknown): PylonRouteControl | null {
  if (err instanceof PylonRouteControl) return err;
  if (
    err != null &&
    typeof err === "object" &&
    (err as { digest?: unknown }).digest === REACT_NOT_FOUND_DIGEST
  ) {
    return new PylonRouteControl("notFound");
  }
  return null;
}

export interface SsrCookieOptions {
  path?: string;
  domain?: string;
  maxAge?: number;
  expires?: Date | string;
  /** Defaults to true (secure default). Pass false for a client-readable cookie. */
  httpOnly?: boolean;
  secure?: boolean;
  /** Defaults to "lax". */
  sameSite?: "strict" | "lax" | "none";
}

/**
 * Reject CR / LF / NUL — the characters that turn a header or cookie
 * value into HTTP response splitting. The host re-checks at the wire
 * edge, but failing here gives the developer a clear error instead of a
 * silently-dropped header.
 */
function assertNoControlChars(s: string, label: string): void {
  if (/[\r\n\0]/.test(s)) {
    throw new Error(`pylon ssr: ${label} must not contain CR, LF, or NUL`);
  }
}

// RFC 6265 / RFC 7230 token — used for cookie + header NAMES.
const TOKEN_RE = /^[A-Za-z0-9!#$%&'*+\-.^_`|~]+$/;

function serializeCookie(
  name: string,
  value: string,
  opts: SsrCookieOptions = {},
): string {
  // Cookie name must be a token — reject anything that could smuggle an
  // attribute or a newline through the name (the value is encoded below).
  if (!TOKEN_RE.test(name)) {
    throw new Error(`pylon ssr: invalid cookie name ${JSON.stringify(name)}`);
  }
  let c = `${name}=${encodeURIComponent(value)}`;
  if (opts.maxAge != null) c += `; Max-Age=${Math.floor(opts.maxAge)}`;
  if (opts.expires) {
    const exp =
      typeof opts.expires === "string"
        ? opts.expires
        : opts.expires.toUTCString();
    assertNoControlChars(exp, "cookie expires");
    c += `; Expires=${exp}`;
  }
  const path = opts.path ?? "/";
  assertNoControlChars(path, "cookie path");
  c += `; Path=${path}`;
  if (opts.domain) {
    assertNoControlChars(opts.domain, "cookie domain");
    c += `; Domain=${opts.domain}`;
  }
  if (opts.httpOnly !== false) c += `; HttpOnly`;
  if (opts.secure) c += `; Secure`;
  const ss = opts.sameSite ?? "lax";
  c += `; SameSite=${ss[0].toUpperCase()}${ss.slice(1)}`;
  return c;
}

export interface ResponseState {
  status: number;
  headers: Record<string, string>;
  cookies: string[];
}

/**
 * The per-render `response` controller handed to every page + layout in
 * props. Pylon already has a backend for data/mutations, so SSR's job is
 * just the response envelope: status, redirects, 404, and the occasional
 * Set-Cookie.
 *
 * IMPORTANT — call these during the SYNCHRONOUS shell render (the
 * component body, before any `await` / Suspense boundary). The HTTP head
 * is committed when the shell is ready; status/headers/cookies set from a
 * suspended subtree that streams in later are lost, and a redirect()/
 * notFound() thrown below a Suspense boundary is caught by React's error
 * handling rather than turned into a 3xx/404 (same constraint as Next's
 * streaming SSR). Per render, not shared across requests.
 */
export interface SsrResponse {
  /** Set the HTTP status (100–599). Default 200. */
  setStatus(code: number): void;
  /** Set a response header (name must be a token; value CR/LF/NUL-free). */
  setHeader(name: string, value: string): void;
  /** Append a Set-Cookie. Defaults: HttpOnly + SameSite=Lax. */
  setCookie(name: string, value: string, opts?: SsrCookieOptions): void;
  /** Throw to send a 3xx (default 307) + Location, no body. Shell-render only. */
  redirect(url: string, status?: number): never;
  /**
   * Throw to send a 404. Renders the nearest `not-found.tsx` (walking up
   * from the page's directory, wrapped in the route's layout chain), or a
   * minimal framework body if none is defined. Shell-render only — a throw
   * below a Suspense boundary is swallowed by React.
   */
  notFound(): never;
}

// `defaultRedirectStatus` is the status `response.redirect(url)` uses when the
// caller doesn't pass one: 307 for a page render (preserve the method on the
// rare redirecting GET), 303 for a route.ts form handler (POST-redirect-GET —
// the browser must follow with a GET, not re-POST).
export function makeResponseController(
  state: ResponseState,
  defaultRedirectStatus = 307,
): SsrResponse {
  return {
    setStatus(code) {
      if (!Number.isInteger(code) || code < 100 || code > 599) {
        throw new Error(
          `pylon ssr: setStatus() expects an HTTP status 100–599, got ${code}`,
        );
      }
      state.status = code;
    },
    setHeader(name, value) {
      if (!TOKEN_RE.test(name)) {
        throw new Error(`pylon ssr: invalid header name ${JSON.stringify(name)}`);
      }
      // `x-pylon-*` is a reserved internal namespace — the host trusts headers
      // like the #277 `x-pylon-cacheable` cache proof as runtime-emitted. Letting
      // userland set one would forge the cache verdict (e.g. mark a personalized
      // render shareable). Reject it loudly.
      if (name.toLowerCase().startsWith("x-pylon-")) {
        throw new Error(
          `pylon ssr: "x-pylon-*" is a reserved internal header namespace and ` +
            `cannot be set via response.setHeader() (got ${JSON.stringify(name)})`,
        );
      }
      assertNoControlChars(value, "header value");
      state.headers[name.toLowerCase()] = value;
    },
    setCookie(name, value, opts) {
      state.cookies.push(serializeCookie(name, value, opts));
    },
    redirect(url, status = defaultRedirectStatus): never {
      assertNoControlChars(url, "redirect url");
      if (!Number.isInteger(status) || status < 300 || status > 399) {
        throw new Error(`pylon ssr: redirect() status must be 3xx, got ${status}`);
      }
      // SECURITY: refuse open redirects. A relative path or a trusted-host
      // absolute is fine; anything else (//evil.com, https://evil.com,
      // javascript:, …) throws — surfaced as a render error, never a silent
      // off-site redirect from `redirect(request-derived-url)`.
      const env = (globalThis as any).process?.env ?? {};
      if (
        !isSafeRedirect(url, {
          publicUrl: env.PYLON_PUBLIC_URL,
          canonicalHost: env.PYLON_CANONICAL_HOST,
          trustedHostsCsv: env.PYLON_TRUSTED_HOSTS,
        })
      ) {
        throw new Error(
          `pylon ssr: redirect() refused an untrusted target ${JSON.stringify(url)} — ` +
            `use a same-site path (e.g. "/dashboard") or add the host to ` +
            `PYLON_TRUSTED_HOSTS. (Refusing to emit an open redirect.)`,
        );
      }
      const e = new PylonRouteControl("redirect");
      e.url = url;
      e.redirectStatus = status;
      throw e;
    },
    notFound(): never {
      throw new PylonRouteControl("notFound");
    },
  };
}

/**
 * Merge page-set headers + cookies into the response_start header map.
 * Cookies are newline-joined under `set-cookie`; the host splits them
 * into one `Set-Cookie` header each (newline is forbidden inside a
 * cookie, so it can't be turned into header injection).
 */
/**
 * Wrap a per-request object so ANY observation — property `get`, `in` (`has`),
 * `Object.keys`/spread/`for…in` (`ownKeys`), or a descriptor probe — calls
 * `onTouch`. Used to mark a render request-specific (vetoing shared caching) the
 * instant it reads auth/headers/cookies. A bare `get` trap misses `in` and
 * `Object.keys`, which would observe the data without tripping the veto.
 * Exported for direct unit testing of that property.
 */
export function makeReadTrackingProxy(
  obj: Record<string, unknown> | undefined,
  onTouch: () => void,
): Record<string, unknown> {
  return makeRevocableReadTrackingProxy(obj, onTouch).proxy;
}

/**
 * Deep clone via JSON round-trip — drops functions / proxies / symbols, keeping
 * only JSON-serializable data. Used to SNAPSHOT the bucket-tail props before any
 * page code can mutate them, so a page can't smuggle identity through a nested
 * field of params/searchParams into a shared cache entry. The deep copy is fully
 * independent of the source: a later mutation of the original object can't reach
 * the snapshot.
 */
export function jsonClone<T>(v: T): T {
  return v == null ? v : (JSON.parse(JSON.stringify(v)) as T);
}

/**
 * Revocable form of {@link makeReadTrackingProxy}. The render path wraps each
 * per-request input (auth / headers / cookies / session) in one of these and
 * REVOKES it once the render is done — so a page that stashed the props object
 * (or `props.auth`) in module-level state and read it on a LATER render gets a
 * hard throw instead of silently reading a prior request's identity WITHOUT
 * tripping this render's read-tracking. Revocation is per-render (each render
 * revokes only its own proxies), so it is sound even when the Bun runner
 * multiplexes concurrent renders. Fail-closed: the stale read throws, the render
 * fails, and nothing is cached.
 */
export function makeRevocableReadTrackingProxy(
  obj: Record<string, unknown> | undefined,
  onTouch: () => void,
): { proxy: Record<string, unknown>; revoke: () => void } {
  return Proxy.revocable((obj ?? {}) as Record<string, unknown>, {
    get(t, p, r) {
      onTouch();
      return Reflect.get(t, p, r);
    },
    has(t, p) {
      onTouch();
      return Reflect.has(t, p);
    },
    ownKeys(t) {
      onTouch();
      return Reflect.ownKeys(t);
    },
    getOwnPropertyDescriptor(t, p) {
      onTouch();
      return Reflect.getOwnPropertyDescriptor(t, p);
    },
  });
}

export function finalizeHeaders(
  state: ResponseState,
  // UNTRUSTED user headers (page-set `state.headers` and, at the form/data-route
  // call sites, a route handler's returned headers). `x-pylon-*` is stripped.
  extra?: Record<string, string>,
  // TRUSTED runtime headers (e.g. the #277 `x-pylon-cacheable` proof). The ONLY
  // legitimate source of `x-pylon-*`; merged last and never stripped.
  internal?: Record<string, string>,
): Record<string, string> {
  // The host TRUSTS `x-pylon-*` headers (cache verdict, etc.). They must come
  // ONLY from `internal` — strip them from BOTH page-set headers AND `extra` so
  // userland can't forge the verdict through any path (setHeader is also
  // rejected at the source, this is defense-in-depth + covers route-handler
  // headers that flow through `extra`).
  const h: Record<string, string> = {};
  const mergeStripped = (src?: Record<string, string>) => {
    if (!src) return;
    for (const [k, v] of Object.entries(src)) {
      if (k.toLowerCase().startsWith("x-pylon-")) continue;
      h[k] = v; // later sources override earlier (page < extra)
    }
  };
  mergeStripped(state.headers);
  mergeStripped(extra);
  if (internal) Object.assign(h, internal); // trusted, never stripped
  if (!h["content-type"]) h["content-type"] = "text/html; charset=utf-8";
  if (state.cookies.length > 0) {
    // Preserve a set-cookie value set via setHeader() (rare) and join it
    // with setCookie() entries rather than clobbering it. The host splits
    // the newline-joined value into one Set-Cookie header each.
    const existing = h["set-cookie"];
    const all = existing ? [existing, ...state.cookies] : state.cookies;
    h["set-cookie"] = all.join("\n");
  }
  return h;
}

/**
 * Phase 1 SSR handler. Resolves the component, renders it via
 * react-dom/server.renderToReadableStream, pumps chunks back to the
 * host as base64-encoded NDJSON.
 *
 * Errors fall back to a type:"error" frame so the host can return a
 * 500 with the error body. Mid-stream errors (after the first chunk
 * has flushed) are uncatchable here — React's `onError` would have
 * to feed into a separate signal, deferred to Phase 1.5.
 */
/**
 * Page SEO metadata. A page exports `export const metadata = {...}`
 * (static) or `export async function generateMetadata(props)` (dynamic,
 * e.g. param-derived titles). Kept flat — no deep nesting beyond og/twitter.
 *
 * React 19 hoists the resulting <title>/<meta>/<link> into <head>. A page
 * `title` overrides a layout's static `<title>` (both render; the browser
 * uses the last, which is the page's). React does NOT dedupe arbitrary
 * `<meta>`, so set `description`/OG in EITHER the layout OR page metadata,
 * not both, to avoid duplicate tags.
 */
/** A single OpenGraph image (for `openGraph.images` — multiple images). */
export interface OgImage {
  url: string;
  secureUrl?: string;
  type?: string;
  width?: number;
  height?: number;
  alt?: string;
}

export interface SsrMetadata {
  title?: string;
  description?: string;
  keywords?: string | string[];
  canonical?: string;
  robots?: string;
  /** `<meta name="author">` — one tag per author. */
  authors?: string | string[];
  /** `<meta name="theme-color">` — browser UI tint for the page. */
  themeColor?: string;
  openGraph?: {
    title?: string;
    description?: string;
    image?: string;
    /** `og:image:secure_url` — set automatically to the https image URL. */
    imageSecureUrl?: string;
    /** `og:image:type` (e.g. "image/png"). */
    imageType?: string;
    /** `og:image:width` / `og:image:height` in pixels. */
    imageWidth?: number;
    imageHeight?: number;
    imageAlt?: string;
    /** Additional images beyond the primary `image` (each emits its own
     *  `og:image` + dimensions). Provide absolute URLs. */
    images?: OgImage[];
    url?: string;
    type?: string;
    /** `og:locale` (e.g. "en_US"). */
    locale?: string;
    /** `og:site_name` — the brand the page belongs to (e.g. "Pylon").
     *  Discord and other unfurlers show this above the title. */
    siteName?: string;
    /** `article:*` tags for `og:type=article` pages. */
    article?: {
      author?: string | string[];
      publishedTime?: string;
      modifiedTime?: string;
      section?: string;
      tags?: string | string[];
    };
  };
  twitter?: {
    card?: string;
    title?: string;
    description?: string;
    image?: string;
    /** `twitter:site` / `twitter:creator` — @handles. */
    site?: string;
    creator?: string;
    /** `twitter:image:alt` — alt text for the card image. */
    imageAlt?: string;
  };
  /** `<link rel="icon">` / `<link rel="apple-touch-icon">`. Auto-wired
   *  from the app/icon.* + app/apple-icon.* + app/favicon.ico file
   *  conventions, or set explicitly. */
  icons?: {
    icon?: { url: string; type?: string; sizes?: string };
    apple?: { url: string; type?: string; sizes?: string };
  };
  /** Alternate URLs. `languages` emits `<link rel="alternate" hreflang>`
   *  per locale; `canonical` is an alias for the top-level `canonical`. */
  alternates?: {
    canonical?: string;
    languages?: Record<string, string>;
  };
  /** Structured data, emitted as `<script type="application/ld+json">`.
   *  One object or an array (each item gets its own script). Serialized with
   *  `<` escaped so values can't break out of the script element. */
  jsonLd?: Record<string, unknown> | Record<string, unknown>[];
}

/**
 * Build a React fragment of <title>/<meta>/<link> from a page's metadata.
 * React 19 auto-hoists these into <head> wherever they render, and the
 * host's </head> splice preserves them. React escapes all text/attrs, so
 * there's no manual XSS handling. Returns null when there's nothing to emit.
 */
export function renderMetadata(React: any, m: SsrMetadata | undefined): any {
  if (!m) return null;
  // Mark every emitted <meta>/<link> with `data-pylon-meta` so the client
  // runtime can swap exactly these tags on a client-side navigation — and
  // leave the layout's charset/viewport and Pylon's injected stylesheet
  // links untouched. <title> is excluded (the client syncs document.title
  // directly). The metadata fragment is server-only (the client renders the
  // page component alone), so React on the client never owns these nodes;
  // this manual marking is what makes the nav-time swap safe.
  const el = (type: any, props: any, ...children: any[]) =>
    type === "meta" || type === "link"
      ? React.createElement(type, { "data-pylon-meta": "", ...props }, ...children)
      : React.createElement(type, props, ...children);
  const kids: any[] = [];
  if (m.title != null) kids.push(el("title", { key: "t" }, m.title));
  if (m.description != null) {
    kids.push(el("meta", { key: "d", name: "description", content: m.description }));
  }
  const kw = Array.isArray(m.keywords) ? m.keywords.join(", ") : m.keywords;
  if (kw) kids.push(el("meta", { key: "kw", name: "keywords", content: kw }));
  if (m.robots) kids.push(el("meta", { key: "r", name: "robots", content: m.robots }));
  const authors = Array.isArray(m.authors) ? m.authors : m.authors ? [m.authors] : [];
  authors.forEach((a, i) => {
    if (a) kids.push(el("meta", { key: `au${i}`, name: "author", content: a }));
  });
  if (m.themeColor) {
    kids.push(el("meta", { key: "tc", name: "theme-color", content: m.themeColor }));
  }
  if (m.canonical) {
    kids.push(el("link", { key: "c", rel: "canonical", href: m.canonical }));
  } else if (m.alternates?.canonical) {
    kids.push(el("link", { key: "c", rel: "canonical", href: m.alternates.canonical }));
  }
  const langs = m.alternates?.languages;
  if (langs) {
    Object.entries(langs).forEach(([lang, href], i) => {
      if (href) {
        kids.push(el("link", { key: `alt${i}`, rel: "alternate", hrefLang: lang, href }));
      }
    });
  }
  const og = m.openGraph;
  if (og) {
    if (og.title != null) kids.push(el("meta", { key: "ogt", property: "og:title", content: og.title }));
    if (og.description != null) kids.push(el("meta", { key: "ogd", property: "og:description", content: og.description }));
    if (og.image) {
      kids.push(el("meta", { key: "ogi", property: "og:image", content: og.image }));
      if (og.imageSecureUrl) kids.push(el("meta", { key: "ogis", property: "og:image:secure_url", content: og.imageSecureUrl }));
      if (og.imageType) kids.push(el("meta", { key: "ogit", property: "og:image:type", content: og.imageType }));
      if (og.imageWidth != null) kids.push(el("meta", { key: "ogiw", property: "og:image:width", content: String(og.imageWidth) }));
      if (og.imageHeight != null) kids.push(el("meta", { key: "ogih", property: "og:image:height", content: String(og.imageHeight) }));
      if (og.imageAlt) kids.push(el("meta", { key: "ogia", property: "og:image:alt", content: og.imageAlt }));
    }
    if (Array.isArray(og.images)) {
      og.images.forEach((img, i) => {
        if (!img || !img.url) return;
        kids.push(el("meta", { key: `ogim${i}`, property: "og:image", content: img.url }));
        if (img.secureUrl) kids.push(el("meta", { key: `ogims${i}`, property: "og:image:secure_url", content: img.secureUrl }));
        if (img.type) kids.push(el("meta", { key: `ogimt${i}`, property: "og:image:type", content: img.type }));
        if (img.width != null) kids.push(el("meta", { key: `ogimw${i}`, property: "og:image:width", content: String(img.width) }));
        if (img.height != null) kids.push(el("meta", { key: `ogimh${i}`, property: "og:image:height", content: String(img.height) }));
        if (img.alt) kids.push(el("meta", { key: `ogima${i}`, property: "og:image:alt", content: img.alt }));
      });
    }
    if (og.url) kids.push(el("meta", { key: "ogu", property: "og:url", content: og.url }));
    if (og.type) kids.push(el("meta", { key: "ogy", property: "og:type", content: og.type }));
    if (og.locale) kids.push(el("meta", { key: "ogl", property: "og:locale", content: og.locale }));
    if (og.siteName) kids.push(el("meta", { key: "ogsn", property: "og:site_name", content: og.siteName }));
    const art = og.article;
    if (art) {
      const aAuthors = Array.isArray(art.author) ? art.author : art.author ? [art.author] : [];
      aAuthors.forEach((a, i) => {
        if (a) kids.push(el("meta", { key: `oga${i}`, property: "article:author", content: a }));
      });
      if (art.publishedTime) kids.push(el("meta", { key: "ogpt", property: "article:published_time", content: art.publishedTime }));
      if (art.modifiedTime) kids.push(el("meta", { key: "ogmt", property: "article:modified_time", content: art.modifiedTime }));
      if (art.section) kids.push(el("meta", { key: "ogsec", property: "article:section", content: art.section }));
      const aTags = Array.isArray(art.tags) ? art.tags : art.tags ? [art.tags] : [];
      aTags.forEach((t, i) => {
        if (t) kids.push(el("meta", { key: `ogtag${i}`, property: "article:tag", content: t }));
      });
    }
  }
  const tw = m.twitter;
  if (tw) {
    if (tw.card) kids.push(el("meta", { key: "twc", name: "twitter:card", content: tw.card }));
    if (tw.title != null) kids.push(el("meta", { key: "twt", name: "twitter:title", content: tw.title }));
    if (tw.description != null) kids.push(el("meta", { key: "twd", name: "twitter:description", content: tw.description }));
    if (tw.image) kids.push(el("meta", { key: "twi", name: "twitter:image", content: tw.image }));
    if (tw.site) kids.push(el("meta", { key: "tws", name: "twitter:site", content: tw.site }));
    if (tw.creator) kids.push(el("meta", { key: "twcr", name: "twitter:creator", content: tw.creator }));
    if (tw.imageAlt) kids.push(el("meta", { key: "twia", name: "twitter:image:alt", content: tw.imageAlt }));
  }
  const ic = m.icons;
  if (ic) {
    if (ic.icon) {
      const a: Record<string, string> = { key: "icn", rel: "icon", href: ic.icon.url };
      if (ic.icon.type) a.type = ic.icon.type;
      if (ic.icon.sizes) a.sizes = ic.icon.sizes;
      kids.push(el("link", a));
    }
    if (ic.apple) {
      const a: Record<string, string> = { key: "aicn", rel: "apple-touch-icon", href: ic.apple.url };
      if (ic.apple.type) a.type = ic.apple.type;
      if (ic.apple.sizes) a.sizes = ic.apple.sizes;
      kids.push(el("link", a));
    }
  }
  if (m.jsonLd) {
    const items = Array.isArray(m.jsonLd) ? m.jsonLd : [m.jsonLd];
    items.forEach((item, i) => {
      let json: string;
      try {
        json = JSON.stringify(item);
      } catch {
        return; // unserializable (cycle) — skip rather than throw the render
      }
      if (!json) return;
      // Escape `<`, `>`, `&` to their \uXXXX JSON forms so the payload can't
      // break out of the <script> element (e.g. a value containing
      // `</script>`), no matter how the renderer treats raw-text children.
      // JSON.parse decodes these back, so the structured data stays valid. This
      // is why no dangerouslySetInnerHTML is needed: the text child is inert.
      const safe = json
        .replace(/</g, "\\u003c")
        .replace(/>/g, "\\u003e")
        .replace(/&/g, "\\u0026");
      kids.push(
        React.createElement(
          "script",
          { key: `ld${i}`, type: "application/ld+json", "data-pylon-meta": "" },
          safe,
        ),
      );
    });
  }
  return kids.length > 0 ? el(React.Fragment, null, ...kids) : null;
}

const MODULE_EXTS = [".tsx", ".ts", ".jsx", ".js"];

/** Import a project-relative module, trying each common extension. */
export async function importModule(cwd: string, relPath: string): Promise<any> {
  const base = `${cwd}/${relPath}`;
  let lastErr: unknown = null;
  for (const ext of MODULE_EXTS) {
    try {
      return await import(`${base}${ext}`);
    } catch (e) {
      lastErr = e;
    }
  }
  throw lastErr ?? new Error(`could not import module "${relPath}"`);
}

/**
 * Wrap a leaf element in its layout chain (leaf → root). Resolves ALL
 * layouts first so a missing one fails before any chunk is emitted. Reused
 * by the page render and by the not-found / error boundary render.
 */
async function buildLayoutTree(
  cwd: string,
  leaf: any,
  layouts: string[] | undefined,
  props: any,
  React: any,
): Promise<any> {
  if (!layouts || layouts.length === 0) return leaf;
  const layoutComps: any[] = [];
  for (const layoutPath of layouts) {
    let lMod: any;
    try {
      lMod = await importModule(cwd, layoutPath);
    } catch {
      throw new Error(
        `could not import layout "${layoutPath}" — checked .tsx / .ts / .jsx / .js`,
      );
    }
    const LayoutComp = lMod.default ?? lMod.Layout ?? lMod.layout;
    if (typeof LayoutComp !== "function") {
      throw new Error(
        `layout "${layoutPath}" has no default export (or named export "Layout")`,
      );
    }
    layoutComps.push(LayoutComp);
  }
  let tree = leaf;
  for (let i = layoutComps.length - 1; i >= 0; i--) {
    tree = React.createElement(layoutComps[i], props, tree);
  }
  return tree;
}

/**
 * Walk up from a page's directory to the nearest boundary file
 * (not-found / error) — the same render-time, filesystem-resolved model
 * the page + layouts already use, so no build-time manifest threading.
 * Returns the project-relative path (no extension) or null.
 */
function findBoundary(componentPath: string, fileName: string): string | null {
  const fs = require("node:fs");
  const path = require("node:path");
  const cwd = process.cwd();
  // Component paths use "/" — walk up directory by directory.
  let dir = componentPath.replace(/\\/g, "/");
  dir = dir.includes("/") ? dir.slice(0, dir.lastIndexOf("/")) : "";
  while (dir && dir !== "." && dir !== "/") {
    for (const ext of MODULE_EXTS) {
      if (fs.existsSync(path.join(cwd, dir, `${fileName}${ext}`))) {
        return `${dir}/${fileName}`;
      }
    }
    const slash = dir.lastIndexOf("/");
    dir = slash >= 0 ? dir.slice(0, slash) : "";
  }
  return null;
}

// ---------------------------------------------------------------------------
// Social-card image file convention (Next-style `opengraph-image.png` /
// `twitter-image.png` colocated with a `page.tsx`). Drop the file in a
// route folder and Pylon auto-emits the `<meta og:image>` (absolute URL,
// dimensions, type) pointing at the `/_pylon/og` asset endpoint — no
// metadata wiring required. An explicit `metadata.openGraph.image` always
// wins. Resolved fresh per render off the filesystem (same model as
// layouts / boundaries) so dropping a new image is picked up without a
// restart.
// ---------------------------------------------------------------------------

const SOCIAL_IMAGE_EXTS = [".png", ".jpg", ".jpeg", ".webp", ".gif", ".avif"];

/** Walk up from a page's directory to the nearest colocated
 *  `<base>.<imgext>` (Next inheritance: a closer file overrides an
 *  ancestor's). Returns the cwd-relative path WITH extension, or null. */
function findColocatedImage(
  componentPath: string,
  base: string,
  exts: string[] = SOCIAL_IMAGE_EXTS,
): string | null {
  const fs = require("node:fs");
  const path = require("node:path");
  const cwd = process.cwd();
  let dir = componentPath.replace(/\\/g, "/");
  dir = dir.includes("/") ? dir.slice(0, dir.lastIndexOf("/")) : "";
  while (dir && dir !== "." && dir !== "/") {
    for (const ext of exts) {
      if (fs.existsSync(path.join(cwd, dir, `${base}${ext}`))) {
        return `${dir}/${base}${ext}`;
      }
    }
    const slash = dir.lastIndexOf("/");
    dir = slash >= 0 ? dir.slice(0, slash) : "";
  }
  return null;
}

/** Best-effort JPEG dimensions: scan SOF markers in the first 128KB. */
function readJpegSize(fs: any, fd: number): { w: number; h: number } | null {
  const CAP = 128 * 1024;
  const buf = Buffer.alloc(CAP);
  const n = fs.readSync(fd, buf, 0, CAP, 0);
  if (n < 4 || buf[0] !== 0xff || buf[1] !== 0xd8) return null;
  let i = 2;
  while (i + 9 < n) {
    if (buf[i] !== 0xff) {
      i++;
      continue;
    }
    let marker = buf[i + 1];
    while (marker === 0xff && i + 1 < n) {
      i++;
      marker = buf[i + 1];
    }
    const seg = i + 2;
    if (seg + 2 > n) break;
    const len = buf.readUInt16BE(seg);
    const isSOF =
      marker >= 0xc0 && marker <= 0xcf && marker !== 0xc4 && marker !== 0xc8 && marker !== 0xcc;
    if (isSOF) {
      if (seg + 7 <= n) {
        return { h: buf.readUInt16BE(seg + 3), w: buf.readUInt16BE(seg + 5) };
      }
      return null;
    }
    if (marker === 0xd9 || marker === 0xda) break; // EOI / SOS
    i = seg + len;
  }
  return null;
}

/** Content-type + pixel dimensions + mtime for a colocated image. Dims
 *  are best-effort (PNG/GIF from the header, JPEG via SOF scan). */
function readSocialImageMeta(relPath: string): {
  type: string;
  width?: number;
  height?: number;
  v: number;
} {
  const fs = require("node:fs");
  const path = require("node:path");
  const ext = relPath.slice(relPath.lastIndexOf(".")).toLowerCase();
  const type =
    ext === ".png" ? "image/png"
    : ext === ".jpg" || ext === ".jpeg" ? "image/jpeg"
    : ext === ".webp" ? "image/webp"
    : ext === ".gif" ? "image/gif"
    : ext === ".avif" ? "image/avif"
    : ext === ".svg" ? "image/svg+xml"
    : ext === ".ico" ? "image/x-icon"
    : "application/octet-stream";
  let width: number | undefined;
  let height: number | undefined;
  let v = 0;
  try {
    const abs = path.join(process.cwd(), relPath);
    v = Math.floor(fs.statSync(abs).mtimeMs);
    const fd = fs.openSync(abs, "r");
    try {
      const head = Buffer.alloc(32);
      fs.readSync(fd, head, 0, 32, 0);
      if (ext === ".png" && head.toString("latin1", 1, 4) === "PNG") {
        width = head.readUInt32BE(16); // IHDR: 8 sig + 4 len + 4 "IHDR"
        height = head.readUInt32BE(20);
      } else if (ext === ".gif" && head.toString("latin1", 0, 3) === "GIF") {
        width = head.readUInt16LE(6);
        height = head.readUInt16LE(8);
      } else if (ext === ".jpg" || ext === ".jpeg") {
        const d = readJpegSize(fs, fd);
        if (d) {
          width = d.w;
          height = d.h;
        }
      }
    } finally {
      fs.closeSync(fd);
    }
  } catch {
    /* dims/mtime are best-effort */
  }
  return { type, width, height, v };
}

const LOOPBACK_HOST = /^(localhost|127\.|\[?::1|0\.0\.0\.0)/;

/** Normalize a bare host or a full URL down to a lowercase `host` (host:port).
 *  Returns "" for unparseable input. */
function hostOf(value: string): string {
  const t = (value || "").trim();
  if (!t) return "";
  try {
    return (t.includes("://") ? new URL(t).host : t.replace(/^\/+|\/+$/g, "")).toLowerCase();
  } catch {
    return "";
  }
}

/** Pure origin resolution (exported for tests).
 *
 *  SECURITY: the request `Host` (and `X-Forwarded-Proto`) is attacker-
 *  controlled. It's only trusted to build the absolute origin baked into
 *  `og:image` / canonical URLs when it's in the allowlist — the configured
 *  public/canonical host, an explicit `PYLON_TRUSTED_HOSTS` entry, or
 *  loopback. An untrusted (or absent) Host falls back to the configured
 *  public origin. Without this, `Host: evil.com` on a cacheable
 *  (force-static / `revalidate`) render bakes `https://evil.com/_pylon/og…`
 *  into the HTML, which is then teed into the shared ISR/CDN cache and
 *  served to every subsequent visitor (cache poisoning). */
export function resolveOrigin(opts: {
  host?: string;
  forwardedProto?: string;
  publicUrl?: string;
  canonicalHost?: string;
  trustedHostsCsv?: string;
}): string {
  const publicUrl = (opts.publicUrl || "").trim().replace(/\/+$/, "");
  const host = opts.host?.trim().toLowerCase();
  if (host) {
    const allow = new Set<string>();
    const add = (v: string) => {
      const h = hostOf(v);
      if (h) allow.add(h);
    };
    add(opts.publicUrl || "");
    add(opts.canonicalHost || "");
    for (const x of (opts.trustedHostsCsv || "").split(",")) add(x);
    const isLoopback = LOOPBACK_HOST.test(host);
    if (isLoopback || allow.has(host)) {
      // Off-loopback (prod) we ALWAYS use https and never honor the request's
      // X-Forwarded-Proto. The SSR cache is keyed only by host (not proto), so
      // honoring a client-supplied `http` would poison the cached canonical/OG
      // URL with a downgraded scheme for every subsequent visitor. Loopback
      // (dev) may be plain http. (A genuinely non-https prod origin should set
      // PYLON_PUBLIC_URL explicitly, which takes precedence above.)
      const proto = isLoopback
        ? opts.forwardedProto === "https"
          ? "https"
          : "http"
        : "https";
      return `${proto}://${host}`;
    }
  }
  // Untrusted / absent Host → the configured canonical origin. (Prefer the
  // full public URL; fall back to the canonical host as https.)
  if (publicUrl) return publicUrl;
  const canon = hostOf(opts.canonicalHost || "");
  return canon ? `https://${canon}` : "";
}

/** Absolute origin for OG URLs (crawlers require absolute). Trusts the
 *  request Host only when it's allowlisted; otherwise uses PYLON_PUBLIC_URL.
 *  See `resolveOrigin` for the security rationale. */
function resolveRequestOrigin(headers: Record<string, string> | undefined): string {
  const env = (globalThis as any).process?.env ?? {};
  return resolveOrigin({
    host: headers?.["host"],
    forwardedProto: headers?.["x-forwarded-proto"],
    publicUrl: env.PYLON_PUBLIC_URL,
    canonicalHost: env.PYLON_CANONICAL_HOST,
    trustedHostsCsv: env.PYLON_TRUSTED_HOSTS,
  });
}

/** SECURITY: validate a `response.redirect()` target to prevent OPEN REDIRECTS.
 *  Mirrors the OAuth-layer `validate_trusted_redirect` (crates/auth). A relative
 *  same-site path is always allowed; an absolute URL only when it's http(s) to a
 *  trusted host — the app's public/canonical origin, a `PYLON_TRUSTED_HOSTS`
 *  entry, or loopback. Everything else is rejected: protocol-relative
 *  `//evil.com`, backslash tricks (`/\evil.com` — browsers normalize `\`→`/`),
 *  other-origin absolutes, and `javascript:`/`data:` schemes. So the natural
 *  `response.redirect(searchParams.get("next"))` can't be turned into an
 *  off-site redirect by attacker-supplied input. Exported for tests. */
export function isSafeRedirect(
  url: string,
  opts: { publicUrl?: string; canonicalHost?: string; trustedHostsCsv?: string },
): boolean {
  // Relative same-site path: exactly one leading slash. Reject protocol-
  // relative (`//host`) and backslash variants that resolve cross-origin.
  if (url.startsWith("/")) {
    return url.length < 2 || (url[1] !== "/" && url[1] !== "\\");
  }
  if (url.startsWith("\\")) return false; // `\\host` / `\/host`
  // Absolute: only http(s) to a trusted host. A bare-relative ("dashboard"),
  // opaque, or unparseable target throws here → rejected (fail closed).
  let parsed: URL;
  try {
    parsed = new URL(url);
  } catch {
    return false;
  }
  if (parsed.protocol !== "http:" && parsed.protocol !== "https:") return false;
  const host = parsed.host.toLowerCase();
  if (LOOPBACK_HOST.test(host)) return true;
  const allow = new Set<string>();
  const add = (v: string) => {
    const h = hostOf(v);
    if (h) allow.add(h);
  };
  add(opts.publicUrl || "");
  add(opts.canonicalHost || "");
  for (const x of (opts.trustedHostsCsv || "").split(",")) add(x);
  return allow.has(host);
}

/** Merge auto-discovered social-card images into a page's metadata. An
 *  explicit `openGraph.image` / `twitter.image` always wins; otherwise a
 *  colocated `opengraph-image.*` (and `twitter-image.*`, falling back to
 *  the og file) is wired in with absolute URL + dimensions. */
// Icon file conventions. `icon.*` → <link rel="icon">; `apple-icon.*` →
// <link rel="apple-touch-icon">; `favicon.ico` is the legacy fallback for
// the icon link. Unlike og:image, icon links use a RELATIVE URL (resolved
// same-origin by the browser) so no request origin is needed.
const ICON_EXTS = [".png", ".svg", ".ico", ".jpg", ".jpeg"];
const APPLE_ICON_EXTS = [".png", ".jpg", ".jpeg"];

/** `sizes` attribute for an icon link: "any" for vector SVG, "WxH" for a
 *  raster with known dimensions, omitted for .ico (multi-size). */
function iconSizes(rel: string, m: { width?: number; height?: number }): string | undefined {
  if (rel.toLowerCase().endsWith(".svg")) return "any";
  if (m.width && m.height) return `${m.width}x${m.height}`;
  return undefined;
}

/** Merge auto-discovered favicons (icon.* / apple-icon.* / favicon.ico)
 *  into a page's metadata. Explicit `metadata.icons.*` wins. */
export function applyAutoIcons(
  component: string,
  metadata: SsrMetadata | undefined,
): SsrMetadata | undefined {
  const hasIcon = !!metadata?.icons?.icon;
  const hasApple = !!metadata?.icons?.apple;
  if (hasIcon && hasApple) return metadata;

  const iconFile = hasIcon
    ? null
    : findColocatedImage(component, "icon", ICON_EXTS) ??
      findColocatedImage(component, "favicon", [".ico"]);
  const appleFile = hasApple
    ? null
    : findColocatedImage(component, "apple-icon", APPLE_ICON_EXTS);
  if (!iconFile && !appleFile) return metadata;

  const linkFor = (rel: string, v: number): string =>
    `/_pylon/og?src=${encodeURIComponent(rel)}${v ? `&v=${v}` : ""}`;
  const out: SsrMetadata = { ...(metadata ?? {}) };
  out.icons = { ...(out.icons ?? {}) };

  if (iconFile && !hasIcon) {
    const m = readSocialImageMeta(iconFile);
    const sizes = iconSizes(iconFile, m);
    out.icons.icon = {
      url: linkFor(iconFile, m.v),
      type: m.type,
      ...(sizes ? { sizes } : {}),
    };
  }
  if (appleFile && !hasApple) {
    const m = readSocialImageMeta(appleFile);
    out.icons.apple = {
      url: linkFor(appleFile, m.v),
      type: m.type,
      ...(m.width && m.height ? { sizes: `${m.width}x${m.height}` } : {}),
    };
  }
  return out;
}

export function applyAutoSocialImages(
  component: string,
  headers: Record<string, string> | undefined,
  metadata: SsrMetadata | undefined,
): SsrMetadata | undefined {
  const hasOg = !!metadata?.openGraph?.image;
  const hasTw = !!metadata?.twitter?.image;
  if (hasOg && hasTw) return metadata;

  const ogFile = hasOg ? null : findColocatedImage(component, "opengraph-image");
  const twFile = hasTw
    ? null
    : findColocatedImage(component, "twitter-image") ?? ogFile;
  if (!ogFile && !twFile) return metadata;

  const origin = resolveRequestOrigin(headers);
  const urlFor = (rel: string, v: number): string =>
    `${origin}/_pylon/og?src=${encodeURIComponent(rel)}${v ? `&v=${v}` : ""}`;
  const out: SsrMetadata = { ...(metadata ?? {}) };

  if (ogFile && !hasOg) {
    const m = readSocialImageMeta(ogFile);
    const url = urlFor(ogFile, m.v);
    out.openGraph = {
      ...(out.openGraph ?? {}),
      image: url,
      imageType: m.type,
      ...(m.width ? { imageWidth: m.width } : {}),
      ...(m.height ? { imageHeight: m.height } : {}),
      ...(url.startsWith("https:") ? { imageSecureUrl: url } : {}),
    };
  }
  if (twFile && !hasTw) {
    const m = readSocialImageMeta(twFile);
    out.twitter = {
      card: "summary_large_image",
      ...(out.twitter ?? {}),
      image: urlFor(twFile, m.v),
    };
  }
  return out;
}

/**
 * Drain a `renderToReadableStream` reader, injecting `headBlob` immediately
 * before the first `</head>` (or, if the document has none, the blob is
 * never emitted — fragment renders have no head). `</head>` can straddle a
 * chunk boundary, so a small carry buffer (len("</head>") − 1 bytes) is
 * withheld at each chunk's tail until the next read confirms the match.
 * Each emitted slice is handed to `sendChunk` as utf-8 text.
 *
 * Shared by the page render and the boundary render so head injection has
 * exactly one implementation.
 */
async function streamWithHeadInjection(
  reader: ReadableStreamDefaultReader<Uint8Array>,
  headBlob: string,
  sendChunk: (text: string) => void,
): Promise<void> {
  let headInjected = headBlob.length === 0;
  let carry = "";
  const HEAD_CLOSE = "</head>";
  for (;;) {
    const { value, done } = await reader.read();
    if (done) break;
    if (!value || value.byteLength === 0) continue;
    const text = Buffer.from(value).toString("utf8");
    if (!headInjected) {
      const combined = carry + text;
      const idx = combined.indexOf(HEAD_CLOSE);
      if (idx >= 0) {
        sendChunk(combined.slice(0, idx));
        sendChunk(headBlob);
        sendChunk(HEAD_CLOSE);
        const after = combined.slice(idx + HEAD_CLOSE.length);
        if (after) sendChunk(after);
        headInjected = true;
        carry = "";
      } else {
        const keep = HEAD_CLOSE.length - 1;
        if (combined.length > keep) {
          sendChunk(combined.slice(0, combined.length - keep));
          carry = combined.slice(combined.length - keep);
        } else {
          carry = combined;
        }
      }
    } else {
      sendChunk(text);
    }
  }
  if (carry) sendChunk(carry);
}

/**
 * Dev-only browser live-reload client, injected at the end of every SSR
 * page when PYLON_DEV_MODE is set. Subscribes to the runtime's
 * `/_pylon/dev/live` Server-Sent-Events endpoint (see frontend.rs
 * `serve_dev_live_reload`), which streams this process's boot id in a
 * `hello` event. EventSource auto-reconnects when `pylon dev` restarts; the
 * fresh process advertises a new boot id, so a changed id ⇒ the tab reloads.
 * No-ops in browsers without EventSource (none in practice).
 */
const DEV_LIVE_RELOAD_SNIPPET =
  "<script>(function(){if(typeof EventSource===\"undefined\")return;" +
  "var b=null;try{var s=new EventSource(\"/_pylon/dev/live\");" +
  "s.addEventListener(\"hello\",function(e){" +
  "if(b!==null&&e.data!==b){s.close();location.reload();return;}b=e.data;});" +
  "}catch(_){}})();</script>";

/**
 * The dev HUD (a floating bottom-left overlay, dev-only): surfaces the framework
 * decisions a dev otherwise can't see — the cache verdict + WHY a page isn't
 * cached, render mode + timing, sync connection + offline-outbox depth, /api
 * activity + policy denials, and client errors. The cache / api / errors rows
 * expand a detail drawer (click). Written as a plain function and embedded via
 * `.toString()` (Bun strips the TS annotations), so it ships as self-contained
 * browser JS that closes over nothing. Browser globals go through `g` so it
 * typechecks without a DOM lib. Mirrors Next's dev indicator, tuned to Pylon.
 */
function pylonDevHud() {
  const g: any = globalThis;
  const d: any = g.document;
  if (!d || g.__pylonHudMounted) return;
  g.__pylonHudMounted = true;

  let info: any = {};
  try {
    const el = d.getElementById("__PYLON_DEV__");
    if (el) info = JSON.parse(el.textContent || "{}");
  } catch (_e) {}
  // Marker the sync engine checks before publishing its dev status probe.
  g.__PYLON_DEV__ = info;

  // Client errors.
  const errs: string[] = [];
  const onErr = (m: any) => {
    errs.push(String(m));
    if (errs.length > 50) errs.shift();
  };
  if (g.addEventListener) {
    g.addEventListener("error", (e: any) => onErr((e && (e.message || e.error)) || e));
    g.addEventListener("unhandledrejection", (e: any) => onErr((e && e.reason) || e));
  }

  // /api activity: wrap fetch ONCE to observe Pylon API calls (method, path,
  // status, ms, and a policy/error reason on non-2xx). Transparent — returns the
  // ORIGINAL promise untouched; the body is read from a clone only on errors.
  const api: any[] = [];
  if (g.fetch && !g.__pylonFetchWrapped) {
    g.__pylonFetchWrapped = true;
    const orig = g.fetch.bind(g);
    g.fetch = function (input: any, init: any) {
      let url = "";
      try {
        url = typeof input === "string" ? input : (input && input.url) || "";
      } catch (_e) {}
      const method = String(
        (init && init.method) || (input && input.method) || "GET",
      ).toUpperCase();
      const t0 = g.performance && g.performance.now ? g.performance.now() : Date.now();
      const p = orig(input, init);
      if (url.indexOf("/api/") !== -1 && p && p.then) {
        const path = url.replace(/^https?:\/\/[^/]+/, "").split("?")[0];
        const done = (status: number, reason: string) => {
          const t1 = g.performance && g.performance.now ? g.performance.now() : Date.now();
          api.push({
            method,
            path,
            status,
            ms: Math.round((t1 - t0) * 10) / 10,
            reason,
            denied: status === 403,
          });
          if (api.length > 40) api.shift();
        };
        p.then(
          (res: any) => {
            if (!res.ok && res.clone) {
              res
                .clone()
                .json()
                .then((b: any) =>
                  done(
                    res.status,
                    (b && b.error && (b.error.message || b.error.code)) || (b && b.message) || "",
                  ),
                )
                .catch(() => done(res.status, ""));
            } else {
              done(res.status, "");
            }
          },
          () => done(0, "network error"),
        );
      }
      return p;
    };
  }

  const C = {
    ok: "#3fb950",
    warn: "#d29922",
    bad: "#f85149",
    dim: "#6e7681",
    txt: "#e6edf3",
  };
  const make = (tag: string, css: string, text?: string) => {
    const n = d.createElement(tag);
    n.style.cssText = css;
    if (text != null) n.textContent = text;
    return n;
  };
  const dot = (color: string) =>
    make(
      "span",
      "display:inline-block;width:7px;height:7px;border-radius:50%;flex:0 0 auto;background:" +
        color,
    );

  const box = make(
    "div",
    "position:fixed;left:12px;bottom:12px;z-index:2147483646;font:11px/1.5 ui-monospace,SFMono-Regular,Menlo,monospace",
  );
  const panel = make(
    "div",
    "background:#0d1117;border:1px solid #30363d;border-radius:8px;padding:9px 11px;margin-bottom:6px;min-width:260px;max-width:380px;box-shadow:0 8px 28px rgba(0,0,0,.45)",
  );
  const pill = make(
    "button",
    "display:flex;align-items:center;gap:6px;background:#161b22;border:1px solid #30363d;border-radius:999px;padding:5px 11px;color:#e6edf3;cursor:pointer;font:inherit;box-shadow:0 2px 10px rgba(0,0,0,.35)",
  );

  // The expandable detail area below the rows.
  let activeDrawer: string | null = null;
  const drawer = make(
    "div",
    "display:none;margin-top:7px;padding-top:7px;border-top:1px solid #21262d;max-height:200px;overflow:auto",
  );
  const drawerLine = (label: string, value: string, color?: string) => {
    const r = make("div", "display:flex;gap:8px;margin:2px 0");
    r.appendChild(make("span", "color:#8b949e;flex:0 0 110px", label));
    r.appendChild(make("span", "color:" + (color || C.txt) + ";flex:1;word-break:break-word", value));
    drawer.appendChild(r);
  };
  const renderDrawer = () => {
    if (!activeDrawer) {
      drawer.style.display = "none";
      return;
    }
    drawer.style.display = "block";
    drawer.textContent = "";
    if (activeDrawer === "cache") {
      const f = (info.cache && info.cache.flags) || {};
      const optIn = f.bucketOptIn
        ? "auth-bucketed"
        : f.revalidateSecs != null
          ? "revalidate=" + f.revalidateSecs + "s"
          : "none";
      drawerLine("opt-in", optIn, optIn === "none" ? C.warn : C.ok);
      drawerLine("props.auth", f.authTouched ? "read ✗" : "untouched ✓", f.authTouched ? C.bad : C.ok);
      drawerLine(
        "headers/cookies",
        f.dynamicTouched ? "read ✗" : "untouched ✓",
        f.dynamicTouched ? C.bad : C.ok,
      );
      drawerLine("props.session", f.sessionTouched ? "read (bucket bit)" : "untouched", C.dim);
      drawerLine("set-cookie", String(f.cookieCount || 0), f.cookieCount ? C.bad : C.ok);
      drawerLine("strict policies", f.strictPolicies ? "on ✗" : "off ✓", f.strictPolicies ? C.bad : C.ok);
      drawerLine("streaming", f.wantsStream ? "yes" : "no", C.dim);
      drawerLine("status", String(f.status != null ? f.status : "—"), f.status === 200 ? C.ok : C.warn);
    } else if (activeDrawer === "api") {
      if (!api.length) {
        drawerLine("", "no /api calls yet", C.dim);
      } else {
        for (let i = api.length - 1; i >= 0; i--) {
          const a = api[i];
          const col =
            a.status === 0 || a.status >= 500 || a.denied
              ? C.bad
              : a.status >= 400
                ? C.warn
                : C.ok;
          drawer.appendChild(
            make(
              "div",
              "margin:2px 0;color:" + col + ";word-break:break-word",
              a.method +
                " " +
                a.path +
                "  " +
                (a.status || "ERR") +
                " · " +
                a.ms +
                "ms" +
                (a.reason ? "  — " + a.reason : ""),
            ),
          );
        }
      }
    } else if (activeDrawer === "errors") {
      if (!errs.length) {
        drawerLine("", "no errors", C.dim);
      } else {
        for (let i = errs.length - 1; i >= 0; i--) {
          drawer.appendChild(
            make("div", "margin:2px 0;color:" + C.bad + ";word-break:break-word", errs[i]),
          );
        }
      }
    }
  };

  // A row; pass `key` to make it click-to-expand the matching drawer section.
  const rowEl = (label: string, key?: string) => {
    const r = make(
      "div",
      "display:flex;align-items:flex-start;gap:8px;margin:3px 0" + (key ? ";cursor:pointer" : ""),
    );
    r.appendChild(make("span", "color:#8b949e;flex:0 0 60px", key ? label + " ▸" : label));
    const v = make("span", "color:#e6edf3;word-break:break-word;flex:1");
    r.appendChild(v);
    if (key) {
      r.onclick = () => {
        activeDrawer = activeDrawer === key ? null : key;
        renderDrawer();
      };
    }
    panel.appendChild(r);
    return v;
  };

  const cache = info.cache || {};
  const cacheLabel =
    cache.verdict === "dynamic"
      ? "dynamic"
      : cache.verdict + (cache.secs ? " · " + cache.secs + "s" : "");
  rowEl("route").textContent = info.route || "—";
  rowEl("page").textContent = info.component || "—";
  const cacheV = rowEl("cache", "cache");
  cacheV.textContent = cacheLabel + (cache.reason ? "  ·  " + cache.reason : "");
  cacheV.style.color = cache.verdict === "dynamic" ? C.warn : C.ok;
  rowEl("render").textContent =
    (info.renderMode || "ssr") + (info.renderMs != null ? " · " + info.renderMs + "ms" : "");
  const syncV = rowEl("sync");
  const apiV = rowEl("api", "api");
  const errV = rowEl("errors", "errors");
  panel.appendChild(drawer);

  pill.appendChild(dot(cache.verdict === "dynamic" ? C.warn : C.ok));
  pill.appendChild(make("span", "font-weight:600;color:#e6edf3", "pylon"));
  const pillSyncDot = dot(C.dim);
  pill.appendChild(pillSyncDot);

  let open = false;
  try {
    open = g.localStorage && g.localStorage.getItem("pylon.hud.open") === "1";
  } catch (_e) {}
  panel.style.display = open ? "block" : "none";
  pill.onclick = () => {
    open = !open;
    panel.style.display = open ? "block" : "none";
    try {
      g.localStorage && g.localStorage.setItem("pylon.hud.open", open ? "1" : "0");
    } catch (_e) {}
  };

  const refresh = () => {
    const s = g.__pylonDevSync;
    if (s) {
      let st = "?";
      let pend = 0;
      let rows = 0;
      try {
        st = s.status();
        pend = s.pending();
        rows = s.rows();
      } catch (_e) {}
      syncV.textContent = st + " · " + pend + " pending · " + rows + " rows";
      const col = st === "connected" ? C.ok : st === "offline" ? C.bad : C.warn;
      syncV.style.color = col;
      pillSyncDot.style.background = col;
    } else {
      const online = g.navigator ? g.navigator.onLine : true;
      syncV.textContent = "no sync engine · " + (online ? "online" : "offline");
      syncV.style.color = C.dim;
      pillSyncDot.style.background = online ? C.dim : C.bad;
    }
    const failed = api.filter((a) => a.denied || a.status === 0 || a.status >= 400).length;
    apiV.textContent =
      api.length === 0 ? "—" : api.length + " calls" + (failed ? " · " + failed + " failed" : "");
    apiV.style.color = failed ? C.bad : api.length ? C.ok : C.dim;
    errV.textContent = String(errs.length);
    errV.style.color = errs.length ? C.bad : C.dim;
    // Keep the live sections (api / errors) fresh while open.
    if (activeDrawer === "api" || activeDrawer === "errors") renderDrawer();
  };

  box.appendChild(panel);
  box.appendChild(pill);
  const mount = () => (d.body || d.documentElement).appendChild(box);
  if (d.body) mount();
  else if (g.addEventListener) g.addEventListener("DOMContentLoaded", mount);
  refresh();
  if (g.setInterval) g.setInterval(refresh, 1000);
}

/**
 * Dev-only tail chunk: the `__PYLON_DEV__` info blob (cache verdict, render
 * mode/timing, route) + the HUD bootstrap. Embedded after the page tail so the
 * marker + probe are in place before the deferred client entry boots the sync
 * engine. `<` is escaped so the JSON can't break out of the script.
 */
export function buildDevHudChunk(devInfo: Record<string, unknown>): string {
  const json = JSON.stringify(devInfo)
    .replace(/</g, "\\u003c")
    .replace(/\u2028/g, "\\u2028")
    .replace(/\u2029/g, "\\u2029");
  return (
    `<script id="__PYLON_DEV__" type="application/json">${json}</script>` +
    `<script>(${pylonDevHud.toString()})();</script>`
  );
}

/**
 * Build the <head> blob for a boundary render: the union of every route's
 * stylesheet links from the client build manifest. Boundary modules aren't
 * bundled as their own client entry, but they render inside the same
 * layout/shell as pages, so without the app's global CSS a 404/500 page
 * would look broken. Returns "" if the manifest can't be loaded — the
 * boundary still renders (unstyled); CSS must never block the error path.
 */
async function collectBoundaryHeadBlob(): Promise<string> {
  try {
    const { getManifest } = await import("./ssr-client-bundler");
    const manifest = await getManifest();
    const prefix = manifest.public_prefix || "/_pylon/build/";
    const seen = new Set<string>();
    // Fonts first: the @font-face faces + preloads must be discoverable before
    // the app stylesheet that references the font family.
    let blob = await buildFontHeadBlob();
    for (const route of Object.values(manifest.routes || {}) as any[]) {
      for (const css of (route.css || []) as string[]) {
        if (seen.has(css)) continue;
        seen.add(css);
        blob += `<link rel="stylesheet" href="${prefix}${css}">`;
      }
    }
    return blob;
  } catch {
    return "";
  }
}

/**
 * Build the self-hosted-font `<head>` blob: the inlined `@font-face` + `:root`
 * variable `<style>` (so the size-adjusted fallback is defined before first
 * paint — that's what makes the swap zero-CLS) plus a `<link rel=preload>` for
 * each woff2. Global / route-independent. Returns "" when the app declares no
 * fonts. URLs resolve against the live `public_prefix` (same-origin or CDN).
 */
async function buildFontHeadBlob(): Promise<string> {
  try {
    const { getManifest } = await import("./ssr-client-bundler");
    const manifest: any = await getManifest();
    const fonts = manifest?.fonts;
    if (!fonts) return "";
    const { renderFontFaceCss } = await import("./ssr-fonts");
    const prefix = manifest.public_prefix || "/_pylon/build/";
    const css = renderFontFaceCss(fonts, prefix);
    let blob = css ? `<style data-pylon-fonts>${css}</style>` : "";
    for (const href of (fonts.preload || []) as string[]) {
      // `crossorigin` is REQUIRED on font preloads even same-origin — fonts
      // fetch in CORS mode, so without it the browser double-fetches.
      blob += `<link rel="preload" as="font" type="font/woff2" href="${prefix}${href}" crossorigin>`;
    }
    return blob;
  } catch {
    return "";
  }
}

/**
 * Build the hydration tail appended after React's stream EOFs: the
 * `__PYLON_DATA__` JSON blob (props + ssrData) + the per-route entry
 * `<script>` that hydrates it, + (dev) the live-reload snippet. Shared by the
 * page render AND the now-hydrated boundary render (#279) so a boundary
 * hydrates through the EXACT same path as a page.
 *
 * `kind` marks an error/not-found boundary so the client knows whether to
 * synthesize a `reset()`. For an error boundary, `errorForClient` is the SAFE
 * projection ({message, digest}) — the raw `Error` (and its stack) is NEVER
 * serialized (the dev overlay owns dev stacks; preserves the #270 posture).
 */
export function buildHydrationTail(args: {
  component: string;
  layouts: string[];
  props: any;
  ssrData: Record<string, any>;
  manifestRoute: { file: string; imports: string[]; css: string[] } | null;
  publicPrefix: string;
  manifestErr: string | null;
  kind?: "error" | "not-found";
  errorForClient?: { message: string; digest?: string };
  // PPR Phase 0 (auth-bucketed caching): when set, this render is being stored
  // as a SHARED cache entry keyed only on session-cookie PRESENCE, so its
  // hydration tail must carry an IDENTITY-FREE auth — the binary `{ signedIn }`
  // bit the bucket is keyed on, and NOTHING ELSE (no user_id / tenant_id /
  // roles / email). Two different signed-in users hitting the same bucketed
  // page MUST produce a byte-identical tail, or the shared cache replays one
  // user's identity to another (the #277 leak class, at the body level). The
  // raw `auth` is replaced AFTER the live-handle strip below.
  bucketAuth?: { signedIn: boolean };
}): string {
  // Strip live, non-serializable handles (serverData / response / reset) + the
  // request headers/cookies (SECURITY: never expose the session cookie to
  // client JS — see #270). The raw `error` Error is dropped too; an error
  // boundary's client-visible error rides in `errorForClient` instead.
  const {
    serverData: _sd,
    response: _resp,
    reset: _reset,
    headers: _h,
    cookies: _c,
    error: _err,
    ...restProps
  } = args.props ?? {};
  let serializableProps: any;
  if (args.bucketAuth) {
    // Bucketed render → SHARED entry. Serialize a strict ALLOWLIST of the
    // framework props that are provably bucket-uniform — never a spread of the
    // page's props object. A page can alias identity onto a custom key
    // (`props.leak = props.auth`) without tripping read-tracking; spreading
    // `restProps` would then serialize that identity into the shared tail. The
    // allowlisted fields are all path/route-derived (in the cache key) or the
    // collapsed binary auth bit. (params/searchParams are keyed by pathname; a
    // bucket request has no query, so searchParams is empty.)
    serializableProps = {
      url: restProps.url,
      params: restProps.params,
      searchParams: restProps.searchParams,
      auth: { signedIn: args.bucketAuth.signedIn },
      session: { exists: args.bucketAuth.signedIn },
      headers: {},
      cookies: {},
    };
  } else {
    serializableProps = { ...restProps, headers: {}, cookies: {} };
  }
  if (args.errorForClient) serializableProps.error = args.errorForClient;
  const hydrationPayload: any = {
    component: args.component,
    layouts: args.layouts ?? [],
    props: serializableProps,
    ssrData: args.ssrData,
  };
  if (args.kind) hydrationPayload.kind = args.kind;
  // Escape `<` (closes a </script> breakout) + U+2028/U+2029 (JSON-valid but
  // JS statement terminators). Regex form keeps the separators visible in
  // source rather than as invisible literals.
  const json = JSON.stringify(hydrationPayload)
    .replace(/</g, "\\u003c")
    .replace(/\u2028/g, "\\u2028")
    .replace(/\u2029/g, "\\u2029");
  let tail = `<script id="__PYLON_DATA__" type="application/json">${json}</script>`;
  if (args.manifestRoute) {
    // A cross-origin CDN module (`public_prefix` is an absolute http(s) URL) is
    // fetched in CORS mode; `crossorigin` makes the matching modulepreload
    // reusable (no double fetch) and surfaces load errors. No-op for the
    // same-origin `/_pylon/build/` default.
    const co = /^https?:\/\//i.test(args.publicPrefix) ? " crossorigin" : "";
    tail += `<script type="module"${co} src="${args.publicPrefix}${args.manifestRoute.file}"></script>`;
  } else {
    tail += `<script>console.warn(${JSON.stringify(`[pylon ssr] hydration disabled: ${args.manifestErr}`)})</script>`;
  }
  if (isDevMode()) tail += DEV_LIVE_RELOAD_SNIPPET;
  return tail;
}

/**
 * The layout chain for a component, walked top-down from `app/` to the
 * component's own directory — IDENTICAL to the bundler's `discoverRoutes`
 * accumulation (and the SDK's `discoverAppRoutes`). A boundary's hydration
 * needs the SERVER tree wrapped in the SAME layouts the bundler baked into
 * its client entry; the catch path otherwise has only the failing PAGE's
 * layouts, which would mismatch a root boundary covering a nested page.
 */
function resolveLayoutChain(componentRelPath: string, cwd: string): string[] {
  const fs = require("node:fs");
  const path = require("node:path");
  const rel = componentRelPath.replace(/\\/g, "/");
  const dir = rel.includes("/") ? rel.slice(0, rel.lastIndexOf("/")) : "";
  const parts = dir.split("/").filter(Boolean);
  const layouts: string[] = [];
  let acc = "";
  for (const part of parts) {
    acc = acc ? `${acc}/${part}` : part;
    for (const ext of MODULE_EXTS) {
      if (fs.existsSync(path.join(cwd, acc, `layout${ext}`))) {
        layouts.push(`${acc}/layout`);
        break;
      }
    }
  }
  return layouts;
}

/**
 * A short, non-reversible correlation id for an error — surfaced to the
 * client error boundary as `error.digest` (matching server logs) WITHOUT
 * carrying any stack content. FNV-1a over message+stack, 8 hex chars.
 */
export function errorDigest(err: any): string {
  const s = `${err?.message ?? ""}\n${err?.stack ?? ""}`;
  let h = 0x811c9dc5;
  for (let i = 0; i < s.length; i++) {
    h ^= s.charCodeAt(i);
    h = Math.imul(h, 0x01000193);
  }
  return (h >>> 0).toString(16).padStart(8, "0");
}

/**
 * Render a boundary (not-found/error) tree, stream it at `status`, and —
 * when the boundary has a client bundle entry (#279) — append the hydration
 * tail so onClick/useState/`reset()` work. With no manifest entry the
 * boundary still renders (server-only, styled) — CSS/hydration must never
 * block the error path.
 */
async function renderBoundaryToClient(
  React: any,
  renderToReadableStream: any,
  tree: any,
  send: Send,
  callId: string,
  status: number,
  headers: Record<string, string>,
  tail?: {
    component: string;
    layouts: string[];
    props: any;
    ssrData: Record<string, any>;
    kind: "error" | "not-found";
    errorForClient?: { message: string; digest?: string };
  },
): Promise<void> {
  const stream: ReadableStream<Uint8Array> = await renderToReadableStream(tree, {
    onError(e: unknown) {
      // eslint-disable-next-line no-console
      console.error("[ssr] boundary render error:", e);
    },
  });
  // Resolve the boundary's own client entry (keyed by its component path) so
  // the head gets ITS css/modulepreloads and the body-tail loads ITS script.
  let manifestRoute:
    | { file: string; imports: string[]; css: string[] }
    | null = null;
  let publicPrefix = "/_pylon/build/";
  let headBlob = "";
  if (tail) {
    try {
      const { getManifest } = await import("./ssr-client-bundler");
      const manifest = await getManifest();
      publicPrefix = manifest.public_prefix || publicPrefix;
      manifestRoute = manifest.routes[tail.component] ?? null;
    } catch {
      manifestRoute = null;
    }
  }
  if (manifestRoute) {
    headBlob += await buildFontHeadBlob();
    const co = /^https?:\/\//i.test(publicPrefix) ? " crossorigin" : "";
    for (const css of manifestRoute.css) {
      headBlob += `<link rel="stylesheet" href="${publicPrefix}${css}">`;
    }
    for (const chunk of manifestRoute.imports) {
      headBlob += `<link rel="modulepreload"${co} href="${publicPrefix}${chunk}">`;
    }
  } else {
    // No per-boundary entry → fall back to the global stylesheet union so the
    // page is at least styled (static). (collectBoundaryHeadBlob emits fonts.)
    headBlob = await collectBoundaryHeadBlob();
  }
  // renderToReadableStream resolved without throwing → safe to commit the
  // head now, then drain the (already-rendered) shell, injecting CSS.
  send({ type: "response_start", call_id: callId, status, headers });
  const sendChunk = (text: string) => {
    if (!text) return;
    send({
      type: "render_chunk",
      call_id: callId,
      data: Buffer.from(text, "utf8").toString("base64"),
    });
  };
  await streamWithHeadInjection(stream.getReader(), headBlob, sendChunk);
  if (tail && manifestRoute) {
    const tailHtml = buildHydrationTail({
      component: tail.component,
      layouts: tail.layouts,
      props: tail.props,
      ssrData: tail.ssrData,
      manifestRoute,
      publicPrefix,
      manifestErr: null,
      kind: tail.kind,
      errorForClient: tail.errorForClient,
    });
    sendChunk(tailHtml);
  }
  send({ type: "render_done", call_id: callId });
}

/**
 * Resolve + render a boundary component wrapped in the route's layout
 * chain. Returns true if it rendered (caller returns), false to fall back.
 */
async function tryRenderBoundary(
  opts: {
    React: any;
    renderToReadableStream: any;
    cwd: string;
    componentPath: string;
    fileName: "not-found" | "error";
    layouts: string[] | undefined;
    props: any;
    send: Send;
    callId: string;
    status: number;
    headers: Record<string, string>;
  },
): Promise<boolean> {
  const { React, renderToReadableStream, cwd, componentPath, fileName, props, send, callId, status, headers } =
    opts;
  if (!React || !renderToReadableStream || !props) return false;
  const rel = findBoundary(componentPath, fileName);
  if (!rel) return false;
  try {
    const mod = await importModule(cwd, rel);
    const Comp = mod.default ?? mod.Component ?? mod.NotFound ?? mod.Error;
    if (typeof Comp !== "function") return false;
    // The boundary hydrates through its OWN layout chain (walked from app/ to
    // the boundary's dir) — NOT the failing page's chain — so the server tree
    // matches the client entry the bundler baked for this boundary (#279).
    const boundaryLayouts = resolveLayoutChain(rel, cwd);
    // For an error boundary, project the thrown Error to the SAFE client shape
    // ({message, digest}) and give BOTH server + client the SAME value (zero
    // hydration mismatch) + a no-op reset() server-side. The raw Error/stack
    // never reaches the client (the dev overlay owns dev stacks; #270).
    let errorForClient: { message: string; digest?: string } | undefined;
    let compProps = props;
    if (fileName === "error") {
      const rawErr = props.error;
      errorForClient = {
        message: rawErr?.message ?? String(rawErr ?? "Error"),
        digest: errorDigest(rawErr),
      };
      compProps = { ...props, error: errorForClient, reset: () => {} };
    }
    let tree = React.createElement(Comp, compProps);
    tree = await buildLayoutTree(cwd, tree, boundaryLayouts, compProps, React);
    await renderBoundaryToClient(
      React,
      renderToReadableStream,
      tree,
      send,
      callId,
      status,
      headers,
      {
        component: rel,
        layouts: boundaryLayouts,
        props: compProps,
        // Catch-path boundaries don't set up serverData/use() (the by-name
        // not-found dispatch through handleRenderRoute does); empty ssrData.
        ssrData: {},
        kind: fileName === "error" ? "error" : "not-found",
        errorForClient,
      },
    );
    return true;
  } catch (e) {
    // Boundary render itself failed — no tertiary fallback; let the caller
    // emit its default (fixed 404 body / type:"error" → 500).
    // eslint-disable-next-line no-console
    console.error(`[ssr] ${fileName}.tsx boundary failed to render:`, e);
    return false;
  }
}

/**
 * Deterministic stringify (keys sorted recursively) so a `serverData` call's
 * cache key is identical on the server (here) and on the client (the
 * hydration shim in ssr-client-bundler's client-runtime). MUST stay in sync
 * with `stableStringify` in that template.
 */
function stableStringify(v: any): string {
  if (v === null || v === undefined || typeof v !== "object") {
    return JSON.stringify(v ?? null);
  }
  if (Array.isArray(v)) return "[" + v.map(stableStringify).join(",") + "]";
  const keys = Object.keys(v).sort();
  return (
    "{" +
    keys.map((k) => JSON.stringify(k) + ":" + stableStringify(v[k])).join(",") +
    "}"
  );
}

// The read methods a page reaches through `serverData`. Mirrors the
// DbReader read surface (writes are blocked server-side). Kept in sync with
// the client-runtime shim's method list.
const SERVER_DATA_METHODS = [
  "get",
  "list",
  "lookup",
  "query",
  "queryGraph",
  "paginate",
  "search",
] as const;

/**
 * Wrap a DbReader so each `serverData.x(...)` call returns a PROMISE CACHED
 * by (method, args) — required for React 19 `use()`, which re-invokes the
 * call on the post-suspense re-render and must get the same (now-resolved)
 * promise instead of a fresh pending one (else it suspends forever). Each
 * resolved value is also recorded into `valueCache` keyed identically, so it
 * can be serialized into `__PYLON_DATA__.ssrData` and replayed on the client
 * — keeping hydration free of mismatches.
 */
function makeServerData(reader: any, valueCache: Record<string, any>): any {
  const promiseCache = new Map<string, Promise<any>>();
  const wrap = (r: any, prefix: string): any => {
    const out: any = {};
    for (const m of SERVER_DATA_METHODS) {
      out[m] = (...args: any[]) => {
        const key = prefix + m + ":" + stableStringify(args);
        let p = promiseCache.get(key);
        if (!p) {
          p = Promise.resolve(r[m](...args)).then((value: any) => {
            valueCache[key] = value;
            return value;
          });
          promiseCache.set(key, p);
        }
        return p;
      };
    }
    return out;
  };
  const sd = wrap(reader, "");
  if (reader.unsafe) sd.unsafe = wrap(reader.unsafe, "u:");
  return sd;
}

/**
 * #278: does this route STREAM (vs buffer the whole document)? Streaming is
 * opt-in: a `loading.tsx` (route-level Suspense) or `export const streaming =
 * true` (inner-boundary). Pure for testing.
 */
export function computeWantsStream(hasLoading: boolean, mod: any): boolean {
  return hasLoading || mod?.streaming === true;
}

/**
 * #277: how long an opt-in page stays cacheable, in seconds — or null if it
 * never opted in. `export const revalidate = N` (N>0) → N; `dynamic:
 * "force-static"` → a year (only a deploy invalidates); else null. Pure.
 */
export function computeRevalidateSecs(mod: any): number | null {
  if (typeof mod?.revalidate === "number" && mod.revalidate > 0) {
    return Math.floor(mod.revalidate);
  }
  if (mod?.dynamic === "force-static") return 31536000;
  return null;
}

/**
 * #277 cache verdict — the security-critical predicate, extracted pure so the
 * leak class (a personalized/streaming render marked cacheable) is a TEST, not
 * a mental walkthrough. INVARIANT: result ⟹ !wantsStream (a streaming render
 * commits its head before auth/cookies/status are final, so it can never be
 * cached). Fail-closed: every condition must hold.
 */
export function computeCacheVerdict(args: {
  revalidateSecs: number | null;
  forceDynamic: boolean;
  authTouched: boolean;
  // True when the render read any OTHER per-request input that is not part of
  // the cache key — request `headers` (incl. non-bucketed ones) or `cookies`
  // (incl. via generateMetadata). Such a read makes the output request-specific,
  // so it must veto shared caching exactly like `authTouched`. (Keyed inputs —
  // pathname, and Host via the host bucket — are handled by the cache key and do
  // not set this.)
  dynamicTouched: boolean;
  // True when the render read `props.session.exists` (Phase 0). For the PLAIN
  // anonymous cache this is a veto — the output varies by signed-in bucket, and
  // the anon cache holds one entry for all. (The auth-BUCKET verdict, which keys
  // the cache on the bucket, permits it; this gate does not.)
  sessionTouched: boolean;
  cookieCount: number;
  strictPolicies: boolean;
  wantsStream: boolean;
  status: number;
}): boolean {
  return (
    args.revalidateSecs != null &&
    !args.forceDynamic &&
    !args.authTouched &&
    !args.dynamicTouched &&
    !args.sessionTouched &&
    args.cookieCount === 0 &&
    !args.strictPolicies &&
    !args.wantsStream &&
    args.status === 200
  );
}

/**
 * PPR Phase 0 — the AUTH-BUCKET verdict: may this render be stored as a SHARED
 * cache entry keyed only on session-cookie PRESENCE (one signed-in shell + one
 * signed-out shell, both IDENTITY-FREE)? Pure, exported, leak-class tested.
 *
 * Like `computeCacheVerdict` EXCEPT `sessionTouched` is ALLOWED (reading
 * `props.session.exists` to render a binary signed-in/out nav is the whole
 * point — that bit is what the bucket keys on, and it's identical across all
 * users in the same bucket). Everything else still vetoes:
 *
 *  - `authTouched` — reading real `props.auth` (user_id/tenant_id/roles) makes
 *    the output identity-SPECIFIC, not bucket-uniform. This is the load-bearing
 *    gate: ANY identity-dependent output requires reading auth, so vetoing on
 *    `authTouched` proves a bucketable body depends on nothing but the binary
 *    session bit.
 *  - `dynamicTouched` — reading request headers/cookies makes it request-specific.
 *  - `strictPolicies` — in strict mode serverData reads are auth-FILTERED, so the
 *    body/ssrData would vary by identity even without the page reading auth. In
 *    the default (non-strict) mode serverData reads bypass the policy gate and
 *    return identity-independent rows, so the cached body is safe to share within
 *    the bucket. Strict mode therefore must NOT bucket.
 *  - cookieCount / forceDynamic / wantsStream / non-200 — same reasons as the
 *    anon verdict (a Set-Cookie, a streaming head, or an error/redirect can't be
 *    safely shared).
 *
 * Requires the explicit `bucketOptIn` (`export const cache = "auth-bucketed"`)
 * AND a TTL via `export const revalidate = N` — fail-closed: no opt-in or no TTL
 * → no bucket.
 */
export function computeBucketVerdict(args: {
  bucketOptIn: boolean;
  revalidateSecs: number | null;
  forceDynamic: boolean;
  authTouched: boolean;
  dynamicTouched: boolean;
  cookieCount: number;
  strictPolicies: boolean;
  wantsStream: boolean;
  status: number;
}): boolean {
  return (
    args.bucketOptIn &&
    args.revalidateSecs != null &&
    !args.forceDynamic &&
    !args.authTouched &&
    !args.dynamicTouched &&
    args.cookieCount === 0 &&
    !args.strictPolicies &&
    !args.wantsStream &&
    args.status === 200
  );
}

/**
 * Dev HUD: explain the cache verdict for a finished render in human terms — the
 * verdict, its TTL, and (when dynamic) the SINGLE reason it isn't cached. Pure so
 * it's unit-tested and matches `computeCacheVerdict`/`computeBucketVerdict`
 * exactly. The "why isn't my page caching?" answer the framework otherwise throws
 * away.
 */
export function describeCacheVerdict(v: {
  bucketOptIn: boolean;
  cacheable: boolean;
  bucketable: boolean;
  revalidateSecs: number | null;
  authTouched: boolean;
  dynamicTouched: boolean;
  sessionTouched: boolean;
  cookieCount: number;
  strictPolicies: boolean;
  wantsStream: boolean;
  forceDynamic: boolean;
  status: number;
}): { verdict: "bucketed" | "cacheable" | "dynamic"; secs: number | null; reason: string } {
  if (v.bucketable) {
    return {
      verdict: "bucketed",
      secs: v.revalidateSecs,
      reason: "shared cache, keyed on session presence",
    };
  }
  if (v.cacheable) {
    return {
      verdict: "cacheable",
      secs: v.revalidateSecs,
      reason: "anonymous shared cache",
    };
  }
  // Dynamic — surface the single most useful reason (mirrors the verdict order).
  let reason: string;
  if (!v.bucketOptIn && v.revalidateSecs == null) {
    reason = "not opted in — add `export const revalidate = N` (or `cache = \"auth-bucketed\"`)";
  } else if (v.status !== 200) {
    reason = `status ${v.status} (only 200 is cacheable)`;
  } else if (v.forceDynamic) {
    reason = "dynamic = \"force-dynamic\"";
  } else if (v.wantsStream) {
    reason = "streaming render (loading.tsx / streaming = true)";
  } else if (v.cookieCount > 0) {
    reason = "the render set a cookie";
  } else if (v.strictPolicies) {
    reason = "PYLON_STRICT_FN_POLICIES (serverData reads are auth-filtered)";
  } else if (v.authTouched) {
    reason = "read props.auth (identity-specific)";
  } else if (v.dynamicTouched) {
    reason = "read request headers/cookies";
  } else if (v.sessionTouched) {
    reason = v.bucketOptIn
      ? "read props.session"
      : "read props.session — add `export const cache = \"auth-bucketed\"` to bucket-cache it";
  } else {
    reason = "not cacheable";
  }
  return { verdict: "dynamic", secs: null, reason };
}

/**
 * #278: diff the response head committed at `response_start` against the final
 * state after EOF, to catch a late response.* mutation from a suspended subtree
 * that the already-sent head couldn't carry. Returns the dropped pieces, or
 * null if nothing was lost. Pure.
 */
export function diffCommittedResponse(
  snapshot: { status: number; cookies: string[]; headerKeys: string[] },
  final: { status: number; cookies: string[]; headers: Record<string, string> },
): { droppedCookies: string[]; statusChanged: boolean; newHeaderKeys: string[] } | null {
  const droppedCookies = final.cookies.filter(
    (c) => !snapshot.cookies.includes(c),
  );
  const statusChanged = final.status !== snapshot.status;
  const newHeaderKeys = Object.keys(final.headers)
    .sort()
    .filter((k) => !snapshot.headerKeys.includes(k));
  if (droppedCookies.length || statusChanged || newHeaderKeys.length) {
    return { droppedCookies, statusChanged, newHeaderKeys };
  }
  return null;
}

// ===========================================================================
// Data-route file conventions: app/sitemap.ts → /sitemap.xml,
// app/robots.ts → /robots.txt (Next.js-style). @pylonsync/sdk discovers these
// (routes with kind "sitemap"/"robots") so the host routes /sitemap.xml +
// /robots.txt through the SSR RPC; here we detect them by the module basename,
// call the default export (awaiting if async — so it can hit the DB / enumerate
// dynamic pages), serialize the return to XML / plain text, and stream it with
// the right content-type. No React render.
// ===========================================================================

/** One URL entry in a sitemap (mirrors Next's `MetadataRoute.Sitemap[number]`). */
export interface SitemapEntry {
  url: string;
  lastModified?: string | Date;
  changeFrequency?:
    | "always"
    | "hourly"
    | "daily"
    | "weekly"
    | "monthly"
    | "yearly"
    | "never";
  priority?: number;
  /** hreflang alternates, e.g. `{ languages: { "en-US": "https://…/en" } }`. */
  alternates?: { languages?: Record<string, string> };
}

/** Return type of a default export in `app/sitemap.ts`. */
export type Sitemap = SitemapEntry[];

export interface RobotsRule {
  userAgent?: string | string[];
  allow?: string | string[];
  disallow?: string | string[];
  crawlDelay?: number;
}

/** Return type of a default export in `app/robots.ts`. */
export interface Robots {
  rules: RobotsRule | RobotsRule[];
  sitemap?: string | string[];
  host?: string;
}

/** Serialize sitemap entries to a sitemaps.org 0.9 XML document. */
export function serializeSitemap(entries: Sitemap | undefined): string {
  const list = Array.isArray(entries) ? entries : [];
  const esc = (s: unknown): string =>
    String(s)
      .replace(/&/g, "&amp;")
      .replace(/</g, "&lt;")
      .replace(/>/g, "&gt;")
      .replace(/"/g, "&quot;")
      .replace(/'/g, "&apos;");
  const iso = (d: string | Date): string =>
    d instanceof Date ? d.toISOString() : String(d);
  const needsXhtml = list.some(
    (e) =>
      e?.alternates?.languages &&
      Object.keys(e.alternates.languages).length > 0,
  );
  const ns =
    'xmlns="http://www.sitemaps.org/schemas/sitemap/0.9"' +
    (needsXhtml ? ' xmlns:xhtml="http://www.w3.org/1999/xhtml"' : "");
  const body = list
    .filter((e) => e && typeof e.url === "string")
    .map((e) => {
      let u = `<url><loc>${esc(e.url)}</loc>`;
      if (e.lastModified != null)
        u += `<lastmod>${esc(iso(e.lastModified))}</lastmod>`;
      if (e.changeFrequency) u += `<changefreq>${esc(e.changeFrequency)}</changefreq>`;
      if (e.priority != null) u += `<priority>${esc(e.priority)}</priority>`;
      const langs = e.alternates?.languages;
      if (langs) {
        for (const [lang, href] of Object.entries(langs)) {
          u += `<xhtml:link rel="alternate" hreflang="${esc(lang)}" href="${esc(href)}"/>`;
        }
      }
      return `${u}</url>`;
    })
    .join("");
  return `<?xml version="1.0" encoding="UTF-8"?>\n<urlset ${ns}>${body}</urlset>\n`;
}

/** Serialize a robots config to robots.txt text. */
export function serializeRobots(robots: Robots | undefined): string {
  const arr = (v: string | string[] | undefined): string[] =>
    v == null ? [] : Array.isArray(v) ? v : [v];
  const lines: string[] = [];
  const rules = robots
    ? Array.isArray(robots.rules)
      ? robots.rules
      : [robots.rules]
    : [];
  for (const rule of rules) {
    if (!rule) continue;
    const uas = arr(rule.userAgent);
    for (const ua of uas.length ? uas : ["*"]) lines.push(`User-agent: ${ua}`);
    for (const a of arr(rule.allow)) lines.push(`Allow: ${a}`);
    for (const d of arr(rule.disallow)) lines.push(`Disallow: ${d}`);
    if (rule.crawlDelay != null) lines.push(`Crawl-delay: ${rule.crawlDelay}`);
    lines.push("");
  }
  for (const sm of arr(robots?.sitemap)) lines.push(`Sitemap: ${sm}`);
  if (robots?.host) lines.push(`Host: ${robots.host}`);
  return `${lines.join("\n").replace(/\n*$/, "")}\n`;
}

/**
 * Import app/sitemap or app/robots, run its default export, serialize the
 * result, and stream it back with the right content-type. Errors surface as a
 * 500 with a short plain-text message (so a broken sitemap doesn't wedge the
 * runner). 1-hour cache — sitemaps/robots change rarely; tune via a CDN.
 */
export async function handleDataRoute(
  msg: RenderRouteMessage,
  kind: "sitemap" | "robots",
  send: Send,
): Promise<void> {
  const emit = (status: number, contentType: string, body: string): void => {
    send({
      type: "response_start",
      call_id: msg.call_id,
      status,
      headers:
        status === 200
          ? { "content-type": contentType, "cache-control": "public, max-age=3600" }
          : { "content-type": contentType },
    });
    send({
      type: "render_chunk",
      call_id: msg.call_id,
      data: Buffer.from(body, "utf8").toString("base64"),
    });
    send({ type: "render_done", call_id: msg.call_id });
  };
  try {
    const cwd = process.cwd();
    const mod = await importModule(cwd, msg.component);
    const exp = mod.default ?? mod[kind];
    const data = typeof exp === "function" ? await exp() : exp;
    if (kind === "sitemap") {
      emit(200, "application/xml; charset=utf-8", serializeSitemap(data as Sitemap));
    } else {
      emit(200, "text/plain; charset=utf-8", serializeRobots(data as Robots));
    }
  } catch (e: any) {
    emit(
      500,
      "text/plain; charset=utf-8",
      `pylon: failed to generate ${kind} (${msg.component}): ${e?.message ?? String(e)}\n`,
    );
  }
}

export async function handleRenderRoute(
  msg: RenderRouteMessage,
  send: Send,
): Promise<void> {
  // Data-route conventions short-circuit the React render path: app/sitemap →
  // /sitemap.xml (XML), app/robots → /robots.txt (text). Detected by the module
  // basename — the SDK only registers these routes for app/sitemap.* /
  // app/robots.*, so the basename is exactly "sitemap"/"robots" (a real page
  // component always ends in "/page"). Mirrors the not-found/error basename
  // check used for boundaries.
  const dataKind: "sitemap" | "robots" | null = /(^|[\\/])sitemap$/.test(
    msg.component,
  )
    ? "sitemap"
    : /(^|[\\/])robots$/.test(msg.component)
      ? "robots"
      : null;
  if (dataKind) return handleDataRoute(msg, dataKind, send);

  // Dev HUD: wall-clock for the whole render, surfaced in the dev overlay's
  // render row. `performance.now()` is monotonic; harmless in prod (unused).
  const renderStart = isDevMode() ? performance.now() : 0;

  // Declared OUTSIDE the try so the catch can read page-set status/
  // cookies when turning a redirect()/notFound() throw into a response.
  const responseState: ResponseState = {
    status: msg.initial_status ?? 200,
    headers: {},
    cookies: [],
  };
  const response = makeResponseController(responseState);
  // Hoisted out of the try so the catch can render not-found.tsx /
  // error.tsx boundaries (which need React + the renderer + cwd + props).
  const cwd = process.cwd();
  let React: any = null;
  let renderToReadableStream: any = null;
  let props: any = null;
  // Revoke fns for this render's per-request proxies (auth/headers/cookies/
  // session). Declared OUT here so the `finally` revokes them on EVERY exit path
  // (success, redirect/notFound/error boundary, throw) — neutralizing any
  // module-stashed reference to a prior request's identity.
  const proxyRevokers: Array<() => void> = [];
  // Accumulates the resolved results of every `serverData.*` read the page
  // made during render, keyed identically to the client shim. Serialized
  // into `__PYLON_DATA__.ssrData` so hydration replays the same values
  // without a second round trip — and without a server/client mismatch.
  const ssrValueCache: Record<string, any> = {};
  try {
    // react + react-dom are USER deps. ssr-runtime.ts lives in
    // packages/functions/src/, but the user's react install is under
    // their project cwd. `import("react-dom/server")` in this file
    // would resolve against pylon's own node_modules (which doesn't
    // declare react), so we route through a Bun-resolveSync against
    // the user's cwd.
    const resolveFromUser = (spec: string): string =>
      (Bun as any).resolveSync
        ? (Bun as any).resolveSync(spec, cwd)
        : spec;
    // `renderToReadableStream` is only exported from
    // `react-dom/server.browser` (WHATWG streams), not the plain
    // `react-dom/server` (which is Node-stream-style). Try browser
    // first, fall back to the default entry for environments that
    // re-route it (Next runs a custom dist).
    let reactDomServerImport: any;
    try {
      // @ts-ignore — user-dep, resolved at runtime
      reactDomServerImport = await import(
        /* @vite-ignore */ resolveFromUser("react-dom/server.browser")
      );
    } catch {
      // @ts-ignore — user-dep, resolved at runtime
      reactDomServerImport = await import(
        /* @vite-ignore */ resolveFromUser("react-dom/server")
      );
    }
    // @ts-ignore — user-dep, resolved at runtime
    const reactImport = await import(
      /* @vite-ignore */ resolveFromUser("react")
    );
    React = reactImport.default ?? reactImport;
    renderToReadableStream =
      reactDomServerImport.renderToReadableStream ??
      reactDomServerImport.default?.renderToReadableStream;
    if (typeof renderToReadableStream !== "function") {
      throw new Error(
        "react-dom/server.browser does not export renderToReadableStream — install react@>=18 + react-dom@>=18",
      );
    }

    // Resolve the page module (project-relative, extension-agnostic).
    let mod: any;
    try {
      mod = await importModule(cwd, msg.component);
    } catch (e) {
      throw e instanceof Error
        ? e
        : new Error(`could not import component "${msg.component}"`);
    }
    const Component = mod.default ?? mod.Page ?? mod.page;
    if (typeof Component !== "function") {
      throw new Error(
        `component "${msg.component}" has no default export (or named export "Page")`,
      );
    }

    // `serverData` — a read-only DB handle the page reaches during render
    // via React 19 `use()` + <Suspense>. Reuses runtime.ts's RPC pipe
    // (shared `send` + pendingRpcs) keyed by this render's call_id; the
    // Rust render loop answers the frames against the same store + policy
    // gate as a query function's ctx.db, and rejects any write. Promise-
    // cached so `use()` doesn't re-suspend forever; resolved values land in
    // `ssrValueCache` for hydration replay.
    const { buildDbReader } = await import("./runtime");
    const serverData = makeServerData(
      buildDbReader(msg.call_id),
      ssrValueCache,
    );

    // #277 cache-safety proof. A render is shareable (CDN/disk cacheable) ONLY
    // if its output is independent of per-request inputs — so wrap each in a
    // read-tracking Proxy that flips a flag the moment a page/layout/
    // generateMetadata observes it (get / `in` / Object.keys / probe). The
    // proxies are REVOKED in the `finally` (never restored to raw onto `props`),
    // so the live props object never aliases a prior request's identity. The tail
    // serializes from the raw `msg` values via `tailProps` below.
    const track = (
      obj: Record<string, unknown> | undefined,
      onTouch: () => void,
    ) => {
      const { proxy, revoke } = makeRevocableReadTrackingProxy(obj, onTouch);
      proxyRevokers.push(revoke);
      return proxy;
    };

    // Reading auth at all (even for an anon request) opts the render OUT of
    // caching, because the output could differ by identity.
    let authTouched = false;
    const authProxy = track(
      msg.auth as Record<string, unknown> | undefined,
      () => {
        authTouched = true;
      },
    );

    // Same proof for the OTHER per-request inputs that aren't in the cache key:
    // reading `headers` (any header, incl. via generateMetadata) or `cookies`
    // makes the output request-specific. Only USER code reads props.headers/
    // cookies — the framework's metadata path reads `msg.headers` directly.
    let dynamicTouched = false;
    const touchProxy = (obj: Record<string, unknown> | undefined) =>
      track(obj, () => {
        dynamicTouched = true;
      });

    // Phase 0 (auth bucketing): `props.session.exists` — the identity-FREE
    // presence bit, so a page can render a binary auth nav WITHOUT reading real
    // `auth`. Reading it sets `sessionTouched` (separate from authTouched): it
    // vetoes the plain anonymous cache (the output now varies by signed-in
    // bucket) but is PERMITTED by the bucket verdict (which keys the cache on the
    // bucket).
    let sessionTouched = false;
    const sessionProxy = track(
      { exists: msg.session_present === true },
      () => {
        sessionTouched = true;
      },
    );

    props = {
      url: msg.url,
      params: msg.params,
      searchParams: msg.search_params,
      headers: touchProxy(msg.headers as Record<string, unknown> | undefined),
      cookies: touchProxy(msg.cookies as Record<string, unknown> | undefined),
      auth: authProxy,
      session: sessionProxy,
      // Response controller — a page/layout calls response.setStatus /
      // setHeader / setCookie / redirect / notFound to shape the reply.
      response,
      // Read-only server data handle (see above).
      serverData,
    };

    // PPR Phase 0: an immutable, pre-render SNAPSHOT of the only props a bucketed
    // (SHARED) tail may serialize — url/params/searchParams, all path-derived and
    // in the cache key. Deep-cloned NOW, before any page/generateMetadata code
    // runs, because `props.params`/`searchParams` ARE the live `msg` objects: a
    // page could mutate a nested field to smuggle identity (e.g.
    // `props.searchParams.leak = props.auth`) WITHOUT tripping read-tracking, and
    // a shallow allowlist would copy that mutation by reference into the shared
    // tail. Captured only for opted-in pages (zero cost otherwise).
    const bucketOptIn = (mod as any).cache === "auth-bucketed";
    const bucketTailBase = bucketOptIn
      ? {
          url: msg.url,
          params: jsonClone(msg.params),
          searchParams: jsonClone(msg.search_params),
        }
      : null;

    // SEO metadata: static `export const metadata` or dynamic
    // `export async function generateMetadata(props)`. Awaited before the
    // first byte, so keep it to cheap derivations (params → title); heavy
    // data belongs in the page body behind <Suspense>.
    let metadata: SsrMetadata | undefined = mod.metadata;
    if (typeof mod.generateMetadata === "function") {
      metadata = await mod.generateMetadata(props);
    }
    // File conventions: auto-wire <meta og:image>/<twitter:image> from a
    // colocated opengraph-image.* / twitter-image.*, and <link rel="icon">
    // from icon.* / apple-icon.* / favicon.ico — unless the page set them.
    metadata = applyAutoSocialImages(msg.component, msg.headers, metadata);
    metadata = applyAutoIcons(msg.component, metadata);
    const metaFragment = renderMetadata(React, metadata);

    // loading.tsx (#278): the nearest `loading` module — walked up from the
    // page dir, like not-found/error — becomes ONE route-level Suspense
    // fallback wrapping the page. When present, the shell (layouts) + this
    // skeleton flush immediately and React reveals the real page content when
    // the page's top-level `use()` resolves, instead of buffering the whole
    // document (see the `allReady` gate below). A page with no loading.tsx
    // keeps the byte-identical buffered single-flush path.
    //
    // The skeleton is SERVER-ONLY and must not read `serverData` (a read would
    // suspend the FALLBACK itself, delaying the shell). It gets the page props
    // for url/params/searchParams/auth.
    const loadingRel = findBoundary(msg.component, "loading");
    let Loading: any = null;
    if (loadingRel) {
      try {
        const lMod = await importModule(cwd, loadingRel);
        const L = lMod.default ?? lMod.Loading ?? lMod.loading;
        if (typeof L === "function") Loading = L;
      } catch {
        // A broken loading.tsx must never block the page — fall back to the
        // buffered path.
        Loading = null;
      }
    }

    // The page leaf, optionally wrapped in the single Suspense boundary.
    let pageLeaf: any = React.createElement(Component, props);
    if (Loading) {
      pageLeaf = React.createElement(
        React.Suspense,
        { fallback: React.createElement(Loading, props) },
        pageLeaf,
      );
    }

    // Streaming decision (#278). Computed from STATIC module exports only —
    // knowable before any await, so the buffer/cache decision never reads
    // non-final render state. A page STREAMS (shell + each inner <Suspense>
    // fallback flush immediately, content reveals as data resolves) when it has
    // a loading.tsx (route-level boundary, #278 Stage 1) OR explicitly opts in
    // with `export const streaming = true` (inner-boundary streaming, Stage 2).
    // Every un-annotated page keeps the byte-identical BUFFERED path (allReady)
    // that 100% of today's prod traffic rides — this is opt-in, never default.
    const wantsStream = computeWantsStream(!!Loading, mod);

    // Resolve the layout chain. Each layout module exports a default
    // function that accepts the same props + `children`. Walk leaf →
    // root: start with the page component as `tree`, then for each
    // layout (innermost first) wrap it as the new tree. Result is
    // the outermost layout containing all nested layouts down to
    // the page. The metadata fragment is the FIRST child so React hoists
    // its <title>/<meta> into the <head> a layout renders.
    let tree: any = metaFragment
      ? React.createElement(React.Fragment, null, metaFragment, pageLeaf)
      : pageLeaf;
    tree = await buildLayoutTree(cwd, tree, msg.layouts, props, React);
    const element = tree;
    const stream: ReadableStream<Uint8Array> = await renderToReadableStream(
      element,
      {
        onError(err: unknown) {
          // React captures render errors during the streaming render
          // and feeds them here. We log to stderr; we do NOT truncate or
          // rewrite the response here — on a streamed render the HTTP head is
          // already committed, so a mid-stream error just closes the body
          // (partial HTML); the dev overlay (#275) only covers failures BEFORE
          // response_start (host-side err channel). Buffered renders surface
          // their error through the catch/boundary path below.
          const ctrl = asRouteControl(err);
          if (ctrl) {
            // A redirect()/notFound() thrown from BELOW a <Suspense> boundary:
            // the shell already committed the head, so React swallowed it and
            // it can't change the response. This is a known limitation on BOTH
            // the buffered and streamed paths (response.* must fire in the
            // synchronous shell). Surface it loudly instead of silently losing.
            // eslint-disable-next-line no-console
            console.error(
              `[ssr] ${ctrl.kind}() called below a <Suspense> boundary was ignored — ` +
                `the HTTP head was already sent. Call response.redirect()/notFound() (or ` +
                `notFound() from @pylonsync/react) in the synchronous shell render, before ` +
                `any await/<Suspense>.`,
            );
            return;
          }
          // eslint-disable-next-line no-console
          console.error("[ssr] renderToReadableStream onError:", err);
        },
      },
    );

    // Wait for ALL Suspense boundaries to resolve before emitting the body,
    // so the HTML is fully formed — no `<!--$?-->` pending markers, no
    // hidden fallback segments, no `$RC` reveal scripts. This is what makes
    // `serverData` + `use()` + <Suspense> hydrate cleanly: the client
    // hydrates a RESOLVED boundary against the SSR'd content (resolved from
    // `ssrData`), instead of fighting React's streaming-reveal scripts +
    // whole-document hydration (which leaves the boundary stuck on its
    // fallback). Pages with no async data have no boundaries, so `allReady`
    // resolves immediately — zero cost for the common case.
    //
    // EXCEPTION (#278): a STREAMING render (loading.tsx route-level boundary,
    // or `export const streaming = true` for inner boundaries) DELIBERATELY
    // skips the buffer — the shell + each <Suspense> fallback flush first, then
    // React reveals each boundary's real content + its reveal script as that
    // boundary's `use()` resolves. Hydration stays clean for ANY number of
    // boundaries because Pylon runs hydrateRoot ONCE, post-EOF: the entry
    // <script> is appended AFTER the full `__PYLON_DATA__` blob (which carries
    // the fully-resolved `ssrData` map) and after all of React's $RC reveals,
    // so the client's `use()` reads a fulfilled value and never re-suspends —
    // there is no progressive hydration racing the stream.
    if (!wantsStream && (stream as any).allReady) {
      await (stream as any).allReady;
    }

    // Headers go out before the first chunk so the host can write the
    // response head.
    // The shell rendered without a redirect()/notFound() throw, so the
    // page's chosen status (default 200) + headers + cookies go out now,
    // before the first body byte.
    //
    // #277 cache verdict (Stage 1, buffered path only — a streaming render
    // commits its head before the body resolves, so `authTouched` isn't final
    // yet). A render is shareable (CDN-cacheable via `public, s-maxage`) ONLY
    // when ALL hold: it opted in (`export const revalidate` / `dynamic:
    // "force-static"`), never read props.auth (authTouched), set no cookie,
    // isn't `force-dynamic`, and per-caller strict policies are OFF — in strict
    // mode serverData reads are auth-filtered, so the output isn't shareable.
    // We emit an INTERNAL `x-pylon-cacheable` header the host turns into a
    // public Cache-Control and STRIPS; its ABSENCE is the fail-closed default
    // (the host keeps no-cache / no-store). The 200-only guard avoids caching
    // an error/redirect.
    const revalidateSecs = computeRevalidateSecs(mod);
    const forceDynamic = (mod as any).dynamic === "force-dynamic";
    const strictPolicies = process.env.PYLON_STRICT_FN_POLICIES === "1";
    // INVARIANT: cacheable ⟹ !wantsStream. A streaming render commits its head
    // (response_start) BEFORE suspended subtrees finish, so `authTouched`,
    // `responseState.cookies`, and `.status` are NOT final here — caching it
    // could share a personalized/non-final body. So `!wantsStream` (NOT just
    // `!Loading`) is the gate: a `streaming = true` page has `Loading` null but
    // `wantsStream` true, and must still be excluded. Fail-closed. (See
    // computeCacheVerdict — pure + unit-tested for the leak class.)
    // PPR Phase 0: `bucketOptIn` (`export const cache = "auth-bucketed"`) was
    // resolved at props-construction time (it gates the immutable bucketTailBase
    // snapshot). A bucketed render and the plain anon cache are mutually exclusive
    // — `cacheable` excludes the opt-in so a bucket page never also emits the anon
    // proof.
    const cacheable =
      !bucketOptIn &&
      computeCacheVerdict({
        revalidateSecs,
        forceDynamic,
        authTouched,
        dynamicTouched,
        sessionTouched,
        cookieCount: responseState.cookies.length,
        strictPolicies,
        wantsStream,
        status: responseState.status,
      });
    // The bucket verdict permits `sessionTouched` (reading the binary signed-in
    // bit) but still vetoes any real-auth read. When true, this render is stored
    // as a shared, identity-free, session-presence-keyed entry — so its
    // hydration tail MUST be anonymized (bucketAuth below) and it advertises the
    // TRUSTED internal `x-pylon-bucket` proof for the host to key + store it.
    const bucketable = computeBucketVerdict({
      bucketOptIn,
      revalidateSecs,
      forceDynamic,
      authTouched,
      dynamicTouched,
      cookieCount: responseState.cookies.length,
      strictPolicies,
      wantsStream,
      status: responseState.status,
    });
    // Dev diagnostics (the agent + HUD signal): the cache verdict + the single
    // actionable reason, computed ONCE here and reused for the dev HUD blob, the
    // `x-pylon-dev` header (→ the host's diagnostics ring + `pylon diagnostics`),
    // and the structured dev log. Null in prod.
    const renderMode = wantsStream ? "ssr-streaming" : "ssr-buffered";
    const devVerdict = isDevMode()
      ? describeCacheVerdict({
          bucketOptIn,
          cacheable,
          bucketable,
          revalidateSecs,
          authTouched,
          dynamicTouched,
          sessionTouched,
          cookieCount: responseState.cookies.length,
          strictPolicies,
          wantsStream,
          forceDynamic,
          status: responseState.status,
        })
      : null;
    // Serialization view of props built from the RAW `msg` values — the live
    // `props` (with its read-tracking proxies) is NEVER mutated back to raw, so
    // it can't become a vehicle that leaks a prior request's identity to a later
    // render (the proxies are revoked in `finally`). headers/cookies are stripped
    // inside buildHydrationTail; auth uses the raw value (so anon auth stays
    // falsy/undefined rather than the proxy's empty `{}`). For a BUCKETED render,
    // url/params/searchParams come from the pre-render `bucketTailBase` snapshot —
    // NOT the live (page-mutable) props — so a page can't smuggle identity through
    // a nested field into the shared tail. buildHydrationTail's bucket allowlist
    // then serializes only these fields + the collapsed { signedIn } bit.
    const tailProps = props
      ? {
          ...props,
          auth: msg.auth,
          headers: msg.headers,
          cookies: msg.cookies,
          session: { exists: msg.session_present === true },
          ...(bucketable && bucketTailBase ? bucketTailBase : {}),
        }
      : props;
    // #278: on a STREAMING render the head commits NOW, before suspended
    // subtrees run. Snapshot what's committed so we can detect (after EOF) a
    // late response.setStatus/setCookie/setHeader from a suspended subtree that
    // got silently dropped — and warn loudly instead of leaving the dev to
    // debug a missing Set-Cookie. Buffered renders need no snapshot (the whole
    // render is done before this point, so nothing can change after).
    const committedSnapshot = wantsStream
      ? {
          status: responseState.status,
          cookies: responseState.cookies.map((c) => String(c)),
          headerKeys: Object.keys(responseState.headers).sort(),
        }
      : null;
    send({
      type: "response_start",
      call_id: msg.call_id,
      status: responseState.status,
      headers: finalizeHeaders(responseState, undefined, {
        // The #277 anon proof and the Phase 0 bucket proof both ride the TRUSTED
        // `internal` channel (never stripped here, always stripped by the host
        // before the client), so userland (page setHeader / route-handler headers
        // via `extra`) can't forge either. Mutually exclusive (bucketOptIn splits
        // the two verdicts).
        ...(bucketable
          ? { "x-pylon-bucket": String(revalidateSecs) }
          : cacheable
            ? { "x-pylon-cacheable": String(revalidateSecs) }
            : {}),
        // Dev-only: the host parses this into its diagnostics ring (served at
        // /_pylon/dev/diagnostics + `pylon diagnostics`). Single-line JSON, no
        // newlines. Stripped before the client like every x-pylon-* header.
        ...(devVerdict
          ? {
              "x-pylon-dev": JSON.stringify({
                verdict: devVerdict.verdict,
                secs: devVerdict.secs,
                reason: devVerdict.reason,
                mode: renderMode,
                component: msg.component,
              }),
            }
          : {}),
      }),
    });

    // Pre-load the manifest BEFORE the React stream starts emitting
    // so we know which `<link rel="stylesheet">` and
    // `<link rel="modulepreload">` tags to inject into the HEAD.
    // We splice them in before `</head>` so the browser starts
    // fetching CSS + chunks concurrently with parsing the body —
    // no FOUC, no waterfall.
    let preloadManifestRoute:
      | { file: string; imports: string[]; css: string[] }
      | null = null;
    let preloadManifestErr: string | null = null;
    let preloadPublicPrefix = "/_pylon/build/";
    try {
      const { getManifest } = await import("./ssr-client-bundler");
      const manifest = await getManifest();
      preloadPublicPrefix = manifest.public_prefix || preloadPublicPrefix;
      preloadManifestRoute = manifest.routes[msg.component] ?? null;
      if (!preloadManifestRoute) {
        preloadManifestErr = `manifest has no entry for "${msg.component}"`;
      }
    } catch (e: any) {
      preloadManifestErr = e?.message || String(e);
    }

    // Build the head-injection blob — stylesheet first, then
    // modulepreloads. The entry script tag stays in the body-tail
    // (it needs the inline __PYLON_DATA__ to have been parsed first).
    let headBlob = "";
    if (preloadManifestRoute) {
      headBlob += await buildFontHeadBlob();
      const co = /^https?:\/\//i.test(preloadPublicPrefix) ? " crossorigin" : "";
      for (const css of preloadManifestRoute.css) {
        headBlob += `<link rel="stylesheet" href="${preloadPublicPrefix}${css}">`;
      }
      for (const chunk of preloadManifestRoute.imports) {
        headBlob += `<link rel="modulepreload"${co} href="${preloadPublicPrefix}${chunk}">`;
      }
    } else {
      // No per-route client entry. This is the unmatched-URL not-found
      // dispatch (the host renders `app/not-found` by name at 404) or any
      // other component without a hydration bundle. It still renders inside
      // the app shell, so inject the global stylesheet(s) — otherwise the
      // 404 page is unstyled. Hydration stays disabled (handled below).
      headBlob += await collectBoundaryHeadBlob();
    }

    // The host can dispatch a boundary module (`app/not-found` / `app/error`)
    // by name for an unmatched-URL 404. Boundaries render server-only — no
    // hydration payload, and no "hydration disabled" warning (that warning is
    // for a real page whose client bundle is missing).
    const isBoundaryComponent = /(^|\/)(not-found|error)$/.test(msg.component);

    // Stream-rewrite: watch for `</head>` and inject `headBlob` before it.
    // `</head>` may straddle chunk boundaries; the shared helper keeps a
    // small carry buffer to catch a split tag. base64 of each utf-8 slice
    // happens in `sendChunk` (Buffer ships with Bun).
    const sendChunk = (text: string) => {
      if (!text) return;
      send({
        type: "render_chunk",
        call_id: msg.call_id,
        data: Buffer.from(text, "utf8").toString("base64"),
      });
    };
    await streamWithHeadInjection(stream.getReader(), headBlob, sendChunk);

    // #278: detect a late response.* mutation from a suspended subtree that the
    // already-committed head couldn't carry, and warn loudly (a silently
    // dropped Set-Cookie reads as "logged out" / missing CSRF in prod). Only
    // for streamed renders — a buffered render finalized before response_start,
    // so nothing changes after. The fix for the dev is to move the call into
    // the synchronous shell (or drop `streaming = true`); we name what was lost.
    if (committedSnapshot) {
      // Same diff the unit tests exercise — call the pure helper so the tested
      // path IS the prod path (no drift).
      const dropped = diffCommittedResponse(committedSnapshot, {
        status: responseState.status,
        cookies: responseState.cookies,
        headers: responseState.headers,
      });
      if (dropped) {
        const parts: string[] = [];
        if (dropped.droppedCookies.length)
          parts.push(
            `Set-Cookie [${dropped.droppedCookies
              .map((c) => {
                const eq = c.indexOf("="); // serialized "name=value; …"
                return eq >= 0 ? c.slice(0, eq) : c;
              })
              .join(", ")}]`,
          );
        if (dropped.statusChanged)
          parts.push(
            `status ${committedSnapshot.status}→${responseState.status}`,
          );
        if (dropped.newHeaderKeys.length)
          parts.push(`headers [${dropped.newHeaderKeys.join(", ")}]`);
        // eslint-disable-next-line no-console
        console.error(
          `[ssr] response.* called below a <Suspense> boundary on a streaming ` +
            `route was DROPPED (the HTTP head already shipped): ${parts.join("; ")}. ` +
            `Set response status/cookies/headers in the synchronous shell render, ` +
            `before any await/<Suspense> — or remove \`export const streaming = true\`.`,
        );
      }
    }

    // Hydration tail. After React's stream EOFs we append the
    // hydration markers so the browser can hydrate:
    //   1. `__PYLON_DATA__` — JSON-typed script with the props the
    //      page was rendered with. The per-route bundle reads this
    //      to seed hydrateRoot.
    //   2. `<link rel="modulepreload">` for every transitive shared
    //      chunk (react, react-dom, client-runtime). These preload
    //      tags fire as soon as the parser sees them; the browser
    //      can start fetching while it's still parsing the body.
    //   3. `<script type="module" src="<route-entry>.js">` — the
    //      per-route entry. It imports the shared chunks, which
    //      were already in the cache from step 2.
    //
    // Per-route entry + chunk paths come from
    // `.pylon/client-build/manifest.json`, which the bundler writes
    // and `getManifest` parses with mtime-keyed caching. Falls back
    // to a no-hydration warning if the manifest can't be loaded
    // (rare — usually means the bundler crashed).
    // Pages always hydrate. A boundary dispatched BY NAME here (the host
    // rendering `app/not-found` at 404) now hydrates too (#279) when it has a
    // client entry - only stay server-only (no tail) when there's no entry to
    // load. `buildHydrationTail` does the props strip (serverData/response +
    // the security headers/cookies strip) + the </script> + U+2028/2029
    // escaping. The CSS/modulepreload links were already injected into <head>.
    const wantsHydration = !isBoundaryComponent || !!preloadManifestRoute;
    if (wantsHydration) {
      const tail = buildHydrationTail({
        component: msg.component,
        layouts: msg.layouts ?? [],
        props: tailProps,
        ssrData: ssrValueCache,
        manifestRoute: preloadManifestRoute,
        publicPrefix: preloadPublicPrefix,
        manifestErr: preloadManifestErr,
        kind: isBoundaryComponent
          ? /(^|\/)error$/.test(msg.component)
            ? "error"
            : "not-found"
          : undefined,
        // A bucketed render is stored shared → its tail must carry ONLY the
        // binary signed-in bit, never this request's real identity.
        bucketAuth: bucketable
          ? { signedIn: msg.session_present === true }
          : undefined,
      });
      sendChunk(tail);
    }

    // Dev HUD (dev only): append the cache-verdict + render-timing blob + the
    // floating overlay, and emit ONE structured log line. After the page tail so
    // the HUD marker/probe are in place before the deferred client entry boots the
    // sync engine. `devVerdict` was computed once above (reused here + in the
    // x-pylon-dev header). The log rides the runtime's inherited stderr, so an
    // agent running `pylon dev` sees the verdict without any extra call.
    if (devVerdict) {
      const renderMs = Math.round((performance.now() - renderStart) * 10) / 10;
      sendChunk(
        buildDevHudChunk({
          route: msg.url,
          component: msg.component,
          renderMode,
          renderMs,
          // verdict + reason + the raw gate flags, so the HUD's cache drawer can
          // show the full per-gate breakdown (which check vetoed caching).
          cache: {
            ...devVerdict,
            flags: {
              bucketOptIn,
              revalidateSecs,
              authTouched,
              dynamicTouched,
              sessionTouched,
              cookieCount: responseState.cookies.length,
              strictPolicies,
              wantsStream,
              status: responseState.status,
            },
          },
        }),
      );
      // The structured dev-log line is emitted host-side (Rust tracing) from the
      // x-pylon-dev header, so it rides the same log stream as every other
      // [pylon] line — no duplicate console.error here.
    }

    send({ type: "render_done", call_id: msg.call_id });
  } catch (err: any) {
    // A page/layout called response.redirect()/response.notFound(), or
    // `notFound()` from @pylonsync/react, during render → short-circuit to a
    // 3xx + Location or a 404 instead of a body. Page-set cookies/headers
    // still ride along.
    const ctrl = asRouteControl(err);
    if (ctrl) {
      if (ctrl.kind === "redirect") {
        send({
          type: "response_start",
          call_id: msg.call_id,
          status: ctrl.redirectStatus ?? 307,
          headers: finalizeHeaders(responseState, {
            location: ctrl.url ?? "/",
          }),
        });
        send({ type: "render_done", call_id: msg.call_id });
        return;
      }
      // notFound() → look for the nearest not-found.tsx walking up from the
      // page's directory; render it (wrapped in the route's layouts) at 404.
      // Falls back to a minimal framework body if none is defined.
      if (
        await tryRenderBoundary({
          React,
          renderToReadableStream,
          cwd,
          componentPath: msg.component,
          fileName: "not-found",
          layouts: msg.layouts,
          props,
          send,
          callId: msg.call_id,
          status: 404,
          headers: finalizeHeaders(responseState),
        })
      ) {
        return;
      }
      const body404 =
        '<!DOCTYPE html><html><head><meta charset="utf-8"><title>404 — Not Found</title></head><body><h1>404</h1><p>This page could not be found.</p></body></html>';
      send({
        type: "response_start",
        call_id: msg.call_id,
        status: 404,
        headers: finalizeHeaders(responseState),
      });
      send({
        type: "render_chunk",
        call_id: msg.call_id,
        data: Buffer.from(body404, "utf8").toString("base64"),
      });
      send({ type: "render_done", call_id: msg.call_id });
      return;
    }
    // Real pre-first-chunk error → look for the nearest error.tsx walking up
    // from the page's directory; render it (wrapped in the route's layouts)
    // at 500 with the thrown error passed in props. Falls back to a host-level
    // 500 (type:"error") if none is defined or the boundary itself throws.
    if (
      await tryRenderBoundary({
        React,
        renderToReadableStream,
        cwd,
        componentPath: msg.component,
        fileName: "error",
        layouts: msg.layouts,
        props: props ? { ...props, error: err } : null,
        send,
        callId: msg.call_id,
        status: 500,
        headers: finalizeHeaders(responseState),
      })
    ) {
      return;
    }
    // In dev, send the full stack as the message so the host can paint a
    // useful error overlay instead of an opaque 500. In prod, send only the
    // message (the host shows a generic page; the stack stays in logs).
    const devMode = isDevMode();
    send({
      type: "error",
      call_id: msg.call_id,
      code: err?.code ?? "SSR_RENDER_FAILED",
      message:
        devMode && err?.stack ? String(err.stack) : err?.message ?? String(err),
    });
  } finally {
    // Revoke this render's per-request proxies. Any reference a page stashed in
    // module-level state (props or props.auth/headers/cookies/session) now throws
    // on access from a LATER render — fail-closed, so a prior request's identity
    // can never silently enter a cached body without tripping read-tracking.
    for (const revoke of proxyRevokers) {
      try {
        revoke();
      } catch {
        // best-effort
      }
    }
  }
}
