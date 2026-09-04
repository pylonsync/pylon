import type { Metadata } from "@pylonsync/react";
import { ComparisonIndexView } from "@pylon-site/ui/components/comparison-index-view";
import { ComparisonHidden } from "@pylon-site/ui/components/comparison-hidden";
import { COMPARISONS_ENABLED } from "@pylon-site/ui/lib/comparison-content";

// Two cached shells keyed on the signed-in bit (see app/page.tsx for the
// auth-bucketed rationale). Cacheable only once COMPARISONS_ENABLED — the
// hidden variant sets a 404, and non-200 renders are never cached.
export const cache = "auth-bucketed";
export const revalidate = 3600; // 1 hour

export const metadata: Metadata = COMPARISONS_ENABLED
	? {
			title: "Pylon vs. Supabase, Convex, Firebase, and InstantDB",
			description:
				"Honest, side-by-side comparisons of Pylon against the realtime backends it overlaps with — Supabase, Convex, Firebase, and InstantDB. Architecture, trade-offs, and migration.",
			canonical: "/vs",
			openGraph: {
				title: "How Pylon compares — Supabase, Convex, Firebase, InstantDB",
				description:
					"Side-by-side comparisons of Pylon against the realtime backends it overlaps with.",
				url: "https://www.pylonsync.com/vs",
				type: "website",
			},
			twitter: {
				card: "summary_large_image",
				title: "How Pylon compares",
				description:
					"Honest comparisons against Supabase, Convex, Firebase, and InstantDB.",
			},
		}
	: { title: "Pylon", robots: "noindex" };

export default function VsIndexPage({
	session,
	response,
}: {
	session?: { exists: boolean };
	response?: { setStatus?: (code: number) => void };
}) {
	const signedIn = Boolean(session?.exists);
	if (!COMPARISONS_ENABLED) {
		response?.setStatus?.(404);
		return <ComparisonHidden signedIn={signedIn} />;
	}
	return <ComparisonIndexView signedIn={signedIn} />;
}
