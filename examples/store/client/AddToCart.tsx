"use client";
/**
 * Client island for the SSR product-detail page. The page server-renders the
 * product; this hydrates just the cart interaction (writes go through the
 * sync-backed cart, so the header count updates live across the app).
 */
import React, { useState } from "react";
import { Check } from "lucide-react";
import { Button } from "@pylonsync/example-ui/button";
import type { Product } from "./lib/types";
import { useCart } from "./lib/cart";

export function AddToCart({ product }: { product: Product }) {
  const cart = useCart();
  const [added, setAdded] = useState(false);

  const handleAdd = () => {
    cart.add(product);
    setAdded(true);
    setTimeout(() => setAdded(false), 1200);
  };

  return (
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
  );
}
