// Comparison-page body — wraps a `Comparison` from
// `apps/web/data/comparisons.ts` in the CRO-shaped layout:
//
//   nav (via MarketingShell, sibling)
//   hero with H1 + lede + primary CTA
//   "choose <competitor>" / "choose Pylon" cards
//   architecture deltas table
//   "what both ship" bullets
//   Pylon-wins items (emphasized)
//   competitor-wins items (honest, less emphasized)
//   migration map table (if present)
//   honest weakness prose block
//   both/and prose block (if present)
//   final CTA banner
//
// Server component. No client interactivity needed for the page
// itself — the nav handles the only stateful piece (burger menu)
// in MarketingShell.

import Link from "next/link";
import { COMPARISON_PAGE_CSS } from "@/lib/comparison-css";
import type { Comparison } from "@/data/comparisons";

export function ComparisonPage({ data }: { data: Comparison }) {
	return (
		<main>
			<style dangerouslySetInnerHTML={{ __html: COMPARISON_PAGE_CSS }} />
			<Hero data={data} />
			<TldrSection data={data} />
			<ArchitectureSection data={data} />
			<SameShapeSection data={data} />
			<PylonBetterSection data={data} />
			<CompetitorBetterSection data={data} />
			{data.migration && <MigrationSection data={data} />}
			<HonestWeaknessSection data={data} />
			{data.bothAnd && <BothAndSection data={data} />}
			<CtaBanner data={data} />
		</main>
	);
}

function Hero({ data }: { data: Comparison }) {
	return (
		<section className="cmp-hero">
			<div className="shell">
				<div className="cmp-breadcrumb">
					<Link href="/">Pylon</Link>
					<span className="cmp-breadcrumb-sep">›</span>
					<Link href="/vs">Compare</Link>
					<span className="cmp-breadcrumb-sep">›</span>
					<span>vs. {data.competitor}</span>
				</div>
				<h1 className="cmp-h1">
					Pylon <span className="vs">vs.</span> {data.competitor}
				</h1>
				<p className="cmp-lede">{data.lede}</p>
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
	);
}

function TldrSection({ data }: { data: Comparison }) {
	return (
		<section className="cmp-section">
			<div className="shell">
				<p className="cmp-eyebrow">TL;DR</p>
				<h2>Pick the one that fits your shape</h2>
				<div className="cmp-tldr">
					<div className="cmp-tldr-card">
						<h3>
							Choose {data.competitor}
							<span className="pill">if</span>
						</h3>
						<p>{data.tldr.chooseCompetitor}</p>
					</div>
					<div className="cmp-tldr-card pick-pylon">
						<h3>
							Choose Pylon
							<span className="pill">if</span>
						</h3>
						<p>{data.tldr.choosePylon}</p>
					</div>
				</div>
			</div>
		</section>
	);
}

function ArchitectureSection({ data }: { data: Comparison }) {
	return (
		<section className="cmp-section">
			<div className="shell">
				<p className="cmp-eyebrow">Architecture</p>
				<h2>Where the two diverge</h2>
				<div className="cmp-table-wrap">
					<table className="cmp-table">
						<thead>
							<tr>
								<th></th>
								<th className="col-pylon">Pylon</th>
								<th>{data.competitor}</th>
							</tr>
						</thead>
						<tbody>
							{data.architecture.map((row) => (
								<tr key={row.dim}>
									<td className="dim">{row.dim}</td>
									<td
										className="pylon"
										dangerouslySetInnerHTML={{ __html: renderInline(row.pylon) }}
									/>
									<td
										className="competitor"
										dangerouslySetInnerHTML={{
											__html: renderInline(row.competitor),
										}}
									/>
								</tr>
							))}
						</tbody>
					</table>
				</div>
			</div>
		</section>
	);
}

function SameShapeSection({ data }: { data: Comparison }) {
	return (
		<section className="cmp-section">
			<div className="shell">
				<p className="cmp-eyebrow">Same shape</p>
				<h2>What both ship</h2>
				<p className="cmp-section-lede">
					Either one is a real choice for {realChoiceFor(data)}. The
					differences below are about emphasis and operational shape, not
					feature presence.
				</p>
				<ul className="cmp-bullets">
					{data.sameShape.map((bullet) => (
						<li key={bullet}>{bullet}</li>
					))}
				</ul>
			</div>
		</section>
	);
}

