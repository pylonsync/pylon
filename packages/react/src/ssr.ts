// SSR page-author types.
//
// These describe the contract the Pylon SSR runtime hands every
// `app/**/page.tsx` (and `layout.tsx`) component in props, plus the
// `metadata` / `generateMetadata` exports. They are TYPE-ONLY — no runtime
// — so a page author writes:
//
//   import type { PageProps, Metadata } from "@pylonsync/react";
//
//   export const metadata: Metadata = { title: "Blog" };
//   export default function Page({ params, serverData, response }: PageProps) { ... }
//
// The source of truth for the shape is the runtime in
// `@pylonsync/functions` (`ssr-runtime.ts`); these mirror it. Keep them in
// sync when the runtime props change.

/**
 * The resolved Pylon auth context for the request that rendered the page.
 * Safe to read in the component body — it is serialized into the hydration
 * payload, so the server render and the client hydration see the same
 * values (no hydration mismatch).
 */
export interface PageAuth {
  /** The signed-in user's id, or null for an anonymous request. */
  user_id: string | null;
  /** True when the session is an admin session (PYLON_ADMIN_EMAILS). */
  is_admin: boolean;
  /** The active tenant/org id, or null. */
  tenant_id: string | null;
  /** Role slugs granted to the session. */
  roles: string[];
}

/** Options for `response.setCookie`. Defaults: HttpOnly + SameSite=Lax. */
export interface SsrCookieOptions {
  path?: string;
  domain?: string;
  maxAge?: number;
  expires?: Date | string;
  /** Defaults to true (secure default). Pass false for a client-readable cookie. */
  httpOnly?: boolean;
  secure?: boolean;
  sameSite?: "Strict" | "Lax" | "None";
}

/**
 * The per-render `response` controller. Pylon already has a backend for
 * data + mutations, so SSR's job is just the HTTP response envelope:
 * status, redirects, 404, and the occasional Set-Cookie.
 *
 * IMPORTANT — call these during the SYNCHRONOUS shell render (the component
 * body, before any `await` / Suspense boundary). The HTTP head is committed
 * when the shell is ready; status/headers/cookies set from a suspended
 * subtree that streams in later are lost, and a `redirect()` / `notFound()`
 * thrown below a Suspense boundary is swallowed by React's error handling
 * rather than turned into a 3xx / 404.
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
   * minimal framework body if none is defined. Shell-render only.
   */
  notFound(): never;
}

/**
 * Read-only database handle a page reaches during render via React 19
 * `use()` + `<Suspense>`. Reads run through the same store + policy gate as
 * a query function's `ctx.db`; writes are rejected. Resolved values are
 * cached and replayed into the hydration payload, so the client does not
 * re-fetch on hydration.
 *
 * ```tsx
 * export default function Page({ serverData }: PageProps) {
 *   const posts = use(serverData.list<Post>("Post"));
 *   return <ul>{posts.map((p) => <li key={p.id}>{p.title}</li>)}</ul>;
 * }
 * ```
 */
export interface ServerData {
  /** Get a single row by id. Resolves to null if not found. */
  get<T = Record<string, unknown>>(entity: string, id: string): Promise<T | null>;
  /** List all (policy-visible) rows for an entity. */
  list<T = Record<string, unknown>>(entity: string): Promise<T[]>;
  /** Look up a row by a field value (e.g. email). Null if not found. */
  lookup<T = Record<string, unknown>>(
    entity: string,
    field: string,
    value: string,
  ): Promise<T | null>;
  /** Query with filters ($gt, $lt, $in, $like, $order, $limit, …). */
  query<T = Record<string, unknown>>(
    entity: string,
    filter: Record<string, unknown>,
  ): Promise<T[]>;
  /** Execute a graph query with nested relation includes. */
  queryGraph<T = Record<string, unknown>>(
    query: Record<string, unknown>,
  ): Promise<T>;
  /** Cursor-paginated list. Pass `cursor` from a previous page's result. */
  paginate<T = Record<string, unknown>>(
    entity: string,
    opts: { numItems: number; cursor?: string | null },
  ): Promise<{ rows?: T[]; page?: T[]; nextCursor?: string | null }>;
  /** Faceted full-text search against an entity with a `search:` config. */
  search<T = Record<string, unknown>>(
    entity: string,
    query: Record<string, unknown>,
  ): Promise<{
    hits: T[];
    facetCounts?: Record<string, Record<string, number>>;
    total: number;
    tookMs?: number;
  }>;
}

/**
 * Props every `page.tsx` / `layout.tsx` receives. Generic over the dynamic
 * route params and the parsed query string, so a route like
 * `app/blog/[slug]/page.tsx` can type them:
 *
 * ```tsx
 * export default function Post({ params }: PageProps<{ slug: string }>) {
 *   return <article>{params.slug}</article>;
 * }
 * ```
 *
 * Note: the incoming request's headers + cookies are intentionally NOT on
 * this type. They are available only during the server render and are
 * stripped from the hydration payload (a session cookie must never reach
 * client JS), so reading them in the component body would hydrate-mismatch.
 * Read request-derived data through `serverData` or a server function.
 */
