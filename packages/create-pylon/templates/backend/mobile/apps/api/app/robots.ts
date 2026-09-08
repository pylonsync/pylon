import type { Robots } from "@pylonsync/react";
import { site } from "@/lib/site";

// app/robots.ts → served at /robots.txt.
const SITE = site.url || process.env.SITE_URL || "http://localhost:4321";

export default function robots(): Robots {
  return {
    // The API is not for crawlers; the marketing and legal pages are.
    rules: { userAgent: "*", allow: "/", disallow: ["/api/"] },
    sitemap: `${SITE}/sitemap.xml`,
  };
}
