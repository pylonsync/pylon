// /vs — comparison index. Each card links to a `/vs/<slug>` page.
// Static, server-rendered, lives next to the dynamic [slug] route.

import type { Metadata } from "next";
import Link from "next/link";
import { MarketingShell } from "@/components/marketing-shell";
import { COMPARISON_PAGE_CSS } from "@/lib/comparison-css";
import { COMPARISONS } from "@/data/comparisons";

export const metadata: Metadata = {
	title: "Pylon vs. Convex, Supabase, Firebase, Colyseus, Nakama, Playroom Kit",
	description:
		"Honest, side-by-side comparisons of Pylon against every realtime backend it overlaps with — Convex, Supabase, Firebase, plus the game-server stack (Colyseus, Nakama, Playroom Kit).",
	alternates: { canonical: "/vs" },
	openGraph: {
		title: "How Pylon compares — Convex, Supabase, Firebase, Colyseus, Nakama, Playroom Kit",
		description:
			"Side-by-side comparisons of Pylon against every realtime backend it overlaps with.",
		url: "https://pylonsync.com/vs",
		type: "website",
	},
};

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

export default async function VsIndexPage() {
	const stars = await getStarCount();
	return (
		<MarketingShell stars={stars}>
			<main>
				<style dangerouslySetInnerHTML={{ __html: COMPARISON_PAGE_CSS }} />
				<section className="cmp-hero">
					<div className="shell">
						<div className="cmp-breadcrumb">
							<Link href="/">Pylon</Link>
							<span className="cmp-breadcrumb-sep">›</span>
							<span>Compare</span>
						</div>
						<h1 className="cmp-h1">
							How Pylon <span className="vs">compares</span>
						</h1>
						<p className="cmp-lede">
							Honest, side-by-side comparisons of Pylon against every realtime
							backend it overlaps with. Each page has a TL;DR, an architecture
							table, where each side wins, and a migration map.
						</p>
						<div className="cmp-ctas">
							<Link
								className="btn accent"
								href="https://cloud.pylonsync.com/signup"
							>
								Start free on Pylon Cloud →
							</Link>
							<a className="btn line" href="https://docs.pylonsync.com">
								Read the docs →
							</a>
						</div>
					</div>
				</section>
				<section className="cmp-section no-border vs-index">
					<div className="shell">
						<div className="vs-index-grid">
							{COMPARISONS.map((c) => (
								<Link key={c.slug} className="vs-card" href={`/vs/${c.slug}`}>
									<h3>Pylon vs. {c.competitor}</h3>
									<p>{c.metaDescription}</p>
									<span className="arrow">Read the comparison →</span>
								</Link>
							))}
						</div>
					</div>
				</section>
			</main>
		</MarketingShell>
	);
}
