"use client";

import { Link } from "@pylonsync/react";
import { ArrowUpRight } from "lucide-react";
import { Button } from "./ui/button";
import { MarketingShell } from "./marketing-shell";
import { PRODUCT_GROUPS } from "../lib/site-nav";
import { getProduct } from "../lib/product-content";
import { ctaUrl } from "../lib/account-urls";

// /product index — every primitive as a card, grouped Build / Ship / Clients
// (the same grouping as the nav mega-menu). Card copy comes from the product
// content map so the index and the detail pages never drift.
export function ProductIndexView({ signedIn = false }: { signedIn?: boolean }) {
	return (
		<MarketingShell signedIn={signedIn}>
			<header className="border-b border-[var(--color-rule)]">
				<div className="mx-auto max-w-[1280px] px-5 pb-14 pt-20 sm:px-8 sm:pb-16 sm:pt-28">
					<div className="font-mono text-[11px] uppercase tracking-[0.14em] text-[var(--color-cobalt)]">
						Product
					</div>
					<h1 className="mt-4 max-w-[20ch] text-[clamp(34px,5vw,60px)] font-semibold leading-[1.02] tracking-[-0.04em] text-[var(--color-ink)]">
						One framework. Every primitive.
					</h1>
					<p className="mt-6 max-w-[58ch] text-[16px] leading-[1.6] text-[var(--color-ink-2)] sm:text-[17px]">
						Database, sync, auth, functions, realtime, storage, search, jobs,
						and SSR ship as one system. Pick a piece to go deeper.
					</p>
				</div>
			</header>

			<div className="mx-auto max-w-[1280px] px-5 py-16 sm:px-8 sm:py-20">
				{PRODUCT_GROUPS.map((group) => (
					<section key={group.title} className="mb-14 last:mb-0">
						<div className="mb-5 font-mono text-[11px] uppercase tracking-[0.14em] text-[var(--color-ink-4)]">
							{group.title}
						</div>
						<div className="grid gap-4 sm:grid-cols-2 lg:grid-cols-3">
							{group.links.map((link) => {
								const slug = link.href.replace("/product/", "");
								const p = getProduct(slug);
								const Icon = link.icon;
								return (
									<Link
										key={link.href}
										href={link.href}
										title={p?.title}
										className="group flex flex-col rounded-[var(--radius-xl)] border border-[var(--color-rule)] bg-[var(--color-paper)] p-6 shadow-[var(--shadow-card)] transition-[filter,box-shadow] duration-300 ease-[var(--ease-out-quart)] hover:shadow-[var(--shadow-card-hover)] focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-[var(--color-cobalt)] focus-visible:ring-offset-2 focus-visible:ring-offset-[var(--color-paper)] [@media(hover:hover)]:grayscale [@media(hover:hover)]:focus-visible:grayscale-0 [@media(hover:hover)]:hover:grayscale-0"
									>
										<div className="mb-4 flex items-center gap-2">
											{Icon && (
												<span className="flex size-9 items-center justify-center rounded-[var(--radius-md)] border border-[var(--color-rule)] bg-[var(--color-paper-1)] text-[var(--color-cobalt)]">
													<Icon className="size-[18px]" />
												</span>
											)}
											<ArrowUpRight className="ml-auto size-3.5 shrink-0 text-[var(--color-ink-4)] opacity-0 transition-opacity duration-200 group-focus-visible:opacity-100 group-hover:opacity-100" />
										</div>
										<div className="text-[16px] font-semibold tracking-tight text-[var(--color-ink)]">
											{link.label}
										</div>
										<p className="mt-2 flex-1 text-[13.5px] leading-[1.55] text-[var(--color-ink-3)]">
											{link.desc}
										</p>
									</Link>
								);
							})}
						</div>
					</section>
				))}
			</div>

			<section className="border-t border-[var(--color-rule)]">
				<div className="mx-auto max-w-[760px] px-5 py-20 text-center sm:px-8 sm:py-24">
					<h2 className="mx-auto max-w-[16ch] text-[clamp(28px,4vw,44px)] font-semibold leading-[1.08] tracking-[-0.035em] text-[var(--color-ink)]">
						Start with one command.
					</h2>
					<p className="mx-auto mt-4 max-w-[44ch] text-[15px] leading-[1.55] text-[var(--color-ink-2)] sm:text-[16px]">
						Scaffold a full-stack Pylon app and have it running locally in
						seconds.
					</p>
					<div className="mt-7 inline-flex items-center gap-3 rounded-[var(--radius-md)] border border-[var(--color-rule)] bg-[var(--color-paper-1)] px-4 py-2.5 font-mono text-[13.5px] text-[var(--color-ink)]">
						<span className="text-[var(--color-cobalt)]">$</span>
						npm create @pylonsync/pylon@latest
					</div>
					<div className="mt-8">
						<Button asChild variant="primary" size="lg">
							<Link href={ctaUrl(signedIn)}>
								{signedIn ? "Open dashboard →" : "Create your account →"}
							</Link>
						</Button>
					</div>
				</div>
			</section>
		</MarketingShell>
	);
}
