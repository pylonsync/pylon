import type { Robots } from "@pylonsync/react";

// app/robots.ts → served at /robots.txt. The default export may also be async.
const BASE = process.env.SITE_URL ?? "http://localhost:4321";

export default function robots(): Robots {
  return {
    rules: { userAgent: "*", allow: "/", disallow: ["/api/"] },
    sitemap: `${BASE}/sitemap.xml`,
  };
}
