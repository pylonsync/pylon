"use client";

import { useState } from "react";
import { Link } from "@pylonsync/react";
import { ArrowUpRight, Check } from "lucide-react";
import { Button } from "./ui/button";
import { MarketingShell } from "./marketing-shell";
import { CodePanel } from "./code-panel";
import { LiveVisual, PRIMITIVE_VISUALS } from "./bento-visuals";
import { ProductSubNav } from "./product-sub-nav";
import { examplesFor } from "../lib/examples-content";
import { getProduct } from "../lib/product-content";
import { ctaUrl } from "../lib/account-urls";

// Detail view for a single /product/<slug> page. Thin client wrapper (so the
// MarketingShell nav island hydrates) over the static content in
// lib/product-content.ts. Looks the primitive up by slug from the bundled map
// rather than receiving the whole object as a serialized prop, so the page
// payload stays small. The route guarantees the slug exists (notFound()
// otherwise), but we guard anyway.
export function ProductView({
	slug,
	signedIn = false,
}: {
	slug: string;
	signedIn?: boolean;
}) {
	const p = getProduct(slug);
	if (!p) {
		return (
			<MarketingShell signedIn={signedIn}>
				<div className="mx-auto max-w-[640px] px-5 py-32 text-center">
					<h1 className="text-[28px] font-semibold tracking-tight">
						Not found
					</h1>
					<p className="mt-3 text-[var(--color-ink-3)]">
						That product page doesn&apos;t exist.
					</p>
					<Button asChild variant="primary" size="lg" className="mt-8">
						<Link href="/product">Browse the product →</Link>
					</Button>
				</div>
			</MarketingShell>
		);
	}

	const related = p.related.map(getProduct).filter(Boolean);
	const visuals = PRIMITIVE_VISUALS[p.slug] ?? [];
	const examples = examplesFor(p.slug);

	return (
		<MarketingShell signedIn={signedIn}>
			<ProductSubNav slug={p.slug} />

			{/* HERO */}
			<header className="border-b border-[var(--color-rule)]">
				<div className="mx-auto max-w-[1280px] px-5 pb-16 pt-16 sm:px-8 sm:pb-20 sm:pt-24">
					<nav className="flex items-center gap-2 font-mono text-[11.5px] text-[var(--color-ink-4)]">
						<Link href="/product" className="hover:text-[var(--color-ink-2)]">
							Product
						</Link>
						<span>›</span>
						<span className="text-[var(--color-ink-2)]">{p.navLabel}</span>
					</nav>

					<div className="mt-8 grid min-w-0 grid-cols-[minmax(0,1fr)] gap-10 lg:grid-cols-[1fr_minmax(0,520px)] lg:gap-12">
						<div>
							<div className="font-mono text-[11px] uppercase tracking-[0.14em] text-[var(--color-brand)]">
								{p.category}
							</div>
							<h1 className="mt-4 max-w-[18ch] text-[clamp(34px,5vw,58px)] font-semibold leading-[1.02] tracking-[-0.04em] text-[var(--color-ink)]">
								{p.title}
							</h1>
							<p className="mt-6 max-w-[52ch] text-[16px] leading-[1.6] text-[var(--color-ink-2)] sm:text-[17px]">
								{p.tagline}
							</p>
							<div className="mt-8 flex flex-col items-stretch gap-2.5 sm:flex-row sm:items-center sm:gap-3">
								<Button asChild variant="primary" size="lg">
									<Link href={ctaUrl(signedIn)}>
										{signedIn ? "Open dashboard →" : "Create your account →"}
									</Link>
								</Button>
								<Button asChild variant="ghost" size="lg">
									<a href="https://docs.pylonsync.com">Read the docs</a>
								</Button>
							</div>
						</div>

						{p.code && (
							<div className="min-w-0 lg:pt-1">
								<CodePanel filename={p.code.label} code={p.code.code} />
							</div>
						)}
					</div>
				</div>
			</header>

			{/* HIGHLIGHTS */}
			<section className="border-b border-[var(--color-rule)] bg-[var(--color-paper-1)]">
				<div className="mx-auto max-w-[1280px] px-5 py-12 sm:px-8 sm:py-14">
					<ul className="grid gap-x-8 gap-y-3.5 sm:grid-cols-2">
						{p.highlights.map((h) => (
							<li key={h} className="flex items-start gap-3">
								<span className="mt-0.5 flex h-5 w-5 shrink-0 items-center justify-center rounded-full bg-[var(--color-brand)]/12 text-[var(--color-brand)]">
									<Check className="size-3" />
								</span>
								<span className="text-[14px] leading-[1.5] text-[var(--color-ink-2)]">
									{h}
								</span>
							</li>
						))}
					</ul>
				</div>
			</section>

			{/* DEEP-DIVE — Supabase's menu-left / panel-right pattern. The three
			    sections used to sit side by side as equal columns, which gave each
			    one a third of the width and no artifact. Same copy, but each section
			    now gets the full panel and something to look at. */}
			<section className="border-b border-[var(--color-rule)] bg-[var(--color-paper-1)]">
				<div className="mx-auto max-w-[1280px] px-5 py-16 sm:px-8 sm:py-20">
					<h2 className="max-w-[20ch] text-[clamp(26px,3.2vw,38px)] font-semibold leading-[1.08] tracking-[-0.035em] text-[var(--color-ink)]">
						How it works.
					</h2>
					<ProductDeepDive product={p} visuals={visuals} />
				</div>
			</section>

			{/* WHAT YOU'D OTHERWISE BUILD — the comparison the rest of the page
			    implies but never states. A hairline ledger rather than a checklist,
			    so it reads as the opposite of the cobalt-check highlights above. */}
			<section>
				<div className="mx-auto max-w-[1280px] px-5 py-16 sm:px-8 sm:py-20">
					<h2 className="max-w-[20ch] text-[clamp(26px,3.2vw,38px)] font-semibold leading-[1.08] tracking-[-0.035em] text-[var(--color-ink)]">
						What you&rsquo;d otherwise wire up.
					</h2>
					<p className="mt-4 max-w-[58ch] text-[15px] leading-[1.6] text-[var(--color-ink-2)] sm:text-[16px]">
						Each line is a piece teams normally assemble for this one capability.
						On Pylon it is part of the runtime.
					</p>
					<ul className="mt-8 grid gap-x-12 sm:grid-cols-2">
						{p.replaces.map((r) => (
							<li
								key={r}
								className="flex items-start gap-3 border-t border-[var(--color-rule)] py-4 text-[14px] leading-[1.55] text-[var(--color-ink-2)]"
							>
								<span className="mt-[9px] h-px w-3 shrink-0 bg-[var(--color-ink-4)]" />
								{r}
							</li>
						))}
					</ul>
				</div>
			</section>

			{/* BUILT WITH IT — real apps whose own `shows` tags name this
			    primitive. Live demos link out; the rest name the create-pylon
			    template that scaffolds them. Slugs with no tagged example render
			    nothing rather than a padded-out placeholder. */}
			{examples.length > 0 && (
				<section className="border-t border-[var(--color-rule)] bg-[var(--color-paper-1)]">
					<div className="mx-auto max-w-[1280px] px-5 py-16 sm:px-8 sm:py-20">
						<div className="flex flex-wrap items-end justify-between gap-4">
							<div>
								<h2 className="max-w-[20ch] text-[clamp(26px,3.2vw,38px)] font-semibold leading-[1.08] tracking-[-0.035em] text-[var(--color-ink)]">
									Apps built with it.
								</h2>
								<p className="mt-4 max-w-[58ch] text-[15px] leading-[1.6] text-[var(--color-ink-2)] sm:text-[16px]">
									Scaffold any of these with{" "}
									<code className="font-mono text-[14px] text-[var(--color-ink)]">
										npm create @pylonsync/pylon
									</code>
									.
								</p>
							</div>
							<Button asChild variant="ghost" size="sm">
								<Link href="/developers/examples">All examples →</Link>
							</Button>
						</div>

						<div className="mt-8 grid gap-4 sm:grid-cols-2 lg:grid-cols-3">
							{examples.map((ex) => (
								<div
									key={ex.name}
									className="flex flex-col rounded-[var(--radius-xl)] border border-[var(--color-rule)] bg-[var(--color-paper)] p-5 shadow-[var(--shadow-card)]"
								>
									<div className="flex items-center gap-2">
										{ex.live && (
											<span className="inline-flex items-center gap-1.5 font-mono text-[10px] uppercase tracking-[0.14em] text-[var(--color-ink-4)]">
												<span
													className="block size-1.5 rounded-full"
													style={{ backgroundColor: "var(--color-status-live)" }}
												/>
												live
											</span>
										)}
										<span className="text-[15px] font-semibold tracking-tight text-[var(--color-ink)]">
											{ex.name}
										</span>
									</div>
									<p className="mt-2 flex-1 text-[13px] leading-[1.55] text-[var(--color-ink-3)]">
										{ex.blurb}
									</p>
									<div className="mt-4 flex flex-wrap gap-1.5">
										{ex.shows.map((tag) => (
											<span
												key={tag}
												className="rounded-full border border-[var(--color-rule)] bg-[var(--color-paper-1)] px-2 py-0.5 font-mono text-[10.5px] text-[var(--color-ink-3)]"
											>
												{tag}
											</span>
										))}
									</div>
									<div className="mt-4 flex items-center gap-3 border-t border-[var(--color-rule)] pt-3">
										{ex.template && (
											<span className="font-mono text-[11px] text-[var(--color-ink-4)]">
												template:{" "}
												<span className="text-[var(--color-brand)]">{ex.template}</span>
											</span>
										)}
										{ex.live && (
											<a
												href={ex.live}
												target="_blank"
												rel="noreferrer"
												className="ml-auto inline-flex items-center gap-1 text-[12.5px] text-[var(--color-brand)] hover:underline"
											>
												Open <ArrowUpRight className="size-3.5" />
											</a>
										)}
									</div>
								</div>
							))}
						</div>
					</div>
				</section>
			)}

			{/* RELATED */}
			{related.length > 0 && (
				<section className="border-t border-[var(--color-rule)] bg-[var(--color-paper-1)]">
					<div className="mx-auto max-w-[1280px] px-5 py-14 sm:px-8 sm:py-16">
						<div className="font-mono text-[11px] uppercase tracking-[0.14em] text-[var(--color-ink-4)]">
							Keep exploring
						</div>
						<div className="mt-6 grid gap-4 sm:grid-cols-3">
							{related.map(
								(r) =>
									r && (
										<Link
											key={r.slug}
											href={`/product/${r.slug}`}
											className="group flex flex-col rounded-[var(--radius-xl)] border border-[var(--color-rule)] bg-[var(--color-paper)] p-5 shadow-[var(--shadow-card)] transition-[filter,box-shadow] duration-300 ease-[var(--ease-out-quart)] hover:shadow-[var(--shadow-card-hover)] focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-[var(--color-brand)] focus-visible:ring-offset-2 focus-visible:ring-offset-[var(--color-paper-1)] [@media(hover:hover)]:grayscale [@media(hover:hover)]:focus-visible:grayscale-0 [@media(hover:hover)]:hover:grayscale-0"
										>
											<div className="flex items-center gap-2">
												<span className="text-[15px] font-semibold tracking-tight text-[var(--color-ink)]">
													{r.navLabel}
												</span>
												<ArrowUpRight className="ml-auto size-3.5 shrink-0 text-[var(--color-ink-4)] opacity-0 transition-opacity duration-200 group-focus-visible:opacity-100 group-hover:opacity-100" />
											</div>
											<div className="mt-1.5 text-[13px] leading-[1.5] text-[var(--color-ink-3)]">
												{r.title}
											</div>
										</Link>
									),
							)}
						</div>
					</div>
				</section>
			)}

			{/* CTA */}
			<section className="border-t border-[var(--color-rule)]">
				<div className="mx-auto max-w-[760px] px-5 py-20 text-center sm:px-8 sm:py-24">
					<h2 className="mx-auto max-w-[16ch] text-[clamp(28px,4vw,44px)] font-semibold leading-[1.08] tracking-[-0.035em] text-[var(--color-ink)]">
						Build it on Pylon.
					</h2>
					<p className="mx-auto mt-4 max-w-[44ch] text-[15px] leading-[1.55] text-[var(--color-ink-2)] sm:text-[16px]">
						One framework for your schema, sync, auth, functions, realtime, and
						SSR. Free to start.
					</p>
					<div className="mt-8 flex flex-col items-stretch justify-center gap-2.5 sm:flex-row sm:items-center sm:gap-3">
						<Button asChild variant="primary" size="lg">
							<Link href={ctaUrl(signedIn)}>
								{signedIn ? "Open dashboard →" : "Create your account →"}
							</Link>
						</Button>
						<Button asChild variant="ghost" size="lg">
							<a href="https://docs.pylonsync.com">Read the docs</a>
						</Button>
					</div>
				</div>
			</section>
		</MarketingShell>
	);
}

