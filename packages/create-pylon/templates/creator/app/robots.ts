import type { Robots } from "@pylonsync/react";

// app/robots.ts → served at /robots.txt. Point SITE_URL at your domain in prod.
const SITE = process.env.SITE_URL ?? "http://localhost:4321";

export default function robots(): Robots {
  return {
    // Keep the owner dashboard and the API out of the index.
    rules: { userAgent: "*", allow: "/", disallow: ["/dashboard", "/login", "/api/"] },
    sitemap: `${SITE}/sitemap.xml`,
  };
}
