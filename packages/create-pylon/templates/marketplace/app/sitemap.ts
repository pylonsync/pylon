import type { Sitemap } from "@pylonsync/react";

// app/sitemap.ts → served at /sitemap.xml. Point SITE_URL at your domain in
// production. We list the static surfaces here; for a real catalog, map over
// active listings (read them via the entity API) and add a `/listing/<slug>`
// entry per row.
const SITE = process.env.SITE_URL ?? "http://localhost:4321";

export default async function sitemap(): Promise<Sitemap> {
  return [
    { url: `${SITE}/`, changeFrequency: "hourly", priority: 1 },
    { url: `${SITE}/sell`, changeFrequency: "monthly", priority: 0.5 },
  ];
}
