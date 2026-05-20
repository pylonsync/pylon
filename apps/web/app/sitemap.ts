import type { MetadataRoute } from "next";

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
	];
}
