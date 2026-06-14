import type { Robots } from "@pylonsync/react";

// app/robots.ts → served at /robots.txt. The default export may also be async.
const SITE = process.env.SITE_URL ?? "http://localhost:4321";

export default function robots(): Robots {
  return {
    // Keep the authenticated app and the API out of the index.
    rules: { userAgent: "*", allow: "/", disallow: ["/dashboard", "/api/"] },
    sitemap: `${SITE}/sitemap.xml`,
  };
}
