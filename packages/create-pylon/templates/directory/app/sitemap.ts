import type { Sitemap } from "@pylonsync/react";

// app/sitemap.ts → served at /sitemap.xml. Point SITE_URL at your domain in
// production. This lists the public homepage; add more URLs as the site grows.
const SITE = process.env.SITE_URL ?? "http://localhost:4321";

export default async function sitemap(): Promise<Sitemap> {
  return [{ url: `${SITE}/`, changeFrequency: "weekly", priority: 1 }];
}
