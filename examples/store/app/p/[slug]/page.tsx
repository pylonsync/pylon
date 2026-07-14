import React, { Suspense } from "react";
import {
  Link,
  useRouteData,
  type GenerateMetadata,
  type Metadata,
  type PageProps,
  type ServerData,
} from "@pylonsync/react";
import { ArrowLeft, Star } from "lucide-react";
import { Card, CardContent } from "@pylonsync/example-ui/card";
import { Badge } from "@pylonsync/example-ui/badge";
import { Separator } from "@pylonsync/example-ui/separator";
import type { Product } from "../../../client/lib/types";
import { gradient, initials } from "../../../client/lib/util";
import { AddToCart } from "../../../client/AddToCart";

// Real SSR detail route reached at /p/<slug>. The product is fetched on the
// SERVER by its slug (serverData.lookup over the by_slug unique index), so the
// HTML ships with real content — shareable URL, SEO, no client round-trip to
// see the product. Only the cart button hydrates as a client island.
// Resolve from the slug, falling back to a raw id lookup so /p/<id> links
// (e.g. from the cart, which stores productId) keep working.
async function resolveProduct(
  serverData: ServerData,
  key: string,
): Promise<Product | null> {
  return (
    (await serverData.lookup<Product>("Product", "slug", key)) ??
    (await serverData.get<Product>("Product", key))
  );
}

// Product pages are anonymous + public (the cart button hydrates as its own
// client island with its own auth), so the SSR output is identical for every
// visitor — a textbook ISR candidate. Opting in with `revalidate` makes Pylon
//   • serve the render from the on-disk ISR + in-memory LRU cache (no per-hit
//     DB lookup or React render),
//   • emit `Cache-Control: public, max-age=…, stale-while-revalidate=…` so the
//     browser HTTP cache + CloudFlare hold it, and
//   • make <Link>'s viewport/hover prefetch reusable — by the time you click a
//     card the HTML is already cached, so navigation is instant with no backend
//     round-trip flash.
// Without this the route is `dynamic`: re-rendered every request with no
// Cache-Control, so even prefetched HTML can't be reused.
export const revalidate = 300; // 5 min; stale-while-revalidate refreshes in bg

export const generateMetadata: GenerateMetadata = async ({
  params,
  serverData,
}): Promise<Metadata> => {
  const p = await resolveProduct(serverData, params.slug);
  if (!p) return { title: "Product not found · Pylon Store" };
  return {
    title: `${p.name} — $${p.price.toFixed(2)} · Pylon Store`,
    description: `${p.brand} ${p.name}. ${String(p.description).slice(0, 150)}`,
  };
};

export default function Page({ params, serverData }: PageProps<{ slug: string }>) {
  // Keyed by slug so each product navigation mounts a fresh <Detail> in the
  // right mode (optimistic vs hard-load); a same-product optimistic→real swap
  // then updates that one instance in place — no remount flash.
  return (
    <main className="mx-auto flex max-w-5xl flex-col gap-6 px-4 py-6 md:px-6">
      <Link
        href="/"
        className="inline-flex items-center gap-1 self-start text-sm text-muted-foreground transition-colors hover:text-foreground"
      >
        <ArrowLeft className="size-4" />
        Back to catalog
      </Link>

      <Suspense fallback={<DetailSkeleton />}>
        <Detail key={params.slug} serverData={serverData} slug={params.slug} />
      </Suspense>
    </main>
  );
}

function Detail({
  serverData,
  slug,
}: {
  serverData: ServerData;
  slug: string;
}) {
  // useRouteData paints the <Link seed> instantly as content, then upgrades to
  // the authoritative row IN PLACE (no Suspense fallback → no flash). On a hard
  // load / direct URL there's no seed, so it suspends and the server streams the
  // real product (the <Suspense> above catches it). See its docs in @pylonsync/react.
  const product = useRouteData<Product | null>(
    () => resolveProduct(serverData, slug),
    [serverData, slug],
  );
  if (!product) {
    return (
      <Card className="p-8 text-center text-sm text-muted-foreground">
        Product not found.{" "}
        <Link href="/" className="underline">
          Browse the catalog
        </Link>
        .
      </Card>
    );
  }
  return <ProductView product={product} />;
}

