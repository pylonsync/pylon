"use client";

import { Link } from "@pylonsync/react";
import { ArrowUpRight } from "lucide-react";
import { Button } from "./ui/button";
import { MarketingShell } from "./marketing-shell";
import { type Example, GROUPS, LIVE_DEMOS } from "../lib/examples-content";

// Example apps. The templates are real create-pylon templates (run the create
// command and pick one); the marketplace is a live, deployed demo. We don't
// link demos we can't stand behind — templates point at the scaffolder, the
// marketplace points at its live URL. Keep this list in sync with the
// TEMPLATE_REGISTRY in pylon's packages/create-pylon/bin/create-pylon.js.

function TemplateCard({ ex }: { ex: Example }) {
	return (
		<div className="flex flex-col rounded-[var(--radius-lg)] border border-[var(--color-rule)] bg-[var(--color-paper-1)] p-6">
			<div className="text-[16px] font-semibold tracking-tight text-[var(--color-ink)]">
				{ex.name}
			</div>
			<p className="mt-2 flex-1 text-[13.5px] leading-[1.55] text-[var(--color-ink-3)]">
				{ex.blurb}
			</p>
			<div className="mt-4 flex flex-wrap gap-1.5">
				{ex.shows.map((tag) => (
					<span
						key={tag}
						className="rounded-full border border-[var(--color-rule)] bg-[var(--color-paper)] px-2 py-0.5 font-mono text-[10.5px] text-[var(--color-ink-4)]"
					>
						{tag}
					</span>
				))}
			</div>
			{ex.template && (
				<div className="mt-5 rounded-[var(--radius-sm)] border border-[var(--color-rule)] bg-[var(--color-paper)] px-3 py-2 font-mono text-[11.5px] text-[var(--color-ink-2)]">
					template:{" "}
					<span className="text-[var(--color-cobalt)]">{ex.template}</span>
				</div>
			)}
		</div>
	);
}

// A live, deployed demo — the whole card links out to the running app.
function LiveDemoCard({ ex }: { ex: Example }) {
	return (
		<a
			href={ex.live}
			className="group flex flex-col rounded-[var(--radius-xl)] border border-[var(--color-rule)] shadow-[var(--shadow-card)] transition-[filter,box-shadow] duration-300 ease-[var(--ease-out-quart)] hover:shadow-[var(--shadow-card-hover)] focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-[var(--color-cobalt)] focus-visible:ring-offset-2 [@media(hover:hover)]:grayscale [@media(hover:hover)]:focus-visible:grayscale-0 [@media(hover:hover)]:hover:grayscale-0 bg-[var(--color-paper)] p-6"
		>
			<div className="flex items-center gap-2">
				<span className="inline-flex items-center gap-1.5 rounded-full bg-[var(--color-status-live)]/12 px-2.5 py-0.5 font-mono text-[10.5px] uppercase tracking-[0.08em] text-[var(--color-status-live)]">
					<span className="block h-1.5 w-1.5 rounded-full bg-[var(--color-status-live)]" />
					Live
				</span>
				<h3 className="text-[16px] font-semibold tracking-tight text-[var(--color-ink)]">
					{ex.name}
				</h3>
			</div>
			<p className="mt-2 flex-1 text-[13.5px] leading-[1.55] text-[var(--color-ink-3)]">
				{ex.blurb}
			</p>
			<div className="mt-4 flex flex-wrap gap-1.5">
				{ex.shows.map((tag) => (
					<span
						key={tag}
						className="rounded-full border border-[var(--color-rule)] bg-[var(--color-paper)] px-2 py-0.5 font-mono text-[10.5px] text-[var(--color-ink-4)]"
					>
						{tag}
					</span>
				))}
			</div>
			<div className="mt-4 flex items-center justify-between">
				{ex.template ? (
					<span className="rounded-[var(--radius-sm)] border border-[var(--color-rule)] bg-[var(--color-paper)] px-2.5 py-1 font-mono text-[11px] text-[var(--color-ink-2)]">
						template:{" "}
						<span className="text-[var(--color-cobalt)]">{ex.template}</span>
					</span>
				) : (
					<span />
				)}
				<span className="inline-flex shrink-0 items-center gap-1.5 text-[12.5px] font-medium text-[var(--color-cobalt)]">
					Open
					<ArrowUpRight className="size-3.5" />
				</span>
			</div>
		</a>
	);
}

