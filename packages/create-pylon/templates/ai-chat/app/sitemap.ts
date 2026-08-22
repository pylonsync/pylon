import type { Sitemap } from "@pylonsync/react";

// app/sitemap.ts → served at /sitemap.xml. The default export can be async, so
// it can enumerate pages from your database. Point SITE_URL at your domain in
// production.
const SITE = process.env.SITE_URL ?? "http://localhost:4321";

export default async function sitemap(): Promise<Sitemap> {
  // Only public pages belong here. The signed-in app is private.
  return [{ url: `${SITE}/`, changeFrequency: "weekly", priority: 1 }];
}
