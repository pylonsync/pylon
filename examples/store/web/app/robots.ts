import type { MetadataRoute } from "next";

const SITE = process.env.NEXT_PUBLIC_SITE_URL ?? "http://localhost:5179";

export default function robots(): MetadataRoute.Robots {
  return {
    rules: [
      {
        userAgent: "*",
        allow: ["/", "/p/"],
        disallow: ["/account", "/checkout", "/orders/"],
      },
    ],
    sitemap: `${SITE}/sitemap.xml`,
  };
}
