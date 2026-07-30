"use client";

import type { ReactNode } from "react";
import { MarketingNav } from "./marketing-nav";
import { SiteFooter } from "./site-footer";

// Chrome for every public marketing page: the mega-menu nav on top, the
// IA-linked footer on the bottom, the page body in between. The `dark` class
// + paper/ink token surface match the homepage so the whole site reads as one
// design system. `signedIn` is resolved by each route's server component from
// the shared SessionStore and threaded down so the nav CTA is correct in the
// first paint (no flash). Client component so the nav island hydrates.
export function MarketingShell({
	signedIn = false,
	children,
}: {
	signedIn?: boolean;
	children: ReactNode;
}) {
	return (
		<div className="min-h-screen bg-[var(--color-paper)] text-[var(--color-ink)]">
			<MarketingNav signedIn={signedIn} />
			<main>{children}</main>
			<SiteFooter />
		</div>
	);
}

// Shared section primitives so every marketing page lays out identically to
// the homepage. Kept here (next to the shell) so the product / solutions /
// comparison views all import from one place.

export function MarketingSection({
	id,
	className = "",
	children,
}: {
	id?: string;
	className?: string;
	children: ReactNode;
}) {
	return (
		<section
			id={id}
			className={`border-t border-[var(--color-rule)] ${className}`}
		>
			<div className="mx-auto max-w-[1100px] px-5 py-20 sm:px-8 sm:py-24">
				{children}
			</div>
		</section>
	);
}

export function Eyebrow({
	children,
	tone = "muted",
}: {
	children: ReactNode;
	tone?: "muted" | "cobalt";
}) {
	return (
		<div
			className={`font-mono text-[11px] uppercase tracking-[0.14em] ${
				tone === "cobalt"
					? "text-[var(--color-cobalt)]"
					: "text-[var(--color-ink-4)]"
			}`}
		>
			{children}
		</div>
	);
}
