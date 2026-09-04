import type { Metadata } from "@pylonsync/react";
import { ComparisonView } from "@pylon-cloud/ui/components/comparison-view";
import { ComparisonHidden } from "@pylon-cloud/ui/components/comparison-hidden";
import { COMPARISONS_ENABLED, getComparison } from "@pylon-cloud/ui/lib/comparison-content";

// Two cached shells per slug keyed on the signed-in bit (see app/page.tsx
// for the auth-bucketed rationale). Cacheable only once COMPARISONS_ENABLED —
// the hidden variant sets a 404, and non-200 renders are never cached.
export const cache = "auth-bucketed";
export const revalidate = 3600; // 1 hour

interface VsPageProps {
	params: { slug: string };
	session?: { exists: boolean };
	response?: { setStatus?: (code: number) => void };
}

export function generateMetadata({ params }: VsPageProps): Metadata {
	if (!COMPARISONS_ENABLED) return { title: "Pylon", robots: "noindex" };
	const c = getComparison(params.slug);
	if (!c) return { title: "Compare — Pylon" };
	const title = `${c.keyword}: Pylon — full-stack framework for coding agents`;
	const url = `https://www.pylonsync.com/vs/${c.slug}`;
	return {
		title,
		description: c.metaDescription,
		canonical: `/vs/${c.slug}`,
		openGraph: {
			title,
			description: c.metaDescription,
			url,
			type: "article",
		},
		twitter: {
			card: "summary_large_image",
			title,
			description: c.metaDescription,
		},
	};
}

export default function VsSlugPage({ params, session, response }: VsPageProps) {
	const signedIn = Boolean(session?.exists);
	if (!COMPARISONS_ENABLED) {
		response?.setStatus?.(404);
		return <ComparisonHidden signedIn={signedIn} />;
	}
	if (!getComparison(params.slug)) response?.setStatus?.(404);
	return <ComparisonView slug={params.slug} signedIn={signedIn} />;
}