// The product panel. Rendered both as the optimistic seed paint (from the
// clicked card) and, once the SSR fetch lands, from the authoritative row —
// same markup either way, so the swap is seamless.
function ProductView({ product }: { product: Product }) {
  return (
    <div className="grid gap-8 md:grid-cols-[1.1fr_1fr]">
      <div
        className="flex aspect-square items-center justify-center rounded-xl text-6xl font-bold text-white/90"
        style={{ background: gradient(product.name, product.brand) }}
      >
        {initials(product.name)}
      </div>

      <Card className="flex flex-col">
        <CardContent className="flex flex-col gap-4 p-6">
          <div className="flex flex-wrap items-center gap-2 text-xs font-semibold uppercase tracking-wider text-muted-foreground">
            {product.brand}
            {product.featured ? (
              <Badge variant="secondary" className="text-[10px]">
                Featured
              </Badge>
            ) : null}
            {(product.tags ?? "")
              .split(",")
              .filter(Boolean)
              .map((t) => (
                <Badge key={t} variant="outline" className="text-[10px]">
                  {t}
                </Badge>
              ))}
          </div>
          <h1 className="text-2xl font-semibold leading-tight">
            {product.name}
          </h1>
          <div className="flex items-center gap-3 text-sm">
            <span className="flex items-center gap-1 text-muted-foreground">
              <Star className="size-4 fill-current text-amber-400" />
              {product.rating.toFixed(1)}
            </span>
            <Separator orientation="vertical" className="h-4" />
            <Badge variant={product.stock > 0 ? "secondary" : "destructive"}>
              {product.stock > 0 ? `${product.stock} in stock` : "Out of stock"}
            </Badge>
          </div>
          <div className="text-3xl font-bold">${product.price.toFixed(2)}</div>
          <p className="text-sm leading-relaxed text-muted-foreground">
            {product.description}
          </p>

          <Separator />

          <dl className="grid grid-cols-3 gap-3 text-sm">
            <Attr label="Category" value={product.category} />
            <Attr label="Color" value={product.color} />
            <Attr label="SKU" value={(product.slug ?? product.id).slice(-8)} />
          </dl>

          {product.sizes ? (
            <div className="flex flex-col gap-1.5">
              <span className="text-[11px] font-semibold uppercase tracking-wider text-muted-foreground">
                Size
              </span>
              <div className="flex flex-wrap gap-2">
                {product.sizes
                  .split(",")
                  .filter(Boolean)
                  .map((s) => (
                    <span
                      key={s}
                      className="rounded-md border px-3 py-1.5 text-sm text-foreground"
                    >
                      {s}
                    </span>
                  ))}
              </div>
            </div>
          ) : null}

          <AddToCart product={product} />
        </CardContent>
      </Card>
    </div>
  );
}

function Attr({ label, value }: { label: string; value: string }) {
  return (
    <div className="flex flex-col gap-0.5">
      <dt className="text-[11px] font-semibold uppercase tracking-wider text-muted-foreground">
        {label}
      </dt>
      <dd className="text-sm capitalize text-foreground">{value}</dd>
    </div>
  );
}

// Shown only on a hard load whose data isn't ready and there's no seed (the
// common navigation case paints ProductView from the card seed instead). Renders
// just the grid — Page owns the <main> + back link.
function DetailSkeleton() {
  return (
    <div className="grid gap-8 md:grid-cols-[1.1fr_1fr]">
      <div className="aspect-square animate-pulse rounded-xl bg-muted" />
      <div className="space-y-3 p-6">
        <div className="h-3 w-1/4 animate-pulse rounded bg-muted" />
        <div className="h-6 w-3/4 animate-pulse rounded bg-muted" />
        <div className="h-4 w-1/3 animate-pulse rounded bg-muted" />
        <div className="h-9 w-1/3 animate-pulse rounded bg-muted" />
        <div className="h-20 animate-pulse rounded bg-muted" />
      </div>
    </div>
  );
}
