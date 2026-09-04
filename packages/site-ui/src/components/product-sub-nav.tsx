"use client";

import { Link } from "@pylonsync/react";
import { PRODUCT_GROUPS } from "../lib/site-nav";

// Secondary nav for the /product/* pages. Without it the only way from one
// primitive to another is the three "Keep exploring" cards at the very bottom,
// so comparing two products means scrolling to the end of one page and hoping
// the other is one of the three cross-links.
//
// Twelve entries won't fit a row at every width, so the strip scrolls
// horizontally rather than wrapping into a second line that pushes the hero
// down. The scrollbar is hidden; the fades at each edge are the affordance.
export function ProductSubNav({ slug }: { slug?: string }) {
	const links = PRODUCT_GROUPS.flatMap((g) => g.links);
	return (
		<div className="sticky top-[60px] z-30 border-b border-[var(--color-rule)] bg-[var(--color-paper)]/85 backdrop-blur-md">
			<div className="relative mx-auto max-w-[1280px]">
				<div
					aria-hidden
					className="pointer-events-none absolute inset-y-0 left-0 z-10 w-8 bg-gradient-to-r from-[var(--color-paper)] to-transparent"
				/>
				<div
					aria-hidden
					className="pointer-events-none absolute inset-y-0 right-0 z-10 w-8 bg-gradient-to-l from-[var(--color-paper)] to-transparent"
				/>
				<nav
					aria-label="Product"
					className="flex gap-1 overflow-x-auto px-5 py-2 sm:px-8 [-ms-overflow-style:none] [scrollbar-width:none] [&::-webkit-scrollbar]:hidden"
				>
					{links.map((l) => {
						const active = l.href === `/product/${slug}`;
						const Icon = l.icon;
						return (
							<Link
								key={l.href}
								href={l.href}
								aria-current={active ? "page" : undefined}
								className={`inline-flex shrink-0 items-center gap-1.5 rounded-[var(--radius-md)] px-2.5 py-1.5 text-[13px] tracking-tight transition-colors ${
									active
										? "bg-[var(--color-paper-2)] font-medium text-[var(--color-ink)]"
										: "text-[var(--color-ink-3)] hover:bg-[var(--color-paper-1)] hover:text-[var(--color-ink)]"
								}`}
							>
								{Icon && (
									<Icon
										className={`size-[15px] shrink-0 ${active ? "text-[var(--color-brand)]" : ""}`}
									/>
								)}
								{l.label}
							</Link>
						);
					})}
				</nav>
			</div>
		</div>
	);
}
