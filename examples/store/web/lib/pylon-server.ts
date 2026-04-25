/**
 * Server-side Pylon API client used by Next.js Server Components.
 *
 * The Pylon sync engine (`@pylonsync/react`) is browser-only — it
 * relies on IndexedDB and WebSockets — so server-rendered pages
 * can't use it. Instead, server components hit Pylon's HTTP REST
 * surface directly.
 *
 * For public reads (catalog, product detail) we don't need any auth
 * token, since the Product entity has `allowRead: "true"`. For
 * authenticated reads we'd forward the user's session cookie, but
 * the SEO-relevant pages here are all public.
 */
import "server-only";

import type { Product, SearchResult } from "./types";

const BASE_URL = process.env.PYLON_BASE_URL ?? "http://localhost:4321";

type PylonError = { error: { code: string; message: string } };

async function pylonFetch<T>(
  path: string,
  init: RequestInit & { revalidate?: number; tags?: string[] } = {},
): Promise<T> {
  const { revalidate, tags, ...rest } = init;
  const res = await fetch(`${BASE_URL}${path}`, {
    ...rest,
    headers: {
      "Content-Type": "application/json",
      // Server-side requests bypass CORS, but Pylon's policy still
      // expects an Origin header on state-changing methods. Reads are
      // exempt; we only set this defensively.
      Origin: BASE_URL,
      ...(rest.headers ?? {}),
    },
    next: revalidate != null || tags ? { revalidate, tags } : undefined,
  });
  if (!res.ok) {
    const body = (await res.json().catch(() => ({}))) as Partial<PylonError>;
    const code = body.error?.code ?? "PYLON_ERROR";
    const message = body.error?.message ?? `Pylon ${path} → ${res.status}`;
    throw Object.assign(new Error(message), { code, status: res.status });
  }
  return (await res.json()) as T;
}

/**
 * Search the Product catalog server-side. Used by the catalog page so
 * the initial render contains real product cards (good for SEO + LCP).
 */
export async function searchProducts(opts: {
  query?: string;
  filters?: Record<string, string>;
  facets?: string[];
  sort?: [string, "asc" | "desc"];
  page?: number;
  pageSize?: number;
}): Promise<SearchResult> {
  return pylonFetch<SearchResult>("/api/search/Product", {
    method: "POST",
    body: JSON.stringify({
      query: opts.query ?? "",
      filters: opts.filters ?? {},
      facets: opts.facets ?? ["brand", "category", "color"],
      sort: opts.sort,
      page: opts.page ?? 0,
      pageSize: opts.pageSize ?? 24,
    }),
    // Cache for 30 seconds. Catalog content moves slowly; the seed is
    // generated once and rarely changes. Stale-while-revalidate keeps
    // the page snappy without serving stuck data forever.
    revalidate: 30,
    tags: ["product-search"],
  });
}

/** Fetch a single Product row by id. Used by `/p/[id]` for SSR. */
export async function getProduct(id: string): Promise<Product | null> {
  try {
    return await pylonFetch<Product>(`/api/entities/Product/${encodeURIComponent(id)}`, {
      revalidate: 60,
      tags: [`product:${id}`],
    });
  } catch (err) {
    if ((err as { status?: number }).status === 404) return null;
    throw err;
  }
}

/** Fetch every Product id for the sitemap. */
export async function listProductIds(limit = 1000): Promise<string[]> {
  type ListResp = { data: Pick<Product, "id">[] };
  const res = await pylonFetch<ListResp>(
    `/api/entities/Product?limit=${limit}`,
    { revalidate: 300, tags: ["product-list"] },
  );
  return res.data.map((p) => p.id);
}