// Vertical tabs: section titles on the left, the selected section's artifact
// and copy on the right. The artifact list is built only from what a product
// genuinely has — its running visuals, its code sample, its highlights — so no
// panel is filled with invented capabilities. Products with fewer artifacts
// than sections cycle; the copy still differs per tab.
function ProductDeepDive({
	product,
	visuals,
}: {
	product: ReturnType<typeof getProduct> & {};
	visuals: { caption: string; Visual: () => React.ReactElement }[];
}) {
	const [active, setActive] = useState(0);
	if (!product) return null;

	type Artifact =
		| { kind: "visual"; caption: string; Visual: () => React.ReactElement }
		| { kind: "code" }
		| { kind: "highlights" };

	const artifacts: Artifact[] = [
		...visuals.map((v) => ({
			kind: "visual" as const,
			caption: v.caption,
			Visual: v.Visual,
		})),
		...(product.code ? [{ kind: "code" as const }] : []),
		{ kind: "highlights" as const },
	];

	const section = product.sections[active];
	const artifact = artifacts[active % artifacts.length];
	if (!section || !artifact) return null;

	return (
		<div className="mt-10 grid min-w-0 grid-cols-[minmax(0,1fr)] gap-8 lg:grid-cols-[minmax(0,300px)_minmax(0,1fr)] lg:gap-14">
			<div
				role="tablist"
				aria-orientation="vertical"
				aria-label="How it works"
				className="flex flex-col gap-1"
			>
				{product.sections.map((s, i) => (
					<button
						key={s.title}
						type="button"
						role="tab"
						id={`deepdive-tab-${i}`}
						aria-selected={i === active}
						aria-controls={`deepdive-panel-${i}`}
						onClick={() => setActive(i)}
						className={`flex items-baseline gap-3 rounded-[var(--radius-md)] px-3 py-3 text-left transition-colors focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-[var(--color-brand)] ${
							i === active
								? "bg-[var(--color-paper)] shadow-[var(--shadow-card)]"
								: "hover:bg-[var(--color-paper)]/60"
						}`}
					>
						<span
							className={`font-mono text-[11px] tabular-nums ${
								i === active
									? "text-[var(--color-brand)]"
									: "text-[var(--color-ink-4)]"
							}`}
						>
							{String(i + 1).padStart(2, "0")}
						</span>
						<span
							className={`text-[15px] font-semibold leading-snug tracking-tight ${
								i === active
									? "text-[var(--color-ink)]"
									: "text-[var(--color-ink-2)]"
							}`}
						>
							{s.title}
						</span>
					</button>
				))}
			</div>

			<div
				role="tabpanel"
				id={`deepdive-panel-${active}`}
				aria-labelledby={`deepdive-tab-${active}`}
				className="min-w-0"
			>
				{artifact.kind === "visual" && (
					<LiveVisual caption={artifact.caption}>
						<artifact.Visual />
					</LiveVisual>
				)}
				{artifact.kind === "code" && product.code && (
					<CodePanel filename={product.code.label} code={product.code.code} />
				)}
				{artifact.kind === "highlights" && (
					<ul className="overflow-hidden rounded-[var(--radius-xl)] border border-[var(--color-rule)] bg-[var(--color-paper)] shadow-[var(--shadow-card)]">
						{product.highlights.map((h) => (
							<li
								key={h}
								className="flex items-start gap-3 border-b border-[var(--color-rule)] px-5 py-3.5 text-[13.5px] leading-[1.55] text-[var(--color-ink-2)] last:border-b-0"
							>
								<Check className="mt-0.5 size-3.5 shrink-0 text-[var(--color-brand)]" />
								{h}
							</li>
						))}
					</ul>
				)}
				<p className="mt-5 max-w-[62ch] text-[14.5px] leading-[1.65] text-[var(--color-ink-2)]">
					{section.body}
				</p>
			</div>
		</div>
	);
}
