"use client";

import { useState } from "react";
import { Check } from "lucide-react";
import { Button } from "@pylonsync/example-ui/button";
import { cn } from "@pylonsync/example-ui/utils";
import { useCart } from "./cart-store";
import type { Product } from "@/lib/types";

type Variant = "default" | "outline" | "ghost" | "secondary";

export function AddToCartButton({
  product,
  className,
  size = "sm",
  variant = "outline",
  showLabel,
}: {
  product: Product;
  className?: string;
  size?: "default" | "sm" | "lg" | "xs";
  variant?: Variant;
  showLabel?: boolean;
}) {
  const { add } = useCart();
  const [added, setAdded] = useState(false);

  const handle = () => {
    add(product);
    setAdded(true);
    setTimeout(() => setAdded(false), 1200);
  };

  return (
    <Button
      type="button"
      size={size}
      variant={variant}
      onClick={(e) => {
        e.preventDefault();
        e.stopPropagation();
        handle();
      }}
      className={cn(className)}
      disabled={product.stock === 0}
    >
      {added ? (
        <>
          <Check className="size-4" />
          {showLabel ? "Added to cart" : "Added"}
        </>
      ) : (
        "Add to cart"
      )}
    </Button>
  );
}
