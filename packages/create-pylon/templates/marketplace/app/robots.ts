import type { Robots } from "@pylonsync/react";

// app/robots.ts → served at /robots.txt. Point SITE_URL at your domain in prod.
const SITE = process.env.SITE_URL ?? "http://localhost:4321";

export default function robots(): Robots {
  return {
    // Browse + listings are indexable; keep the personal inbox and API out.
    rules: { userAgent: "*", allow: "/", disallow: ["/me", "/api/"] },
    sitemap: `${SITE}/sitemap.xml`,
  };
}
