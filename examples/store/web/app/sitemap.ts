import type { MetadataRoute } from "next";
import { listProductIds } from "@/lib/pylon-server";

const SITE = process.env.NEXT_PUBLIC_SITE_URL ?? "http://localhost:5179";

export default async function sitemap(): Promise<MetadataRoute.Sitemap> {
  const ids = await listProductIds(5000).catch(() => [] as string[]);
  const now = new Date();
  const productEntries = ids.map((id) => ({
    url: `${SITE}/p/${encodeURIComponent(id)}`,
    lastModified: now,
    changeFrequency: "weekly" as const,
    priority: 0.6,
  }));
  return [
    {
      url: SITE,
      lastModified: now,
      changeFrequency: "daily",
      priority: 1.0,
    },
    ...productEntries,
  ];
}
