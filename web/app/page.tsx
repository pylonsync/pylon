import type { Metadata } from "@pylonsync/react";
import { MarketingPage } from "@pylon-cloud/ui/components/marketing-home";

// Auth-bucketed output cache: the runtime keeps TWO identity-free shells
// (signed-in / signed-out) keyed on the binary `session.exists` bit and picks
// the right one per request — so the signed-in nav CTA is correct in the
// FIRST server-rendered byte (no client-side swap, no flash) while renders
// still hit the origin cache. Reading full `props.auth` here would make the
// output identity-specific and kill caching entirely — only read
// `session.exists`. (On Cloudflare Free the HTML passes through to the
// origin's ~ms bucketed cache; a Business+ plan + cookie-keyed cache rule
// would edge-cache both shells with no code change.)
export const cache = "auth-bucketed";
export const revalidate = 3600; // 1 hour

export const metadata: Metadata = {
	title: "Pylon: full-stack framework for coding agents",
	description:
		"Declare an entity once. Pylon derives the table, the access rules, the API, and the typed React client. SQLite by default, Postgres when you need it.",
	canonical: "https://www.pylonsync.com",
	openGraph: {
		title: "Pylon: full-stack framework for coding agents",
		description:
			"Declare an entity once. Pylon derives the table, the access rules, the API, and the typed React client.",
		url: "https://www.pylonsync.com",
		siteName: "Pylon",
		type: "website",
	},
	twitter: {
		card: "summary_large_image",
		title: "Pylon: full-stack framework for coding agents",
		description:
			"Declare an entity once. Pylon derives the table, the access rules, the API, and the typed React client.",
	},
};

export default function HomePage({
	session,
}: {
	session?: { exists: boolean };
}) {
	return <MarketingPage initialSignedIn={Boolean(session?.exists)} />;
}
