import type { Sitemap } from "@pylonsync/react";

// app/sitemap.ts → served at /sitemap.xml. The default export can be async, so
// it can enumerate dynamic pages from your database. Point SITE_URL at your
// domain in production.
const SITE = process.env.SITE_URL ?? "http://localhost:3000";

export default async function sitemap(): Promise<Sitemap> {
  const staticRoutes: Sitemap = [
    { url: `${SITE}/`, changeFrequency: "weekly", priority: 1 },
    { url: `${SITE}/notes`, changeFrequency: "daily", priority: 0.8 },
    { url: `${SITE}/counter`, changeFrequency: "monthly", priority: 0.5 },
  ];

  // The export is async, so you can enumerate dynamic pages from a DB read:
  //
  //   const notes = await fetchNotes();
  //   const noteRoutes: Sitemap = notes.map((n) => ({
  //     url: `${SITE}/notes/${n.id}`,
  //     lastModified: n.updatedAt,
  //   }));
  //   return [...staticRoutes, ...noteRoutes];

  return staticRoutes;
}
