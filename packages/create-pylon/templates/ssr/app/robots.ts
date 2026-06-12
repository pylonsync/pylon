import type { Robots } from "@pylonsync/react";

// app/robots.ts → served at /robots.txt. The default export may also be async.
const SITE = process.env.SITE_URL ?? "http://localhost:3000";

export default function robots(): Robots {
  return {
    rules: { userAgent: "*", allow: "/" },
    sitemap: `${SITE}/sitemap.xml`,
  };
}
