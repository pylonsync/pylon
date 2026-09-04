import type { Metadata } from "@pylonsync/react";
import { ContactView } from "@pylon-cloud/ui/components/contact-view";
import { JsonLd } from "../../lib/agent/json-ld";
import { organizationPageGraph } from "../../lib/agent/jsonld";
import { SITE_URL } from "../../lib/site";

// Two cached shells keyed on the signed-in bit — see app/page.tsx for the
// full auth-bucketed rationale.
export const cache = "auth-bucketed";
export const revalidate = 3600; // 1 hour

const DESCRIPTION =
	"How to reach Pylon: support@pylonsync.com for the framework, GitHub issues for bugs, security@pylonsync.com for vulnerabilities, and Stack0 Cloud for hosting and billing. Built in Dallas, Texas.";

export const metadata: Metadata = {
	title: "Contact Pylon: support, security, and hosting",
	description: DESCRIPTION,
	canonical: `${SITE_URL}/contact`,
	openGraph: {
		title: "Contact Pylon",
		description: DESCRIPTION,
		url: `${SITE_URL}/contact`,
		siteName: "Pylon",
		type: "website",
	},
	twitter: {
		card: "summary_large_image",
		title: "Contact Pylon",
		description: DESCRIPTION,
	},
};

export default function ContactPage({
	session,
}: {
	session?: { exists: boolean };
}) {
	return (
		<>
			<JsonLd
				graph={organizationPageGraph({
					path: "/contact",
					name: "Contact Pylon",
					description: DESCRIPTION,
				})}
			/>
			<ContactView signedIn={Boolean(session?.exists)} />
		</>
	);
}
