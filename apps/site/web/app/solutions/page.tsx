import type { Metadata } from "@pylonsync/react";
import { SolutionsIndexView } from "@pylon-cloud/ui/components/solutions-index-view";

// Two cached shells keyed on the signed-in bit — see app/page.tsx for the
// full auth-bucketed rationale.
export const cache = "auth-bucketed";
export const revalidate = 3600; // 1 hour
export const metadata: Metadata = {
	title: "Pylon solutions: local-first, collaboration, AI, and mobile",
	description:
		"How teams use Pylon: local-first apps, realtime collaboration, AI apps & agents, and native mobile with the Swift SDK. The same primitives, very different products.",
	canonical: "/solutions",
	openGraph: {
		title: "Pylon Solutions: local-first, collaboration, AI, mobile",
		description:
			"How teams put Pylon's primitives to work across very different products.",
		url: "https://www.pylonsync.com/solutions",
		type: "website",
	},
	twitter: {
		card: "summary_large_image",
		title: "Pylon Solutions",
		description:
			"Local-first apps, realtime collaboration, AI apps & agents, and native mobile.",
	},
};

export default function SolutionsIndexPage({
	session,
}: {
	session?: { exists: boolean };
}) {
	return <SolutionsIndexView signedIn={Boolean(session?.exists)} />;
}
