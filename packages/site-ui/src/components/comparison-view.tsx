"use client";

import { Link } from "@pylonsync/react";
import { Check } from "lucide-react";
import { Button } from "./ui/button";
import { MarketingShell } from "./marketing-shell";
import { getComparison, type Comparison } from "../lib/comparison-content";
import { ctaUrl } from "../lib/account-urls";

// Render of a single /vs/<slug> comparison, with the cloud design system. The
// structured data (FAQPage + BreadcrumbList) renders inline so crawlers and AI
// engines see it in the SSR HTML.
export function ComparisonView({
	slug,
	signedIn = false,
}: {
	slug: string;
	signedIn?: boolean;
}) {
	const c = getComparison(slug);
	if (!c) {
		return (
			<MarketingShell signedIn={signedIn}>
				<div className="mx-auto max-w-[640px] px-5 py-32 text-center">
					<h1 className="text-[28px] font-semibold tracking-tight">
						Not found
					</h1>
					<Button asChild variant="primary" size="lg" className="mt-8">
						<Link href="/vs">All comparisons →</Link>
					</Button>
				</div>
			</MarketingShell>
		);
	}

	return (
		<MarketingShell signedIn={signedIn}>
			{/* JSON-LD structured data. Source is the static comparison map (no
			    user input); we still escape "<" → < so a stray "</script>"
			    in any string can never break out of the script element. */}
			<script
				type="application/ld+json"
				dangerouslySetInnerHTML={{ __html: structuredDataJson(c) }}
			/>

			{/* HERO */}
			<header className="border-b border-[var(--color-rule)]">
				<div className="mx-auto max-w-[900px] px-5 pb-16 pt-16 sm:px-8 sm:pb-20 sm:pt-24">
					<nav className="flex items-center gap-2 font-mono text-[11.5px] text-[var(--color-ink-4)]">
						<Link href="/vs" className="hover:text-[var(--color-ink-2)]">
							Compare
						</Link>
						<span>›</span>
						<span className="text-[var(--color-ink-2)]">
							vs. {c.competitor}
						</span>
					</nav>
					<h1 className="mt-7 text-[clamp(34px,5vw,58px)] font-semibold leading-[1.02] tracking-[-0.04em] text-[var(--color-ink)]">
						Pylon vs.{" "}
						<span className="text-[var(--color-brand)]">{c.competitor}</span>
					</h1>
					<p className="mt-6 max-w-[64ch] text-[16px] leading-[1.6] text-[var(--color-ink-2)] sm:text-[18px]">
						{c.lede}
					</p>
					<div className="mt-8 flex flex-col items-stretch gap-2.5 sm:flex-row sm:items-center sm:gap-3">
						<Button asChild variant="primary" size="lg">
							<Link href={ctaUrl(signedIn)}>
								{signedIn ? "Open dashboard →" : "Create your account →"}
							</Link>
						</Button>
						<Button asChild variant="ghost" size="lg">
							<a href={c.competitorUrl}>What is {c.competitor}?</a>
						</Button>
					</div>
				</div>
			</header>

			{/* TL;DR */}
			<section className="border-b border-[var(--color-rule)] bg-[var(--color-paper-1)]">
				<div className="mx-auto max-w-[900px] px-5 py-14 sm:px-8 sm:py-16">
					<div className="font-mono text-[11px] uppercase tracking-[0.14em] text-[var(--color-ink-4)]">
						TL;DR
					</div>
					<div className="mt-6 grid gap-4 lg:grid-cols-2">
						<div className="rounded-[var(--radius-lg)] border border-[var(--color-rule)] bg-[var(--color-paper)] p-6">
							<div className="text-[14px] font-semibold tracking-tight text-[var(--color-ink)]">
								Choose {c.competitor} if…
							</div>
							<p className="mt-3 text-[13.5px] leading-[1.6] text-[var(--color-ink-2)]">
								{c.tldr.chooseCompetitor}
							</p>
						</div>
						<div className="rounded-[var(--radius-lg)] border border-[var(--color-brand)]/50 bg-[var(--color-paper)] p-6">
							<div className="text-[14px] font-semibold tracking-tight text-[var(--color-brand)]">
								Choose Pylon if…
							</div>
							<p className="mt-3 text-[13.5px] leading-[1.6] text-[var(--color-ink-2)]">
								{c.tldr.choosePylon}
							</p>
						</div>
					</div>
				</div>
			</section>

			{/* ARCHITECTURE TABLE */}
			<section className="border-b border-[var(--color-rule)]">
				<div className="mx-auto max-w-[900px] px-5 py-14 sm:px-8 sm:py-16">
					<h2 className="text-[22px] font-semibold tracking-tight text-[var(--color-ink)]">
						Architecture at a glance
					</h2>
					<div className="mt-6 overflow-x-auto rounded-[var(--radius-lg)] border border-[var(--color-rule)]">
						<table className="w-full border-collapse text-left text-[13px]">
							<thead>
								<tr className="border-b border-[var(--color-rule)] bg-[var(--color-paper-1)]">
									<th className="px-4 py-3 font-mono text-[10.5px] uppercase tracking-[0.08em] text-[var(--color-ink-4)]" />
									<th className="px-4 py-3 font-semibold text-[var(--color-brand)]">
										Pylon
									</th>
									<th className="px-4 py-3 font-semibold text-[var(--color-ink-2)]">
										{c.competitor}
									</th>
								</tr>
							</thead>
							<tbody>
								{c.architecture.map((row) => (
									<tr
										key={row.dim}
										className="border-b border-[var(--color-rule-soft)] last:border-b-0"
									>
										<td className="px-4 py-3 font-medium text-[var(--color-ink-3)]">
											{row.dim}
										</td>
										<td className="px-4 py-3 text-[var(--color-ink)]">
											{row.pylon}
										</td>
										<td className="px-4 py-3 text-[var(--color-ink-2)]">
											{row.competitor}
										</td>
									</tr>
								))}
							</tbody>
						</table>
					</div>
				</div>
			</section>

			{/* SAME SHAPE */}
			<section className="border-b border-[var(--color-rule)] bg-[var(--color-paper-1)]">
				<div className="mx-auto max-w-[900px] px-5 py-14 sm:px-8 sm:py-16">
					<h2 className="text-[22px] font-semibold tracking-tight text-[var(--color-ink)]">
						What both ship
					</h2>
					<ul className="mt-6 grid gap-x-8 gap-y-3 sm:grid-cols-2">
						{c.sameShape.map((s) => (
							<li key={s} className="flex items-start gap-3">
								<span className="mt-0.5 flex h-5 w-5 shrink-0 items-center justify-center rounded-full bg-[var(--color-brand)]/12 text-[var(--color-brand)]">
									<Check className="size-3" />
								</span>
								<span className="text-[14px] leading-[1.5] text-[var(--color-ink-2)]">
									{s}
								</span>
							</li>
						))}
					</ul>
				</div>
			</section>

			{/* WHERE EACH WINS */}
			<section className="border-b border-[var(--color-rule)]">
				<div className="mx-auto max-w-[900px] px-5 py-14 sm:px-8 sm:py-16">
					<div className="grid gap-12 lg:grid-cols-2">
						<div>
							<h2 className="text-[20px] font-semibold tracking-tight text-[var(--color-ink)]">
								Where {c.competitor} wins
							</h2>
							<div className="mt-6 flex flex-col gap-5">
								{c.competitorBetter.map((item) => (
									<div key={item.title}>
										<div className="text-[14px] font-semibold tracking-tight text-[var(--color-ink)]">
											{item.title}
										</div>
										<p className="mt-1.5 text-[13px] leading-[1.6] text-[var(--color-ink-3)]">
											{item.body}
										</p>
									</div>
								))}
							</div>
						</div>
						<div>
							<h2 className="text-[20px] font-semibold tracking-tight text-[var(--color-brand)]">
								Where Pylon wins
							</h2>
							<div className="mt-6 flex flex-col gap-5">
								{c.pylonBetter.map((item) => (
									<div key={item.title}>
										<div className="text-[14px] font-semibold tracking-tight text-[var(--color-ink)]">
											{item.title}
										</div>
										<p className="mt-1.5 text-[13px] leading-[1.6] text-[var(--color-ink-3)]">
											{item.body}
										</p>
									</div>
								))}
							</div>
						</div>
					</div>
				</div>
			</section>

			{/* MIGRATION */}
			{c.migration && (
				<section className="border-b border-[var(--color-rule)] bg-[var(--color-paper-1)]">
					<div className="mx-auto max-w-[900px] px-5 py-14 sm:px-8 sm:py-16">
						<h2 className="text-[22px] font-semibold tracking-tight text-[var(--color-ink)]">
							Moving from {c.competitor}
						</h2>
						<div className="mt-6 overflow-x-auto rounded-[var(--radius-lg)] border border-[var(--color-rule)] bg-[var(--color-paper)]">
							<table className="w-full border-collapse text-left text-[12.5px]">
								<thead>
									<tr className="border-b border-[var(--color-rule)]">
										<th className="px-4 py-3 font-semibold text-[var(--color-ink-2)]">
											{c.competitor}
										</th>
										<th className="px-4 py-3 font-semibold text-[var(--color-brand)]">
											Pylon
										</th>
									</tr>
								</thead>
								<tbody>
									{c.migration.map((row) => (
										<tr
											key={row.competitor}
											className="border-b border-[var(--color-rule-soft)] last:border-b-0"
										>
											<td className="px-4 py-3 font-mono text-[var(--color-ink-3)]">
												{row.competitor}
											</td>
											<td className="px-4 py-3 font-mono text-[var(--color-ink)]">
												{row.pylon}
											</td>
										</tr>
									))}
								</tbody>
							</table>
						</div>
					</div>
				</section>
			)}

			{/* HONEST WEAKNESS */}
			<section className="border-b border-[var(--color-rule)]">
				<div className="mx-auto max-w-[900px] px-5 py-14 sm:px-8 sm:py-16">
					<div className="rounded-[var(--radius-lg)] border border-[var(--color-rule)] bg-[var(--color-paper-1)] p-6 sm:p-8">
						<div className="font-mono text-[10.5px] uppercase tracking-[0.1em] text-[var(--color-ink-4)]">
							The honest take
						</div>
						<p className="mt-4 text-[14.5px] leading-[1.65] text-[var(--color-ink-2)]">
							{c.honestWeakness}
						</p>
					</div>
				</div>
			</section>

			{/* CTA */}
			<section>
				<div className="mx-auto max-w-[760px] px-5 py-20 text-center sm:px-8 sm:py-24">
					<h2 className="mx-auto max-w-[18ch] text-[clamp(28px,4vw,44px)] font-semibold leading-[1.08] tracking-[-0.035em] text-[var(--color-ink)]">
						Try Pylon for yourself.
					</h2>
					<p className="mx-auto mt-4 max-w-[44ch] text-[15px] leading-[1.55] text-[var(--color-ink-2)] sm:text-[16px]">
						Scaffold a full-stack app in seconds and deploy free on Cloud.
					</p>
					<div className="mt-7 inline-flex items-center gap-3 rounded-[var(--radius-md)] border border-[var(--color-rule)] bg-[var(--color-paper-1)] px-4 py-2.5 font-mono text-[13.5px] text-[var(--color-ink)]">
						<span className="text-[var(--color-brand)]">$</span>
						npm create @pylonsync/pylon@latest
					</div>
					<div className="mt-8 flex justify-center">
						<Button asChild variant="primary" size="lg">
							<Link href="/vs">See all comparisons →</Link>
						</Button>
					</div>
				</div>
			</section>
		</MarketingShell>
	);
}

// Serialize the JSON-LD graph, escaping "<" so a "</script>" sequence in any
// string can never terminate the <script> element early (XSS hardening, even
// though the source data is static and trusted).
function structuredDataJson(data: Comparison): string {
	return JSON.stringify(buildStructuredData(data)).replace(/</g, "\\u003c");
}

function buildStructuredData(data: Comparison) {
	const pageUrl = `https://www.pylonsync.com/vs/${data.slug}`;
	return {
		"@context": "https://schema.org",
		"@graph": [
			{
				"@type": "BreadcrumbList",
				itemListElement: [
					{
						"@type": "ListItem",
						position: 1,
						name: "Pylon",
						item: "https://www.pylonsync.com/",
					},
					{
						"@type": "ListItem",
						position: 2,
						name: "Compare",
						item: "https://www.pylonsync.com/vs",
					},
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
