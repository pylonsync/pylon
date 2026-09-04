import type { Metadata } from "@pylonsync/react";
import { ExamplesView } from "@pylon-site/ui/components/examples-view";

// Two cached shells keyed on the signed-in bit — see app/page.tsx for the
// full auth-bucketed rationale.
export const cache = "auth-bucketed";
export const revalidate = 3600; // 1 hour

export const metadata: Metadata = {
	title: "Pylon examples: starter apps and live demos",
	description:
		"Production-shaped Pylon examples: full-stack starters (SaaS, chat, todo), business websites (waitlist, agency, restaurant, shop, directory), AI apps, and a live marketplace demo. Scaffold one in seconds.",
	canonical: "/developers/examples",
	openGraph: {
		title: "Pylon examples: starter apps and live demos",
		description:
			"Full-stack starters, business websites, AI apps, and a live marketplace demo built from real create-pylon templates.",
		url: "https://www.pylonsync.com/developers/examples",
		type: "website",
	},
	twitter: {
		card: "summary_large_image",
		title: "Pylon examples",
		description:
			"Production-shaped starter apps, business websites, AI apps, and a live marketplace demo.",
	},
};

export default function ExamplesPage({
	session,
}: {
	session?: { exists: boolean };
}) {
	return <ExamplesView signedIn={Boolean(session?.exists)} />;
}
