import type { Metadata } from "@pylonsync/react";
import { ProductView } from "@pylon-cloud/ui/components/product-view";
import { getProduct } from "@pylon-cloud/ui/lib/product-content";

// Dynamic route for every /product/<slug> primitive page. Per-slug SEO comes
// from generateMetadata reading the content map; the page body renders the
// shared ProductView. Two cached shells per slug, keyed on the signed-in bit
// (`session.exists`) — see app/page.tsx for the full auth-bucketed rationale.
// An unknown slug gets a 404 status + the ProductView "not found" surface
// (non-200 renders are never cached).

export const cache = "auth-bucketed";
export const revalidate = 3600; // 1 hour
interface ProductPageProps {
	params: { slug: string };
	session?: { exists: boolean };
	response?: { setStatus?: (code: number) => void };
}

export function generateMetadata({ params }: ProductPageProps): Metadata {
	const p = getProduct(params.slug);
	if (!p) return { title: "Product — Pylon" };
	const url = `https://pylonsync.com/product/${p.slug}`;
	return {
		title: p.metaTitle,
		description: p.metaDescription,
		canonical: `/product/${p.slug}`,
		openGraph: {
			title: p.metaTitle,
			description: p.metaDescription,
			url,
			type: "article",
		},
		twitter: {
			card: "summary_large_image",
			title: p.metaTitle,
			description: p.metaDescription,
		},
	};
}

export default function ProductSlugPage({
	params,
	session,
	response,
}: ProductPageProps) {
	if (!getProduct(params.slug)) response?.setStatus?.(404);
	return (
		<ProductView slug={params.slug} signedIn={Boolean(session?.exists)} />
	);
}
