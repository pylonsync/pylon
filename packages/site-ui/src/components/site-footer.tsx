import { Link } from "@pylonsync/react";
import { PylonMark } from "./brand";
import { UpdatesSignup } from "./updates-signup";
import { COMPARISONS_ENABLED } from "../lib/comparison-content";
import {
	COMPARISONS,
	DEVELOPERS,
	PRODUCT_GROUPS,
	SOLUTIONS,
	type NavLink,
} from "../lib/site-nav";

// Shared marketing-site footer. Reads the same IA source as the nav so the
// columns stay in lockstep with the mega-menu. Not a client component — it has
// no interactivity, so it renders fine in server pages too. The brand column
// carries the Dallas line (Eric builds in Dallas) + the live-status pip.

function FooterLink({ link }: { link: NavLink }) {
	const cls =
		"text-[13px] leading-[1.5] text-[var(--color-ink-3)] transition-colors hover:text-[var(--color-ink)]";
	return link.external ? (
		<a href={link.href} className={cls}>
			{link.label}
		</a>
	) : (
		<Link href={link.href} className={cls}>
			{link.label}
		</Link>
	);
}

function FooterCol({ title, links }: { title: string; links: NavLink[] }) {
	return (
		<div className="flex flex-col gap-3">
			<div className="font-mono text-[10.5px] uppercase tracking-[0.12em] text-[var(--color-ink-4)]">
				{title}
			</div>
			<div className="flex flex-col gap-2.5">
				{links.map((l) => (
					<FooterLink key={l.href} link={l} />
				))}
			</div>
		</div>
	);
}

export function SiteFooter() {
	// Product column: the Build primitives plus Cloud + Swift, flattened.
	const productLinks = PRODUCT_GROUPS.flatMap((g) => g.links);

	return (
		<footer className="border-t border-[var(--color-rule)] bg-[var(--color-paper)]">
			<div className="mx-auto max-w-[1280px] px-5 py-14 sm:px-8 sm:py-16">
				<div className="grid grid-cols-2 gap-x-6 gap-y-10 sm:grid-cols-3 lg:grid-cols-12 lg:gap-x-8">
					{/* Brand */}
					<div className="col-span-2 sm:col-span-3 lg:col-span-3">
						<Link
							href="/"
							className="inline-flex items-center gap-2 text-[var(--color-ink)]"
						>
							<PylonMark size={20} />
							<span className="text-[15px] font-semibold leading-none tracking-tight">
								Pylon <span className="font-normal text-[var(--color-ink-3)]">by Stack0</span>
							</span>
						</Link>
						<p className="mt-4 max-w-[28ch] text-[13px] leading-[1.55] text-[var(--color-ink-3)]">
							The open source full-stack framework for coding agents. Define the app in TypeScript and deploy it anywhere.
						</p>
						<div className="mt-5 flex items-center gap-4">
							<a
								href="https://github.com/pylonsync/pylon"
								className="text-[13px] text-[var(--color-ink-3)] transition-colors hover:text-[var(--color-ink)]"
							>
								GitHub
							</a>
							<a
								href="https://x.com/pylonsync"
								className="text-[13px] text-[var(--color-ink-3)] transition-colors hover:text-[var(--color-ink)]"
							>
								X / Twitter
							</a>
						</div>
						<UpdatesSignup />
					</div>

					{/* Columns */}
					{/* Comparisons are the only thing left in the fourth column now
					    that pricing lives on Stack0 Cloud. Hidden, the column isn't
					    rendered at all and the three above widen to keep the row at
					    12: 3 (brand) + 3 + 3 + 3, vs. 3 + 3 + 2 + 2 + 2 with it. */}
					<div className="lg:col-span-3">
						<FooterCol title="Product" links={productLinks} />
					</div>
					<div className={COMPARISONS_ENABLED ? "lg:col-span-2" : "lg:col-span-3"}>
						<FooterCol title="Solutions" links={SOLUTIONS} />
					</div>
					<div className={COMPARISONS_ENABLED ? "lg:col-span-2" : "lg:col-span-3"}>
						<FooterCol title="Developers" links={DEVELOPERS} />
					</div>
					{COMPARISONS_ENABLED && (
						<div className="lg:col-span-2">
							<FooterCol title="Compare" links={COMPARISONS} />
						</div>
					)}
				</div>

				{/* Bottom row */}
				<div className="mt-14 flex flex-col items-start gap-3 border-t border-[var(--color-rule)] pt-6 font-mono text-[11px] text-[var(--color-ink-4)] sm:flex-row sm:items-center sm:justify-between">
					<div className="flex flex-wrap items-center gap-x-4 gap-y-1.5">
						<span>© 2026 Pylon · Built in Dallas</span>
						{/* About + Contact are trust anchors: the pages a person — or an
						    agent answering for one — checks before depending on a
						    project. They belong on every page, not just in a sitemap. */}
						<Link
							href="/about"
							className="transition-colors hover:text-[var(--color-ink)]"
						>
							About
						</Link>
						<Link
							href="/contact"
							className="transition-colors hover:text-[var(--color-ink)]"
						>
							Contact
						</Link>
						<Link
							href="/privacy"
							className="transition-colors hover:text-[var(--color-ink)]"
						>
							Privacy
						</Link>
						<Link
							href="/terms"
							className="transition-colors hover:text-[var(--color-ink)]"
						>
							Terms
						</Link>
					</div>
					{/*
					  status.pylonsync.com was retired in the brand split and now
					  answers nothing, so this footer link — which renders on every
					  page of both sites — was dead. The status page itself lives
					  with the product.
					*/}
					<a
						href="https://www.usesmallware.com/status"
						className="inline-flex items-center gap-2 transition-colors hover:text-[var(--color-ink)]"
					>
						<span className="block h-1.5 w-1.5 rounded-full bg-[var(--color-status-live)] shadow-[0_0_0_3px_var(--color-status-live-soft)]" />
						All systems operational
					</a>
				</div>
			</div>
		</footer>
	);
}
