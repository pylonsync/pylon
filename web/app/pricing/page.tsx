import type { Metadata } from "@pylonsync/react";
import type { PageProps } from "@pylonsync/react";

// The framework has no price — it's open source and free to self-host. What
// used to live here was Smallware's plan table, which put the hosting
// product's pricing on the framework's domain and read as though Pylon
// itself cost $25.
//
// The URL isn't just deleted: it shipped in this site's sitemap and is
// indexed, and "pylon pricing" is a real query. A 301 sends it to the page
// that actually answers it. usesmallware.com is named in PYLON_TRUSTED_HOSTS
// (fly.toml) — the runtime's open-redirect guard refuses an off-site
// Location otherwise, and it should.
const PRICING = "https://www.usesmallware.com/pricing";

export const metadata: Metadata = { title: "Pricing", robots: "noindex" };

// `response.redirect`, not the `redirect()` from @pylonsync/react — that one
// is client-only and a silent no-op during a server render.
export default function PricingPage({
	response,
}: {
	response: PageProps["response"];
}) {
	// 301, not the default 307: the move is permanent, and search engines
	// should transfer the URL rather than keep re-crawling this one.
	response.redirect(PRICING, 301);
	return null;
}
