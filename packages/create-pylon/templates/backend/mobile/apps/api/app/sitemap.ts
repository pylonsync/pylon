import type { Sitemap } from "@pylonsync/react";
import { site } from "@/lib/site";

// app/sitemap.ts → served at /sitemap.xml. This is a server-only route
// module, never bundled for the browser, so reading process.env here is
// safe (page components cannot).
const SITE = site.url || process.env.SITE_URL || "http://localhost:4321";

export default function sitemap(): Sitemap {
  return [
    { url: `${SITE}/`, changeFrequency: "weekly", priority: 1 },
    { url: `${SITE}/support`, changeFrequency: "monthly", priority: 0.5 },
    { url: `${SITE}/privacy`, changeFrequency: "yearly", priority: 0.3 },
    { url: `${SITE}/terms`, changeFrequency: "yearly", priority: 0.3 },
  ];
}
