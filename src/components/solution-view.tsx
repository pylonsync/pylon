"use client";

import { Link } from "@pylonsync/react";
import { Button } from "./ui/button";
import { MarketingShell } from "./marketing-shell";
import { getSolution } from "../lib/solutions-content";
import { getProduct } from "../lib/product-content";

// Detail view for a single /solutions/<slug> use-case page. Use-case framing:
// the problem, the capabilities that solve it, the primitives that power each.
export function SolutionView({
	slug,
	signedIn = false,
}: {
	slug: string;
	signedIn?: boolean;
}) {
	const s = getSolution(slug);
	if (!s) {
		return (
			<MarketingShell signedIn={signedIn}>
				<div className="mx-auto max-w-[640px] px-5 py-32 text-center">
					<h1 className="text-[28px] font-semibold tracking-tight">
						Not found
					</h1>
					<Button asChild variant="primary" size="lg" className="mt-8">
						<Link href="/solutions">Browse solutions →</Link>
					</Button>
				</div>
			</MarketingShell>
		);
	}

	const builtWith = s.primitives.map(getProduct).filter(Boolean);

	return (
		<MarketingShell signedIn={signedIn}>
			{/* HERO */}
			<header className="border-b border-[var(--color-rule)]">
				<div className="mx-auto max-w-[1280px] px-5 pb-16 pt-16 sm:px-8 sm:pb-20 sm:pt-24">
					<nav className="flex items-center gap-2 font-mono text-[11.5px] text-[var(--color-ink-4)]">
						<Link href="/solutions" className="hover:text-[var(--color-ink-2)]">
							Solutions
						</Link>
						<span>›</span>
						<span className="text-[var(--color-ink-2)]">{s.navLabel}</span>
					</nav>
					<div className="mt-8 max-w-[760px]">
						<div className="font-mono text-[11px] uppercase tracking-[0.14em] text-[var(--color-cobalt)]">
							{s.category}
						</div>
						<h1 className="mt-4 max-w-[18ch] text-[clamp(34px,5vw,58px)] font-semibold leading-[1.02] tracking-[-0.04em] text-[var(--color-ink)]">
							{s.title}
						</h1>
						<p className="mt-6 text-[16px] leading-[1.6] text-[var(--color-ink-2)] sm:text-[18px]">
							{s.tagline}
						</p>
						<div className="mt-8 flex flex-col items-stretch gap-2.5 sm:flex-row sm:items-center sm:gap-3">
							<Button asChild variant="primary" size="lg">
								<Link href={signedIn ? "/dashboard" : "/signup"}>
									{signedIn ? "Open dashboard →" : "Create your account →"}
								</Link>
							</Button>
							<Button asChild variant="ghost" size="lg">
								<a href="https://docs.pylonsync.com">Read the docs</a>
							</Button>
						</div>
					</div>
				</div>
			</header>

			{/* THE PROBLEM */}
			<section className="border-b border-[var(--color-rule)] bg-[var(--color-paper-1)]">
				<div className="mx-auto max-w-[1280px] px-5 py-14 sm:px-8 sm:py-16">
					<div className="font-mono text-[11px] uppercase tracking-[0.14em] text-[var(--color-ink-4)]">
						The problem
					</div>
					<p className="mt-5 max-w-[68ch] text-[17px] leading-[1.6] text-[var(--color-ink-2)] sm:text-[19px]">
						{s.problem}
					</p>
				</div>
			</section>

			{/* CAPABILITIES */}
			<section>
				<div className="mx-auto max-w-[1280px] px-5 py-16 sm:px-8 sm:py-20">
					<div className="font-mono text-[11px] uppercase tracking-[0.14em] text-[var(--color-cobalt)]">
						How Pylon solves it
					</div>
					<div className="mt-8 flex flex-col gap-px overflow-hidden rounded-[var(--radius-lg)] border border-[var(--color-rule)] bg-[var(--color-rule)]">
						{s.capabilities.map((c, i) => (
							<div
								key={c.title}
								className="grid gap-x-8 gap-y-3 bg-[var(--color-paper)] p-7 sm:p-8 lg:grid-cols-[1fr_minmax(0,420px)]"
							>
								<div>
									<div className="flex items-center gap-3">
										<span className="font-mono text-[12px] tabular-nums text-[var(--color-ink-4)]">
											{String(i + 1).padStart(2, "0")}
										</span>
										<h2 className="text-[19px] font-semibold tracking-tight text-[var(--color-ink)]">
											{c.title}
										</h2>
									</div>
									<p className="mt-3 text-[14.5px] leading-[1.6] text-[var(--color-ink-2)]">
										{c.body}
									</p>
								</div>
								<div className="flex flex-wrap content-start gap-2 lg:justify-end">
									{c.primitives.map((slug) => {
										const p = getProduct(slug);
										if (!p) return null;
										return (
											<Link
												key={slug}
												href={`/product/${slug}`}
												className="inline-flex items-center rounded-full border border-[var(--color-rule)] bg-[var(--color-paper-1)] px-3 py-1 font-mono text-[11.5px] text-[var(--color-ink-2)] transition-colors hover:border-[var(--color-cobalt)]/50 hover:text-[var(--color-ink)]"
											>
												{p.navLabel}
											</Link>
										);
									})}
								</div>
							</div>
						))}
					</div>
				</div>
			</section>

			{/* BUILT WITH */}
			{builtWith.length > 0 && (
				<section className="border-t border-[var(--color-rule)] bg-[var(--color-paper-1)]">
					<div className="mx-auto max-w-[1280px] px-5 py-14 sm:px-8 sm:py-16">
						<div className="font-mono text-[11px] uppercase tracking-[0.14em] text-[var(--color-ink-4)]">
							Built with
						</div>
						<div className="mt-6 grid gap-4 sm:grid-cols-3">
							{builtWith.map(
								(p) =>
									p && (
										<Link
											key={p.slug}
											href={`/product/${p.slug}`}
											className="group flex flex-col rounded-[var(--radius-xl)] border border-[var(--color-rule)] shadow-[var(--shadow-card)] transition-[filter,box-shadow] duration-300 ease-[var(--ease-out-quart)] hover:shadow-[var(--shadow-card-hover)] focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-[var(--color-cobalt)] focus-visible:ring-offset-2 [@media(hover:hover)]:grayscale [@media(hover:hover)]:focus-visible:grayscale-0 [@media(hover:hover)]:hover:grayscale-0 bg-[var(--color-paper)] p-5"
										>
											<div className="text-[15px] font-semibold tracking-tight text-[var(--color-ink)]">
												{p.navLabel}
											</div>
											<div className="mt-1.5 text-[13px] leading-[1.5] text-[var(--color-ink-3)]">
												{p.title}
											</div>
											<span className="mt-4 text-[12.5px] text-[var(--color-cobalt)]">
												Learn more →
											</span>
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
						Ship it on Pylon.
					</h2>
					<p className="mx-auto mt-4 max-w-[44ch] text-[15px] leading-[1.55] text-[var(--color-ink-2)] sm:text-[16px]">
						Scaffold an app in seconds, deploy free on Cloud, scale when you
						need to.
					</p>
					<div className="mt-8 flex flex-col items-stretch justify-center gap-2.5 sm:flex-row sm:items-center sm:gap-3">
						<Button asChild variant="primary" size="lg">
							<Link href={signedIn ? "/dashboard" : "/signup"}>
								{signedIn ? "Open dashboard →" : "Create your account →"}
							</Link>
						</Button>
						<Button asChild variant="ghost" size="lg">
							<Link href="/product">Explore the product</Link>
						</Button>
					</div>
				</div>
			</section>
		</MarketingShell>
	);
}
