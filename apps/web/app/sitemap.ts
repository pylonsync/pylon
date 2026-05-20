import type { MetadataRoute } from "next";
import { COMPARISONS } from "@/data/comparisons";

// /sitemap.xml — every indexable page on the marketing site, with
// real lastModified dates so search engines know what's actually
// fresh vs. stale.
//
// Docs site lives on docs.pylonsync.com (Mintlify, separate
// subdomain) and has its own sitemap, so it's deliberately not
// listed here. Cross-host sitemap references confuse search
// console more than they help.
export default function sitemap(): MetadataRoute.Sitemap {
	const now = new Date();
	return [
		{
			url: "https://pylonsync.com/",
			lastModified: now,
			changeFrequency: "weekly",
			priority: 1.0,
		},
		{
			url: "https://pylonsync.com/skill",
			lastModified: now,
			changeFrequency: "weekly",
			priority: 0.8,
		},
		{
			url: "https://pylonsync.com/vs",
			lastModified: now,
			changeFrequency: "monthly",
			priority: 0.7,
		},
		// One entry per competitor — high-intent comparison surfaces
		// targeting "<competitor> alternative" queries.
		...COMPARISONS.map((c) => ({
			url: `https://pylonsync.com/vs/${c.slug}`,
			lastModified: now,
			changeFrequency: "monthly" as const,
			priority: 0.8,
		})),
	];
}
