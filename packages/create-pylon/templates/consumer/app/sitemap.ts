import type { Sitemap } from "@pylonsync/react";

// app/sitemap.ts → served at /sitemap.xml. The default export can be async,
// so it can list post and profile pages from the database once you have
// real ones. Point SITE_URL at your domain in production.
const BASE = process.env.SITE_URL ?? "http://localhost:4321";

export default async function sitemap(): Promise<Sitemap> {
  return [
    { url: `${BASE}/`, changeFrequency: "hourly", priority: 1 },
    { url: `${BASE}/explore`, changeFrequency: "hourly", priority: 0.8 },
  ];
}
