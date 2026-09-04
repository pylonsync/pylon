import { PRODUCT_SLUGS } from "@pylon-site/ui/lib/product-content";
import { SOLUTION_SLUGS } from "@pylon-site/ui/lib/solutions-content";
import { COMPARISONS_ENABLED, comparisonSlugs } from "@pylon-site/ui/lib/comparison-content";
import { SITE_URL } from "../lib/site";

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
// hand-maintained XML.
//
// Canonical host is www. It used to be the apex here while every page's
// `canonical` said www, so each sitemap URL sent the crawler through a 308 to
// a URL that disagreed with the one it was told was canonical.
const SITE = SITE_URL;

export default function sitemap(): Sitemap {
  const entries: Sitemap = [
    { url: `${SITE}/`, changeFrequency: "weekly", priority: 1 },
    // No /pricing — the framework is free, and Stack0 Cloud's plans live on
    // cloud.stack0.dev. The route still exists here only to 301 there.
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
    { url: `${SITE}/developers`, changeFrequency: "monthly", priority: 0.8 },
    {
      url: `${SITE}/developers/examples`,
      changeFrequency: "monthly",
      priority: 0.6,
    },
    { url: `${SITE}/skill`, changeFrequency: "monthly", priority: 0.7 },
    { url: `${SITE}/about`, changeFrequency: "monthly", priority: 0.6 },
    { url: `${SITE}/contact`, changeFrequency: "monthly", priority: 0.6 },
    { url: `${SITE}/privacy`, changeFrequency: "yearly", priority: 0.3 },
    { url: `${SITE}/terms`, changeFrequency: "yearly", priority: 0.3 },
    // `/smallware` used to be listed here and answers 404 — the page moved to
    // cloud.stack0.dev in the brand split and the entry outlived it. A dead URL
    // in a sitemap is a crawler telling itself the site is stale.
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
