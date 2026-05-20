import type { MetadataRoute } from "next";

// /robots.txt — explicit allow-all + sitemap pointer.
//
// Before this file existed the route 404'd via the Next.js
// not-found page, so Googlebot got "no robots.txt directives" plus
// a 200-with-noindex-meta on the same URL, which sometimes confuses
// crawl budget allocation on young domains.
//
// Sitemap reference is the canonical-host form (pylonsync.com bare).
// Make sure Vercel's domain config has bare as canonical and www
// redirects to bare — otherwise this URL 307s and search engines
// follow the redirect, but cleaner to point at the live URL.
export default function robots(): MetadataRoute.Robots {
	return {
		rules: [{ userAgent: "*", allow: "/" }],
		sitemap: "https://pylonsync.com/sitemap.xml",
		host: "https://pylonsync.com",
	};
}
