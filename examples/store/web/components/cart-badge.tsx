"use client";

import { Badge } from "@pylonsync/example-ui/badge";
import { useCart } from "./cart-store";

export function CartBadge() {
  const { count } = useCart();
  if (count === 0) return null;
  return (
    <Badge
      variant="default"
      className="absolute -right-1.5 -top-1.5 h-5 min-w-5 justify-center rounded-full px-1 text-[10px]"
    >
      {count}
    </Badge>
  );
}
