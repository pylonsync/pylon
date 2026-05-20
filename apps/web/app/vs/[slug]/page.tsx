// /vs/[slug] — dynamic route generating one CRO-shaped marketing
// page per competitor from the `COMPARISONS` data file. Static at
// build time via `generateStaticParams`, so each route ships as
// pre-rendered HTML with full SEO scaffolding.
//
// Per-page metadata (title, description, canonical, og) comes from
// `generateMetadata`. JSON-LD (FAQPage + BreadcrumbList) renders
// in the page body so AI engines and rich-result crawlers see
// structured data without needing a separate sitemap.

import type { Metadata } from "next";
import { notFound } from "next/navigation";
import { ComparisonPage } from "@/components/comparison-page";
import { MarketingShell } from "@/components/marketing-shell";
import {
	COMPARISONS,
	comparisonSlugs,
	getComparison,
	type Comparison,
} from "@/data/comparisons";

// Build all 6 pages at build time. Next.js will 404 anything else.
export function generateStaticParams() {
	return comparisonSlugs().map((slug) => ({ slug }));
}

// Per-competitor metadata. Title uses the SEO-keyword shape
// (`<Competitor> alternative` is the high-intent query).
export async function generateMetadata({
	params,
}: {
	params: Promise<{ slug: string }>;
}): Promise<Metadata> {
	const { slug } = await params;
	const data = getComparison(slug);
	if (!data) return {};
	const title = `${data.keyword}: Pylon — Realtime backend for TypeScript apps`;
	const url = `https://pylonsync.com/vs/${data.slug}`;
	return {
		title,
		description: data.metaDescription,
		alternates: { canonical: `/vs/${data.slug}` },
		openGraph: {
			title,
			description: data.metaDescription,
			url,
			type: "article",
		},
		twitter: {
			card: "summary_large_image",
			title,
			description: data.metaDescription,
		},
	};
}

// Fetch the GitHub star count at build time for the nav badge.
// Same primitive as `/` uses; cached by Vercel's ISR.
async function getStarCount(): Promise<number | null> {
	try {
		const res = await fetch("https://api.github.com/repos/pylonsync/pylon", {
			headers: {
				Accept: "application/vnd.github+json",
				"User-Agent": "pylonsync-marketing-site",
			},
			next: { revalidate: 3600 },
		});
		if (!res.ok) return null;
		const json = (await res.json()) as { stargazers_count?: number };
		return typeof json.stargazers_count === "number"
			? json.stargazers_count
			: null;
	} catch {
		return null;
	}
}

export default async function VsCompetitorPage({
	params,
}: {
	params: Promise<{ slug: string }>;
}) {
	const { slug } = await params;
	const data = getComparison(slug);
	if (!data) notFound();
	const stars = await getStarCount();
	return (
		<MarketingShell stars={stars}>
			<script
				type="application/ld+json"
				// Page-specific structured data. Two graphs:
				//   FAQPage — the "Choose X if / Choose Pylon if" pair
				//     becomes a 2-question FAQ. Eligible for FAQ rich
				//     results when Google decides to show them.
				//   BreadcrumbList — Pylon › Compare › vs. <competitor>.
				//     Drives breadcrumb display in the SERP snippet.
				dangerouslySetInnerHTML={{
					__html: JSON.stringify(buildStructuredData(data)),
				}}
			/>
			<ComparisonPage data={data} />
		</MarketingShell>
	);
}

function buildStructuredData(data: Comparison) {
	const pageUrl = `https://pylonsync.com/vs/${data.slug}`;
	return {
		"@context": "https://schema.org",
		"@graph": [
			{
				"@type": "BreadcrumbList",
				itemListElement: [
					{ "@type": "ListItem", position: 1, name: "Pylon", item: "https://pylonsync.com/" },
					{ "@type": "ListItem", position: 2, name: "Compare", item: "https://pylonsync.com/vs" },
					{
						"@type": "ListItem",
						position: 3,
						name: `vs. ${data.competitor}`,
						item: pageUrl,
					},
				],
			},
			{
				"@type": "FAQPage",
				mainEntity: [
					{
						"@type": "Question",
						name: `When should I choose ${data.competitor}?`,
						acceptedAnswer: {
							"@type": "Answer",
							text: data.tldr.chooseCompetitor,
						},
					},
					{
						"@type": "Question",
						name: "When should I choose Pylon?",
						acceptedAnswer: {
							"@type": "Answer",
							text: data.tldr.choosePylon,
						},
					},
				],
			},
		],
	};
}

// Force static rendering at build time — the page has zero
// per-request data; the GitHub star fetch is ISR-cached. Without
// this Next sometimes opts the page into dynamic rendering when
// it sees a `fetch` call.
export const dynamic = "force-static";

// Re-export so unused-import lint doesn't trip when the file is
// audited in isolation.
export { COMPARISONS };
