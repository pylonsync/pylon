import type { Metadata } from "@pylonsync/react";
import { PricingView } from "@pylon-cloud/ui/components/pricing-view";

// Two cached shells keyed on the signed-in bit — see app/page.tsx for the
// full auth-bucketed rationale. Only ever read `session.exists` here; full
// `props.auth` would make the render uncacheable.
export const cache = "auth-bucketed";
export const revalidate = 3600; // 1 hour

export const metadata: Metadata = {
	title: "Pylon pricing: free to self-host, $25 on Cloud",
	description:
		"The Pylon framework is open source and free to self-host. Smallware is free for hobby projects and $25 per org per month for production, with no per-seat fees.",
	canonical: "/pricing",
	openGraph: {
		title: "Pylon pricing: free to self-host, $25 on Cloud",
		description:
			"Open-source framework, free to self-host. Cloud is free for hobby, $25 per org for production.",
		url: "https://pylonsync.com/pricing",
		type: "website",
	},
	twitter: {
		card: "summary_large_image",
		title: "Pylon pricing",
		description:
			"Free to self-host. Cloud free for hobby, $25 per org for production.",
	},
};

export default function PricingPage({
	session,
}: {
	session?: { exists: boolean };
}) {
	return <PricingView signedIn={Boolean(session?.exists)} />;
}
