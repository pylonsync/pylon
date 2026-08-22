import type { Metadata } from "@pylonsync/react";
import { AboutView } from "@pylon-cloud/ui/components/about-view";
import { JsonLd } from "../../lib/agent/json-ld";
import { organizationPageGraph } from "../../lib/agent/jsonld";
import { SITE_URL } from "../../lib/site";

// Two cached shells keyed on the signed-in bit — see app/page.tsx for the
// full auth-bucketed rationale.
export const cache = "auth-bucketed";
export const revalidate = 3600; // 1 hour

const DESCRIPTION =
	"Pylon is an MIT-licensed full-stack framework: typed schema, row-level policies, server functions, live queries, auth, and React server rendering in one binary. Built in Dallas, Texas, and paid for by Smallware, its managed hosting.";

export const metadata: Metadata = {
	title: "About Pylon: the framework, the licence, and who builds it",
	description: DESCRIPTION,
	canonical: `${SITE_URL}/about`,
	openGraph: {
		title: "About Pylon",
		description: DESCRIPTION,
		url: `${SITE_URL}/about`,
		siteName: "Pylon",
		type: "website",
	},
	twitter: {
		card: "summary_large_image",
		title: "About Pylon",
		description: DESCRIPTION,
	},
};

export default function AboutPage({
	session,
}: {
	session?: { exists: boolean };
}) {
	return (
		<>
			<JsonLd
				graph={organizationPageGraph({
					path: "/about",
					name: "About Pylon",
					description: DESCRIPTION,
				})}
			/>
			<AboutView signedIn={Boolean(session?.exists)} />
		</>
	);
}
