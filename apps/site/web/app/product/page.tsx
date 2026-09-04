import type { Metadata } from "@pylonsync/react";
import { ProductIndexView } from "@pylon-site/ui/components/product-index-view";

// Two cached shells keyed on the signed-in bit — see app/page.tsx for the
// full auth-bucketed rationale.
export const cache = "auth-bucketed";
export const revalidate = 3600; // 1 hour
export const metadata: Metadata = {
	title: "Pylon product: one framework, every primitive",
	description:
		"Explore the database, sync, auth, server functions, realtime, storage, search, and SSR primitives that ship with Pylon.",
	canonical: "/product",
	openGraph: {
		title: "Pylon: one framework, every primitive",
		description:
			"Database, sync, auth, functions, realtime, storage, search, and SSR.",
		url: "https://www.pylonsync.com/product",
		type: "website",
	},
	twitter: {
		card: "summary_large_image",
		title: "Pylon: one framework, every primitive",
		description:
			"Pylon ships database, sync, auth, functions, realtime, search, and more in one framework.",
	},
};

export default function ProductIndexPage({
	session,
}: {
	session?: { exists: boolean };
}) {
	return <ProductIndexView signedIn={Boolean(session?.exists)} />;
}
