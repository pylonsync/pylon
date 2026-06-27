import type { Sitemap } from "@pylonsync/react";

// app/sitemap.ts → served at /sitemap.xml. The default export can be async, so
// it can enumerate dynamic pages from your database. Point SITE_URL at your
// domain in production.
const SITE = process.env.SITE_URL ?? "http://localhost:4321";

export default async function sitemap(): Promise<Sitemap> {
  // Only public pages belong here — /dashboard is private (and noindex), so
  // it's intentionally left out.
  const staticRoutes: Sitemap = [
    { url: `${SITE}/`, changeFrequency: "weekly", priority: 1 },
    { url: `${SITE}/login`, changeFrequency: "yearly", priority: 0.3 },
    { url: `${SITE}/signup`, changeFrequency: "yearly", priority: 0.5 },
  ];

  // Example — enumerate dynamic pages from a DB read:
  //
  //   const posts = await fetchPublishedPosts();
  //   const postRoutes: Sitemap = posts.map((p) => ({
  //     url: `${SITE}/blog/${p.slug}`,
  //     lastModified: p.updatedAt,
  //   }));
  //   return [...staticRoutes, ...postRoutes];

  return staticRoutes;
}
