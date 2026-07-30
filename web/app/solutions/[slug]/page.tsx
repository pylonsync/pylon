import type { Metadata } from "@pylonsync/react";
import { SolutionView } from "@pylon-cloud/ui/components/solution-view";
import { getSolution } from "@pylon-cloud/ui/lib/solutions-content";

// Two cached shells per slug keyed on the signed-in bit — see app/page.tsx
// for the full auth-bucketed rationale.
export const cache = "auth-bucketed";
export const revalidate = 3600; // 1 hour

interface SolutionPageProps {
	params: { slug: string };
	session?: { exists: boolean };
	response?: { setStatus?: (code: number) => void };
}

export function generateMetadata({ params }: SolutionPageProps): Metadata {
	const s = getSolution(params.slug);
	if (!s) return { title: "Solutions — Pylon" };
	const url = `https://pylonsync.com/solutions/${s.slug}`;
	return {
		title: s.metaTitle,
		description: s.metaDescription,
		canonical: `/solutions/${s.slug}`,
		openGraph: {
			title: s.metaTitle,
			description: s.metaDescription,
			url,
			type: "article",
		},
		twitter: {
			card: "summary_large_image",
			title: s.metaTitle,
			description: s.metaDescription,
		},
	};
}

export default function SolutionSlugPage({
	params,
	session,
	response,
}: SolutionPageProps) {
	if (!getSolution(params.slug)) response?.setStatus?.(404);
	return (
		<SolutionView slug={params.slug} signedIn={Boolean(session?.exists)} />
	);
}
