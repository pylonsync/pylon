import type { Metadata } from "@pylonsync/react";
import { DevelopersView } from "@pylon-site/ui/components/developers-view";
import { SITE_URL } from "../../lib/site";

// Two cached shells keyed on the signed-in bit — see app/page.tsx for the
// full auth-bucketed rationale.
export const cache = "auth-bucketed";
export const revalidate = 3600; // 1 hour

const DESCRIPTION =
	"Pylon developer resources: the documentation, the Pylon CLI on npm, starter templates, the agent skill, llms.txt, the Pylon MCP server, and the OpenAPI 3.1 spec for the agent API. All public, all free, no account.";

export const metadata: Metadata = {
	title: "Pylon developer resources: docs, CLI, MCP server, API spec",
	description: DESCRIPTION,
	canonical: `${SITE_URL}/developers`,
	openGraph: {
		title: "Pylon developer resources",
		description: DESCRIPTION,
		url: `${SITE_URL}/developers`,
		siteName: "Pylon",
		type: "website",
	},
	twitter: {
		card: "summary_large_image",
		title: "Pylon developer resources",
		description: DESCRIPTION,
	},
};

export default function DevelopersPage({
	session,
}: {
	session?: { exists: boolean };
}) {
	return <DevelopersView signedIn={Boolean(session?.exists)} />;
}
