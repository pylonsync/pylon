import type { Robots } from "@pylonsync/react";

// A private dashboard: nothing here should be indexed.
export default function robots(): Robots {
  return { rules: { userAgent: "*", disallow: "/" } };
}
