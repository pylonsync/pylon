import type { Robots } from "@pylonsync/react";

// app/robots.ts → served at /robots.txt. Point SITE_URL at your domain in
// production. The default export may also be async.
const SITE = process.env.SITE_URL ?? "http://localhost:4321";

export default function robots(): Robots {
  return {
    // Keep the signed-in app and the API out of the index. `llms.txt` and the
    // sitemap stay crawlable — they are what a crawler or agent reads first.
    rules: { userAgent: "*", allow: "/", disallow: ["/api/", "/login"] },
    sitemap: `${SITE}/sitemap.xml`,
  };
}
