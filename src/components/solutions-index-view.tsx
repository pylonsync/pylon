"use client";

import { Link } from "@pylonsync/react";
import { Button } from "./ui/button";
import { MarketingShell } from "./marketing-shell";
import { SOLUTIONS } from "../lib/site-nav";
import { ctaUrl } from "../lib/account-urls";

// /solutions index — the four use-case verticals as cards.
export function SolutionsIndexView({ signedIn = false }: { signedIn?: boolean }) {
	return (
		<MarketingShell signedIn={signedIn}>
			<header className="border-b border-[var(--color-rule)]">
				<div className="mx-auto max-w-[1280px] px-5 pb-14 pt-20 sm:px-8 sm:pb-16 sm:pt-28">
					<div className="font-mono text-[11px] uppercase tracking-[0.14em] text-[var(--color-cobalt)]">
						Solutions
					</div>
					<h1 className="mt-4 max-w-[20ch] text-[clamp(34px,5vw,60px)] font-semibold leading-[1.02] tracking-[-0.04em] text-[var(--color-ink)]">
						What are you building?
					</h1>
					<p className="mt-6 max-w-[58ch] text-[16px] leading-[1.6] text-[var(--color-ink-2)] sm:text-[17px]">
						The same primitives compose into very different products. Here&apos;s
						how teams put Pylon to work.
					</p>
				</div>
			</header>

			<div className="mx-auto max-w-[1280px] px-5 py-16 sm:px-8 sm:py-20">
				<div className="grid gap-4 sm:grid-cols-2">
					{SOLUTIONS.map((s) => {
						const Icon = s.icon;
						return (
							<Link
								key={s.href}
								href={s.href}
								className="group flex flex-col rounded-[var(--radius-xl)] border border-[var(--color-rule)] shadow-[var(--shadow-card)] transition-[filter,box-shadow] duration-300 ease-[var(--ease-out-quart)] hover:shadow-[var(--shadow-card-hover)] focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-[var(--color-cobalt)] focus-visible:ring-offset-2 [@media(hover:hover)]:grayscale [@media(hover:hover)]:focus-visible:grayscale-0 [@media(hover:hover)]:hover:grayscale-0 bg-[var(--color-paper)] p-7"
							>
								{Icon && (
									<span className="mb-4 flex size-10 items-center justify-center rounded-[var(--radius-md)] border border-[var(--color-rule)] bg-[var(--color-paper)] text-[var(--color-cobalt)]">
										<Icon className="size-5" />
									</span>
								)}
								<div className="text-[18px] font-semibold tracking-tight text-[var(--color-ink)]">
									{s.label}
								</div>
								<p className="mt-2.5 flex-1 text-[14px] leading-[1.55] text-[var(--color-ink-3)]">
									{s.desc}
								</p>
								<span className="mt-6 text-[13px] text-[var(--color-cobalt)]">
									See how →
								</span>
							</Link>
						);
					})}
				</div>
			</div>

			<section className="border-t border-[var(--color-rule)]">
				<div className="mx-auto max-w-[760px] px-5 py-20 text-center sm:px-8 sm:py-24">
					<h2 className="mx-auto max-w-[16ch] text-[clamp(28px,4vw,44px)] font-semibold leading-[1.08] tracking-[-0.035em] text-[var(--color-ink)]">
						One framework for all of it.
					</h2>
					<div className="mt-8 flex flex-col items-stretch justify-center gap-2.5 sm:flex-row sm:items-center sm:gap-3">
						<Button asChild variant="primary" size="lg">
							<Link href={ctaUrl(signedIn)}>
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