export function ExamplesView({ signedIn = false }: { signedIn?: boolean }) {
	return (
		<MarketingShell signedIn={signedIn}>
			<header className="border-b border-[var(--color-rule)]">
				<div className="mx-auto max-w-[1100px] px-5 pb-14 pt-20 sm:px-8 sm:pb-16 sm:pt-28">
					<div className="font-mono text-[11px] uppercase tracking-[0.14em] text-[var(--color-cobalt)]">
						Examples
					</div>
					<h1 className="mt-4 max-w-[20ch] text-[clamp(34px,5vw,60px)] font-semibold leading-[1.02] tracking-[-0.04em] text-[var(--color-ink)]">
						Production-shaped starting points.
					</h1>
					<p className="mt-6 max-w-[60ch] text-[16px] leading-[1.6] text-[var(--color-ink-2)] sm:text-[17px]">
						Scaffold any of these in seconds and read the schema, policies,
						functions, and React client working together.
					</p>
					<div className="mt-7 inline-flex items-center gap-3 rounded-[var(--radius-md)] border border-[var(--color-rule)] bg-[var(--color-paper-1)] px-4 py-2.5 font-mono text-[13.5px] text-[var(--color-ink)]">
						<span className="text-[var(--color-cobalt)]">$</span>
						npm create @pylonsync/pylon@latest
					</div>
				</div>
			</header>

			{/* Live demos */}
			<section className="border-b border-[var(--color-rule)]">
				<div className="mx-auto max-w-[1100px] px-5 py-14 sm:px-8 sm:py-16">
					<div className="font-mono text-[11px] uppercase tracking-[0.14em] text-[var(--color-ink-4)]">
						Live demos
					</div>
					<p className="mt-2 max-w-[60ch] text-[13.5px] leading-[1.55] text-[var(--color-ink-3)]">
						Real Pylon apps deployed on Smallware. Open any of them without signing up.
					</p>
					<div className="mt-6 grid gap-4 sm:grid-cols-2">
						{LIVE_DEMOS.map((ex) => (
							<LiveDemoCard key={ex.name} ex={ex} />
						))}
					</div>
				</div>
			</section>

			{/* Templates, grouped */}
			<div className="mx-auto max-w-[1100px] px-5 py-16 sm:px-8 sm:py-20">
				{GROUPS.map((group, i) => (
					<div key={group.label} className={i > 0 ? "mt-16" : ""}>
						<div className="font-mono text-[11px] uppercase tracking-[0.14em] text-[var(--color-ink-4)]">
							{group.label}
						</div>
						<p className="mt-2 max-w-[60ch] text-[13.5px] leading-[1.55] text-[var(--color-ink-3)]">
							{group.blurb}
						</p>
						<div className="mt-6 grid gap-4 sm:grid-cols-2 lg:grid-cols-3">
							{group.items.map((ex) => (
								<TemplateCard key={ex.name} ex={ex} />
							))}
						</div>
					</div>
				))}
			</div>

			<section className="border-t border-[var(--color-rule)]">
				<div className="mx-auto max-w-[760px] px-5 py-20 text-center sm:px-8 sm:py-24">
					<h2 className="mx-auto max-w-[18ch] text-[clamp(28px,4vw,44px)] font-semibold leading-[1.08] tracking-[-0.035em] text-[var(--color-ink)]">
						Read the source.
					</h2>
					<p className="mx-auto mt-4 max-w-[44ch] text-[15px] leading-[1.55] text-[var(--color-ink-2)] sm:text-[16px]">
						Every template and the framework itself are open source on GitHub.
					</p>
					<div className="mt-8 flex flex-col items-stretch justify-center gap-2.5 sm:flex-row sm:items-center sm:gap-3">
						<Button asChild variant="primary" size="lg">
							<a href="https://github.com/pylonsync/pylon">Browse on GitHub →</a>
						</Button>
						<Button asChild variant="ghost" size="lg">
							<Link href="/skill">Set up the Claude skill</Link>
						</Button>
					</div>
				</div>
			</section>
		</MarketingShell>
	);
}