export interface PageProps<
  TParams extends Record<string, string> = Record<string, string>,
  TSearchParams extends Record<string, string> = Record<string, string>,
> {
  /** The incoming URL path (e.g. `/blog/hello-world`). */
  url: string;
  /** Dynamic-segment matches keyed by name (e.g. `{ slug: "hello-world" }`). */
  params: TParams;
  /** Parsed query string (e.g. `?start=10` → `{ start: "10" }`). */
  searchParams: TSearchParams;
  /** The resolved auth context for the request. */
  auth: PageAuth;
  /** The HTTP response controller (status / headers / cookies / redirect). */
  response: SsrResponse;
  /** Read-only database handle for in-render data (use with `use()`). */
  serverData: ServerData;
}

/**
 * Page SEO metadata. Export `const metadata` (static) or
 * `async function generateMetadata(props)` (dynamic, e.g. param-derived
 * titles) from a `page.tsx` / `layout.tsx`. React 19 hoists the resulting
 * `<title>` / `<meta>` / `<link>` into `<head>`.
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

export interface Metadata {
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
    imageSecureUrl?: string;
    imageType?: string;
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
  /** Structured data (schema.org), emitted as `<script type="application/ld+json">`.
   *  Pass one object or an array (each item gets its own script). The payload is
   *  serialized with `<`/`>`/`&` escaped so it can't break out of the script. */
  jsonLd?: Record<string, unknown> | Record<string, unknown>[];
}

/**
 * Signature of a dynamic `generateMetadata` export. Receives the same props
 * as the page; return (or resolve to) the page's `Metadata`. Awaited before
 * the first byte, so keep it to cheap derivations.
 */
export type GenerateMetadata<
  TParams extends Record<string, string> = Record<string, string>,
  TSearchParams extends Record<string, string> = Record<string, string>,
> = (
  props: PageProps<TParams, TSearchParams>,
) => Metadata | Promise<Metadata>;

/**
 * Return type for an `app/sitemap.ts` default export (Next-shaped). The runtime
 * serializes it to `/sitemap.xml`. The export may be sync or async, so it can
 * enumerate dynamic pages from the DB:
 *
 *   export default async function sitemap(): Promise<Sitemap> {
 *     const posts = await getPosts();
 *     return [{ url: "https://x.com/", priority: 1 }, ...posts.map(...)];
 *   }
 */
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
export type Sitemap = SitemapEntry[];

export interface RobotsRule {
  userAgent?: string | string[];
  allow?: string | string[];
  disallow?: string | string[];
  crawlDelay?: number;
}

/**
 * Return type for an `app/robots.ts` default export (Next-shaped). The runtime
 * serializes it to `/robots.txt`.
 *
 *   export default function robots(): Robots {
 *     return {
 *       rules: { userAgent: "*", allow: "/", disallow: "/admin" },
 *       sitemap: "https://x.com/sitemap.xml",
 *     };
 *   }
 */
export interface Robots {
  rules: RobotsRule | RobotsRule[];
  sitemap?: string | string[];
  host?: string;
}

/**
 * Per-route configuration, declared as top-level `export const` in a
 * `page.tsx` (Next-shaped). All optional. The runtime reads these statically
 * before the render.
 *
 * ```ts
 * export const revalidate = 60;          // CDN + origin-disk cache for 60s
 * export const dynamic = "force-dynamic"; // never cache
 * export const streaming = true;          // progressive Suspense streaming
 * ```
 *
 * - `revalidate` — seconds an auth-independent render stays cacheable
 *   (`public, s-maxage=N`); also kept in the origin disk cache. See the
 *   "Anonymous output caching" rules — it only takes effect if the render
 *   never reads `auth`, sets no cookie, and isn't running strict policies.
 * - `dynamic` — `"force-static"` caches until the next deploy; `"force-dynamic"`
 *   opts out of all caching.
 * - `streaming` — opt into PROGRESSIVE streaming: the shell + each inner
 *   `<Suspense>` fallback flush immediately, then each boundary's real content
 *   streams in as its data resolves (instead of buffering the whole document).
 *   A page with a `loading.tsx` already streams at the route level; this
 *   extends it to a page's OWN inner boundaries. Mutually exclusive with
 *   caching: a streaming render commits its HTTP head before suspended
 *   subtrees finish, so it can never be marked cacheable (and a deep
 *   `<Suspense>` throw resolves via its `error.tsx` at HTTP 200, not a 5xx).
 *   Set `response.*` (status/cookies) only in the synchronous shell — late
 *   calls from a suspended subtree are dropped (the runtime warns).
 */
export interface RouteSegmentConfig {
  revalidate?: number;
  dynamic?: "force-static" | "force-dynamic";
  streaming?: boolean;
}

