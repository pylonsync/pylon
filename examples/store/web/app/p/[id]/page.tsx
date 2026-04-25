/**
 * Product detail page — server-rendered for SEO.
 *
 * Fully crawlable, indexable, OG-tagged. JSON-LD `Product` schema is
 * emitted inline so search engines pick up structured data (price,
 * availability, rating).
 */
import type { Metadata } from "next";
import { notFound } from "next/navigation";
import Link from "next/link";
import { ArrowLeft, Star } from "lucide-react";
import { Button } from "@pylonsync/example-ui/button";
import { Card, CardContent } from "@pylonsync/example-ui/card";
import { Badge } from "@pylonsync/example-ui/badge";
import { Separator } from "@pylonsync/example-ui/separator";
import { getProduct } from "@/lib/pylon-server";
import { gradient, initials } from "@/lib/util";
import { AddToCartButton } from "@/components/add-to-cart-button";

const SITE = process.env.NEXT_PUBLIC_SITE_URL ?? "http://localhost:5179";

export async function generateMetadata({
  params,
}: {
  params: Promise<{ id: string }>;
}): Promise<Metadata> {
  const { id } = await params;
  const product = await getProduct(decodeURIComponent(id));
  if (!product) {
    return { title: "Product not found" };
  }
  const url = `${SITE}/p/${encodeURIComponent(product.id)}`;
  return {
    title: product.name,
    description:
      product.description.length > 155
        ? product.description.slice(0, 152) + "…"
        : product.description,
    alternates: { canonical: url },
    openGraph: {
      type: "website",
      title: product.name,
      description: product.description,
      url,
      images: [
        {
          // No real image — use a deterministic SVG placeholder route
          // for OG so social cards render something visual. Falls back
          // gracefully if that route isn't deployed.
          url: `${SITE}/og/${encodeURIComponent(product.id)}`,
          width: 1200,
          height: 630,
          alt: product.name,
        },
      ],
    },
    twitter: {
      card: "summary_large_image",
      title: product.name,
      description: product.description,
    },
  };
}

export default async function ProductDetailPage({
  params,
}: {
  params: Promise<{ id: string }>;
}) {
  const { id } = await params;
  const product = await getProduct(decodeURIComponent(id));
  if (!product) notFound();

  // JSON-LD product schema. Crawlers parse this and surface rich
  // results (price, rating, availability) in the SERP.
  const jsonLd = {
    "@context": "https://schema.org",
    "@type": "Product",
    name: product.name,
    description: product.description,
    sku: product.id,
    brand: { "@type": "Brand", name: product.brand },
    category: product.category,
    color: product.color,
    aggregateRating: {
      "@type": "AggregateRating",
      ratingValue: product.rating,
      reviewCount: 1,
    },
    offers: {
      "@type": "Offer",
      url: `${SITE}/p/${encodeURIComponent(product.id)}`,
      priceCurrency: "USD",
      price: product.price.toFixed(2),
      availability:
        product.stock > 0
          ? "https://schema.org/InStock"
          : "https://schema.org/OutOfStock",
    },
  };

  return (
    <main className="mx-auto flex max-w-5xl flex-col gap-6 px-4 py-6 md:px-6">
      <script
        type="application/ld+json"
        // eslint-disable-next-line react/no-danger
        dangerouslySetInnerHTML={{ __html: JSON.stringify(jsonLd) }}
      />

      <Button variant="ghost" size="sm" className="self-start" asChild>
        <Link href="/">
          <ArrowLeft className="size-4" />
          Back to catalog
        </Link>
      </Button>

      <div className="grid gap-8 md:grid-cols-[1.1fr_1fr]">
        <div
          className="flex aspect-square items-center justify-center rounded-xl text-6xl font-bold text-white/90"
          style={{ background: gradient(product.name, product.brand) }}
        >
          {initials(product.name)}
        </div>

        <Card className="flex flex-col">
          <CardContent className="flex flex-col gap-4 p-6">
            <div className="text-xs font-semibold uppercase tracking-wider text-muted-foreground">
              {product.brand}
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
                {product.stock > 0
                  ? `${product.stock} in stock`
                  : "Out of stock"}
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
              <Attr label="SKU" value={product.id.slice(-8)} />
            </dl>

            <AddToCartButton
              product={product}
              size="lg"
              className="mt-2 w-full"
              variant="default"
              showLabel
            />
          </CardContent>
        </Card>
      </div>
    </main>
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