function PylonBetterSection({ data }: { data: Comparison }) {
	return (
		<section className="cmp-section pylon-wins">
			<div className="shell">
				<p className="cmp-eyebrow">Where Pylon wins</p>
				<h2>What you get with Pylon you don&apos;t with {data.competitor}</h2>
				<div className="cmp-items">
					{data.pylonBetter.map((item) => (
						<div key={item.title} className="cmp-item">
							<h4>{item.title}</h4>
							<p dangerouslySetInnerHTML={{ __html: renderInline(item.body) }} />
						</div>
					))}
				</div>
			</div>
		</section>
	);
}

function CompetitorBetterSection({ data }: { data: Comparison }) {
	return (
		<section className="cmp-section">
			<div className="shell">
				<p className="cmp-eyebrow">Where {data.competitor} wins</p>
				<h2>What {data.competitor} does better today</h2>
				<p className="cmp-section-lede">
					Honest comparison — these are real reasons to pick {data.competitor}.
					If any of them are dealbreakers, choose accordingly.
				</p>
				<div className="cmp-items">
					{data.competitorBetter.map((item) => (
						<div key={item.title} className="cmp-item">
							<h4>{item.title}</h4>
							<p dangerouslySetInnerHTML={{ __html: renderInline(item.body) }} />
						</div>
					))}
				</div>
			</div>
		</section>
	);
}

function MigrationSection({ data }: { data: Comparison }) {
	if (!data.migration) return null;
	return (
		<section className="cmp-section">
			<div className="shell">
				<p className="cmp-eyebrow">Migration</p>
				<h2>Coming from {data.competitor}</h2>
				<p className="cmp-section-lede">
					Most of the dev surface translates one-to-one. The biggest deltas
					show up as differences in shape, not features missing.
				</p>
				<div className="cmp-table-wrap">
					<table className="cmp-table">
						<thead>
							<tr>
								<th>{data.competitor}</th>
								<th className="col-pylon">Pylon</th>
							</tr>
						</thead>
						<tbody>
							{data.migration.map((row, i) => (
								<tr key={`${row.competitor}-${i}`}>
									<td
										className="competitor"
										dangerouslySetInnerHTML={{
											__html: renderInline(row.competitor),
										}}
									/>
									<td
										className="pylon"
										dangerouslySetInnerHTML={{ __html: renderInline(row.pylon) }}
									/>
								</tr>
							))}
						</tbody>
					</table>
				</div>
			</div>
		</section>
	);
}

function HonestWeaknessSection({ data }: { data: Comparison }) {
	return (
		<section className="cmp-section">
			<div className="shell">
				<p className="cmp-eyebrow">Honest weakness</p>
				<h2>Where Pylon loses</h2>
				<div className="cmp-prose">{data.honestWeakness}</div>
			</div>
		</section>
	);
}

function BothAndSection({ data }: { data: Comparison }) {
	if (!data.bothAnd) return null;
	return (
		<section className="cmp-section no-border">
			<div className="shell">
				<p className="cmp-eyebrow">Both / and</p>
				<h2>When using both is the right call</h2>
				<div className="cmp-prose">{data.bothAnd}</div>
			</div>
		</section>
	);
}

function CtaBanner({ data }: { data: Comparison }) {
	return (
		<section>
			<div className="shell">
				<div className="cmp-cta">
					<h2>Try Pylon — free Hobby tier on Cloud</h2>
					<p>
						No card, no setup. Run a real Pylon project against managed
						Postgres in under a minute. Migrate from {data.competitor} when
						you&apos;re ready — or run both.
					</p>
					<Link
						className="btn accent"
						href="https://cloud.pylonsync.com/signup"
					>
						Start free on Pylon Cloud →
					</Link>
				</div>
			</div>
		</section>
	);
}

// Lightweight inline markdown: backticks → <code>, otherwise plain
// text. The data file uses backticks for code spans the same way
// the docs MDX does; we render them via dangerouslySetInnerHTML
// after escaping. No external markdown lib for a 6-line job.
function renderInline(s: string): string {
	const escaped = s
		.replace(/&/g, "&amp;")
		.replace(/</g, "&lt;")
		.replace(/>/g, "&gt;")
		.replace(/"/g, "&quot;");
	return escaped.replace(/`([^`]+)`/g, "<code>$1</code>");
}

function realChoiceFor(data: Comparison): string {
	// Tiny narrative variation so each page reads slightly different
	// in the "same shape" intro. Single source of truth so we don't
	// re-derive in the data file.
	switch (data.slug) {
		case "convex":
			return "a TypeScript-first reactive backend";
		case "supabase":
			return "an open-source Firebase-style backend";
		case "firebase":
			return "a managed mobile-first backend";
		case "colyseus":
		case "nakama":
			return "a multiplayer game backend with built-in matchmaking";
		case "playroom":
			return "a quick-to-ship multiplayer surface";
		default:
			return "this use case";
	}
}
