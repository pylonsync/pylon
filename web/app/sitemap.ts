import { PRODUCT_SLUGS } from "@pylon-cloud/ui/lib/product-content";
import { SOLUTION_SLUGS } from "@pylon-cloud/ui/lib/solutions-content";
import { COMPARISONS_ENABLED, comparisonSlugs } from "@pylon-cloud/ui/lib/comparison-content";

type SitemapEntry = {
	url: string;
	lastModified?: string;
	changeFrequency?:
		| "always"
		| "hourly"
		| "daily"
		| "weekly"
		| "monthly"
		| "yearly"
		| "never";
	priority?: number;
};
type Sitemap = SitemapEntry[];

// app/sitemap.ts → served at /sitemap.xml. Enumerated from the IA content maps,
// so adding a /product, /solutions, or /vs page is picked up automatically — no
// hand-maintained XML. Canonical host is pylonsync.com.
const SITE = "https://pylonsync.com";

export default function sitemap(): Sitemap {
  const entries: Sitemap = [
    { url: `${SITE}/`, changeFrequency: "weekly", priority: 1 },
    { url: `${SITE}/pricing`, changeFrequency: "monthly", priority: 0.9 },
    { url: `${SITE}/product`, changeFrequency: "weekly", priority: 0.9 },
    { url: `${SITE}/solutions`, changeFrequency: "monthly", priority: 0.8 },
    ...(COMPARISONS_ENABLED
      ? [
          {
            url: `${SITE}/vs`,
            changeFrequency: "monthly" as const,
            priority: 0.7,
          },
        ]
      : []),
    {
      url: `${SITE}/developers/examples`,
      changeFrequency: "monthly",
      priority: 0.6,
    },
    { url: `${SITE}/skill`, changeFrequency: "monthly", priority: 0.7 },
    { url: `${SITE}/smallware`, changeFrequency: "monthly", priority: 0.9 },
  ];
  for (const slug of PRODUCT_SLUGS) {
    entries.push({
      url: `${SITE}/product/${slug}`,
      changeFrequency: "monthly",
      priority: 0.8,
    });
  }
  for (const slug of SOLUTION_SLUGS) {
    entries.push({
      url: `${SITE}/solutions/${slug}`,
      changeFrequency: "monthly",
      priority: 0.7,
    });
  }
  if (COMPARISONS_ENABLED) {
    for (const slug of comparisonSlugs()) {
      entries.push({
        url: `${SITE}/vs/${slug}`,
        changeFrequency: "monthly",
        priority: 0.7,
      });
    }
  }
  return entries;
}
