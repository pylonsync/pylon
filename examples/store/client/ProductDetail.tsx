/**
 * Product detail — single-product page reached via #/p/<id>.
 *
 * `db.useQueryOne` keeps the page in lockstep with the catalog: if a
 * background mutation changes the price or stock, the detail page
 * reflects it without a refresh.
 */
import { useState } from "react";
import { db } from "@pylonsync/react";
import { ArrowLeft, Check, Star } from "lucide-react";
import { Button } from "@/components/ui/button";
import { Card, CardContent } from "@/components/ui/card";
import { Badge } from "@/components/ui/badge";
import { Separator } from "@/components/ui/separator";
import type { Product } from "./lib/types";
import { gradient, initials, navigate } from "./lib/util";

export function ProductDetail({
  id,
  onAddToCart,
}: {
  id: string;
  onAddToCart: (p: Product) => void;
}) {
  const { data: product, loading, error } = db.useQueryOne<Product>("Product", id);
  const [added, setAdded] = useState(false);

  const handleAdd = () => {
    if (!product) return;
    onAddToCart(product);
    setAdded(true);
    setTimeout(() => setAdded(false), 1200);
  };

  return (
    <main className="mx-auto flex max-w-5xl flex-col gap-6 px-4 py-6 md:px-6">
      <Button
        variant="ghost"
        size="sm"
        className="self-start"
        onClick={() => navigate("#/")}
      >
        <ArrowLeft className="size-4" />
        Back to catalog
      </Button>

      {loading ? (
        <DetailSkeleton />
      ) : error || !product ? (
        <Card className="p-8 text-center text-sm text-muted-foreground">
          {error
            ? `Couldn't load product: ${error.message}`
            : "Product not found."}
        </Card>
      ) : (
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

              <Button
                size="lg"
                className="mt-2"
                onClick={handleAdd}
                disabled={product.stock === 0}
              >
                {added ? (
                  <>
                    <Check className="size-4" />
                    Added to cart
                  </>
                ) : (
                  "Add to cart"
                )}
              </Button>
            </CardContent>
          </Card>
        </div>
      )}
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
