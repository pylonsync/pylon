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
export interface Metadata {
  title?: string;
  description?: string;
  keywords?: string | string[];
  canonical?: string;
  robots?: string;
  openGraph?: {
    title?: string;
    description?: string;
    image?: string;
    imageSecureUrl?: string;
    imageType?: string;
    imageWidth?: number;
    imageHeight?: number;
    imageAlt?: string;
    url?: string;
    type?: string;
  };
  twitter?: {
    card?: string;
    title?: string;
    description?: string;
    image?: string;
  };
  icons?: {
    icon?: { url: string; type?: string; sizes?: string };
    apple?: { url: string; type?: string; sizes?: string };
  };
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