/**
 * Props an `app/.../error.tsx` boundary receives. Error boundaries are now
 * HYDRATED (interactive — useState/onClick/effects work), so `reset` is a
 * real callback that re-attempts rendering the segment (a transient error
 * clears to the page; a deterministic one re-shows the boundary).
 *
 * `error` carries ONLY the thrown error's `message` plus a short,
 * non-reversible `digest` (a correlation id matching the server log). The
 * stack NEVER reaches the client — read it from the dev overlay
 * (`PYLON_DEV_MODE`) or the server logs.
 *
 * ```tsx
 * export default function Error({ error, reset }: ErrorBoundaryProps) {
 *   return (
 *     <div>
 *       <p>Something went wrong: {error.message}</p>
 *       <button onClick={reset}>Try again</button>
 *     </div>
 *   );
 * }
 * ```
 */
export interface ErrorBoundaryProps {
  error: { message: string; digest?: string };
  reset: () => void;
}

/**
 * Props an `app/.../not-found.tsx` boundary receives. Not-found boundaries
 * are hydrated (interactive) too, but — matching Next — receive NO `reset`.
 * Same shape as a page.
 */
export type NotFoundProps = PageProps;

/**
 * Parsed form fields handed to a `route.ts` handler. Mirrors URLSearchParams
 * `get`/`getAll`/`has` semantics over the submitted body.
 */
export interface FormFields {
  /** First value for `name`, or null. */
  get(name: string): string | null;
  /** All values for `name` (empty array if none). */
  getAll(name: string): string[];
  has(name: string): boolean;
  /** Raw map: name → value (single) or values (repeated field). */
  readonly fields: Record<string, string | string[]>;
}

/**
 * Read + write DB handle for a `route.ts` form handler (mutation-shaped). The
 * read surface is `ServerData`; writes go through the same policy-checked,
 * broadcast-firing path a mutation's `ctx.db` uses.
 */
export interface FormDb extends ServerData {
  insert<T = Record<string, unknown>>(
    entity: string,
    data: Record<string, unknown>,
  ): Promise<T>;
  update<T = Record<string, unknown>>(
    entity: string,
    id: string,
    data: Record<string, unknown>,
  ): Promise<T>;
  delete(entity: string, id: string): Promise<void>;
}

/**
 * The request a `route.ts` POST/PUT/PATCH/DELETE handler receives. Shape the
 * reply through `response` — usually `response.redirect("/x?ok=1")` (303
 * POST-redirect-GET, the default) after a write, so a no-JS browser follows
 * with a GET. Enforce trust with `auth` inside the handler (forms run with the
 * standard function trust model).
 *
 * ```ts
 * import type { RouteHandler } from "@pylonsync/react";
 * export const POST: RouteHandler = async ({ form, db, response, auth }) => {
 *   const body = form.get("body");
 *   if (!body) return response.redirect("/notes?error=empty");
 *   await db.insert("Note", { body });
 *   response.redirect("/notes?created=1");
 * };
 * ```
 */
export interface FormRequest<
  TParams extends Record<string, string> = Record<string, string>,
  TSearchParams extends Record<string, string> = Record<string, string>,
> {
  form: FormFields;
  params: TParams;
  searchParams: TSearchParams;
  auth: PageAuth;
  cookies: Record<string, string>;
  headers: Record<string, string>;
  db: FormDb;
  response: SsrResponse;
}

/** Signature of a `route.ts` method handler export (POST/PUT/PATCH/DELETE). */
export type RouteHandler<
  TParams extends Record<string, string> = Record<string, string>,
  TSearchParams extends Record<string, string> = Record<string, string>,
> = (req: FormRequest<TParams, TSearchParams>) => void | Promise<void>;

/**
 * What a `route.ts` `GET` (raw) handler returns. The body is streamed verbatim
 * with `contentType` (default `text/plain; charset=utf-8`), `status` (default
 * 200), and any extra `headers` — no React render, no hydration tail. This is
 * the GET analogue of `sitemap.ts`/`robots.ts`, at an arbitrary route path:
 * RSS/Atom feeds, dynamic XML, plain text, JSON, `.well-known` documents.
 */
export interface RawResponse {
  body?: string;
  /** Defaults to `text/plain; charset=utf-8`. */
  contentType?: string;
  /** Defaults to 200. */
  status?: number;
  /** Extra response headers (e.g. `cache-control`). Lower-cased by the host. */
  headers?: Record<string, string>;
}

/**
 * Signature of a `route.ts` `GET` export — a RAW handler. Return a
 * {@link RawResponse} to stream a body, or use `response.redirect()` /
 * `response.notFound()` and return nothing.
 *
 * ```ts
 * import type { RawRouteHandler } from "@pylonsync/react";
 * export const GET: RawRouteHandler = async () => ({
 *   body: "<rss>…</rss>",
 *   contentType: "application/xml; charset=utf-8",
 *   headers: { "cache-control": "public, max-age=300" },
 * });
 * ```
 */
export type RawRouteHandler<
  TParams extends Record<string, string> = Record<string, string>,
  TSearchParams extends Record<string, string> = Record<string, string>,
> = (
  req: FormRequest<TParams, TSearchParams>,
) => RawResponse | void | Promise<RawResponse | void>;
